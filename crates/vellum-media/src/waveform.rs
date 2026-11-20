//! waveform peaks: the smallest and largest sample in every block of audio, which is all a
//! waveform display needs at any zoom level coarser than single samples.
//!
//! the whole file is decoded once to 16 bit pcm and folded into peaks as it streams past, so
//! memory stays small even for a two hour recording. the peaks are cached as a compact
//! binary file and read back for drawing.

use std::path::Path;

use serde::Serialize;
use ts_rs::TS;

use crate::control::Control;
use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::Ffmpeg;
use crate::ffmpeg::process::Running;
use crate::probe::MediaInfo;

/// samples folded into one peak. about 10 ms at 48 kHz, fine enough for the audio editor to
/// zoom in a long way before it needs the samples themselves.
pub const SAMPLES_PER_PEAK: u32 = 512;
/// more channels than this are mixed down to stereo for the overview
const MAX_CHANNELS: u16 = 8;
const MAGIC: &[u8; 4] = b"VWAV";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waveform {
    pub channels: u16,
    pub sample_rate: u32,
    pub samples_per_peak: u32,
    /// `[min, max]` per block per channel, channels interleaved: block 0 channel 0, block 0
    /// channel 1, block 1 channel 0, ...
    pub peaks: Vec<[i16; 2]>,
}

/// a waveform squeezed into a fixed number of columns, ready to draw
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WaveformOverview {
    pub channels: u16,
    pub columns: u32,
    /// `min, max` pairs from -1 to 1, per column per channel, channels interleaved
    pub peaks: Vec<f32>,
    pub duration_secs: f64,
}

impl Waveform {
    pub fn generate(
        ffmpeg: &Ffmpeg,
        path: &Path,
        info: &MediaInfo,
        control: &dyn Control,
    ) -> MediaResult<Self> {
        let audio = info
            .audio
            .first()
            .ok_or_else(|| MediaError::decode(path, "it has no audio"))?;
        let source_channels = u16::try_from(audio.channels).unwrap_or(2).max(1);
        let channels = if source_channels > MAX_CHANNELS {
            2
        } else {
            source_channels
        };
        let sample_rate = audio.sample_rate.unwrap_or(48_000).max(1);
        let mut cmd = ffmpeg.command();
        cmd.arg("-i")
            .arg(path)
            .args(["-map", "0:a:0", "-ac", &channels.to_string()])
            .args(["-f", "s16le", "-acodec", "pcm_s16le", "pipe:1"]);

        let expected_bytes = info
            .duration_secs
            .map(|d| d * f64::from(sample_rate) * f64::from(channels) * 2.0)
            .filter(|b| *b > 0.0);
        let mut folder = Folder::new(channels);
        let mut leftover: Option<u8> = None;
        let mut seen = 0_u64;
        let mut running = Running::spawn(cmd)?;
        running.stream(control, |chunk| {
            seen += chunk.len() as u64;
            let mut bytes = chunk;
            if let Some(first) = leftover.take() {
                let Some((&second, rest)) = bytes.split_first() else {
                    leftover = Some(first);
                    return Ok(());
                };
                folder.push(i16::from_le_bytes([first, second]));
                bytes = rest;
            }
            let pairs = bytes.chunks_exact(2);
            leftover = pairs.remainder().first().copied();
            for pair in pairs {
                folder.push(i16::from_le_bytes([pair[0], pair[1]]));
            }
            if let Some(total) = expected_bytes {
                control.progress((seen as f64 / total).min(1.0) as f32);
            }
            Ok(())
        })?;
        running.finish()?;
        Ok(Self {
            channels,
            sample_rate,
            samples_per_peak: SAMPLES_PER_PEAK,
            peaks: folder.finish(),
        })
    }

    /// how many blocks each channel has
    pub fn blocks(&self) -> usize {
        self.peaks.len() / usize::from(self.channels.max(1))
    }

    pub fn duration_secs(&self) -> f64 {
        self.blocks() as f64 * f64::from(self.samples_per_peak) / f64::from(self.sample_rate.max(1))
    }

