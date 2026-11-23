#!/usr/bin/env bash
# The owner's test game: the standalone DDNet Linux release (open source, from ddnet.org) with the overlay
# library preloaded, on a virtual screen. A local DDNet server is started so the client really is in a game
# (that is when DDNet turns on relative mouse mode). Checks: Shift+Tab opens the overlay, the real relative mode
# and grab are released, the game gets no clicks while open, and everything comes back on close.
#
# Usage: scripts/overlay-ddnet-test.sh [outdir] [OpenGL|Vulkan]
# Env:   OVERLAY_TEST_DISPLAY (default :58)   OVERLAY_TEST_CLICK "x y" = an overlay button to click
set -u
cd "$(dirname "$0")/.."
OUT="${1:-target/overlay-ddnet}"
BACKEND="${2:-OpenGL}"
D="${OVERLAY_TEST_DISPLAY:-:58}"
CLICK="${OVERLAY_TEST_CLICK:-159 369}"
VER="${DDNET_VERSION:-20.2.1}"
T="${CARGO_TARGET_DIR:-$PWD/target}"
LIB="$T/debug/libkettle_overlay.so"
mkdir -p "$OUT" target/ddnet; OUT="$(cd "$OUT" && pwd)"
TAG="ddnet-$BACKEND"
DIR="$PWD/target/ddnet/DDNet-$VER-linux_x86_64"
if [ ! -x "$DIR/DDNet" ]; then
  curl -fsSL -o target/ddnet/ddnet.tar.xz "https://ddnet.org/downloads/DDNet-$VER-linux_x86_64.tar.xz" &&
    tar -xf target/ddnet/ddnet.tar.xz -C target/ddnet || { echo "cannot download DDNet"; exit 1; }
fi
fails=0
ok()   { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; fails=$((fails + 1)); }
check() { if eval "$2"; then ok "$1"; else fail "$1"; fi; }
shot() { ffmpeg -loglevel error -y -f x11grab -video_size 1280x720 -i "$D" -frames:v 1 "$OUT/$TAG-$1.png"; }
wait_for() { for _ in $(seq 1 ${3:-150}); do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 0.2; done; return 1; }
export DISPLAY="$D"
LOG="$OUT/$TAG-hook.log"; rm -f "$LOG"
HOMEDIR="$OUT/$TAG-home"; rm -rf "$HOMEDIR"; mkdir -p "$HOMEDIR"

Xvfb "$D" -screen 0 1280x720x24 +extension GLX +render -noreset >/dev/null 2>&1 &
XVFB=$!
trap 'kill $GAME $SERVER 2>/dev/null; kill $XVFB 2>/dev/null' EXIT
sleep 1

# A local server on the loopback; the client joins it.
(cd "$DIR" && HOME="$HOMEDIR" ./DDNet-Server "sv_port 8309;sv_register 0;sv_name overlay-test" >"$OUT/$TAG-server.out" 2>&1) &
SERVER=$!
sleep 2

XDG="$OUT/$TAG-xdg"; mkdir -p "$XDG/vulkan/implicit_layer.d"
cat >"$XDG/vulkan/implicit_layer.d/kettle_overlay.json" <<EOF
{ "file_format_version": "1.0.0", "layer": { "name": "VK_LAYER_KETTLE_overlay", "type": "GLOBAL",
  "library_path": "$LIB", "api_version": "1.3.0", "implementation_version": "1", "description": "Kettle",
  "functions": { "vkNegotiateLoaderLayerInterfaceVersion": "kettle_vk_negotiate",
    "vkGetInstanceProcAddr": "kettle_vk_gipa", "vkGetDeviceProcAddr": "kettle_vk_gdpa" },
  "enable_environment": { "KETTLE_OVERLAY": "1" }, "disable_environment": { "KETTLE_OVERLAY_DISABLE": "1" } } }
EOF
ICD=$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -1)

(cd "$DIR" && HOME="$HOMEDIR" XDG_DATA_HOME="$XDG" VK_ICD_FILENAMES="$ICD" LD_PRELOAD="$LIB" KETTLE_OVERLAY=1 \
  KETTLE_OVERLAY_DEMO=1 KETTLE_OVERLAY_LOG="$LOG" LIBGL_ALWAYS_SOFTWARE=1 SDL_AUDIODRIVER=dummy SDL_VIDEODRIVER=x11 \
  ./DDNet "gfx_backend $BACKEND;gfx_screen_width 1280;gfx_screen_height 720;gfx_fullscreen 0;gfx_borderless 0;snd_enable 0;cl_skip_start_menu 1;gfx_vsync 0;connect 127.0.0.1:8309" \
  >"$OUT/$TAG-game.out" 2>&1) &
GAME=$!
MARK=gl; [ "$BACKEND" = Vulkan ] && MARK=vk
wait_for "$LOG" "$MARK first frame" 300 || echo "note: no overlay frame yet (the overlay only draws when open or with the HUD)"
sleep 12   # joining the local game
shot 1-ingame
xdotool search --name DDNet windowfocus 2>/dev/null; xdotool mousemove 640 360; sleep 1

# in a game, DDNet has relative mode on; the library has not touched it
xdotool key shift+Tab; sleep 3
check "$BACKEND: Shift+Tab opens the overlay in DDNet" "grep -q 'visible open=true' '$LOG'"
check "$BACKEND: DDNet had relative mouse mode on and the library released it" \
  "grep -q 'sdl before-open real rel=Some(1)' '$LOG' && grep -q 'sdl open real rel=Some(0) cursor=Some(1) grab=Some(0)' '$LOG'"
shot 2-overlay-open
xdotool mousemove ${CLICK% *} ${CLICK#* }; sleep 0.7
shot 3-hover
xdotool click 1; sleep 1.5
check "$BACKEND: a click on the overlay is taken by the overlay" "grep -q '^action' '$LOG' || grep -q 'visible open=false' '$LOG'"
shot 4-after-click
if grep -q 'visible open=false' "$LOG"; then :; else xdotool key shift+Tab; sleep 3; fi
check "$BACKEND: closing restores relative mouse mode" "grep -q 'sdl close real rel=Some(1)' '$LOG'"
shot 5-closed
echo "failures: $fails"
exit $((fails > 0))
