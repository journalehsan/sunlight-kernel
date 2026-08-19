#!/usr/bin/env bash
set -euo pipefail

# Deterministic two-second Ogg Vorbis fixture for Melody Mina runtime checks.
# Left is 440 Hz and right is 660 Hz, making channel swaps/collapse audible.
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output="$repo_root/assets/sounds/melody-mina-test-48k-stereo.ogg"

ffmpeg -hide_banner -loglevel error -y \
    -f lavfi \
    -i "aevalsrc=0.20*sin(2*PI*440*t)|0.20*sin(2*PI*660*t):s=48000:d=2:c=stereo" \
    -c:a libvorbis -q:a 3 -map_metadata -1 "$output"

ffprobe -v error -select_streams a:0 \
    -show_entries stream=codec_name,sample_rate,channels,duration \
    -of default=noprint_wrappers=1 "$output"
