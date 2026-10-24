#!/bin/sh
# Build a native Apple Silicon app bundle with the libreOMR Finder/Dock icon.
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
    echo "This script requires macOS (iconutil)." >&2
    exit 1
fi

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

build_profile=release
build_binary=true
while [ "$#" -gt 0 ]; do
    case "$1" in
        --profile)
            [ "$#" -ge 2 ] || { echo "--profile requires ci or release" >&2; exit 1; }
            build_profile=$2
            shift 2
            ;;
        --no-build) build_binary=false; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done
case "$build_profile" in ci|release) ;; *) echo "Unsupported profile: $build_profile" >&2; exit 1 ;; esac
if [ "$(uname -m)" != "arm64" ]; then
    echo "The macOS app bundle supports Apple Silicon only." >&2
    exit 1
fi

# Keep the bundle and binary in the same predictable project-local directory.
if [ "$build_binary" = true ]; then
    cargo build --profile "$build_profile" --locked --bin libreomr --target-dir "$project_dir/target"
fi

bundle_dir="$project_dir/target/$build_profile/libreOMR.app"
iconset_dir="$project_dir/target/libreomr.iconset"
mkdir -p "$bundle_dir/Contents/MacOS" "$bundle_dir/Contents/Resources" "$iconset_dir"

for size in 16 32 128 256 512; do
    cp "assets/icons/${size}x${size}/org.libreomr.libreomr.png" \
        "$iconset_dir/icon_${size}x${size}.png"
    retina_size=$((size * 2))
    cp "assets/icons/${retina_size}x${retina_size}/org.libreomr.libreomr.png" \
        "$iconset_dir/icon_${size}x${size}@2x.png"
done

iconutil -c icns "$iconset_dir" -o "$bundle_dir/Contents/Resources/libreomr.icns"
cp "target/$build_profile/libreomr" "$bundle_dir/Contents/MacOS/libreomr"

python3 - "$bundle_dir/Contents/Info.plist" <<'PY'
import json
from pathlib import Path
import plistlib
import subprocess
import sys

data = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"
], text=True))
version = next(p["version"] for p in data["packages"] if p["id"] in data["workspace_members"])
numeric_version = version.split("-", 1)[0].split("+", 1)[0]
with Path(sys.argv[1]).open("wb") as destination:
    plistlib.dump({
        "CFBundleName": "libreOMR",
        "CFBundleDisplayName": "libreOMR",
        "CFBundleIdentifier": "org.libreomr.libreomr",
        "CFBundleExecutable": "libreomr",
        "CFBundleIconFile": "libreomr.icns",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": numeric_version,
        "CFBundleVersion": numeric_version,
        "CFBundleGetInfoString": f"libreOMR {version}",
        "LSMinimumSystemVersion": "11.0",
        "NSHighResolutionCapable": True,
    }, destination)
PY

echo "Built: $bundle_dir"