    /// folds the blocks into `columns` columns
    pub fn overview(&self, columns: u32) -> WaveformOverview {
        let channels = usize::from(self.channels.max(1));
        let blocks = self.blocks();
        let columns_usize = (columns.max(1) as usize).min(blocks.max(1));
        let mut peaks = Vec::with_capacity(columns_usize * channels * 2);
        for column in 0..columns_usize {
            let from = column * blocks / columns_usize;
            let to = ((column + 1) * blocks / columns_usize)
                .max(from + 1)
                .min(blocks);
            for channel in 0..channels {
                let mut low = i16::MAX;
                let mut high = i16::MIN;
                for block in from..to {
                    let [lo, hi] = self.peaks[block * channels + channel];
                    low = low.min(lo);
                    high = high.max(hi);
                }
                if from >= to {
                    (low, high) = (0, 0);
                }
                peaks.push(f32::from(low) / 32768.0);
                peaks.push(f32::from(high) / 32768.0);
            }
        }
        WaveformOverview {
            channels: self.channels,
            columns: u32::try_from(columns_usize).unwrap_or(u32::MAX),
            peaks,
            duration_secs: self.duration_secs(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.peaks.len() * 4);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&self.samples_per_peak.to_le_bytes());
        out.extend_from_slice(&(self.peaks.len() as u64).to_le_bytes());
        for [lo, hi] in &self.peaks {
            out.extend_from_slice(&lo.to_le_bytes());
            out.extend_from_slice(&hi.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let rest = bytes.strip_prefix(MAGIC)?;
        let channels = u16::from_le_bytes(rest.get(0..2)?.try_into().ok()?);
        let sample_rate = u32::from_le_bytes(rest.get(2..6)?.try_into().ok()?);
        let samples_per_peak = u32::from_le_bytes(rest.get(6..10)?.try_into().ok()?);
        let count = u64::from_le_bytes(rest.get(10..18)?.try_into().ok()?);
        let body = rest.get(18..)?;
        if channels == 0 || body.len() as u64 != count.checked_mul(4)? {
            return None;
        }
        let peaks = body
            .chunks_exact(4)
            .map(|c| {
                [
                    i16::from_le_bytes([c[0], c[1]]),
                    i16::from_le_bytes([c[2], c[3]]),
                ]
            })
            .collect();
        Some(Self {
            channels,
            sample_rate,
            samples_per_peak,
            peaks,
        })
    }
}

/// folds interleaved samples into per channel min and max blocks
struct Folder {
    channels: usize,
    current: Vec<[i16; 2]>,
    channel: usize,
    frames: u32,
    out: Vec<[i16; 2]>,
}

impl Folder {
    fn new(channels: u16) -> Self {
        let channels = usize::from(channels.max(1));
        Self {
            channels,
            current: vec![[i16::MAX, i16::MIN]; channels],
            channel: 0,
            frames: 0,
            out: Vec::new(),
        }
    }

    fn push(&mut self, sample: i16) {
        let cell = &mut self.current[self.channel];
        cell[0] = cell[0].min(sample);
        cell[1] = cell[1].max(sample);
        self.channel += 1;
        if self.channel == self.channels {
            self.channel = 0;
            self.frames += 1;
            if self.frames == SAMPLES_PER_PEAK {
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        self.out.extend_from_slice(&self.current);
        self.current.fill([i16::MAX, i16::MIN]);
        self.frames = 0;
    }

    fn finish(mut self) -> Vec<[i16; 2]> {
        if self.frames > 0 {
            self.flush();
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_fold_into_blocks_per_channel() {
        let mut folder = Folder::new(2);
        for i in 0..SAMPLES_PER_PEAK + 10 {
            let v = i16::try_from(i % 100).unwrap();
            folder.push(v); // left goes up
            folder.push(-v); // right goes down
        }
        let peaks = folder.finish();
        assert_eq!(peaks.len(), 4);
        assert_eq!(peaks[0], [0, 99]);
        assert_eq!(peaks[1], [-99, 0]);
        assert_eq!(peaks[2], [12, 21]);
    }

    #[test]
    fn overview_squeezes_and_bytes_round_trip() {
        let wave = Waveform {
            channels: 1,
            sample_rate: 512,
            samples_per_peak: SAMPLES_PER_PEAK,
            peaks: vec![[-100, 100], [-16384, 32767], [0, 0], [-32768, 10]],
        };
        assert!((wave.duration_secs() - 4.0).abs() < 1e-9);
        let view = wave.overview(2);
        assert_eq!(view.columns, 2);
        assert_eq!(
            view.peaks,
            vec![-0.5, 32767.0 / 32768.0, -1.0, 10.0 / 32768.0]
        );
        // asking for more columns than blocks gives one column per block
        assert_eq!(wave.overview(100).columns, 4);
        assert_eq!(Waveform::from_bytes(&wave.to_bytes()), Some(wave));
        assert_eq!(Waveform::from_bytes(b"VWAV"), None);
    }
}
