//! [`MediaService`]: the media core as the app uses it. it owns the cache and the ffmpeg
//! tools, follows the media settings, and turns slow work into jobs.
//!
//! requests from the ui never wait on slow work. `thumbnail`, `filmstrip` and `waveform`
//! answer at once from the cache, or start a background job and answer `None`; the result
//! arrives later as a [`MediaEvent`]. asking again while a job is running does not start a
//! second one.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;
use vellum_core::events::{Event, EventBus};
use vellum_core::jobs::{JobPriority, JobSpec};
use vellum_core::settings::{AppSettings, MediaSettings, SettingsChanged};
use vellum_core::{JobContext, JobError, JobId, JobQueue};

use crate::cache::{CacheKey, CacheStats, MediaCache};
use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::probe_media;
use crate::ffmpeg::{
    FfmpegStatus, FfmpegTools, FrameIndex, HardwareSupport, Reporter, detect_hardware,
};
use crate::images::{DecodeOptions, probe_image};
use crate::info::MediaInfo;
use crate::sniff::{Format, sniff};
use crate::strip::strip_metadata;
use crate::thumbnail::{Filmstrip, encode_thumbnail, make_filmstrip, make_thumbnail};
use crate::waveform::{Peaks, WaveformView, make_peaks};

/// bump to throw away cached probe results when `MediaInfo` changes shape
const PROBE_VERSION: &str = "probe-1";
const HARDWARE_FILE: &str = "hardware.json";

/// a thumbnail on disk, or none when the file has no picture (audio without cover art)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Thumbnail {
    pub file: Option<String>,
    pub width: u32,
    pub height: u32,
}

/// what the settings window shows about the media core
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaStatus {
    pub ffmpeg: Option<FfmpegStatus>,
    /// why ffmpeg is missing, in words for the user
    pub ffmpeg_error: Option<String>,
    pub hardware: Option<HardwareSupport>,
    pub checking_hardware: bool,
    pub cache: Option<CacheStats>,
    pub cache_error: Option<String>,
}

/// sent when something the ui asked for is ready, failed, or the status changed
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum MediaEvent {
    ThumbnailReady {
        path: String,
        size: u32,
        thumbnail: Thumbnail,
    },
    FilmstripReady {
        path: String,
        filmstrip: Filmstrip,
    },
    /// ask for the waveform again to get it, now from the cache
    WaveformReady {
        path: String,
    },
    /// `what` is "thumbnail", "filmstrip" or "waveform"
    Failed {
        path: String,
        what: String,
        error: String,
    },
    StatusChanged {
        status: MediaStatus,
    },
}

impl Event for MediaEvent {
    const NAME: &'static str = "media:changed";
}

#[derive(Clone)]
pub struct MediaService {
    inner: Arc<Inner>,
}

struct Inner {
    jobs: JobQueue,
    bus: EventBus,
    cache_dir: PathBuf,
    cache: Option<MediaCache>,
    cache_error: Option<String>,
    tools: RwLock<Result<FfmpegTools, String>>,
    hardware: RwLock<Option<HardwareSupport>>,
    checking_hardware: Mutex<bool>,
    settings: RwLock<MediaSettings>,
    /// requests with a job running, so asking twice starts one job
    pending: Mutex<HashSet<String>>,
}

/// takes a request off the pending list however its job ends, even by panicking
struct PendingGuard {
    inner: Arc<Inner>,
    request: String,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.inner.pending.lock().remove(&self.request);
    }
}

impl MediaService {
    /// opens the cache in `cache_dir`, finds ffmpeg and starts checking hardware encoders
    /// in the background. nothing here stops the app starting: a missing ffmpeg or a broken
    /// cache shows up in [`MediaStatus`] instead.
    pub fn start(
        cache_dir: &Path,
        settings: &MediaSettings,
        jobs: JobQueue,
        bus: EventBus,
    ) -> Self {
        let limit = u64::from(settings.cache_limit_mb) * 1024 * 1024;
        let (cache, cache_error) = match MediaCache::open(cache_dir, limit) {
            Ok(cache) => (Some(cache), None),
            Err(err) => {
                tracing::error!(%err, "the media cache is off");
                (
                    None,
                    Some(format!("{err}. Thumbnails will be made again each time.")),
                )
            }
        };
        let tools = locate(settings);
        let service = Self {
            inner: Arc::new(Inner {
                jobs,
                bus,
                cache_dir: cache_dir.to_owned(),
                cache,
                cache_error,
                tools: RwLock::new(tools),
                hardware: RwLock::new(None),
                checking_hardware: Mutex::new(false),
                settings: RwLock::new(settings.clone()),
                pending: Mutex::new(HashSet::new()),
            }),
        };
        service.check_hardware(false);
        service
    }

