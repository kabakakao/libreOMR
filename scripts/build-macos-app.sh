#!/bin/sh
# Build a local macOS app bundle with the libreOMR Finder/Dock icon.
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
    echo "This script requires macOS (sips and iconutil)." >&2
    exit 1
fi

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

# Keep the bundle and binary in the same predictable project-local directory.
cargo build --release --locked --bin libreomr --target-dir "$project_dir/target"

bundle_dir="$project_dir/target/release/libreOMR.app"
iconset_dir="$project_dir/target/libreomr.iconset"
mkdir -p "$bundle_dir/Contents/MacOS" "$bundle_dir/Contents/Resources" "$iconset_dir"

for size in 16 32 128 256 512; do
    sips -z "$size" "$size" assets/libreomr-icon-dark.png \
        --out "$iconset_dir/icon_${size}x${size}.png" >/dev/null
    retina_size=$((size * 2))
    sips -z "$retina_size" "$retina_size" assets/libreomr-icon-dark.png \
        --out "$iconset_dir/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$iconset_dir" -o "$bundle_dir/Contents/Resources/libreomr.icns"
cp target/release/libreomr "$bundle_dir/Contents/MacOS/libreomr"

cat > "$bundle_dir/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>libreOMR</string>
    <key>CFBundleDisplayName</key><string>libreOMR</string>
    <key>CFBundleIdentifier</key><string>org.libreomr.libreomr</string>
    <key>CFBundleExecutable</key><string>libreomr</string>
    <key>CFBundleIconFile</key><string>libreomr.icns</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "Built: $bundle_dir"
