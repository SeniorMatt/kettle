#!/bin/sh
# Screenshots of Kettle in demo mode on a virtual screen (used by the Check workflow).
# The window is 935 x 1079, the size of each half of target/ref/steam-vs-kettle.webp, so shots can be put
# next to the reference pixel for pixel. Clicks come from xdotool when it is installed.
# Usage: scripts/ci-screenshots.sh <kettle binary> <output dir>
set -u
bin=$1; out=$2; mkdir -p "$out"
export DISPLAY=:57 WGPU_BACKEND=vulkan KETTLE_DEMO=120 KETTLE_DEMO_INSTALLED=1 KETTLE_SHOT_SIZE=935x1079 KETTLE_FONT=builtin
export KETTLE_DEMO_APPS=440,570,730,105600,252490,292030,367520,413150,504230,620,1091500,1145360,1245620,374320
icd=$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -1); [ -n "$icd" ] && export VK_ICD_FILENAMES="$icd"
Xvfb :57 -screen 0 1000x1080x24 -nolisten tcp >/dev/null 2>&1 &
xvfb=$!; sleep 2
grab() { ffmpeg -loglevel error -y -f x11grab -video_size 1000x1080 -i :57 -frames:v 1 "$out/$1.png" || true; }
# shot <name> <KETTLE_SHOT_OPEN> [<x> <y> <what>...]: after the page is up, move/click/press, then grab.
# Actions: "c:x:y" click, "d:x:y" double click, "r:x:y" right click, "m:x:y" move, "k:Key" key.
shot() {
    name=$1; open=$2; shift 2
    home=$(mktemp -d)
    HOME=$home XDG_CONFIG_HOME=$home/.config XDG_DATA_HOME=$home/.local/share XDG_CACHE_HOME=$home/.cache \
        KETTLE_SHOT_OPEN=$open "$bin" >"$out/$name.log" 2>&1 &
    pid=$!; sleep 30
    grab "$name"
    if command -v xdotool >/dev/null 2>&1; then
        # No window manager here: give the window the keyboard focus by hand.
        wid=$(xdotool search --pid "$pid" 2>/dev/null | tail -1); [ -n "$wid" ] && xdotool windowfocus "$wid" 2>/dev/null
        for a in "$@"; do
            case $a in
                c:*) xdotool mousemove "$(echo "$a" | cut -d: -f2)" "$(echo "$a" | cut -d: -f3)" click 1 ;;
                d:*) xdotool mousemove "$(echo "$a" | cut -d: -f2)" "$(echo "$a" | cut -d: -f3)" click --repeat 2 --delay 80 1 ;;
                r:*) xdotool mousemove "$(echo "$a" | cut -d: -f2)" "$(echo "$a" | cut -d: -f3)" click 3 ;;
                m:*) xdotool mousemove "$(echo "$a" | cut -d: -f2)" "$(echo "$a" | cut -d: -f3)" ;;
                k:*) xdotool key "$(echo "$a" | cut -d: -f2)" ;;
            esac
            sleep 1.5
        done
        [ $# -gt 0 ] && { sleep 1; grab "$name-after"; }
        echo "window alive after actions: $(kill -0 $pid 2>/dev/null && echo yes || echo no)" >>"$out/$name.log"
    fi
    kill $pid 2>/dev/null; sleep 1; kill -9 $pid 2>/dev/null; rm -rf "$home"
}
shot library ""
shot game "game:440"
# Filter popup, Escape closes only the popup (the window and the page stay).
shot popup-filter "" c:249:188 k:Escape
# Play drop-down of an installed game page.
shot game-menu "game:440" c:400:325
# Context menu of a list row, and a collection header click.
shot rowmenu "" r:120:243
shot header-fold "" c:14:216
shot collection "collection:Favorites"
shot collection-header-click "" c:100:216
shot settings "settings:interface"
kill $xvfb
ls -la "$out"