    /// keeps following the app settings on a background thread for as long as the bus lives
    pub fn follow_settings(&self) {
        let changes = self.inner.bus.subscribe::<SettingsChanged<AppSettings>>();
        let service = self.clone();
        let spawned = thread::Builder::new()
            .name("vellum-media-settings".into())
            .spawn(move || {
                while let Some(change) = changes.recv() {
                    service.apply_settings(&change.settings.media);
                }
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "media settings changes will need a restart");
        }
    }

    pub fn apply_settings(&self, next: &MediaSettings) {
        let previous = self.inner.settings.read().clone();
        if previous == *next {
            return;
        }
        *self.inner.settings.write() = next.clone();
        if previous.cache_limit_mb != next.cache_limit_mb
            && let Some(cache) = &self.inner.cache
            && let Err(err) = cache.set_limit(u64::from(next.cache_limit_mb) * 1024 * 1024)
        {
            tracing::warn!(%err, "could not apply the new cache limit");
        }
        if previous.ffmpeg_path != next.ffmpeg_path {
            *self.inner.tools.write() = locate(next);
            *self.inner.hardware.write() = None;
            self.check_hardware(false);
        }
        self.emit_status();
    }

    pub fn tools(&self) -> MediaResult<FfmpegTools> {
        self.inner
            .tools
            .read()
            .clone()
            .map_err(|_| MediaError::FfmpegMissing)
    }

    fn tools_opt(&self) -> Option<FfmpegTools> {
        self.inner.tools.read().as_ref().ok().cloned()
    }

    pub fn cache(&self) -> Option<&MediaCache> {
        self.inner.cache.as_ref()
    }

    /// the folder the webview may load thumbnails from
    pub fn blob_dir(&self) -> Option<PathBuf> {
        self.inner.cache.as_ref().map(MediaCache::blob_dir)
    }

    pub fn hardware(&self) -> Option<HardwareSupport> {
        self.inner.hardware.read().clone()
    }

    pub fn status(&self) -> MediaStatus {
        let tools = self.inner.tools.read();
        MediaStatus {
            ffmpeg: tools.as_ref().ok().map(FfmpegStatus::from),
            ffmpeg_error: tools.as_ref().err().cloned(),
            hardware: self.hardware(),
            checking_hardware: *self.inner.checking_hardware.lock(),
            cache: self.inner.cache.as_ref().and_then(|c| c.stats().ok()),
            cache_error: self.inner.cache_error.clone(),
        }
    }

    fn emit_status(&self) {
        self.inner.bus.emit(MediaEvent::StatusChanged {
            status: self.status(),
        });
    }

    /// empties the cache and says how it looks afterwards
    pub fn clean_cache(&self) -> MediaResult<CacheStats> {
        let cache = self
            .inner
            .cache
            .as_ref()
            .ok_or_else(|| MediaError::Cache("the cache is off".into()))?;
        let stats = cache.clear()?;
        self.emit_status();
        Ok(stats)
    }

    /// tries the hardware encoders again, eg after a driver update
    pub fn recheck_hardware(&self) {
        self.check_hardware(true);
    }

