#!/usr/bin/env bash
# Build dist/kettle-x86_64.AppImage (one portable file).
# Usage: scripts/build-appimage.sh
#   APPIMAGETOOL=/path/to/appimagetool  use a tool you already have
set -euo pipefail
cd "$(dirname "$0")/.."

TOOLS="${XDG_CACHE_HOME:-$HOME/.cache}/kettle-build"
TOOL="${APPIMAGETOOL:-$TOOLS/appimagetool-x86_64.AppImage}"
mkdir -p "$TOOLS" dist

if [ ! -x "$TOOL" ]; then
  echo "Downloading appimagetool..."
  curl -fsSL -o "$TOOL" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$TOOL"
fi

CARGO="${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" "$CARGO" build --release -p kettle -p sr-overlay-hook

APPDIR="$(mktemp -d)/kettle.AppDir"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps"
cp target/release/kettle "$APPDIR/usr/bin/"
# The in-game overlay library; Kettle copies it to its data folder at start (crates/app/src/overlay_install.rs).
mkdir -p "$APPDIR/usr/lib" && cp target/release/libkettle_overlay.so "$APPDIR/usr/lib/"
# And its 32-bit copy for 32-bit games (needs `rustup target add i686-unknown-linux-gnu` and gcc-multilib;
# KETTLE_SKIP_I686=1 leaves it out, e.g. on a machine without them).
if [ -z "${KETTLE_SKIP_I686:-}" ]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" "$CARGO" build --release -p sr-overlay-hook --lib --target i686-unknown-linux-gnu
  cp target/i686-unknown-linux-gnu/release/libkettle_overlay.so "$APPDIR/usr/lib/libkettle_overlay32.so"
fi
cp packaging/kettle.desktop "$APPDIR/" && cp packaging/kettle.desktop "$APPDIR/usr/share/applications/"
cp packaging/kettle.svg "$APPDIR/" && cp packaging/kettle.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/"
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/kettle" "$@"
RUN
chmod +x "$APPDIR/AppRun"

# APPIMAGE_EXTRACT_AND_RUN: works without FUSE on the build machine.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" dist/kettle-x86_64.AppImage >/dev/null
rm -rf "$(dirname "$APPDIR")"
ls -lh dist/kettle-x86_64.AppImage
