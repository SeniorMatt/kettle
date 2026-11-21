#!/bin/sh
# Takes the README screenshots (docs/images/screenshot-*.png) from demo mode on a virtual screen.
# Works locally and on CI (needs Xvfb, ffmpeg and lavapipe; pngquant/oxipng/PIL are used when present).
# Usage: scripts/readme-screenshots.sh [kettle binary] [output dir]
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-}
if [ -z "$bin" ]; then
    for c in "$root/target/release/kettle" "$root/target/debug/kettle"; do [ -x "$c" ] && { bin=$c; break; }; done
fi
[ -x "$bin" ] || { echo "kettle binary not found; pass it as the first argument" >&2; exit 1; }
out=${2:-$root/docs/images}; mkdir -p "$out"
raw=$(mktemp -d)
D=:59
export DISPLAY=$D WGPU_BACKEND=vulkan KETTLE_DEMO=60 KETTLE_DEMO_INSTALLED=1 KETTLE_SHOT_SIZE=1600x900 KETTLE_FONT=builtin
export KETTLE_DEMO_APPS=440,570,730,105600,252490,292030,367520,413150,504230,620,1091500,1145360,1245620,374320,1086940,1174180,271590,489830
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
[ -f "$VK_ICD_FILENAMES" ] || { icd=$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -1); [ -n "$icd" ] && VK_ICD_FILENAMES=$icd; }
Xvfb $D -screen 0 1600x900x24 -nolisten tcp >/dev/null 2>&1 &
xvfb=$!; sleep 2
cleanup() { kill "$xvfb" 2>/dev/null; rm -rf "$raw"; }
trap cleanup EXIT

# shot <name> <KETTLE_SHOT_OPEN>
shot() {
    name=$1; open=$2
    home=$(mktemp -d)
    HOME=$home XDG_CONFIG_HOME=$home/.config XDG_DATA_HOME=$home/.local/share XDG_CACHE_HOME=$home/.cache \
        KETTLE_SHOT_OPEN=$open "$bin" >"$raw/$name.log" 2>&1 &
    pid=$!
    sleep "${SHOT_WAIT:-30}"   # lets covers download and the first animations finish
    ffmpeg -loglevel error -y -f x11grab -video_size 1600x900 -i $D -frames:v 1 "$raw/$name.png" || true
    kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null; rm -rf "$home"
    [ -s "$raw/$name.png" ] || { echo "no screenshot for $name" >&2; return; }
    if command -v pngquant >/dev/null 2>&1; then
        pngquant --force --quality 80-95 --output "$out/screenshot-$name.png" "$raw/$name.png"
    elif python3 -c "import PIL" 2>/dev/null; then
        python3 -c "import sys;from PIL import Image;Image.open(sys.argv[1]).convert('RGB').quantize(256).save(sys.argv[2],optimize=True)" \
            "$raw/$name.png" "$out/screenshot-$name.png"
    else
        cp "$raw/$name.png" "$out/screenshot-$name.png"
    fi
    command -v oxipng >/dev/null 2>&1 && oxipng -q -o 4 --strip safe "$out/screenshot-$name.png"
}
shot library ""
shot game "game:440"
shot settings "settings:interface"
shot store "store"
shot downloads "downloads"
ls -la "$out"/screenshot-*.png