    fn check_hardware(&self, force: bool) {
        let Some(tools) = self.tools_opt() else {
            return;
        };
        let saved = self.inner.cache_dir.join(HARDWARE_FILE);
        if !force
            && let Some(known) = read_hardware(&saved)
            && known.ffmpeg_version == tools.version
        {
            *self.inner.hardware.write() = Some(known);
            return;
        }
        {
            let mut checking = self.inner.checking_hardware.lock();
            if *checking {
                return;
            }
            *checking = true;
        }
        self.emit_status();
        let service = self.clone();
        let spawned = thread::Builder::new()
            .name("vellum-hardware-check".into())
            .spawn(move || {
                match detect_hardware(&tools) {
                    Ok(support) => {
                        if let Ok(text) = serde_json::to_string_pretty(&support) {
                            let _ = fs::write(&saved, text);
                        }
                        *service.inner.hardware.write() = Some(support);
                    }
                    Err(err) => tracing::warn!(%err, "could not check hardware encoders"),
                }
                *service.inner.checking_hardware.lock() = false;
                service.emit_status();
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not check hardware encoders");
            *self.inner.checking_hardware.lock() = false;
        }
    }

    /// the format and description of a file, from the cache when it has not changed
    pub fn probe(&self, path: &Path) -> MediaResult<MediaInfo> {
        Ok(self.describe(path)?.1)
    }

    fn describe(&self, path: &Path) -> MediaResult<(Format, MediaInfo)> {
        let format = sniff(path)?;
        let key = CacheKey::for_file(path, "probe", PROBE_VERSION)?;
        if let Some(info) = self.inner.cache.as_ref().and_then(|c| c.get_meta(&key)) {
            return Ok((format, info));
        }
        let tools = self.tools_opt();
        let info = if format.is_image() {
            probe_image(path, format, tools.as_ref())?
        } else {
            probe_media(tools.as_ref().ok_or(MediaError::FfmpegMissing)?, path)?
        };
        if let Some(cache) = &self.inner.cache
            && let Err(err) = cache.put_meta(&key, &info)
        {
            tracing::warn!(%err, "could not cache a probe");
        }
        Ok((format, info))
    }

    /// decoding options that follow the memory setting
    pub fn decode_options(&self) -> DecodeOptions {
        DecodeOptions {
            max_bytes: u64::from(self.inner.settings.read().max_image_mb) * 1024 * 1024,
            ..DecodeOptions::default()
        }
    }

    /// a thumbnail that fits a `size` square, or none while one is being made
    pub fn thumbnail(&self, path: &Path, size: u32) -> MediaResult<Option<Thumbnail>> {
        let size = size.clamp(16, 1024);
        let key = CacheKey::for_file(path, "thumb", &size.to_string())?;
        if let Some(found) = self
            .inner
            .cache
            .as_ref()
            .map(|c| c.get(&key))
            .transpose()?
            .flatten()
            && let Some(thumb) = found.meta_as::<Thumbnail>()
        {
            return Ok(Some(Thumbnail {
                file: found.file.map(|f| f.display().to_string()),
                ..thumb
            }));
        }
        let path = path.to_owned();
        let title = format!("Thumbnail for {}", file_name(&path));
        self.background(
            &format!("thumb:{size}:{}", key.path),
            title,
            move |service, _ctx| {
                let made = service.make_thumbnail(&path, size, &key);
                let display = path.display().to_string();
                match made {
                    Ok(thumbnail) => {
                        service.inner.bus.emit(MediaEvent::ThumbnailReady {
                            path: display,
                            size,
                            thumbnail,
                        });
                        Ok(())
                    }
                    Err(err) => {
                        service.failed(&display, "thumbnail", &err);
                        Err(err)
                    }
                }
            },
        );
        Ok(None)
    }

    fn make_thumbnail(&self, path: &Path, size: u32, key: &CacheKey) -> MediaResult<Thumbnail> {
        let (format, info) = self.describe(path)?;
        let tools = self.tools_opt();
        let image = make_thumbnail(path, format, &info, size, tools.as_ref())?;
        let Some(image) = image else {
            let none = Thumbnail {
                file: None,
                width: 0,
                height: 0,
            };
            // remember that there is nothing to show, so the file is not opened again
            if let Some(cache) = &self.inner.cache {
                cache.put_meta(key, &none)?;
            }
            return Ok(none);
        };
        let (bytes, ext) = encode_thumbnail(&image)?;
        let meta = Thumbnail {
            file: None,
            width: image.width(),
            height: image.height(),
        };
        let Some(cache) = &self.inner.cache else {
            return Err(MediaError::Cache(
                "the cache is off, so thumbnails cannot be kept".into(),
            ));
        };
        let meta_json =
            serde_json::to_string(&meta).map_err(|e| MediaError::Cache(e.to_string()))?;
        let entry = cache.put_file(key, ext, Some(meta_json), |target| {
            fs::write(target, &bytes).map_err(|err| MediaError::io(target, err))
        })?;
        Ok(Thumbnail {
            file: entry.file.map(|f| f.display().to_string()),
            ..meta
        })
    }

    /// `count` frames across a video, `height` pixels tall, or none while being made
    pub fn filmstrip(
        &self,
        path: &Path,
        count: u32,
        height: u32,
    ) -> MediaResult<Option<Filmstrip>> {
        let params = format!("{count}x{height}");
        let key = CacheKey::for_file(path, "filmstrip", &params)?;
        if let Some(found) = self
            .inner
            .cache
            .as_ref()
            .map(|c| c.get(&key))
            .transpose()?
            .flatten()
            && let (Some(strip), Some(file)) = (found.meta_as::<Filmstrip>(), found.file)
        {
            return Ok(Some(Filmstrip {
                file: file.display().to_string(),
                ..strip
            }));
        }
        let path = path.to_owned();
        let title = format!("Filmstrip for {}", file_name(&path));
        self.background(
            &format!("filmstrip:{params}:{}", key.path),
            title,
            move |service, ctx| {
                let display = path.display().to_string();
                match service.make_filmstrip(&path, count, height, &key, ctx) {
                    Ok(filmstrip) => {
                        service.inner.bus.emit(MediaEvent::FilmstripReady {
                            path: display,
                            filmstrip,
                        });
                        Ok(())
                    }
                    Err(err) => {
                        service.failed(&display, "filmstrip", &err);
                        Err(err)
                    }
                }
            },
        );
        Ok(None)
    }

    fn make_filmstrip(
        &self,
        path: &Path,
        count: u32,
        height: u32,
        key: &CacheKey,
        ctx: &JobContext,
    ) -> MediaResult<Filmstrip> {
        let tools = self.tools()?;
        let (_, info) = self.describe(path)?;
        let (strip, layout) = make_filmstrip(&tools, path, &info, count, height, ctx)?;
        let cache = self
            .inner
            .cache
            .as_ref()
            .ok_or_else(|| MediaError::Cache("the cache is off".into()))?;
        let meta = serde_json::to_string(&layout).map_err(|e| MediaError::Cache(e.to_string()))?;
        let entry = cache.put_file(key, "jpg", Some(meta), |target| {
            image::DynamicImage::ImageRgb8(strip)
                .save_with_format(target, image::ImageFormat::Jpeg)
                .map_err(|err| MediaError::decode(err.to_string()))
        })?;
        Ok(Filmstrip {
            file: entry
                .file
                .map(|f| f.display().to_string())
                .unwrap_or_default(),
            ..layout
        })
    }

    /// the waveform drawn `columns` wide, or none while it is being measured
    pub fn waveform(&self, path: &Path, columns: u32) -> MediaResult<Option<WaveformView>> {
        let key = CacheKey::for_file(path, "waveform", "")?;
        if let Some(found) = self
            .inner
            .cache
            .as_ref()
            .map(|c| c.get(&key))
            .transpose()?
            .flatten()
            && let (Some(mut peaks), Some(file)) = (found.meta_as::<Peaks>(), found.file)
        {
            let bytes = fs::read(&file).map_err(|err| MediaError::io(&file, err))?;
            peaks.set_bytes(&bytes);
            return Ok(Some(peaks.view(columns)));
        }
        let path = path.to_owned();
        let title = format!("Waveform for {}", file_name(&path));
        self.background(
            &format!("waveform:{}", key.path),
            title,
            move |service, ctx| {
                let display = path.display().to_string();
                match service.make_waveform(&path, &key, ctx) {
                    Ok(()) => {
                        service
                            .inner
                            .bus
                            .emit(MediaEvent::WaveformReady { path: display });
                        Ok(())
                    }
                    Err(err) => {
                        service.failed(&display, "waveform", &err);
                        Err(err)
                    }
                }
            },
        );
        Ok(None)
    }

    fn make_waveform(&self, path: &Path, key: &CacheKey, ctx: &JobContext) -> MediaResult<()> {
        let tools = self.tools()?;
        let (_, info) = self.describe(path)?;
        let peaks = make_peaks(&tools, path, &info, ctx)?;
        let cache = self
            .inner
            .cache
            .as_ref()
            .ok_or_else(|| MediaError::Cache("the cache is off".into()))?;
        let meta = serde_json::to_string(&peaks).map_err(|e| MediaError::Cache(e.to_string()))?;
        cache.put_file(key, "peaks", Some(meta), |target| {
            fs::write(target, peaks.to_bytes()).map_err(|err| MediaError::io(target, err))
        })?;
        Ok(())
    }

    /// the timestamps of every frame, for frame accurate seeking. building one reads the
    /// whole file's packet headers, so long videos should call this from a job.
    pub fn frame_index(&self, path: &Path, reporter: &dyn Reporter) -> MediaResult<FrameIndex> {
        let key = CacheKey::for_file(path, "frame-index", "1")?;
        if let Some(index) = self.inner.cache.as_ref().and_then(|c| c.get_meta(&key)) {
            return Ok(index);
        }
        let tools = self.tools()?;
        let (_, info) = self.describe(path)?;
        let video = info
            .video
            .as_ref()
            .ok_or_else(|| MediaError::Unsupported("This file has no video.".into()))?;
        let index = FrameIndex::build(
            &tools,
            path,
            Some(video.stream_index),
            video.frame_count,
            reporter,
        )?;
        if let Some(cache) = &self.inner.cache {
            cache.put_meta(&key, &index)?;
        }
        Ok(index)
    }

    /// starts a job that writes a copy of `path` without metadata. the job's result is
    /// `{ "path": "<the copy>" }`, so the notification can open it.
    pub fn strip_metadata(&self, path: &Path) -> MediaResult<JobId> {
        // reading the file first catches a missing or unreadable file before any job starts
        let (format, info) = self.describe(path)?;
        let service = self.clone();
        let path = path.to_owned();
        let spec = JobSpec::io(format!("Remove metadata from {}", file_name(&path)))
            .with_priority(JobPriority::High);
        Ok(self.inner.jobs.submit(spec, move |ctx: &JobContext| {
            ctx.set_message("Writing a clean copy");
            let tools = service.tools_opt();
            let out = strip_metadata(&path, format, tools.as_ref(), ctx, info.duration_secs)?;
            Ok(json!({ "path": out.display().to_string() }))
        }))
    }

    fn failed(&self, path: &str, what: &str, err: &MediaError) {
        if matches!(err, MediaError::Cancelled) {
            return;
        }
        tracing::debug!(path, what, %err, "media request failed");
        self.inner.bus.emit(MediaEvent::Failed {
            path: path.to_owned(),
            what: what.to_owned(),
            error: err.to_string(),
        });
    }

    /// runs `work` as a background job unless the same request is already running
    fn background(
        &self,
        request: &str,
        title: String,
        work: impl Fn(&MediaService, &JobContext) -> MediaResult<()> + Send + 'static,
    ) {
        if !self.inner.pending.lock().insert(request.to_owned()) {
            return;
        }
        let guard = Mutex::new(Some(PendingGuard {
            inner: self.inner.clone(),
            request: request.to_owned(),
        }));
        let service = self.clone();
        let spec = JobSpec::cpu(title)
            .with_priority(JobPriority::Low)
            .in_background();
        self.inner.jobs.submit(spec, move |ctx: &JobContext| {
            // dropped when this run ends, which clears the pending request
            let _guard = guard.lock().take();
            work(&service, ctx).map_err(JobError::from)?;
            Ok(serde_json::Value::Null)
        });
    }
}

fn locate(settings: &MediaSettings) -> Result<FfmpegTools, String> {
    let custom = Some(Path::new(&settings.ffmpeg_path)).filter(|p| !p.as_os_str().is_empty());
    FfmpegTools::locate(custom).map_err(|err| err.to_string())
}

fn read_hardware(path: &Path) -> Option<HardwareSupport> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use image::{Rgb, RgbImage};
    use vellum_core::jobs::JobLimits;

