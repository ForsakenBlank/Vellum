//! waveform peaks: the loudest and quietest sample in each short stretch of audio, the
//! shape every audio editor draws.
//!
//! peaks are made once at full detail (256 samples each, about 5 ms) and cached. the ui
//! asks for as many columns as it has pixels and gets them merged down from that.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::{FfmpegTools, Reporter, run_reading};
use crate::info::MediaInfo;

/// samples per peak at full detail
const SAMPLES_PER_PEAK: u32 = 256;
/// files longer than this many peaks get coarser ones, so a ten hour recording does not
/// make a hundred megabyte cache entry
const MAX_PEAKS: u64 = 1_000_000;

/// the full detail peaks, as cached
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peaks {
    pub sample_rate: u32,
    /// 1 or 2. more channels are mixed down to stereo.
    pub channels: u32,
    pub samples_per_peak: u32,
    pub duration_secs: f64,
    /// for each peak, for each channel: min then max, scaled to the i16 range
    #[serde(skip)]
    pub values: Vec<i16>,
}

impl Peaks {
    pub fn len(&self) -> usize {
        self.values.len() / (2 * self.channels.max(1) as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// merges peaks down to `columns` columns for drawing
    pub fn view(&self, columns: u32) -> WaveformView {
        let channels = self.channels.max(1) as usize;
        let len = self.len();
        let columns = (columns.max(1) as usize).min(len.max(1));
        let mut peaks = Vec::with_capacity(columns * channels * 2);
        for col in 0..columns {
            let start = col * len / columns;
            let end = ((col + 1) * len / columns).max(start + 1).min(len);
            for ch in 0..channels {
                let mut lo = i16::MAX;
                let mut hi = i16::MIN;
                for peak in start..end {
                    let at = (peak * channels + ch) * 2;
                    lo = lo.min(self.values[at]);
                    hi = hi.max(self.values[at + 1]);
                }
                if start >= end {
                    (lo, hi) = (0, 0);
                }
                peaks.push(f32::from(lo) / 32767.0);
                peaks.push(f32::from(hi) / 32767.0);
            }
        }
        WaveformView {
            channels: self.channels,
            duration_secs: self.duration_secs,
            columns: u32::try_from(columns).unwrap_or(u32::MAX),
            peaks,
        }
    }

    /// the values as little endian bytes, for the cache blob
    pub fn to_bytes(&self) -> Vec<u8> {
        self.values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    pub fn set_bytes(&mut self, bytes: &[u8]) {
        self.values = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
    }
}

/// what the ui draws
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WaveformView {
    pub channels: u32,
    pub duration_secs: f64,
    pub columns: u32,
    /// for each column, for each channel: min then max, from -1 to 1
    pub peaks: Vec<f32>,
}

/// decodes the first audio stream and measures its peaks
pub fn make_peaks(
    tools: &FfmpegTools,
    path: &Path,
    info: &MediaInfo,
    reporter: &dyn Reporter,
) -> MediaResult<Peaks> {
    let audio = info
        .audio
        .as_ref()
        .ok_or_else(|| MediaError::Unsupported("This file has no sound.".into()))?;
    let channels = audio.channels.clamp(1, 2);
    let sample_rate = audio.sample_rate.max(1);
    let duration = audio.duration_secs.or(info.duration_secs).unwrap_or(0.0);
    let total_frames = (duration * f64::from(sample_rate)) as u64;
    let samples_per_peak =
        u32::try_from((total_frames / MAX_PEAKS).max(u64::from(SAMPLES_PER_PEAK)))
            .unwrap_or(u32::MAX);

    let mut cmd = tools.ffmpeg();
    cmd.arg("-i")
        .arg(path)
        .args(["-map", &format!("0:{}", audio.stream_index)])
        .args(["-vn", "-sn", "-ac", &channels.to_string()])
        .args(["-c:a", "pcm_f32le", "-f", "f32le", "pipe:1"]);
    let (values, frames_read) = run_reading(&mut cmd, "ffmpeg", |stdout| {
        let mut builder = PeakBuilder::new(channels as usize, samples_per_peak as usize);
        let mut buf = vec![0u8; 64 * 1024];
        let mut carry: Vec<u8> = Vec::new();
        loop {
            reporter.checkpoint()?;
            let n = stdout
                .read(&mut buf)
                .map_err(|err| MediaError::io(path, err))?;
            if n == 0 {
                break;
            }
            // reads can split a sample, so keep the leftover bytes for next time
            carry.extend_from_slice(&buf[..n]);
            let whole = carry.len() / 4 * 4;
            for sample in carry[..whole].chunks_exact(4) {
                builder.push(f32::from_le_bytes([
                    sample[0], sample[1], sample[2], sample[3],
                ]));
            }
            carry.drain(..whole);
            if total_frames > 0 {
                reporter.progress((builder.frames as f64 / total_frames as f64).min(1.0) as f32);
            }
        }
        Ok(builder.finish())
    })?;
    Ok(Peaks {
        sample_rate,
        channels,
        samples_per_peak,
        duration_secs: if duration > 0.0 {
            duration
        } else {
            frames_read as f64 / f64::from(sample_rate)
        },
        values,
    })
}

struct PeakBuilder {
    channels: usize,
    per_peak: usize,
    lo: Vec<f32>,
    hi: Vec<f32>,
    channel: usize,
    in_peak: usize,
    frames: u64,
    values: Vec<i16>,
}

impl PeakBuilder {
    fn new(channels: usize, per_peak: usize) -> Self {
        Self {
            channels,
            per_peak,
            lo: vec![f32::MAX; channels],
            hi: vec![f32::MIN; channels],
            channel: 0,
            in_peak: 0,
            frames: 0,
            values: Vec::new(),
        }
    }

    fn push(&mut self, sample: f32) {
        let sample = if sample.is_finite() { sample } else { 0.0 };
        self.lo[self.channel] = self.lo[self.channel].min(sample);
        self.hi[self.channel] = self.hi[self.channel].max(sample);
        self.channel += 1;
        if self.channel == self.channels {
            self.channel = 0;
            self.frames += 1;
            self.in_peak += 1;
            if self.in_peak == self.per_peak {
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        for ch in 0..self.channels {
            self.values.push(to_i16(self.lo[ch]));
            self.values.push(to_i16(self.hi[ch]));
            self.lo[ch] = f32::MAX;
            self.hi[ch] = f32::MIN;
        }
        self.in_peak = 0;
    }

    fn finish(mut self) -> (Vec<i16>, u64) {
        if self.in_peak > 0 {
            self.flush();
        }
        (self.values, self.frames)
    }
}

fn to_i16(value: f32) -> i16 {
    (value.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{NoReporter, probe_media, testing};

    #[test]
    fn peaks_track_the_loudest_and_quietest_samples() {
        let mut builder = PeakBuilder::new(2, 2);
        for sample in [0.5, -0.1, -0.25, 0.2, 1.5, 0.0] {
            builder.push(sample);
        }
        let (values, frames) = builder.finish();
        assert_eq!(frames, 3);
        // peak 1: left min -0.25 max 0.5, right min -0.1 max 0.2
        // peak 2 (partial): left 1.0 (clamped), right 0.0
        assert_eq!(values, [-8192, 16384, -3277, 6553, 32767, 32767, 0, 0]);
    }

    #[test]
    fn views_merge_columns_without_losing_extremes() {
        let peaks = Peaks {
            sample_rate: 48000,
            channels: 1,
            samples_per_peak: 256,
            duration_secs: 1.0,
            values: vec![-10, 10, -20, 5, -1, 30, -5, 5],
        };
        let view = peaks.view(2);
        assert_eq!(view.columns, 2);
        let scaled: Vec<i32> = view
            .peaks
            .iter()
            .map(|v| (v * 32767.0).round() as i32)
            .collect();
        assert_eq!(scaled, [-20, 10, -5, 30]);
        // asking for more columns than peaks gives one per peak
        assert_eq!(peaks.view(100).columns, 4);
        let mut copy = peaks.clone();
        copy.set_bytes(&peaks.to_bytes());
        assert_eq!(copy.values, peaks.values);
    }

    #[test]
    fn a_real_tone_has_the_expected_loudness() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        // a half volume 440 hz tone in the left channel, silence in the right. ffmpeg's sine
        // source plays at an eighth of full scale, so four times louder is half volume.
        testing::make(
            &tools,
            "-f lavfi -i sine=f=440:d=2:sample_rate=48000 -af volume=4,pan=stereo|c0=c0|c1=0*c0",
            &path,
        );
        let info = probe_media(&tools, &path).unwrap();
        let peaks = make_peaks(&tools, &path, &info, &NoReporter).unwrap();
        assert_eq!(peaks.channels, 2);
        assert_eq!(peaks.len(), 96000 / 256);
        let view = peaks.view(10);
        for column in view.peaks.chunks_exact(4) {
            let (left_lo, left_hi, right_lo, right_hi) =
                (column[0], column[1], column[2], column[3]);
            assert!(
                (left_hi - 0.5).abs() < 0.05 && (left_lo + 0.5).abs() < 0.05,
                "{column:?}"
            );
            assert!(right_hi.abs() < 0.01 && right_lo.abs() < 0.01, "{column:?}");
        }
    }
}
