"""Capture the README images from the native app (graphical session required)."""

import argparse
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, help="Path to a ui-screenshot build")
    parser.add_argument("--output", type=Path, default=Path("assets/screenshots"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    binary = (args.binary or Path("target/ci/libreomr" + (".exe" if os.name == "nt" else ""))).resolve()
    output = args.output.resolve()
    if not binary.is_file():
        parser.error(f"Screenshot binary not found: {binary}")
    output.mkdir(parents=True, exist_ok=True)
    base = os.environ.copy()
    base.pop("EFRAME_SCREENSHOT_TO", None)
    # Captures are independent of the user's saved language and theme.
    for name in list(base):
        if name.startswith("LIBREOMR_") and "SCREENSHOT" in name:
            del base[name]
    base["LIBREOMR_SCREENSHOT_IMAGE"] = str(root / "examples/public-domain/bach-bwv1013.pdf")
    base["LIBREOMR_SCREENSHOT_LANGUAGE"] = "en"
    for filename, theme, extra in [
        ("overview.png", "light", {}),
        ("correction.png", "light", {"LIBREOMR_REVIEW_SCREENSHOT": "details"}),
        ("dark.png", "dark", {}),
        ("settings.png", "light", {"LIBREOMR_SETTINGS_SCREENSHOT": "1"}),
    ]:
        environment = dict(base, LIBREOMR_SCREENSHOT_THEME=theme,
                           LIBREOMR_SCREENSHOT_TO=str(output / filename), **extra)
        subprocess.run([str(binary)], cwd=root, env=environment, check=True, timeout=600)
        if not (output / filename).is_file():
            raise RuntimeError(f"The application did not write {filename}")
        print(f"Captured {filename}")


if __name__ == "__main__":
    main()