    use super::*;
    use crate::ffmpeg::{NoReporter, testing};

    fn service(dir: &Path) -> (MediaService, vellum_core::events::Subscription<MediaEvent>) {
        let bus = EventBus::new();
        let events = bus.subscribe::<MediaEvent>();
        let jobs = JobQueue::new(JobLimits::default(), bus.clone());
        let service = MediaService::start(&dir.join("cache"), &MediaSettings::default(), jobs, bus);
        (service, events)
    }

    fn next_ready(events: &vellum_core::events::Subscription<MediaEvent>) -> MediaEvent {
        loop {
            let event = events
                .recv_timeout(Duration::from_secs(30))
                .expect("no media event arrived");
            if !matches!(event, MediaEvent::StatusChanged { .. }) {
                return event;
            }
        }
    }

    #[test]
    fn the_ui_event_map_knows_media_events() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/src/lib/events.ts");
        let source = fs::read_to_string(path).unwrap();
        assert!(source.contains(&format!("\"{}\"", MediaEvent::NAME)));
    }

    #[test]
    fn thumbnails_are_made_once_then_served_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let (service, events) = service(dir.path());
        let photo = dir.path().join("photo.png");
        RgbImage::from_pixel(640, 480, Rgb([20, 120, 220]))
            .save(&photo)
            .unwrap();

        assert_eq!(service.thumbnail(&photo, 160).unwrap(), None);
        // a second ask while the job runs does not start another
        let _ = service.thumbnail(&photo, 160).unwrap();
        let MediaEvent::ThumbnailReady {
            thumbnail, size, ..
        } = next_ready(&events)
        else {
            panic!("expected a thumbnail");
        };
        assert_eq!((size, thumbnail.width, thumbnail.height), (160, 160, 120));
        let file = PathBuf::from(thumbnail.file.unwrap());
        assert!(file.starts_with(service.blob_dir().unwrap()));

        let cached = service.thumbnail(&photo, 160).unwrap().unwrap();
        assert_eq!(cached.file.as_deref(), Some(file.to_str().unwrap()));
        assert!(events.recv_timeout(Duration::from_millis(300)).is_none());
        // the background job left the queue once it was done
        assert!(service.inner.jobs.list().is_empty());
    }

    #[test]
    fn files_that_cannot_be_read_report_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (service, events) = service(dir.path());
        let bad = dir.path().join("bad.png");
        fs::write(&bad, b"\x89PNG\r\n\x1a\nnot really").unwrap();
        assert_eq!(service.thumbnail(&bad, 64).unwrap(), None);
        let MediaEvent::Failed { what, .. } = next_ready(&events) else {
            panic!("expected a failure");
        };
        assert_eq!(what, "thumbnail");
        // the request is not stuck as pending, so asking again tries again
        assert!(service.inner.pending.lock().is_empty());
    }

    #[test]
    fn probes_are_cached_until_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (service, _events) = service(dir.path());
        let photo = dir.path().join("photo.png");
        RgbImage::new(10, 10).save(&photo).unwrap();
        assert_eq!(service.probe(&photo).unwrap().image.unwrap().width, 10);
        assert_eq!(service.cache().unwrap().stats().unwrap().entries, 1);
        RgbImage::new(30, 10).save(&photo).unwrap();
        assert_eq!(service.probe(&photo).unwrap().image.unwrap().width, 30);
    }

    #[test]
    fn strip_metadata_runs_as_a_visible_job_with_an_openable_result() {
        let dir = tempfile::tempdir().unwrap();
        let (service, _events) = service(dir.path());
        let photo = dir.path().join("photo.jpg");
        RgbImage::from_pixel(8, 8, Rgb([1, 2, 3]))
            .save(&photo)
            .unwrap();
        let id = service.strip_metadata(&photo).unwrap();
        let done = service
            .inner
            .jobs
            .wait(id, Duration::from_secs(10))
            .unwrap();
        assert!(!done.background);
        let out = done.result.unwrap()["path"].as_str().unwrap().to_owned();
        assert!(out.ends_with("photo-clean.jpg"), "{out}");
        assert!(Path::new(&out).is_file());
    }

    #[test]
    fn settings_changes_resize_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let (service, _events) = service(dir.path());
        let next = MediaSettings {
            cache_limit_mb: 300,
            ..MediaSettings::default()
        };
        service.apply_settings(&next);
        assert_eq!(
            service.status().cache.unwrap().limit_bytes,
            300 * 1024 * 1024
        );
    }

    #[test]
    fn waveforms_filmstrips_and_frame_indexes_come_through_the_service() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (service, events) = service(dir.path());
        let clip = dir.path().join("clip.mp4");
        testing::make(
            &tools,
            "-f lavfi -i testsrc2=s=160x120:r=25:d=2 -f lavfi -i sine=d=2 -c:v libx264 -c:a aac",
            &clip,
        );

        assert!(service.waveform(&clip, 50).unwrap().is_none());
        assert!(matches!(
            next_ready(&events),
            MediaEvent::WaveformReady { .. }
        ));
        let view = service.waveform(&clip, 50).unwrap().unwrap();
        assert_eq!(view.columns, 50);
        assert!(view.peaks.iter().any(|p| p.abs() > 0.05));

        assert!(service.filmstrip(&clip, 4, 30).unwrap().is_none());
        let MediaEvent::FilmstripReady { filmstrip, .. } = next_ready(&events) else {
            panic!("expected a filmstrip");
        };
        assert_eq!((filmstrip.tile_width, filmstrip.tile_height), (40, 30));
        assert!(Path::new(&filmstrip.file).is_file());
        assert_eq!(service.filmstrip(&clip, 4, 30).unwrap(), Some(filmstrip));

        let index = service.frame_index(&clip, &NoReporter).unwrap();
        assert_eq!(index.len(), 50);
        // and from the cache the second time
        assert_eq!(service.frame_index(&clip, &NoReporter).unwrap(), index);
    }
}
