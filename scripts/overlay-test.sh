#!/usr/bin/env bash
# Proves the in-game overlay on a virtual X screen: a small SDL2 program that behaves like DDNet (OpenGL, relative
# mouse, window grab, hidden cursor) runs with the overlay library preloaded; xdotool presses Shift+Tab, types,
# clicks and moves the mouse. Checks what the game saw and what the library did, and takes screenshots.
#
# Usage: scripts/overlay-test.sh [outdir]      (build first: cargo build -p sr-overlay-hook --lib --example sdl_test)
# Env:   OVERLAY_TEST_DISPLAY (default :58)   OVERLAY_TEST_CLICK "x y" = a button of the overlay to click
set -u
cd "$(dirname "$0")/.."
OUT="${1:-target/overlay-test}"
D="${OVERLAY_TEST_DISPLAY:-:58}"
CLICK="${OVERLAY_TEST_CLICK:-159 369}"
T="${CARGO_TARGET_DIR:-$PWD/target}"
LIB="$T/debug/libkettle_overlay.so"
APP="$T/debug/examples/sdl_test"
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd)"
rm -f "$OUT"/*.log "$OUT"/quit
fails=0
ok()   { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; fails=$((fails + 1)); }
check() { if eval "$2"; then ok "$1"; else fail "$1"; fi; }
shot() { ffmpeg -loglevel error -y -f x11grab -video_size 1280x720 -i "$D" -frames:v 1 "$OUT/$1.png"; }
wait_for() { for _ in $(seq 1 100); do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 0.2; done; return 1; }
export DISPLAY="$D"

Xvfb "$D" -screen 0 1280x720x24 +extension GLX +render -noreset >/dev/null 2>&1 &
XVFB=$!
trap 'touch "$OUT/quit"; sleep 0.5; kill $GAME 2>/dev/null; kill $XVFB 2>/dev/null' EXIT
sleep 1

rm -rf "$OUT/shots"
LD_PRELOAD="$LIB" KETTLE_OVERLAY=1 KETTLE_OVERLAY_DEMO=hud KETTLE_OVERLAY_LOG="$OUT/hook.log" KETTLE_OVERLAY_SHOTDIR="$OUT/shots" \
  TEST_LOG="$OUT/game.log" TEST_QUIT="$OUT/quit" LIBGL_ALWAYS_SOFTWARE=1 SDL_VIDEODRIVER=x11 "$APP" \
  >"$OUT/app.out" 2>&1 &
GAME=$!
wait_for "$OUT/game.log" ready || { echo "the test program did not start"; cat "$OUT/app.out"; exit 1; }
sleep 1
xdotool search --name sdl_test windowfocus 2>/dev/null
xdotool mousemove 640 360
sleep 0.5
mark() { wc -l <"$OUT/game.log"; }
since() { tail -n +"$(($1 + 1))" "$OUT/game.log"; }

# 1. Before: the game gets input.
m=$(mark); xdotool key a; xdotool click 1; sleep 0.5
check "game gets keys and clicks before the overlay opens" "since $m | grep -q 'key down scancode=4' && since $m | grep -q 'button down 1'"
shot 1-game

# 2. Shift+Tab opens.
m=$(mark); xdotool key shift+Tab; sleep 2
check "Shift+Tab opens the overlay" "grep -q 'visible open=true' '$OUT/hook.log'"
check "the game never sees Tab" "! since $m | grep -q 'scancode=43'"
check "relative mode, grab and hidden cursor are released" \
  "grep -q 'sdl open real rel=Some(0) cursor=Some(1) grab=Some(0)' '$OUT/hook.log'"
check "the game still believes its own state (rel=1 grab=1 cursor=0)" "since $m | grep -q 'state rel=1 grab=1 cursor=0'"
shot 2-overlay-open

# 3. While open the game gets nothing; the overlay does.
m=$(mark); xdotool key b; xdotool mousemove 500 300; xdotool mousemove_relative 20 10; sleep 0.5
check "keys and mouse motion do not reach the game while open" \
  "! since $m | grep -Eq '^(key|button|motion)'"
xdotool mousemove ${CLICK% *} ${CLICK#* }; sleep 0.5
shot 3-hover
m=$(mark); xdotool click 1; sleep 1
check "a click on the overlay does not reach the game" "! since $m | grep -q '^button'"
check "the click was taken by the overlay (action sent)" "grep -q '^action' '$OUT/hook.log'"
shot 4-after-click

# 4. Shift+Tab closes and everything is back.
m=$(mark); xdotool key shift+Tab; sleep 2
check "Shift+Tab closes the overlay" "grep -q 'visible open=false' '$OUT/hook.log'"
check "relative mode, grab and hidden cursor are restored" \
  "grep -q 'sdl close real rel=Some(1) cursor=Some(0) grab=Some(1)' '$OUT/hook.log'"
m=$(mark); xdotool key c; xdotool mousemove_relative 7 3; xdotool click 1; sleep 0.7
check "the game gets keys, motion and clicks again" \
  "since $m | grep -q 'key down scancode=6' && since $m | grep -q '^motion' && since $m | grep -q 'button down 1'"
shot 5-game-again

# 5. F12 takes a screenshot of the game's frame: a PNG appears, the game never sees the key.
m=$(mark); xdotool key F12; sleep 2
check "F12 saves a screenshot (a PNG of the window's size)" \
  "f=\$(ls '$OUT'/shots/*.png 2>/dev/null | head -1); [ -n \"\$f\" ] && file \"\$f\" | grep -q 'PNG image data, [0-9]* x [0-9]*' && grep -q '^screenshot ' '$OUT/hook.log'"
check "the game never sees F12" "! since $m | grep -q 'scancode=69'"

echo "failures: $fails"
exit $((fails > 0))
