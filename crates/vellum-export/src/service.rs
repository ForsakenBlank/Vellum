//! the quick tools as the app runs them. a quick tool session is a document holding its
//! files and options; [`QuickTools::run`] reads it and starts one job per file (a
//! slideshow is one job for all of them), each owned by the document so its view can list
//! its own runs. a job hands back a [`ToolResult`], whose `path` drives "open" and "show in
//! folder" and the finished notification.
//!
//! the estimates the views show while options change (the plan for a video, the size of a
//! gif) are quick enough to answer directly, off the ui thread.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use vellum_core::app::AppServices;
use vellum_core::documents::DocumentError;
use vellum_core::files::display_name;
use vellum_core::jobs::{JobContext, JobError, JobId, JobSpec};
use vellum_core::settings::ExportSettings;
use vellum_media::cache::SourceKey;
use vellum_media::probe::{MediaInfo, MediaKind};
use vellum_media::{Control, MediaError, MediaResult, MediaService, Unattended};

use crate::anim::frames::{self, EditedFrame, GifFrames};
use crate::anim::{self, AnimFormat, AnimOptions, AnimSource, Dither};
use crate::convert::{self, ConvertOptions, ConvertTarget};
use crate::size::budget::{self, VideoBudget, VideoPlan, VideoSource, human_bytes};
use crate::size::{self, SizeJob, SizeOptions, TargetSize};
use crate::{Env, ToolResult, naming};

pub const EXPORT_TO_SIZE: &str = "export_to_size";
pub const VIDEO_TO_GIF: &str = "video_to_gif";
pub const GIF_TOOLS: &str = "gif_tools";
pub const CONVERT: &str = "convert";

