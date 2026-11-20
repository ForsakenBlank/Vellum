//! the media core as the running app uses it: previews made as background jobs and kept in
//! the cache, probe results cached too, live settings, and events telling the ui when a
//! preview is ready.
//!
//! asking for a preview never blocks. [`MediaService::thumbnail`] answers at once with the
//! cached file, "pending" (a job is making it) or the reason it failed, and a
//! [`MediaAsset`] event follows when a pending one is done. asking twice while a job runs
//! does not start a second job, and a failure is remembered so a broken file is not retried
//! every time the home screen draws, until the file itself changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use serde_json::json;
use ts_rs::TS;
use vellum_core::AppSettings;
use vellum_core::app::AppServices;
use vellum_core::events::{Event, EventBus};
use vellum_core::files::display_name;
use vellum_core::jobs::{JobContext, JobId, JobPriority, JobQueue, JobSpec};
use vellum_core::settings::{MediaSettings, SettingsChanged};

use crate::cache::{CacheStats, MediaCache, SourceKey};
use crate::control::Control;
use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::hw::{self, HardwareSupport};
use crate::ffmpeg::{Ffmpeg, FfmpegInfo};
use crate::metadata;
use crate::probe::{self, MediaInfo, MediaKind};
use crate::seek::FrameIndex;
use crate::thumbs::{self, THUMB_SIZE};
use crate::waveform::{Waveform, WaveformOverview};

/// bumped whenever a preview recipe changes, so old cached previews are made again
const RECIPE: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AssetKind {
    Thumbnail,
    Filmstrip,
    Waveform,
}

