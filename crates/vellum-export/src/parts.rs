//! joining many files into one and cutting one file into parts.
//!
//! both copy the streams when they can, which is instant and loses nothing: files that
//! were all recorded the same way join straight away, and cuts land on the nearest
//! keyframe. when the files differ (a phone clip and a screen recording, say), joining
//! re-encodes everything to the shape of the first file, with black bars where a shape
//! does not match and silence for files that have no sound.

use std::path::{Path, PathBuf};

use vellum_media::ffmpeg::encode::{self, quality_args};
use vellum_media::ffmpeg::hw::VideoCodec;
use vellum_media::probe::{MediaInfo, MediaKind};
use vellum_media::{Control, MediaError, MediaResult};

use crate::convert::{ConvertOptions, ConvertTarget, audio_settings, can_remux, is_audio};
use crate::{Env, ToolResult, finish_partial, naming, num, output_args, run_ffmpeg};

/// the shortest part a split makes, anything less is almost certainly a typo
pub const MIN_PART_SECONDS: f64 = 0.5;

/// a path inside an ffconcat list, quoted the way the concat reader expects
fn concat_entry(path: &Path) -> String {
    format!(
        "file '{}'\n",
        path.display().to_string().replace('\'', r"'\''")
    )
}

/// the picture size every joined clip is fitted into: the first clip's, shrunk to the
/// size limits and rounded to even numbers so every encoder takes it
fn joined_size(width: Option<u32>, height: Option<u32>, options: &ConvertOptions) -> (u32, u32) {
    let (w, h) = (width.unwrap_or(1280).max(2), height.unwrap_or(720).max(2));
    let scale_w = options
        .max_width
        .map_or(1.0, |m| f64::from(m) / f64::from(w));
    let scale_h = options
        .max_height
        .map_or(1.0, |m| f64::from(m) / f64::from(h));
    let scale = scale_w.min(scale_h).min(1.0);
    let even = |v: f64| ((v.round() as u32).max(2)) & !1;
    (even(f64::from(w) * scale), even(f64::from(h) * scale))
}

/// true when every file holds the same kind of streams, so they can be glued together
/// without decoding anything
pub fn can_join_by_copy(
    infos: &[MediaInfo],
    target: ConvertTarget,
    options: &ConvertOptions,
) -> bool {
    let Some(first) = infos.first() else {
        return false;
    };
    let resized = options.max_width.is_some() || options.max_height.is_some();
    if resized || infos.iter().any(|i| !can_remux(i, target, &plain(options))) {
        return false;
    }
    let video_key = |i: &MediaInfo| {
        i.video.as_ref().map(|v| {
            (
                v.codec.clone(),
                i.width,
                i.height,
                v.pixel_format.clone(),
                v.rotation,
            )
        })
    };
    let audio_key = |i: &MediaInfo| {
        i.audio
            .first()
            .map(|a| (a.codec.clone(), a.sample_rate, a.channels))
    };
    let same_video = is_audio(target) || infos.iter().all(|i| video_key(i) == video_key(first));
    let same_audio = (options.mute && !is_audio(target))
        || infos.iter().all(|i| audio_key(i) == audio_key(first));
    same_video && same_audio
}

/// the options with everything that blocks a copy and does not apply here taken out
fn plain(options: &ConvertOptions) -> ConvertOptions {
    ConvertOptions {
        start: None,
        end: None,
        max_width: None,
        max_height: None,
        ..options.clone()
    }
}

