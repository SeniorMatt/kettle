#!/usr/bin/env bash
# Cross-build the Windows release from Linux: dist/kettle-windows-x86_64.zip
# Needs: rustup target add x86_64-pc-windows-gnu, and gcc-mingw-w64-x86-64.
set -euo pipefail
cd "$(dirname "$0")/.."

CARGO="${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" "$CARGO" build --release -p kettle --target x86_64-pc-windows-gnu

mkdir -p dist
python3 - <<'PY'
import zipfile
with zipfile.ZipFile("dist/kettle-windows-x86_64.zip", "w", zipfile.ZIP_DEFLATED) as z:
    z.write("target/x86_64-pc-windows-gnu/release/kettle.exe", "kettle/kettle.exe")
    for f in ["README.md", "PRIVACY.md", "LICENSE-MIT", "LICENSE-APACHE"]:
        z.write(f, "kettle/" + f)
PY
ls -lh dist/kettle-windows-x86_64.zip