impl AssetKind {
    fn name(self) -> &'static str {
        match self {
            Self::Thumbnail => "thumbnail",
            Self::Filmstrip => "filmstrip",
            Self::Waveform => "waveform",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum AssetState {
    /// `file` is none when there is nothing to show, eg audio without cover art
    Ready {
        file: Option<String>,
        #[ts(type = "unknown")]
        meta: Option<serde_json::Value>,
    },
    Pending,
    Failed {
        error: String,
    },
}

/// a preview of one file, sent as `media:asset` whenever one finishes
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaAsset {
    /// the path the ui asked about, exactly as it asked
    pub source: String,
    pub kind: AssetKind,
    pub state: AssetState,
}

impl Event for MediaAsset {
    const NAME: &'static str = "media:asset";
}

/// what this machine can do, sent as `media:capabilities` once hardware checks finish
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaCapabilities {
    /// none when ffmpeg was not found
    pub ffmpeg: Option<FfmpegInfo>,
    /// none while the hardware check is still running
    pub hardware: Option<HardwareSupport>,
}

impl Event for MediaCapabilities {
    const NAME: &'static str = "media:capabilities";
}

#[derive(Clone)]
pub struct MediaService {
    inner: Arc<Inner>,
}

struct Inner {
    ffmpeg: Option<Ffmpeg>,
    hardware: RwLock<Option<HardwareSupport>>,
    allow_hardware: AtomicBool,
    cache: MediaCache,
    jobs: JobQueue,
    bus: EventBus,
    /// asset key to the job making it
    pending: Mutex<HashMap<String, JobId>>,
    /// asset key to the version of the source that failed, and why
    failed: Mutex<HashMap<String, (SourceKey, String)>>,
}

impl MediaService {
    /// starts the media core for the app: finds ffmpeg, opens the cache, checks hardware
    /// encoders in the background and follows the media settings
    pub fn start(app: &AppServices, cache_dir: &Path) -> Self {
        let ffmpeg = match Ffmpeg::locate() {
            Ok(ffmpeg) => {
                tracing::info!(path = %ffmpeg.info().path, version = %ffmpeg.info().version, "found ffmpeg");
                Some(ffmpeg)
            }
            Err(err) => {
                tracing::warn!(%err, "video and audio are unavailable");
                app.notifications.push(
                    vellum_core::notifications::NewNotification::new(
                        vellum_core::notifications::NotificationLevel::Warning,
                        "FFmpeg was not found",
                    )
                    .body(err.to_string()),
                );
                None
            }
        };
        let settings = app.settings.get().media;
        let service = Self::new(
            ffmpeg,
            cache_dir,
            &settings,
            app.jobs.clone(),
            app.bus.clone(),
        );
        service.detect_hardware();
        service.follow_settings();
        service
    }

    /// the service without the app around it, for tests and the cli
    pub fn new(
        ffmpeg: Option<Ffmpeg>,
        cache_dir: &Path,
        settings: &MediaSettings,
        jobs: JobQueue,
        bus: EventBus,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                ffmpeg,
                hardware: RwLock::new(None),
                allow_hardware: AtomicBool::new(settings.hardware_acceleration),
                cache: MediaCache::open(cache_dir, limit_bytes(settings)),
                jobs,
                bus,
                pending: Mutex::new(HashMap::new()),
                failed: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn ffmpeg(&self) -> Option<&Ffmpeg> {
        self.inner.ffmpeg.as_ref()
    }

    pub fn cache_dir(&self) -> &Path {
        self.inner.cache.dir()
    }

    pub fn capabilities(&self) -> MediaCapabilities {
        MediaCapabilities {
            ffmpeg: self.inner.ffmpeg.as_ref().map(|f| f.info().clone()),
            hardware: self.inner.hardware.read().clone(),
        }
    }

    /// whether exports may use a hardware encoder, from settings
    pub fn hardware_allowed(&self) -> bool {
        self.inner.allow_hardware.load(Ordering::Relaxed)
    }

    fn detect_hardware(&self) {
        let Some(ffmpeg) = self.inner.ffmpeg.clone() else {
            return;
        };
        let inner = self.inner.clone();
        let spawned = thread::Builder::new()
            .name("vellum-hw-detect".into())
            .spawn(move || {
                let support = hw::detect(&ffmpeg).unwrap_or_else(|err| {
                    tracing::warn!(%err, "could not list encoders");
                    HardwareSupport::default()
                });
                let names: Vec<&str> = support
                    .encoders
                    .iter()
                    .filter(|e| e.hardware.is_some())
                    .map(|e| e.name.as_str())
                    .collect();
                tracing::info!(hardware = ?names, "hardware encoders checked");
                *inner.hardware.write() = Some(support.clone());
                inner.bus.emit(MediaCapabilities {
                    ffmpeg: Some(ffmpeg.info().clone()),
                    hardware: Some(support),
                });
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "hardware encoders will not be used");
        }
    }

    fn follow_settings(&self) {
        let changes = self.inner.bus.subscribe::<SettingsChanged<AppSettings>>();
        // a weak handle so the thread does not keep the service alive forever
        let inner = Arc::downgrade(&self.inner);
        let spawned = thread::Builder::new()
            .name("vellum-media-settings".into())
            .spawn(move || {
                while let Some(change) = changes.recv() {
                    let Some(inner) = inner.upgrade() else { break };
                    let media = &change.settings.media;
                    inner.cache.set_limit(limit_bytes(media));
                    inner
                        .allow_hardware
                        .store(media.hardware_acceleration, Ordering::Relaxed);
                }
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "media settings will apply after a restart");
        }
    }

    /// everything about a file. cached, so asking again is instant until the file changes.
    pub fn probe(&self, path: &Path) -> MediaResult<MediaInfo> {
        let key = SourceKey::of(path)?;
        let params = format!("r{RECIPE}");
        if let Some(meta) = self
            .inner
            .cache
            .get(&key, "probe", &params)
            .and_then(|e| e.meta)
            && let Ok(info) = serde_json::from_str::<MediaInfo>(&meta)
        {
            return Ok(MediaInfo {
                path: path.display().to_string(),
                ..info
            });
        }
        let info = probe::probe(path, self.ffmpeg())?;
        if let Ok(json) = serde_json::to_string(&info)
            && let Err(err) = self.inner.cache.put_meta(&key, "probe", &params, &json)
        {
            tracing::debug!(%err, "probe result not cached");
        }
        Ok(info)
    }

    pub fn thumbnail(&self, path: &Path) -> MediaAsset {
        let params = format!("r{RECIPE}:{THUMB_SIZE}");
        self.asset(
            path,
            AssetKind::Thumbnail,
            params,
            JobPriority::High,
            |service, path, ctx| {
                let info = service.probe(path)?;
                let Some(image) = thumbs::thumbnail(service.ffmpeg(), path, &info, THUMB_SIZE)?
                else {
                    return Ok(None);
                };
                ctx.checkpoint()?;
                let (bytes, ext) = thumbs::encode_preview(&image)?;
                let meta = json!({ "width": image.width(), "height": image.height() });
                Ok(Some((bytes, ext, Some(meta))))
            },
        )
    }

    /// `count` frames across the video, each `height` pixels tall, in one image
    pub fn filmstrip(&self, path: &Path, count: u32, height: u32) -> MediaAsset {
        let params = format!("r{RECIPE}:{count}x{height}");
        self.asset(
            path,
            AssetKind::Filmstrip,
            params,
            JobPriority::Normal,
            move |service, path, ctx| {
                let info = service.probe(path)?;
                if info.kind != MediaKind::Video {
                    return Ok(None);
                }
                let ffmpeg = service.ffmpeg().ok_or(MediaError::FfmpegMissing)?;
                let strip = thumbs::filmstrip(ffmpeg, path, &info, count, height, ctx)?;
                let (bytes, ext) = thumbs::encode_preview(&strip.image)?;
                let meta = json!({
                    "frameWidth": strip.frame_width,
                    "frameHeight": strip.frame_height,
                    "frames": strip.times.len(),
                    "times": strip.times,
                });
                Ok(Some((bytes, ext, Some(meta))))
            },
        )
    }

    pub fn waveform(&self, path: &Path) -> MediaAsset {
        let params = format!("r{RECIPE}");
        self.asset(
            path,
            AssetKind::Waveform,
            params,
            JobPriority::Normal,
            |service, path, ctx| {
                let info = service.probe(path)?;
                if info.audio.is_empty() {
                    return Ok(None);
                }
                let ffmpeg = service.ffmpeg().ok_or(MediaError::FfmpegMissing)?;
                let wave = Waveform::generate(ffmpeg, path, &info, ctx)?;
                let meta =
                    json!({ "channels": wave.channels, "durationSecs": wave.duration_secs() });
                Ok(Some((wave.to_bytes(), "vwav", Some(meta))))
            },
        )
    }

    /// the cached waveform squeezed to `columns`, or none if it has not been made yet
    pub fn waveform_overview(
        &self,
        path: &Path,
        columns: u32,
    ) -> MediaResult<Option<WaveformOverview>> {
        let key = SourceKey::of(path)?;
        let Some(file) = self
            .inner
            .cache
            .get(&key, AssetKind::Waveform.name(), &format!("r{RECIPE}"))
            .and_then(|e| e.file)
        else {
            return Ok(None);
        };
        let bytes = std::fs::read(&file).map_err(|err| MediaError::io(&file, err))?;
        Ok(Waveform::from_bytes(&bytes).map(|w| w.overview(columns.clamp(1, 8192))))
    }

    /// the frame index of a video, cached after the first time
    pub fn frame_index(&self, path: &Path, control: &dyn Control) -> MediaResult<FrameIndex> {
        let key = SourceKey::of(path)?;
        let params = format!("r{RECIPE}");
        if let Some(file) = self
            .inner
            .cache
            .get(&key, "frame-index", &params)
            .and_then(|e| e.file)
            && let Some(index) = std::fs::read(&file)
                .ok()
                .and_then(|b| FrameIndex::from_bytes(&b))
        {
            return Ok(index);
        }
        let ffmpeg = self.ffmpeg().ok_or(MediaError::FfmpegMissing)?;
        let index = FrameIndex::build(ffmpeg, path, control)?;
        if let Err(err) = self.inner.cache.put_file(
            &key,
            "frame-index",
            &params,
            &index.to_bytes(),
            "vfix",
            None,
        ) {
            tracing::debug!(%err, "frame index not cached");
        }
        Ok(index)
    }

    /// starts a job that writes a copy without metadata next to the file. the job result
    /// has the new path, so the notification can open it.
    pub fn strip_metadata(&self, path: &Path) -> JobId {
        let service = self.clone();
        let path = path.to_path_buf();
        let title = format!("Strip metadata from {}", display_name(&path));
        self.inner
            .jobs
            .submit(JobSpec::io(title), move |ctx: &JobContext| {
                ctx.set_indeterminate();
                let output = metadata::strip_metadata(service.ffmpeg(), &path, ctx)?;
                Ok(json!({ "path": output.display().to_string() }))
            })
    }

    pub fn cache_stats(&self) -> CacheStats {
        self.inner.cache.stats()
    }

    pub fn clean_cache(&self) -> MediaResult<CacheStats> {
        self.inner.failed.lock().clear();
        self.inner.cache.clean()
    }

    /// the shared request logic for every preview kind
    fn asset<F>(
        &self,
        path: &Path,
        kind: AssetKind,
        params: String,
        priority: JobPriority,
        make: F,
    ) -> MediaAsset
    where
        F: Fn(
                &MediaService,
                &Path,
                &dyn Control,
            )
                -> MediaResult<Option<(Vec<u8>, &'static str, Option<serde_json::Value>)>>
            + Send
            + Sync
            + 'static,
    {
        let source = path.display().to_string();
        let reply = |state| MediaAsset {
            source: source.clone(),
            kind,
            state,
        };
        let key = match SourceKey::of(path) {
            Ok(key) => key,
            Err(err) => {
                return reply(AssetState::Failed {
                    error: err.to_string(),
                });
            }
        };
        let asset_key = format!("{}:{params}:{}", kind.name(), key.path);
        if let Some(entry) = self.inner.cache.get(&key, kind.name(), &params) {
            let meta = entry.meta.and_then(|m| serde_json::from_str(&m).ok());
            return reply(AssetState::Ready {
                file: entry.file.map(|f| f.display().to_string()),
                meta,
            });
        }
        if let Some((failed_key, error)) = self.inner.failed.lock().get(&asset_key)
            && *failed_key == key
        {
            return reply(AssetState::Failed {
                error: error.clone(),
            });
        }
        let mut pending = self.inner.pending.lock();
        // a job that crashed never cleared its entry, and background jobs leave the queue
        // when they finish, so only a job still in the queue counts as pending
        if let Some(id) = pending.get(&asset_key) {
            if self.inner.jobs.snapshot(*id).is_some() {
                return reply(AssetState::Pending);
            }
            pending.remove(&asset_key);
        }

        let service = self.clone();
        let job_path = PathBuf::from(path);
        let job_key = asset_key.clone();
        let job_source = source.clone();
        let make = Arc::new(make);
        let title = format!("Preview {}", display_name(path));
        let spec = JobSpec::cpu(title).with_priority(priority).background();
        let id = self.inner.jobs.submit(spec, move |ctx: &JobContext| {
            let result = make(&service, &job_path, ctx).and_then(|made| match made {
                None => Ok(AssetState::Ready {
                    file: None,
                    meta: None,
                }),
                Some((bytes, ext, meta)) => {
                    let meta_text = meta.as_ref().map(serde_json::Value::to_string);
                    let file = service.inner.cache.put_file(
                        &key,
                        kind.name(),
                        &params,
                        &bytes,
                        ext,
                        meta_text.as_deref(),
                    )?;
                    Ok(AssetState::Ready {
                        file: Some(file.display().to_string()),
                        meta,
                    })
                }
            });
            service.inner.pending.lock().remove(&job_key);
            let state = match result {
                Ok(state) => state,
                Err(MediaError::Cancelled) => return Err(vellum_core::JobError::Cancelled),
                Err(err) => {
                    tracing::debug!(%err, source = %job_source, "preview failed");
                    let error = err.to_string();
                    service
                        .inner
                        .failed
                        .lock()
                        .insert(job_key.clone(), (key.clone(), error.clone()));
                    AssetState::Failed { error }
                }
            };
            service.inner.bus.emit(MediaAsset {
                source: job_source.clone(),
                kind,
                state,
            });
            Ok(serde_json::Value::Null)
        });
        pending.insert(asset_key, id);
        reply(AssetState::Pending)
    }
}

fn limit_bytes(settings: &MediaSettings) -> u64 {
    u64::from(settings.cache_limit_mb) * 1024 * 1024
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the ui keeps its own map from event names to types, see `ui/src/lib/events.ts`
    #[test]
    fn ui_event_map_lists_media_events() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/src/lib/events.ts");
        let source = std::fs::read_to_string(path).unwrap();
        for name in [MediaAsset::NAME, MediaCapabilities::NAME] {
            assert!(
                source.contains(&format!("\"{name}\"")),
                "ui/src/lib/events.ts does not mention \"{name}\""
            );
        }
    }
}
