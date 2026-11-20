#!/usr/bin/env bash
# makes a folder of small media files covering every format vellum reads, for trying the
# app by hand. nothing here is committed: run it again whenever the folder is gone.
#
#   tests/media/generate.sh [output folder]
#
# needs ffmpeg (6.0 or newer). heif-enc from libheif adds a heic photo when it is installed.
# the rust tests make their own files the same way, so they do not need this script.

set -euo pipefail

out="${1:-$(dirname "$0")/out}"
mkdir -p "$out"
cd "$out"

ff() { ffmpeg -hide_banner -loglevel error -y "$@"; }
# read once: grep -q on a live pipe would stop ffmpeg early and trip pipefail
encoders="$(ffmpeg -hide_banner -encoders 2>/dev/null)"
has_encoder() { grep -q " $1 " <<<"$encoders"; }

echo "writing test media to $(pwd)"

# stills
ff -f lavfi -i testsrc2=s=640x480 -frames:v 1 photo.png
ff -i photo.png -q:v 3 photo.jpg
ff -i photo.png photo.webp
ff -i photo.png photo.tiff
ff -i photo.png photo.bmp
ff -f lavfi -i "color=c=red@0.5:s=320x240,format=rgba" -frames:v 1 see-through.png
# a jpeg taken "sideways": stored landscape, shown portrait through its exif orientation.
# ffmpeg cannot write exif, so this one is a plain jpeg turned by its pixels instead.
ff -i photo.png -vf transpose=1 -q:v 3 portrait.jpg
if has_encoder libaom-av1; then
  ff -i photo.png -c:v libaom-av1 -still-picture 1 photo.avif
fi
if command -v heif-enc >/dev/null 2>&1; then
  heif-enc -q 80 -o photo.heic photo.png >/dev/null 2>&1 || echo "skipped heic: heif-enc has no hevc encoder"
fi
cat > logo.svg <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80" viewBox="0 0 120 80">
  <rect width="120" height="80" rx="12" fill="#1d1a16"/>
  <circle cx="40" cy="40" r="22" fill="#e8a23a"/>
  <text x="70" y="48" font-family="sans-serif" font-size="20" fill="#f3ede4">V</text>
</svg>
SVG

# animations
ff -f lavfi -i testsrc2=s=320x240:r=12:d=2 -vf "split[a][b];[a]palettegen[p];[b][p]paletteuse" -loop 0 animation.gif
ff -f lavfi -i testsrc2=s=320x240:r=12:d=2 -loop 0 animation.webp
ff -f lavfi -i testsrc2=s=320x240:r=12:d=2 -plays 0 -f apng animation.png

# video
ff -f lavfi -i testsrc2=s=1280x720:r=30:d=6 -f lavfi -i sine=f=440:d=6 \
  -c:v libx264 -pix_fmt yuv420p -c:a aac -metadata title="Holiday" \
  -metadata location="+51.5074-000.1278/" clip-h264.mp4
# a phone clip: filmed sideways, ten bit hevc tagged as hlg hdr
if has_encoder libx265; then
  ff -f lavfi -i testsrc2=s=1920x1080:r=30:d=4 -c:v libx265 -preset ultrafast \
    -x265-params log-level=error -pix_fmt yuv420p10le -tag:v hvc1 \
    -color_primaries bt2020 -color_trc arib-std-b67 -colorspace bt2020nc flat-hevc.mp4
  ff -display_rotation 90 -i flat-hevc.mp4 -c copy phone-hevc-hlg.mp4
  rm flat-hevc.mp4
fi
# variable frame rate: 30 fps for a second, then 10
ff -f lavfi -i testsrc2=s=640x360:r=30:d=3 \
  -vf "setpts='if(lt(N,30),N/30,1+(N-30)/10)/TB'" -fps_mode vfr -c:v libx264 variable-fps.mkv
if has_encoder libvpx-vp9; then
  ff -f lavfi -i testsrc2=s=640x360:r=25:d=3 -f lavfi -i sine=d=3 -c:v libvpx-vp9 -b:v 500k \
    -c:a libopus clip.webm
fi
if has_encoder prores_ks; then
  ff -f lavfi -i testsrc2=s=640x360:r=25:d=2 -c:v prores_ks -profile:v 0 clip-prores.mov
fi
# two black seconds first, so the thumbnail has to look further in
ff -f lavfi -i color=c=black:s=640x360:r=25:d=2 -f lavfi -i testsrc2=s=640x360:r=25:d=3 \
  -filter_complex "[0][1]concat=n=2:v=1" -c:v libx264 fade-in.mp4

# audio
ff -f lavfi -i "sine=f=330:d=5,volume=4" tone.wav
ff -f lavfi -i "sine=f=330:d=5,volume=4" -c:a flac tone.flac
ff -f lavfi -i "sine=f=330:d=5,volume=4" -c:a libopus tone.opus 2>/dev/null || true
ff -f lavfi -i "sine=f=330:d=5,volume=4" -c:a aac tone.m4a
ff -f lavfi -i "sine=f=330:d=5,volume=4" -i photo.png -map 0 -map 1 -c:a libmp3lame \
  -c:v mjpeg -disposition:v attached_pic -metadata artist="Someone" song-with-cover.mp3

# damaged files, to check the errors read well
printf 'this is not a video' > broken.mp4
head -c 2000 photo.jpg > cut-short.jpg

echo "done: $(ls | wc -l | tr -d ' ') files"
