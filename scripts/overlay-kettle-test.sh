#!/usr/bin/env bash
# The data Kettle feeds the overlay, end to end on a virtual screen: a fake Kettle (scripts/overlay-fake-kettle.py)
# sends playtime, achievements, friends and pictures over the real socket to the library inside the SDL test game;
# F12 takes a screenshot into the folder Kettle named, and the library tells Kettle where it went. Takes
# screenshots of the overlay to look at (total playtime, achievements, avatars).
#
# Usage: scripts/overlay-kettle-test.sh [outdir]   (build first: cargo build -p sr-overlay-hook --lib --example sdl_test)
# Env:   OVERLAY_TEST_DISPLAY (default :58)
set -u
cd "$(dirname "$0")/.."
OUT="${1:-target/overlay-kettle}"
D="${OVERLAY_TEST_DISPLAY:-:58}"
T="${CARGO_TARGET_DIR:-$PWD/target}"
LIB="$T/debug/libkettle_overlay.so"
APP="$T/debug/examples/sdl_test"
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd)"
rm -rf "$OUT"/*.log "$OUT"/quit "$OUT"/shots "$OUT"/sock
fails=0
ok()   { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; fails=$((fails + 1)); }
check() { if eval "$2"; then ok "$1"; else fail "$1"; fi; }
shot() { ffmpeg -loglevel error -y -f x11grab -video_size 1280x720 -i "$D" -frames:v 1 "$OUT/$1.png"; }
wait_for() { for _ in $(seq 1 100); do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 0.2; done; return 1; }
export DISPLAY="$D"

Xvfb "$D" -screen 0 1280x720x24 +extension GLX +render -noreset >/dev/null 2>&1 &
XVFB=$!
FAKE_SHOT_DIR="$OUT/shots" python3 -I scripts/overlay-fake-kettle.py "$OUT/sock" "$OUT/kettle.log" "$OUT/quit" &
FAKE=$!
trap 'touch "$OUT/quit"; sleep 0.5; kill $GAME 2>/dev/null; kill $FAKE 2>/dev/null; kill $XVFB 2>/dev/null' EXIT
sleep 1

LD_PRELOAD="$LIB" KETTLE_OVERLAY=1 KETTLE_OVERLAY_SOCKET="$OUT/sock" KETTLE_OVERLAY_LOG="$OUT/hook.log" \
  TEST_LOG="$OUT/game.log" TEST_QUIT="$OUT/quit" LIBGL_ALWAYS_SOFTWARE=1 SDL_VIDEODRIVER=x11 "$APP" \
  >"$OUT/app.out" 2>&1 &
GAME=$!
wait_for "$OUT/game.log" ready || { echo "the test program did not start"; cat "$OUT/app.out"; exit 1; }
check "the library connected to Kettle (hello)" "wait_for '$OUT/kettle.log' '\"t\":\"hello\"'"
sleep 1
xdotool search --name sdl_test windowfocus 2>/dev/null
xdotool mousemove 640 360; sleep 0.5
xdotool key shift+Tab; sleep 2
check "Shift+Tab opens the overlay" "grep -q 'visible open=true' '$OUT/hook.log'"
check "Kettle was told it is visible" "grep -q '\"t\":\"visible\",\"open\":true' '$OUT/kettle.log'"
shot 1-with-kettle-data
xdotool key shift+Tab; sleep 1.5
xdotool key F12; sleep 2
check "F12 saved a PNG into the folder Kettle named" "ls '$OUT'/shots/*.png >/dev/null 2>&1"
check "the library told Kettle where the screenshot went" "grep -q '\"t\":\"screenshot\",\"path\":\"$OUT/shots/' '$OUT/kettle.log'"

# A lost Kettle: the game keeps running and the overlay still opens with what it has.
kill $FAKE 2>/dev/null; wait $FAKE 2>/dev/null; sleep 1
xdotool key shift+Tab; sleep 2
check "Kettle gone: the overlay still opens" "[ \$(grep -c 'visible open=true' '$OUT/hook.log') -ge 2 ]"
check "Kettle gone: the game still runs" "kill -0 $GAME 2>/dev/null"
shot 2-kettle-gone

echo "failures: $fails"
exit $((fails > 0))
