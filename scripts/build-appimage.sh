#!/usr/bin/env bash
# Build dist/steam-rust-x86_64.AppImage (one portable file).
# Usage: scripts/build-appimage.sh
#   APPIMAGETOOL=/path/to/appimagetool  use a tool you already have
set -euo pipefail
cd "$(dirname "$0")/.."

TOOLS="${XDG_CACHE_HOME:-$HOME/.cache}/steam-rust-build"
TOOL="${APPIMAGETOOL:-$TOOLS/appimagetool-x86_64.AppImage}"
mkdir -p "$TOOLS" dist

if [ ! -x "$TOOL" ]; then
  echo "Downloading appimagetool..."
  curl -fsSL -o "$TOOL" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$TOOL"
fi

CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" cargo build --release -p steam-rust

APPDIR="$(mktemp -d)/steam-rust.AppDir"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps"
cp target/release/steam-rust "$APPDIR/usr/bin/"
cp packaging/steam-rust.desktop "$APPDIR/" && cp packaging/steam-rust.desktop "$APPDIR/usr/share/applications/"
cp packaging/steam-rust.svg "$APPDIR/" && cp packaging/steam-rust.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/"
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/steam-rust" "$@"
RUN
chmod +x "$APPDIR/AppRun"

# APPIMAGE_EXTRACT_AND_RUN: works without FUSE on the build machine.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" dist/steam-rust-x86_64.AppImage >/dev/null
rm -rf "$(dirname "$APPDIR")"
ls -lh dist/steam-rust-x86_64.AppImage