/// joins `inputs` into one file of `target`, in the order given
pub fn merge(
    env: &Env,
    inputs: &[(PathBuf, MediaInfo)],
    target: ConvertTarget,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let ffmpeg = env.ffmpeg()?;
    let [(first_path, first), ..] = inputs else {
        return Err(MediaError::Unsupported(
            "Add at least two files to join.".to_owned(),
        ));
    };
    if inputs.len() < 2 {
        return Err(MediaError::Unsupported(
            "Add at least two files to join.".to_owned(),
        ));
    }
    let sound_only = is_audio(target);
    for (_, info) in inputs {
        let fits = match info.kind {
            MediaKind::Video => true,
            MediaKind::Audio => sound_only,
            _ => false,
        };
        if !fits {
            return Err(MediaError::Unsupported(if sound_only {
                format!("{} has no sound to join.", info.name)
            } else {
                format!(
                    "{} is not a video. Join videos with videos, or pick a sound format to join only their sound.",
                    info.name
                )
            }));
        }
        if sound_only && info.audio.is_empty() {
            return Err(MediaError::Unsupported(format!(
                "{} has no sound to join.",
                info.name
            )));
        }
    }

    let total: f64 = inputs.iter().filter_map(|(_, i)| i.duration_secs).sum();
    let output = naming::output_for(
        first_path,
        env.output_folder,
        Some("joined"),
        target.extension(),
    );
    let partial = encode::partial_path(output.path());
    let temp = tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
    let infos: Vec<MediaInfo> = inputs.iter().map(|(_, i)| i.clone()).collect();
    let copy = can_join_by_copy(&infos, target, options);
    let mut result = ToolResult::file(first_path, output.path());

    let mut args: Vec<String> = Vec::new();
    if copy {
        control.message("The files match, so they are joined without re-encoding");
        let list_path = temp.path().join("join.txt");
        let mut list = String::from("ffconcat version 1.0\n");
        for (path, _) in inputs {
            list.push_str(&concat_entry(path));
        }
        std::fs::write(&list_path, list).map_err(|err| MediaError::io(&list_path, err))?;
        args.extend([
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list_path.display().to_string(),
        ]);
        if sound_only {
            args.extend(["-map".into(), "0:a:0".into(), "-vn".into()]);
        } else {
            args.extend(["-map".into(), "0:v:0".into()]);
            if !options.mute {
                args.extend(["-map".into(), "0:a:0?".into()]);
            }
        }
        args.extend(["-c".into(), "copy".into()]);
        result
            .changes
            .push("Joined without re-encoding, so nothing was lost.".to_owned());
    } else {
        control.message("The files differ, so they are re-encoded to match");
        let keep_sound = sound_only || !options.mute;
        let (w, h) = joined_size(first.width, first.height, options);
        let fps = first
            .video
            .as_ref()
            .and_then(|v| v.frame_rate)
            .filter(|f| f.is_finite() && *f > 0.0)
            .unwrap_or(30.0)
            .min(120.0);
        for (path, _) in inputs {
            args.extend(["-i".into(), path.display().to_string()]);
        }
        // a silent track for each clip that has none, as long as the clip
        let mut silence = Vec::new();
        for (i, (_, info)) in inputs.iter().enumerate() {
            if keep_sound && info.audio.is_empty() {
                silence.push((i, inputs.len() + silence.len()));
                args.extend([
                    "-f".into(),
                    "lavfi".into(),
                    "-t".into(),
                    num(info.duration_secs.unwrap_or(0.1).max(0.1)),
                    "-i".into(),
                    "anullsrc=r=48000:cl=stereo".into(),
                ]);
            }
        }
        let mut graph = Vec::new();
        let mut joined = String::new();
        for (i, _) in inputs.iter().enumerate() {
            if !sound_only {
                graph.push(format!(
                    "[{i}:v:0]scale={w}:{h}:force_original_aspect_ratio=decrease,\
                     pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={},format=yuv420p[v{i}]",
                    num(fps)
                ));
                joined.push_str(&format!("[v{i}]"));
            }
            if keep_sound {
                let source = silence
                    .iter()
                    .find(|(clip, _)| *clip == i)
                    .map_or(format!("{i}:a:0"), |(_, input)| format!("{input}:a:0"));
                graph.push(format!(
                    "[{source}]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{i}]"
                ));
                joined.push_str(&format!("[a{i}]"));
            }
        }
        let (v, a) = (u8::from(!sound_only), u8::from(keep_sound));
        let mut outputs = String::new();
        if v == 1 {
            outputs.push_str("[v]");
        }
        if a == 1 {
            outputs.push_str("[a]");
        }
        graph.push(format!(
            "{joined}concat=n={}:v={v}:a={a}{outputs}",
            inputs.len()
        ));
        args.extend(["-filter_complex".into(), graph.join(";")]);
        if v == 1 {
            let codec = if target == ConvertTarget::Webm {
                VideoCodec::Vp9
            } else {
                options.codec
            };
            let encoder = env.hardware.software(codec).ok_or_else(|| {
                MediaError::Unsupported(format!(
                    "This copy of FFmpeg cannot encode {}. Pick another codec.",
                    encode::codec_label(codec)
                ))
            })?;
            args.extend(["-map".into(), "[v]".into()]);
            args.extend(["-c:v".into(), encoder.name.clone()]);
            args.extend(quality_args(encoder, crf(options)));
            if codec == VideoCodec::H265 && target != ConvertTarget::Mkv {
                args.extend(["-tag:v".into(), "hvc1".into()]);
            }
        }
        if a == 1 {
            let audio = audio_settings(target, options);
            args.extend(["-map".into(), "[a]".into()]);
            args.extend(["-c:a".into(), audio.codec.encoder().into()]);
            if let Some(kbps) = audio
                .kbps
                .filter(|_| !matches!(target, ConvertTarget::Wav | ConvertTarget::Flac))
            {
                args.extend(["-b:a".into(), format!("{kbps}k")]);
            }
        }
        if (w, h) != (first.width.unwrap_or(w), first.height.unwrap_or(h)) && !sound_only {
            result.changes.push(format!("Resized to {w}×{h}."));
        }
        result
            .changes
            .push("Re-encoded so the files match, in the shape of the first one.".to_owned());
    }
    if matches!(
        target,
        ConvertTarget::Mp4 | ConvertTarget::Mov | ConvertTarget::M4a
    ) {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.extend(output_args(&partial));
    if let Err(err) = run_ffmpeg(ffmpeg, &args, Some(total), control) {
        let _ = std::fs::remove_file(&partial);
        return Err(err);
    }
    finish_partial(&partial, output.path())?;
    result.bytes = std::fs::metadata(output.path()).ok().map(|m| m.len());
    result.changes.insert(
        0,
        format!(
            "{} files joined, {} long.",
            inputs.len(),
            length_label(total)
        ),
    );
    Ok(result)
}

/// the x264 style crf for a 0 to 100 quality, the same scale the converter uses
fn crf(options: &ConvertOptions) -> u8 {
    (40 - options.quality.min(100) * 24 / 100) as u8
}

/// "1 min 5 s", "42 s"
fn length_label(seconds: f64) -> String {
    let whole = seconds.round().max(0.0) as u64;
    match (whole / 60, whole % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// cuts one video or song into parts `split_seconds` long, into a folder next to it
pub fn split(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    target: ConvertTarget,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let ffmpeg = env.ffmpeg()?;
    let sound_only = is_audio(target);
    match info.kind {
        MediaKind::Video => {}
        MediaKind::Audio if sound_only => {}
        _ => {
            return Err(MediaError::Unsupported(format!(
                "{} cannot be cut into {} parts.",
                info.name,
                target.extension().to_uppercase()
            )));
        }
    }
    if sound_only && info.audio.is_empty() {
        return Err(MediaError::Unsupported(format!(
            "{} has no sound to cut.",
            info.name
        )));
    }
    let length = options.split_seconds;
    if !length.is_finite() || length < MIN_PART_SECONDS {
        return Err(MediaError::Unsupported(format!(
            "Parts must be at least {} seconds long.",
            num(MIN_PART_SECONDS)
        )));
    }
    let duration = info.duration_secs.unwrap_or(0.0);
    if duration <= length {
        return Err(MediaError::Unsupported(format!(
            "{} is only {} long, which is shorter than one part. Pick a shorter length.",
            info.name,
            length_label(duration)
        )));
    }

    let folder = naming::folder_for(input, env.output_folder, "parts");
    std::fs::create_dir_all(folder.path()).map_err(|err| MediaError::io(folder.path(), err))?;
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "part".to_owned());
    // the segment muxer reads % as a pattern, so a % in the name has to be doubled
    let pattern = folder.path().join(format!(
        "{} part %03d.{}",
        stem.replace('%', "%%"),
        target.extension()
    ));
    let copy = !options.split_exact && can_remux(info, target, &plain(options));
    let mut args: Vec<String> = vec!["-i".into(), input.display().to_string()];
    if sound_only {
        args.extend(["-map".into(), "0:a:0".into(), "-vn".into()]);
    } else {
        args.extend(["-map".into(), "0:v:0".into()]);
        if options.mute {
            args.push("-an".into());
        } else {
            args.extend(["-map".into(), "0:a:0?".into()]);
        }
    }
    if copy {
        args.extend(["-c".into(), "copy".into()]);
    } else {
        if !sound_only {
            let codec = if target == ConvertTarget::Webm {
                VideoCodec::Vp9
            } else {
                options.codec
            };
            let encoder = env.hardware.software(codec).ok_or_else(|| {
                MediaError::Unsupported(format!(
                    "This copy of FFmpeg cannot encode {}. Pick another codec.",
                    encode::codec_label(codec)
                ))
            })?;
            args.extend([
                "-vf".into(),
                "format=yuv420p".into(),
                "-c:v".into(),
                encoder.name.clone(),
            ]);
            args.extend(quality_args(encoder, crf(options)));
            // a keyframe at every cut, so each part starts exactly where it should
            args.extend([
                "-force_key_frames".into(),
                format!("expr:gte(t,n_forced*{})", num(length)),
            ]);
        }
        if sound_only || !options.mute {
            let audio = audio_settings(target, options);
            args.extend(["-c:a".into(), audio.codec.encoder().into()]);
            if let Some(kbps) = audio
                .kbps
                .filter(|_| !matches!(target, ConvertTarget::Wav | ConvertTarget::Flac))
            {
                args.extend(["-b:a".into(), format!("{kbps}k")]);
            }
        }
    }
    args.extend([
        "-f".into(),
        "segment".into(),
        "-segment_time".into(),
        num(length),
    ]);
    if !copy {
        // aac starts its sound a few milliseconds early, which nudges every cut just past the
        // keyframe made for it. half a frame of slack lets the cut land on it, as the ffmpeg
        // docs suggest for forced keyframes.
        let fps = info
            .video
            .as_ref()
            .and_then(|v| v.frame_rate)
            .filter(|f| f.is_finite() && *f > 0.0)
            .unwrap_or(30.0);
        let delta = (0.5 / fps).min(length / 4.0);
        args.extend(["-segment_time_delta".into(), num(delta)]);
    }
    args.extend([
        "-segment_start_number".into(),
        "1".into(),
        "-reset_timestamps".into(),
        "1".into(),
    ]);
    if matches!(
        target,
        ConvertTarget::Mp4 | ConvertTarget::Mov | ConvertTarget::M4a
    ) {
        args.extend([
            "-segment_format_options".into(),
            "movflags=+faststart".into(),
        ]);
    }
    args.extend(output_args(&pattern));
    if let Err(err) = run_ffmpeg(ffmpeg, &args, Some(duration), control) {
        let _ = std::fs::remove_dir_all(folder.path());
        return Err(err);
    }
    let parts = std::fs::read_dir(folder.path()).map_or(0, Iterator::count);
    let mut result = ToolResult::file(input, folder.path());
    result.folder = true;
    result.bytes = None;
    result.changes.push(format!(
        "Cut into {parts} parts of {}.",
        length_label(length)
    ));
    if copy {
        result.changes.push(
            "Copied without re-encoding, so each cut is at the nearest keyframe and parts can be a little longer or shorter."
                .to_owned(),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_read_well() {
        assert_eq!(length_label(42.4), "42 s");
        assert_eq!(length_label(60.0), "1 min");
        assert_eq!(length_label(65.0), "1 min 5 s");
    }

    #[test]
    fn concat_entries_escape_quotes() {
        assert_eq!(
            concat_entry(Path::new("/a/it's.mp4")),
            "file '/a/it'\\''s.mp4'\n"
        );
    }

    #[test]
    fn joined_clips_take_the_first_shape_inside_the_limits() {
        let (w, h) = (Some(1921), Some(1081));
        let options = ConvertOptions::default();
        assert_eq!(joined_size(w, h, &options), (1920, 1080));
        let smaller = ConvertOptions {
            max_width: Some(1280),
            ..ConvertOptions::default()
        };
        assert_eq!(joined_size(w, h, &smaller), (1280, 720));
    }
}
