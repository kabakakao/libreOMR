# README screenshots

Actual captures of the native libreOMR interface in English, generated on
October 24, 2026. All four images show the first page of the unmodified
[public-domain Bach score](../../examples/public-domain/README.md).
Recognition proposals were not musically corrected for the captures. In the
correction view, two note proposals are marked reviewed for demonstration and
the second is selected. `settings.png` also shows the language/theme controls.

Build and capture from the project root in a graphical session:

```sh
cargo build --profile ci --locked --features ui-screenshot --bin libreomr
python3 scripts/capture_screenshots.py
```

On Windows, use `python` if `python3` is unavailable. The script automatically
selects `target/ci/libreomr.exe` there. For another build location, pass
`--binary path/to/libreomr`; use `--output target/screenshots` to preserve the
committed images. Captures explicitly select English and the appropriate theme,
independently of saved user preferences. They do not save those overrides.

For a single capture, the native screenshot mode exits the application
automatically after writing the image:

```sh
LIBREOMR_SCREENSHOT_IMAGE=examples/public-domain/bach-bwv1013.pdf \
LIBREOMR_SCREENSHOT_LANGUAGE=en \
LIBREOMR_SCREENSHOT_THEME=light \
LIBREOMR_SCREENSHOT_TO=assets/screenshots/overview.png \
target/ci/libreomr

LIBREOMR_SCREENSHOT_IMAGE=examples/public-domain/bach-bwv1013.pdf \
LIBREOMR_SCREENSHOT_LANGUAGE=en \
LIBREOMR_SCREENSHOT_THEME=light \
LIBREOMR_REVIEW_SCREENSHOT=details \
LIBREOMR_SCREENSHOT_TO=assets/screenshots/correction.png \
target/ci/libreomr

LIBREOMR_SCREENSHOT_IMAGE=examples/public-domain/bach-bwv1013.pdf \
LIBREOMR_SCREENSHOT_LANGUAGE=en \
LIBREOMR_SCREENSHOT_THEME=dark \
LIBREOMR_SCREENSHOT_TO=assets/screenshots/dark.png \
target/ci/libreomr

LIBREOMR_SCREENSHOT_IMAGE=examples/public-domain/bach-bwv1013.pdf \
LIBREOMR_SCREENSHOT_LANGUAGE=en \
LIBREOMR_SCREENSHOT_THEME=light \
LIBREOMR_SETTINGS_SCREENSHOT=1 \
LIBREOMR_SCREENSHOT_TO=assets/screenshots/settings.png \
target/ci/libreomr
```

These examples use macOS/Linux shell syntax. A PowerShell equivalent is:

```powershell
$env:LIBREOMR_SCREENSHOT_IMAGE = 'examples/public-domain/bach-bwv1013.pdf'
$env:LIBREOMR_SCREENSHOT_LANGUAGE = 'en'
$env:LIBREOMR_SCREENSHOT_THEME = 'light'
$env:LIBREOMR_SCREENSHOT_TO = 'target/english-smoke.png'
.\target\ci\libreomr.exe
```

For German captures, set `LIBREOMR_SCREENSHOT_LANGUAGE=de`. For narrow-window
checks, also set `LIBREOMR_COMPACT_SCREENSHOT=1`. Use temporary output paths for
these checks. Visually review both languages, all three theme settings and the
settings window at 760 × 520 and 1280 × 820 before publishing.
