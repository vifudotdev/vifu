#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
VIDEO="$ROOT/recordings/last-train-gameplay-video.webm"
OUTPUT="$ROOT/recordings/last-train-gameplay.mp4"

ffmpeg -y \
  -i "$VIDEO" \
  -stream_loop -1 -i "$ROOT/assets/audio/music/rain-train-ambience.mp3" \
  -stream_loop -1 -i "$ROOT/assets/audio/music/star-archive-trust.mp3" \
  -stream_loop -1 -i "$ROOT/assets/audio/music/moon-gate-finale.mp3" \
  -i "$ROOT/assets/audio/voices/01-mizuki-impossible.mp3" \
  -i "$ROOT/assets/audio/voices/02-shion-failed-doll.mp3" \
  -i "$ROOT/assets/audio/voices/03-gakuto-truth.mp3" \
  -i "$ROOT/assets/audio/voices/04-mizuki-promise-question.mp3" \
  -i "$ROOT/assets/audio/voices/05-kohaku-final-message.mp3" \
  -i "$ROOT/assets/audio/voices/06-shion-confession.mp3" \
  -i "$ROOT/assets/audio/voices/09-gakuto-confession.mp3" \
  -i "$ROOT/assets/audio/voices/07-shion-command.mp3" \
  -i "$ROOT/assets/audio/voices/08-mizuki-ending.mp3" \
  -i "$ROOT/assets/audio/music/break-moon-control.mp3" \
  -filter_complex "\
    [1:a]volume=0.13,atrim=0:16.4,asetpts=PTS-STARTPTS[bg1];\
    [2:a]volume=0.12,atrim=0:21.436,asetpts=PTS-STARTPTS,adelay=16400|16400[bg2];\
    [3:a]volume=0.13,atrim=0:36.38,asetpts=PTS-STARTPTS,adelay=37836|37836[bg3];\
    [4:a]volume=1.12,adelay=878|878[v1];\
    [5:a]volume=1.12,adelay=4202|4202[v2];\
    [6:a]volume=1.12,adelay=7746|7746[v3];\
    [7:a]volume=1.12,adelay=16400|16400[v4];\
    [8:a]volume=1.12,adelay=27769|27769[v5];\
    [9:a]volume=1.12,adelay=37836|37836[v6];\
    [10:a]volume=1.12,adelay=53771|53771[v7];\
    [11:a]volume=1.12,adelay=62483|62483[v8];\
    [12:a]volume=1.12,adelay=67756|67756[v9];\
    [13:a]volume=0.72,adelay=66200|66200[sfx];\
    [bg1][bg2][bg3][v1][v2][v3][v4][v5][v6][v7][v8][v9][sfx]\
    amix=inputs=13:duration=longest:normalize=0,alimiter=limit=0.95[aout]" \
  -map 0:v:0 -map "[aout]" \
  -t 74.216 \
  -c:v libx264 -preset medium -crf 18 -pix_fmt yuv420p \
  -c:a aac -b:a 192k -movflags +faststart \
  "$OUTPUT"

printf '%s\n' "$OUTPUT"
