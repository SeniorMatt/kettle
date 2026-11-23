#!/usr/bin/env bash
# The overlay library in two programs that are not SDL games, on a virtual screen:
#  - glxgears: plain Xlib + GLX (draws through the OpenGL hook, input through the Xlib hooks)
#  - vkcube:   Vulkan through the implicit layer (software Vulkan, lavapipe)
# Usage: scripts/overlay-gl-vk-test.sh [outdir]   Env: OVERLAY_TEST_DISPLAY (default :58)
set -u
cd "$(dirname "$0")/.."
OUT="${1:-target/overlay-test}"
D="${OVERLAY_TEST_DISPLAY:-:58}"
T="${CARGO_TARGET_DIR:-$PWD/target}"
LIB="$T/debug/libkettle_overlay.so"
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd)"
fails=0
ok()   { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; fails=$((fails + 1)); }
check() { if eval "$2"; then ok "$1"; else fail "$1"; fi; }
shot() { ffmpeg -loglevel error -y -f x11grab -video_size 1280x720 -i "$D" -frames:v 1 "$OUT/$1.png"; }
wait_for() { for _ in $(seq 1 100); do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 0.2; done; return 1; }
export DISPLAY="$D"
Xvfb "$D" -screen 0 1280x720x24 +extension GLX +render -noreset >/dev/null 2>&1 &
XVFB=$!
trap 'kill $PID 2>/dev/null; kill $XVFB 2>/dev/null' EXIT
sleep 1

# ---- glxgears: the performance HUD, then Shift+Tab through Xlib ----
rm -rf "$OUT/gears.log" "$OUT/shots-gl" "$OUT/shots-vk"
LD_PRELOAD="$LIB" KETTLE_OVERLAY=1 KETTLE_OVERLAY_DEMO=hud KETTLE_OVERLAY_LOG="$OUT/gears.log" KETTLE_OVERLAY_SHOTDIR="$OUT/shots-gl" \
  LIBGL_ALWAYS_SOFTWARE=1 glxgears -geometry 1280x720+0+0 >"$OUT/gears.out" 2>&1 &
PID=$!
wait_for "$OUT/gears.log" "gl first frame" && ok "glxgears: the OpenGL hook draws" || fail "glxgears: the OpenGL hook draws (no frame)"
sleep 2
shot glxgears-1-hud
xdotool search --name glxgears windowfocus 2>/dev/null; xdotool mousemove 640 360; sleep 0.5
xdotool key shift+Tab; sleep 2
check "glxgears: Shift+Tab opens through the Xlib hooks" "grep -q 'visible open=true' '$OUT/gears.log'"
shot glxgears-2-open
xdotool key shift+Tab; sleep 1.5
check "glxgears: Shift+Tab closes" "grep -q 'visible open=false' '$OUT/gears.log'"
xdotool key F12; sleep 2
check "glxgears: F12 saves a screenshot through glReadPixels" \
  "f=\$(ls '$OUT'/shots-gl/*.png 2>/dev/null | head -1); [ -n \"\$f\" ] && file \"\$f\" | grep -q 'PNG image data'"
kill $PID 2>/dev/null; wait $PID 2>/dev/null

# ---- vkcube: the Vulkan layer (the layer manifest is made like Kettle makes it) ----
XDG="$OUT/xdg"; mkdir -p "$XDG/vulkan/implicit_layer.d"
cat >"$XDG/vulkan/implicit_layer.d/kettle_overlay.json" <<EOF
{
  "file_format_version": "1.0.0",
  "layer": {
    "name": "VK_LAYER_KETTLE_overlay", "type": "GLOBAL", "library_path": "$LIB",
    "api_version": "1.3.0", "implementation_version": "1", "description": "Kettle in-game overlay",
    "functions": {
      "vkNegotiateLoaderLayerInterfaceVersion": "kettle_vk_negotiate",
      "vkGetInstanceProcAddr": "kettle_vk_gipa", "vkGetDeviceProcAddr": "kettle_vk_gdpa"
    },
    "enable_environment": { "KETTLE_OVERLAY": "1" }, "disable_environment": { "KETTLE_OVERLAY_DISABLE": "1" }
  }
}
EOF
ICD=$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -1)
rm -f "$OUT/vkcube.log"
XDG_DATA_HOME="$XDG" VK_ICD_FILENAMES="$ICD" KETTLE_OVERLAY=1 KETTLE_OVERLAY_DEMO=open KETTLE_OVERLAY_LOG="$OUT/vkcube.log" \
  KETTLE_OVERLAY_SHOTDIR="$OUT/shots-vk" KETTLE_OVERLAY_DEMO_SHOT=1 \
  vkcube --width 1280 --height 720 >"$OUT/vkcube.out" 2>&1 &
PID=$!
wait_for "$OUT/vkcube.log" "vk first frame" && ok "vkcube: the Vulkan layer draws into the swapchain" || { fail "vkcube: the Vulkan layer draws (no frame)"; tail -5 "$OUT/vkcube.out"; }
sleep 3
shot vkcube-open
kill -0 $PID 2>/dev/null && ok "vkcube: still running after the overlay frames" || fail "vkcube died"
check "vkcube: a screenshot of the swapchain image was saved" \
  "f=\$(ls '$OUT'/shots-vk/*.png 2>/dev/null | head -1); [ -n \"\$f\" ] && file \"\$f\" | grep -q 'PNG image data, 1280 x 720'"
kill $PID 2>/dev/null; wait $PID 2>/dev/null

echo "failures: $fails"
exit $((fails > 0))