/// the options of a video to gif session
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct GifMakerOptions {
    pub anim: AnimOptions,
    /// step settings down until the result fits `target`
    pub fit_to_size: bool,
    /// none uses the size from settings
    pub target: Option<TargetSize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GifMode {
    /// smaller: fewer colours, lossy, narrower, fewer frames
    #[default]
    Optimise,
    /// the frame editor's list of frames
    Edit,
    /// every frame as a picture
    Split,
    /// an mp4 of the gif
    Video,
}

/// the options of a gif tools session
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct GifToolOptions {
    pub mode: GifMode,
    pub width: Option<u32>,
    pub colors: u32,
    pub lossy: u32,
    pub dither: Dither,
    /// keeps every other frame, each shown twice as long
    pub drop_frames: bool,
    /// the edited frame list, none until the user changes something
    pub frames: Option<Vec<EditedFrame>>,
    /// none keeps the gif's own loop count
    pub loops: Option<u32>,
    pub fit_to_size: bool,
    pub target: Option<TargetSize>,
}

impl Default for GifToolOptions {
    fn default() -> Self {
        Self {
            mode: GifMode::Optimise,
            width: None,
            colors: 256,
            lossy: 40,
            dither: Dither::Sierra,
            drop_frames: false,
            frames: None,
            loops: None,
            fit_to_size: false,
            target: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error(transparent)]
    Document(#[from] DocumentError),
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error("{0}")]
    Invalid(String),
}

/// what export to size will do with a file, shown before the encode starts
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SizePlan {
    pub kind: MediaKind,
    #[ts(type = "number")]
    pub goal_bytes: u64,
    #[ts(type = "number")]
    pub source_bytes: u64,
    pub already_fits: bool,
    /// one line, eg "1280×720 at 30 fps, 1,850 kbps picture, 128 kbps sound"
    pub summary: String,
    pub video: Option<VideoPlan>,
    /// why it cannot be done, eg the target is far too small
    pub problem: Option<String>,
}

/// the likely size of a gif before making it, from a short sample
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnimEstimate {
    #[ts(type = "number")]
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
    pub frames: u32,
}

/// a gif's frames for the frame editor: their timing and a sheet of small pictures
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GifFramesView {
    pub info: GifFrames,
    /// one image holding every frame, row by row
    pub sheet: String,
    pub columns: u32,
    pub frame_width: u32,
    pub frame_height: u32,
}

/// frames on the editor's sheet, so a huge gif cannot make a huge sheet
const MAX_SHEET_FRAMES: usize = 1000;
const SHEET_HEIGHT: u32 = 72;
const SHEET_COLUMNS: u32 = 10;

#[derive(Clone)]
pub struct QuickTools {
    app: AppServices,
    media: MediaService,
}

impl QuickTools {
    pub fn new(app: &AppServices, media: &MediaService) -> Self {
        Self {
            app: app.clone(),
            media: media.clone(),
        }
    }

    fn settings(&self) -> ExportSettings {
        self.app.settings.get().export
    }

    /// starts the quick tool of document `id` on its files, returning the new jobs
    pub fn run(&self, id: &str) -> Result<Vec<JobId>, ToolError> {
        let doc = self.app.documents.state(id)?;
        let tool =
            doc.info.tool.clone().ok_or_else(|| {
                ToolError::Invalid("That document is not a quick tool.".to_owned())
            })?;
        let inputs: Vec<PathBuf> = doc.info.inputs.iter().map(PathBuf::from).collect();
        if inputs.is_empty() {
            return Err(ToolError::Invalid(
                "Add a file first. Drop one on the window or use Add files.".to_owned(),
            ));
        }
        let options = doc.state.get("options").cloned().unwrap_or(Value::Null);
        let read = |what: &str| -> Result<Value, ToolError> {
            if options.is_null() {
                return Ok(Value::Object(serde_json::Map::new()));
            }
            if options.is_object() {
                Ok(options.clone())
            } else {
                Err(ToolError::Invalid(format!(
                    "The {what} options could not be read."
                )))
            }
        };
        let parse_err = |err: serde_json::Error| {
            ToolError::Invalid(format!("The options could not be read: {err}"))
        };
        match tool.as_str() {
            EXPORT_TO_SIZE => {
                let options: SizeOptions =
                    serde_json::from_value(read("size")?).map_err(parse_err)?;
                Ok(self.run_size(id, &inputs, options))
            }
            VIDEO_TO_GIF => {
                let options: GifMakerOptions =
                    serde_json::from_value(read("GIF")?).map_err(parse_err)?;
                Ok(self.run_gif_maker(id, &inputs, options))
            }
            GIF_TOOLS => {
                let options: GifToolOptions =
                    serde_json::from_value(read("GIF")?).map_err(parse_err)?;
                Ok(self.run_gif_tools(id, &inputs, options))
            }
            CONVERT => {
                let options: ConvertOptions =
                    serde_json::from_value(read("convert")?).map_err(parse_err)?;
                self.run_convert(id, &inputs, options)
            }
            other => Err(ToolError::Invalid(format!(
                "{} arrives in a later version of Vellum.",
                self.app.tool_title(other)
            ))),
        }
    }

    /// queues one job that runs `work` with a fresh environment from settings
    fn submit<F>(&self, owner: &str, title: String, work: F) -> JobId
    where
        F: Fn(&Env, &JobContext) -> MediaResult<ToolResult> + Send + 'static,
    {
        let media = self.media.clone();
        let app = self.app.clone();
        let spec = JobSpec::cpu(title).owned_by(owner);
        self.app.jobs.submit(spec, move |ctx: &JobContext| {
            let hardware = media.hardware();
            let settings = app.settings.get();
            let folder = settings.export.output_folder.as_deref().map(Path::new);
            if let Some(folder) = folder {
                std::fs::create_dir_all(folder)
                    .map_err(|err| JobError::fatal(MediaError::io(folder, err)))?;
            }
            let env = Env {
                ffmpeg: media.ffmpeg(),
                hardware: &hardware,
                allow_hardware: media.hardware_allowed(),
                output_folder: folder,
            };
            let result = work(&env, ctx)?;
            Ok(serde_json::to_value(result)?)
        })
    }

    fn run_size(&self, owner: &str, inputs: &[PathBuf], options: SizeOptions) -> Vec<JobId> {
        let settings = self.settings();
        let goal = options.goal_bytes(&settings);
        let label = options.target(&settings).label();
        // a trim only makes sense for one file
        let options = if inputs.len() > 1 {
            SizeOptions {
                start: None,
                end: None,
                ..options
            }
        } else {
            options
        };
        inputs
            .iter()
            .map(|input| {
                let media = self.media.clone();
                let input = input.clone();
                let options = options.clone();
                let label = label.clone();
                let title = format!("Export {} to {label}", display_name(&input));
                self.submit(owner, title, move |env, ctx| {
                    ctx.set_message("Reading the file");
                    let info = media.probe(&input)?;
                    let job = SizeJob {
                        options: &options,
                        goal,
                        label: label.clone(),
                    };
                    size::fit(env, &input, &info, &job, ctx)
                })
            })
            .collect()
    }

    fn run_gif_maker(
        &self,
        owner: &str,
        inputs: &[PathBuf],
        options: GifMakerOptions,
    ) -> Vec<JobId> {
        let settings = self.settings();
        let goal = options.fit_to_size.then(|| {
            budget::target_bytes(
                options
                    .target
                    .map_or_else(|| settings_target(&settings), |t| t.bytes()),
                settings.safety_margin_percent,
            )
        });
        let mut anim = options.anim;
        if inputs.len() > 1 {
            anim.start = None;
            anim.end = None;
            anim.crop = None;
        }
        inputs
            .iter()
            .map(|input| {
                let media = self.media.clone();
                let input = input.clone();
                let anim = anim.clone();
                let title = format!(
                    "Make a {} from {}",
                    anim.format.label(),
                    display_name(&input)
                );
                self.submit(owner, title, move |env, ctx| {
                    let info = media.probe(&input)?;
                    let source = AnimSource::of(&info)?;
                    let output = naming::output_for(
                        &input,
                        env.output_folder,
                        None,
                        anim.format.extension(),
                    );
                    let mut result;
                    if let Some(goal) = goal {
                        let changes = size::gif::fit_anim(
                            env,
                            &input,
                            &source,
                            &anim,
                            goal,
                            output.path(),
                            ctx,
                        )?;
                        result = ToolResult::file(&input, output.path());
                        result.target_bytes = Some(goal);
                        result.changes = changes;
                    } else {
                        anim::render(env, &input, &source, &anim, output.path(), ctx)?;
                        result = ToolResult::file(&input, output.path());
                    }
                    if anim.format == AnimFormat::Gif
                        && let Some(note) = size::gif::mp4_comparison(
                            env,
                            &input,
                            &source,
                            &anim,
                            result.bytes.unwrap_or(0),
                        )?
                    {
                        result.notes.push(note);
                    }
                    Ok(result)
                })
            })
            .collect()
    }

    fn run_gif_tools(
        &self,
        owner: &str,
        inputs: &[PathBuf],
        options: GifToolOptions,
    ) -> Vec<JobId> {
        let settings = self.settings();
        let goal = options.fit_to_size.then(|| {
            budget::target_bytes(
                options
                    .target
                    .map_or_else(|| settings_target(&settings), |t| t.bytes()),
                settings.safety_margin_percent,
            )
        });
        // the frame list belongs to the first gif, the one the editor shows
        let edit_first_only = options.mode == GifMode::Edit;
        inputs
            .iter()
            .enumerate()
            .filter(|(i, _)| !edit_first_only || *i == 0)
            .map(|(_, input)| {
                let media = self.media.clone();
                let input = input.clone();
                let options = options.clone();
                let verb = match options.mode {
                    GifMode::Optimise => "Optimise",
                    GifMode::Edit => "Save edits to",
                    GifMode::Split => "Split",
                    GifMode::Video => "Make a video from",
                };
                let title = format!("{verb} {}", display_name(&input));
                self.submit(owner, title, move |env, ctx| {
                    gif_tool(env, &media, &input, &options, goal, ctx)
                })
            })
            .collect()
    }

    fn run_convert(
        &self,
        owner: &str,
        inputs: &[PathBuf],
        options: ConvertOptions,
    ) -> Result<Vec<JobId>, ToolError> {
        let target = options.target;
        let all_pictures = inputs.iter().all(|p| {
            matches!(
                vellum_core::files::FileKind::of_path(p),
                Some(
                    vellum_core::files::FileKind::Image
                        | vellum_core::files::FileKind::Raw
                        | vellum_core::files::FileKind::Vector
                )
            )
        });
        let slideshow = matches!(
            target,
            Some(
                ConvertTarget::Mp4 | ConvertTarget::Webm | ConvertTarget::Mkv | ConvertTarget::Mov
            )
        ) && all_pictures;
        if slideshow {
            let inputs = inputs.to_vec();
            let target = target.unwrap_or(ConvertTarget::Mp4);
            let title = format!("Make a slideshow of {} pictures", inputs.len());
            return Ok(vec![self.submit(owner, title, move |env, ctx| {
                convert::slideshow(env, &inputs, target, &options, ctx)
            })]);
        }
        Ok(inputs
            .iter()
            .map(|input| {
                let media = self.media.clone();
                let input = input.clone();
                let options = options.clone();
                let what = target.map_or_else(String::new, |t| {
                    format!(" to {}", t.extension().to_uppercase())
                });
                let title = format!("Convert {}{what}", display_name(&input));
                self.submit(owner, title, move |env, ctx| {
                    let info = media.probe(&input)?;
                    convert::convert(env, &input, &info, &options, ctx)
                })
            })
            .collect())
    }

    /// the formats a file can be converted to
    pub fn convert_targets(&self, path: &Path) -> MediaResult<Vec<ConvertTarget>> {
        let info = self.media.probe(path)?;
        Ok(convert::targets_for(info.kind))
    }

    /// what export to size would do with `path`
    pub fn size_plan(&self, path: &Path, options: &SizeOptions) -> MediaResult<SizePlan> {
        let settings = self.settings();
        let info = self.media.probe(path)?;
        let goal = options.goal_bytes(&settings);
        let fits = !options.trimmed() && !options.mute && !options.mono && info.size_bytes <= goal;
        let mut plan = SizePlan {
            kind: info.kind,
            goal_bytes: goal,
            source_bytes: info.size_bytes,
            already_fits: fits,
            summary: String::new(),
            video: None,
            problem: None,
        };
        if fits {
            plan.summary = format!(
                "Already {}, so nothing needs to change.",
                human_bytes(info.size_bytes)
            );
            return Ok(plan);
        }
        plan.summary = match info.kind {
            MediaKind::Video => match video_plan(&info, options, goal) {
                Ok(video) => {
                    let summary = describe_video(&video);
                    plan.video = Some(video);
                    summary
                }
                Err(problem) => {
                    plan.problem = Some(problem);
                    String::new()
                }
            },
            MediaKind::Audio => {
                let duration = trimmed_length(&info, options.start, options.end);
                let kbps = duration.and_then(|d| {
                    size::audio::pick_kbps(options.audio_format, budget::audio_kbps(goal, d))
                });
                match kbps {
                    Some(kbps) => format!(
                        "{} at {kbps} kbps.",
                        match options.audio_format {
                            size::AudioTarget::Mp3 => "MP3",
                            size::AudioTarget::Opus => "Opus",
                        }
                    ),
                    None => {
                        plan.problem = Some(format!(
                            "{} is too small for this much sound. Pick a bigger size or trim it.",
                            human_bytes(goal)
                        ));
                        String::new()
                    }
                }
            }
            MediaKind::Animation => {
                "Tries smaller settings (colours, lossy, size, frame rate) until it fits."
                    .to_owned()
            }
            _ => {
                "Finds the best quality that fits, and scales it down only if it has to.".to_owned()
            }
        };
        Ok(plan)
    }

    /// renders a short piece of the gif to see how big all of it would be
    pub fn anim_estimate(&self, path: &Path, options: &AnimOptions) -> MediaResult<AnimEstimate> {
        let info = self.media.probe(path)?;
        let source = AnimSource::of(&info)?;
        let (width, height) = anim::frame_size(&source, options);
        let seconds = anim::output_seconds(&source, options).unwrap_or(0.0);
        let fps = options.fps.or(source.fps).unwrap_or(15.0);
        let frames = (seconds * fps).round() as u32;
        let hardware = self.media.hardware();
        let env = Env {
            ffmpeg: self.media.ffmpeg(),
            hardware: &hardware,
            allow_hardware: false,
            output_folder: None,
        };
        // a sample from the middle of the clip, without the boomerang, which only doubles it
        let start = options.start.unwrap_or(0.0).max(0.0);
        let end = options.end.or(source.duration).unwrap_or(start + 2.0);
        let span = (end - start).max(0.0);
        let sample = span.min(1.5 * options.speed.max(0.1));
        let sample_start = start + (span - sample) / 2.0;
        let sample_options = AnimOptions {
            start: Some(sample_start),
            end: Some(sample_start + sample),
            boomerang: false,
            reverse: false,
            ..options.clone()
        };
        let temp = tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
        let out = temp
            .path()
            .join(format!("sample.{}", options.format.extension()));
        anim::render(&env, path, &source, &sample_options, &out, &Unattended)?;
        let sample_bytes = std::fs::metadata(&out)
            .map_err(|err| MediaError::io(&out, err))?
            .len();
        let sample_seconds = anim::output_seconds(&source, &sample_options)
            .unwrap_or(1.0)
            .max(0.01);
        let bytes = (sample_bytes as f64 * seconds / sample_seconds).round() as u64;
        Ok(AnimEstimate {
            bytes,
            width,
            height,
            seconds,
            frames,
        })
    }

    /// the frames of a gif for the editor, with a cached sheet of small pictures
    pub fn gif_frames(&self, path: &Path) -> MediaResult<GifFramesView> {
        let key = SourceKey::of(path)?;
        let params = format!("r1:{SHEET_HEIGHT}:{SHEET_COLUMNS}");
        let cache = self.media.cache();
        if let Some(entry) = cache.get(&key, "gif-frames", &params)
            && let (Some(file), Some(meta)) = (entry.file, entry.meta)
            && let Ok(info) = serde_json::from_str::<GifFrames>(&meta)
        {
            return Ok(sheet_view(info, file));
        }
        let (info, images) = frames::read_frames(path)?;
        let (w, h) = (info.width.max(1), info.height.max(1));
        let frame_height = SHEET_HEIGHT;
        let frame_width =
            (u64::from(w) * u64::from(SHEET_HEIGHT) / u64::from(h)).clamp(8, 400) as u32;
        let count = images.len().min(MAX_SHEET_FRAMES) as u32;
        let columns = SHEET_COLUMNS.min(count.max(1));
        let rows = count.div_ceil(columns);
        let mut sheet = RgbaImage::new(frame_width * columns, frame_height * rows.max(1));
        for (i, image) in images.iter().take(MAX_SHEET_FRAMES).enumerate() {
            let small = image::imageops::resize(
                image,
                frame_width,
                frame_height,
                image::imageops::FilterType::Triangle,
            );
            let (col, row) = (i as u32 % columns, i as u32 / columns);
            image::imageops::overlay(
                &mut sheet,
                &small,
                i64::from(col * frame_width),
                i64::from(row * frame_height),
            );
        }
        let mut png = std::io::Cursor::new(Vec::new());
        sheet
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|err| {
                MediaError::Unsupported(format!("The frame sheet could not be made: {err}"))
            })?;
        let meta =
            serde_json::to_string(&info).map_err(|err| MediaError::Cache(err.to_string()))?;
        let file = cache.put_file(
            &key,
            "gif-frames",
            &params,
            &png.into_inner(),
            "png",
            Some(&meta),
        )?;
        Ok(sheet_view(info, file))
    }
}

fn sheet_view(info: GifFrames, file: PathBuf) -> GifFramesView {
    let (w, h) = (info.width.max(1), info.height.max(1));
    let frame_width = (u64::from(w) * u64::from(SHEET_HEIGHT) / u64::from(h)).clamp(8, 400) as u32;
    let count = info.delays_ms.len().min(MAX_SHEET_FRAMES) as u32;
    GifFramesView {
        info,
        sheet: file.display().to_string(),
        columns: SHEET_COLUMNS.min(count.max(1)),
        frame_width,
        frame_height: SHEET_HEIGHT,
    }
}

fn settings_target(settings: &ExportSettings) -> u64 {
    TargetSize {
        amount: settings.target_amount,
        unit: settings.unit,
    }
    .bytes()
}

fn trimmed_length(info: &MediaInfo, start: Option<f64>, end: Option<f64>) -> Option<f64> {
    let full = info.duration_secs?;
    let start = start.unwrap_or(0.0).clamp(0.0, full);
    let end = end.unwrap_or(full).clamp(start, full);
    Some(end - start).filter(|d| *d > 0.0)
}

fn video_plan(info: &MediaInfo, options: &SizeOptions, goal: u64) -> Result<VideoPlan, String> {
    let video = info.video.as_ref().ok_or("This file has no picture.")?;
    let (Some(width), Some(height)) = (info.width, info.height) else {
        return Err("Vellum cannot read the size of the picture.".to_owned());
    };
    let duration = trimmed_length(info, options.start, options.end)
        .ok_or("Vellum cannot read how long this video is.")?;
    let efficiency = match options.codec {
        vellum_media::ffmpeg::hw::VideoCodec::H264 => 1.0,
        vellum_media::ffmpeg::hw::VideoCodec::H265 => 0.65,
        vellum_media::ffmpeg::hw::VideoCodec::Vp9 => 0.7,
        vellum_media::ffmpeg::hw::VideoCodec::Av1 => 0.55,
    };
    budget::plan_video(
        &VideoSource {
            width,
            height,
            fps: video.frame_rate,
            duration,
            has_audio: !info.audio.is_empty(),
            audio_channels: info.audio.first().map_or(2, |a| a.channels),
        },
        goal,
        &VideoBudget {
            strategy: options.strategy,
            efficiency,
            mute: options.mute,
            mono: options.mono,
        },
    )
    .map_err(|err| err.to_string())
}

fn describe_video(plan: &VideoPlan) -> String {
    let fps = plan
        .fps
        .map_or_else(String::new, |f| format!(" at {} fps", crate::num(f)));
    let sound = match plan.audio {
        Some(a) => format!(", {} kbps sound", a.kbps),
        None => ", no sound".to_owned(),
    };
    format!(
        "{}×{}{fps}, {} kbps picture{sound}.",
        plan.width, plan.height, plan.video_kbps
    )
}

/// one run of the gif tools on one gif
fn gif_tool(
    env: &Env,
    media: &MediaService,
    input: &Path,
    options: &GifToolOptions,
    goal: Option<u64>,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let info = media.probe(input)?;
    match options.mode {
        GifMode::Edit => {
            let (gif, _) = frames::read_frames(input)?;
            let edit: Vec<EditedFrame> = options.frames.clone().unwrap_or_else(|| {
                gif.delays_ms
                    .iter()
                    .enumerate()
                    .map(|(i, d)| EditedFrame {
                        source: i as u32,
                        delay_ms: *d,
                    })
                    .collect()
            });
            let output = naming::output_for(input, env.output_folder, Some("edited"), "gif");
            frames::edit_gif(
                input,
                &edit,
                options.loops.unwrap_or(gif.loops),
                output.path(),
                control,
            )?;
            let mut result = ToolResult::file(input, output.path());
            result.changes.push(format!("{} frames.", edit.len()));
            Ok(result)
        }
        GifMode::Split => {
            let folder = naming::folder_for(input, env.output_folder, "frames");
            let count = frames::split(input, folder.path(), control)?;
            let mut result = ToolResult::file(input, folder.path());
            result.folder = true;
            result.bytes = None;
            result.changes.push(format!("Saved {count} frames."));
            Ok(result)
        }
        GifMode::Video => convert::convert(
            env,
            input,
            &info,
            &ConvertOptions {
                target: Some(ConvertTarget::Mp4),
                ..ConvertOptions::default()
            },
            control,
        ),
        GifMode::Optimise => {
            let source = AnimSource::of(&info)?;
            let fps = if options.drop_frames {
                // every other frame, so half the frame rate
                Some(source.fps.unwrap_or(10.0) / 2.0)
            } else {
                None
            };
            let anim = AnimOptions {
                width: options.width,
                fps,
                colors: options.colors,
                lossy: options.lossy,
                dither: options.dither,
                loops: options.loops.unwrap_or(0),
                ..AnimOptions::default()
            };
            let output = naming::output_for(input, env.output_folder, Some("optimised"), "gif");
            let mut result;
            if let Some(goal) = goal {
                let changes =
                    size::gif::fit_anim(env, input, &source, &anim, goal, output.path(), control)?;
                result = ToolResult::file(input, output.path());
                result.target_bytes = Some(goal);
                result.changes = changes;
            } else {
                anim::render(env, input, &source, &anim, output.path(), control)?;
                result = ToolResult::file(input, output.path());
            }
            if let (Some(before), Some(after)) = (result.source_bytes, result.bytes)
                && after >= before
            {
                result.notes.push("This GIF was already well compressed, so the new one is not smaller. Try more lossy compression or a smaller width.".to_owned());
            }
            Ok(result)
        }
    }
}
