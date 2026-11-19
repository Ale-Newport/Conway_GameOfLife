#!/usr/bin/env bash
# Records the README animation (docs/demo.gif) from the real desktop app.
#
# For each laboratory below, the release build of emergence-lab is started
# with --play (so the simulation runs) and --smoke-sequence, which saves numbered screenshots of its own window
# (nothing else on the screen is captured), then exits. ffmpeg joins the
# clips, scales them and builds an optimised GIF palette.
#
# Usage: scripts/record_demo.sh            (needs a display and ffmpeg)
# Env:   CLIP_FRAMES (default 40 screenshots per laboratory),
#        EVERY (frames between screenshots, default 3), WIDTH (default 800)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

command -v ffmpeg >/dev/null 2>&1 || { echo "error: ffmpeg not found" >&2; exit 1; }

CLIP_FRAMES="${CLIP_FRAMES:-40}"
EVERY="${EVERY:-3}"
WIDTH="${WIDTH:-800}"
FPS=10
OUT="docs/demo.gif"
REC="recordings"

# slug:warm-up frames (time for the scenario to load and start moving)
CLIPS=(
    classic-conway:120
    conway-within-conway:240
    ecosystem:240
    continuous-life:150
    3d-life:150
    civilizations:200
)

echo "==> building emergence-lab (release)"
cargo build --release -p emergence-lab
BIN="$ROOT/target/release/emergence-lab"

rm -rf "$REC"
mkdir -p "$REC"
list="$REC/clips.txt"
: > "$list"
for clip in "${CLIPS[@]}"; do
    slug="${clip%%:*}"
    warmup="${clip##*:}"
    echo "==> $slug ($CLIP_FRAMES screenshots, every $EVERY frames after $warmup)"
    "$BIN" --play --mode "$slug" --smoke-screenshot "$REC/$slug/frame.png" --smoke-frames "$warmup" \
        --smoke-sequence "$CLIP_FRAMES" --smoke-every "$EVERY"
    ffmpeg -loglevel error -y -framerate "$FPS" -i "$REC/$slug/frame-%04d.png" \
        -vf "scale=${WIDTH}:-2:flags=lanczos" -c:v libx264 -crf 12 -pix_fmt yuv444p "$REC/$slug.mp4"
    echo "file '$slug.mp4'" >> "$list"
done

echo "==> joining clips into $OUT"
ffmpeg -loglevel error -y -f concat -safe 0 -i "$list" -c copy "$REC/all.mp4"
ffmpeg -loglevel error -y -i "$REC/all.mp4" \
    -vf "fps=$FPS,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    -loop 0 "$OUT"
echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
