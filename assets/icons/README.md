# Application icons

All sizes are exported from the existing `assets/libreomr-icon-dark.png` artwork.
The ICO contains 16, 24, 32, 48, 64, 128 and 256 pixel images for Windows Explorer
and high DPI displays. The PNGs also supply Linux's hicolor icon theme and the
macOS ICNS iconset, including Retina sizes up to 1024 pixels.

To regenerate the committed exports after changing the artwork:

```sh
python -m pip install Pillow
python scripts/generate_icons.py
```

Normal Rust builds and release packaging use the committed files and do not
require Pillow. Windows builds embed the ICO using the resource compiler from
the Windows SDK; macOS bundles use the system's `iconutil`.
