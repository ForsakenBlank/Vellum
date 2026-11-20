//! small test files in every format vellum reads, made on demand with ffmpeg and a few
//! hand written encoders, so the repo carries no binary media.
//!
//! the tests use this, and so does `scripts/make-test-media.sh` for poking at files by hand.
//! each file is only made if this ffmpeg has the encoder for it; [`TestMedia::get`] returns
//! none otherwise so a test can skip with a reason.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::{Ffmpeg, process};

/// one entry per file: name, then ffmpeg arguments that write it (the output path is added)
const FFMPEG_FILES: &[(&str, &[&str])] = &[
    (
        "image.png",
        &["-f", "lavfi", "-i", "testsrc2=s=320x240", "-frames:v", "1"],
    ),
    (
        "alpha.png",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red@0.5:s=64x48,format=rgba",
            "-frames:v",
            "1",
        ],
    ),
    (
        "big.png",
        &[
            "-f",
            "lavfi",
            "-i",
            "gradients=s=4000x3000:seed=1",
            "-frames:v",
            "1",
        ],
    ),
    (
        "photo.jpg",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x480",
            "-frames:v",
            "1",
            "-q:v",
            "3",
        ],
    ),
    (
        "big.jpg",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=4000x3000",
            "-frames:v",
            "1",
            "-q:v",
            "5",
        ],
    ),
    (
        "image.webp",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240",
            "-frames:v",
            "1",
            "-c:v",
            "libwebp",
        ],
    ),
    (
        "anim.webp",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x120:r=10:d=0.5",
            "-c:v",
            "libwebp_anim",
            "-loop",
            "0",
        ],
    ),
    (
        "image.avif",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240",
            "-frames:v",
            "1",
            "-c:v",
            "libaom-av1",
            "-still-picture",
            "1",
            "-cpu-used",
            "8",
        ],
    ),
    (
        "image.jxl",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240",
            "-frames:v",
            "1",
            "-c:v",
            "libjxl",
        ],
    ),
    (
        "image.tiff",
        &["-f", "lavfi", "-i", "testsrc2=s=320x240", "-frames:v", "1"],
    ),
    (
        "image.bmp",
        &["-f", "lavfi", "-i", "testsrc2=s=320x240", "-frames:v", "1"],
    ),
    (
        "image.tga",
        &["-f", "lavfi", "-i", "testsrc2=s=320x240", "-frames:v", "1"],
    ),
    (
        "image.qoi",
        &["-f", "lavfi", "-i", "testsrc2=s=320x240", "-frames:v", "1"],
    ),
    (
        "anim.gif",
        &["-f", "lavfi", "-i", "testsrc2=s=160x120:r=10:d=1"],
    ),
    (
        "anim.png",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x120:r=10:d=0.5",
            "-f",
            "apng",
            "-plays",
            "0",
        ],
    ),
    (
        "h264.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=30:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:r=48000:d=2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-ac",
            "2",
            "-shortest",
            "-metadata",
            "title=Test clip",
        ],
    ),
    (
        "h265.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=25:d=2",
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-x265-params",
            "log-level=error",
            "-tag:v",
            "hvc1",
            "-pix_fmt",
            "yuv420p",
        ],
    ),
    (
        "vp9.webm",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=24:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=330:r=48000:d=2",
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-b:v",
            "300k",
            "-c:a",
            "libopus",
            "-shortest",
        ],
    ),
    (
        // timestamps speed up halfway through: frames 0 to 29 at 30 fps, then 60 fps
        "vfr.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x120:r=30:d=3",
            "-vf",
            "settb=1/1000,setpts='if(lt(N,30),N/30,1+(N-30)/60)/TB'",
            "-fps_mode",
            "passthrough",
            "-enc_time_base:v",
            "1/1000",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ],
    ),
    (
        // every frame a flat grey of 8 times its number, so a test can tell which frame it got.
        // lossless and one keyframe only, so seeking has to decode its way forward.
        "frames.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x48:r=30:d=1,format=gray,geq=lum='N*8'",
            "-c:v",
            "libx264",
            "-qp",
            "0",
            "-g",
            "1000",
            "-pix_fmt",
            "yuv444p",
            "-preset",
            "ultrafast",
        ],
    ),
    (
        "hdr10.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=25:d=1",
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p10le",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
            "-x265-params",
            "log-level=error:hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,50):max-cll=1000,400",
        ],
    ),
    (
        "tone.wav",
        &["-f", "lavfi", "-i", "sine=f=440:r=48000:d=2", "-ac", "2"],
    ),
    (
        "tone.mp3",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:r=44100:d=2",
            "-c:a",
            "libmp3lame",
            "-metadata",
            "artist=Vellum",
        ],
    ),
    (
        "tone.flac",
        &["-f", "lavfi", "-i", "sine=f=440:r=48000:d=2"],
    ),
    (
        "tone.opus",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:r=48000:d=2",
            "-c:a",
            "libopus",
        ],
    ),
];

/// files made without ffmpeg, by code in this module
const HANDMADE_FILES: &[&str] = &[
    "exif.jpg",
    "rotated.mp4",
    "photo.dng",
    "layers.psd",
    "drawing.svg",
];

/// a folder of generated test media
pub struct TestMedia {
    dir: PathBuf,
    made: BTreeMap<String, PathBuf>,
    /// files this ffmpeg could not make, with the reason
    pub skipped: BTreeMap<String, String>,
}

impl TestMedia {
    /// makes every file into `dir`, reusing files that are already there
    pub fn make(ffmpeg: &Ffmpeg, dir: &Path) -> MediaResult<Self> {
        std::fs::create_dir_all(dir).map_err(|err| MediaError::io(dir, err))?;
        let mut media = Self {
            dir: dir.to_path_buf(),
            made: BTreeMap::new(),
            skipped: BTreeMap::new(),
        };
        for (name, args) in FFMPEG_FILES {
            let path = dir.join(name);
            if !path.is_file()
                && let Err(err) = make_with_ffmpeg(ffmpeg, &path, args)
            {
                media.skipped.insert((*name).to_owned(), err.to_string());
                continue;
            }
            media.made.insert((*name).to_owned(), path);
        }
        for name in HANDMADE_FILES {
            let path = dir.join(name);
            let result = if path.is_file() {
                Ok(())
            } else {
                make_by_hand(&media, name, &path)
            };
            match result {
                Ok(()) => {
                    media.made.insert((*name).to_owned(), path);
                }
                Err(err) => {
                    media.skipped.insert((*name).to_owned(), err.to_string());
                }
            }
        }
        Ok(media)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// the path of a file, or none if this machine could not make it
    pub fn get(&self, name: &str) -> Option<&Path> {
        self.made.get(name).map(PathBuf::as_path)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.made.keys().map(String::as_str)
    }
}

fn make_with_ffmpeg(ffmpeg: &Ffmpeg, path: &Path, args: &[&str]) -> MediaResult<()> {
    // write under a temporary name so two test binaries making the same file never see a
    // half written one
    let partial = crate::ffmpeg::encode::partial_path(path);
    let mut cmd = ffmpeg.command();
    cmd.args(args).arg("-y").arg(&partial);
    match process::capture(cmd, Duration::from_secs(120)) {
        Ok(_) => std::fs::rename(&partial, path).map_err(|err| MediaError::io(path, err)),
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

fn make_by_hand(media: &TestMedia, name: &str, path: &Path) -> MediaResult<()> {
    let bytes = match name {
        "exif.jpg" => {
            let source = media
                .get("photo.jpg")
                .ok_or_else(|| MediaError::Unsupported("photo.jpg was not made".into()))?;
            let jpeg = std::fs::read(source).map_err(|err| MediaError::io(source, err))?;
            jpeg_with_exif(&jpeg, 6, Some((51.5, -0.12)))
        }
        "rotated.mp4" => {
            let source = media
                .get("h264.mp4")
                .ok_or_else(|| MediaError::Unsupported("h264.mp4 was not made".into()))?;
            let mp4 = std::fs::read(source).map_err(|err| MediaError::io(source, err))?;
            mp4_rotated_90(mp4)?
        }
        "photo.dng" => linear_dng(96, 64)?,
        "layers.psd" => flat_psd(48, 32, [200, 40, 30]),
        "drawing.svg" => SVG.as_bytes().to_vec(),
        _ => return Err(MediaError::Unsupported(format!("no recipe for {name}"))),
    };
    let partial = crate::ffmpeg::encode::partial_path(path);
    std::fs::write(&partial, bytes).map_err(|err| MediaError::io(&partial, err))?;
    std::fs::rename(&partial, path).map_err(|err| MediaError::io(path, err))
}

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80" viewBox="0 0 120 80">
  <rect width="120" height="80" fill="#2b2b2b"/>
  <circle cx="60" cy="40" r="30" fill="#f2a93b"/>
</svg>
"##;

/// puts an exif block with an orientation and optional gps position into a jpeg, right
/// after the start marker. a small hand built tiff, since ffmpeg cannot write exif.
pub fn jpeg_with_exif(jpeg: &[u8], orientation: u16, gps: Option<(f64, f64)>) -> Vec<u8> {
    // little endian tiff: header, ifd0, then the gps ifd and its values
    let mut tiff: Vec<u8> = b"II*\0".to_vec();
    tiff.extend(8_u32.to_le_bytes());
    let entries: u16 = if gps.is_some() { 2 } else { 1 };
    let ifd0_len = 2 + 12 * u32::from(entries) + 4;
    let gps_offset = 8 + ifd0_len;
    tiff.extend(entries.to_le_bytes());
    // orientation, short, count 1, value inline
    tiff.extend(0x0112_u16.to_le_bytes());
    tiff.extend(3_u16.to_le_bytes());
    tiff.extend(1_u32.to_le_bytes());
    tiff.extend(u32::from(orientation).to_le_bytes());
    if gps.is_some() {
        // gps ifd pointer, long
        tiff.extend(0x8825_u16.to_le_bytes());
        tiff.extend(4_u16.to_le_bytes());
        tiff.extend(1_u32.to_le_bytes());
        tiff.extend(gps_offset.to_le_bytes());
    }
    tiff.extend(0_u32.to_le_bytes());
    if let Some((lat, lon)) = gps {
        // four entries: lat ref, lat, lon ref, lon. rationals live after the ifd.
        let gps_len = 2 + 12 * 4 + 4;
        let values = gps_offset + gps_len;
        tiff.extend(4_u16.to_le_bytes());
        let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
            tiff.extend(tag.to_le_bytes());
            tiff.extend(kind.to_le_bytes());
            tiff.extend(count.to_le_bytes());
            tiff.extend(value.to_le_bytes());
        };
        let lat_ref = if lat >= 0.0 { b'N' } else { b'S' };
        let lon_ref = if lon >= 0.0 { b'E' } else { b'W' };
        entry(1, 2, 2, u32::from(lat_ref));
        entry(2, 5, 3, values);
        entry(3, 2, 2, u32::from(lon_ref));
        entry(4, 5, 3, values + 24);
        tiff.extend(0_u32.to_le_bytes());
        for degrees in [lat.abs(), lon.abs()] {
            let whole = degrees.trunc();
            let minutes = (degrees - whole) * 60.0;
            for (n, d) in [(whole as u32, 1_u32), (minutes.trunc() as u32, 1), (0, 1)] {
                tiff.extend(n.to_le_bytes());
                tiff.extend(d.to_le_bytes());
            }
        }
    }
    let mut segment = b"Exif\0\0".to_vec();
    segment.extend(tiff);
    let len = u16::try_from(segment.len() + 2).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(jpeg.len() + segment.len() + 4);
    out.extend(&jpeg[..2]);
    out.extend([0xff, 0xe1]);
    out.extend(len.to_be_bytes());
    out.extend(segment);
    out.extend(&jpeg[2..]);
    out
}

/// turns the first track of an mp4 a quarter turn by rewriting its display matrix, the way
/// a phone held upright marks its videos. ffmpeg before 7.0 has no option for this.
pub fn mp4_rotated_90(mut mp4: Vec<u8>) -> MediaResult<Vec<u8>> {
    let at = mp4
        .windows(4)
        .position(|w| w == b"tkhd")
        .ok_or_else(|| MediaError::Unsupported("the mp4 has no track header".into()))?;
    let version = mp4.get(at + 4).copied().unwrap_or(0);
    // version, flags, then times, track id and duration are 32 or 64 bit
    let matrix = at + 4 + 4 + if version == 1 { 32 } else { 20 } + 8 + 8;
    let values: [i32; 9] = [0, 0x0001_0000, 0, -0x0001_0000, 0, 0, 0, 0, 0x4000_0000];
    let slot = mp4
        .get_mut(matrix..matrix + 36)
        .ok_or_else(|| MediaError::Unsupported("the mp4 track header is cut short".into()))?;
    for (chunk, value) in slot.chunks_exact_mut(4).zip(values) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    Ok(mp4)
}

/// a linear (already demosaiced) dng with a horizontal gradient, written with rawler's own
/// dng writer. real camera files cannot be generated, so this checks the raw path end to
/// end; vendor formats are rawler's job and covered by its own test suite.
fn linear_dng(width: usize, height: usize) -> MediaResult<Vec<u8>> {
    use rawler::dng::writer::DngWriter;
    use rawler::dng::{DNG_VERSION_V1_4, DngCompression};
    use rawler::formats::tiff::{Rational, SRational};
    use rawler::imgop::xyz::Illuminant;
    use rawler::tags::{DngTag, TiffCommonTag};

    let bad = |err: rawler::formats::tiff::TiffError| MediaError::Unsupported(err.to_string());
    let mut pixels = Vec::with_capacity(width * height * 3);
    for _y in 0..height {
        for x in 0..width {
            let v = ((x * 60_000) / width.max(1)) as u16;
            pixels.extend([v, v / 2, u16::MAX - v]);
        }
    }
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4).map_err(bad)?;
    let mut frame = dng.subframe_on_root(0);
    frame
        .rgb_image_u16(&pixels, width, height, DngCompression::Uncompressed, 1)
        .map_err(bad)?;
    frame.finalize().map_err(bad)?;
    dng.root_ifd_mut().add_tag(TiffCommonTag::Make, "Vellum");
    dng.root_ifd_mut()
        .add_tag(TiffCommonTag::Model, "Test Camera");
    dng.root_ifd_mut()
        .add_tag(DngTag::UniqueCameraModel, "Vellum Test Camera");
    // identity colour matrix and neutral white balance, so colours pass straight through
    let one = SRational::new(1, 1);
    let zero = SRational::new(0, 1);
    dng.color_matrix(
        1,
        Illuminant::D65,
        [one, zero, zero, zero, one, zero, zero, zero, one],
    );
    dng.as_shot_neutral([
        Rational::new(1, 1),
        Rational::new(1, 1),
        Rational::new(1, 1),
    ]);
    dng.close().map_err(bad)?;
    Ok(buf.into_inner())
}

/// a minimal 8 bit rgb psd with no layers, just the flattened image filled with one colour
pub fn flat_psd(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(b"8BPS");
    out.extend(1_u16.to_be_bytes()); // version
    out.extend([0_u8; 6]); // reserved
    out.extend(3_u16.to_be_bytes()); // channels
    out.extend(height.to_be_bytes());
    out.extend(width.to_be_bytes());
    out.extend(8_u16.to_be_bytes()); // bits per channel
    out.extend(3_u16.to_be_bytes()); // rgb colour mode
    out.extend(0_u32.to_be_bytes()); // colour mode data
    out.extend(0_u32.to_be_bytes()); // image resources
    out.extend(0_u32.to_be_bytes()); // layer and mask info
    out.extend(0_u16.to_be_bytes()); // raw, uncompressed image data
    let plane = (width * height) as usize;
    for value in rgb {
        out.extend(std::iter::repeat_n(value, plane));
    }
    out
}
