#!/usr/bin/env python3
"""Export platform icon sizes from the existing artwork (requires Pillow)."""

from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)


def main():
    with Image.open(ROOT / "assets/libreomr-icon-dark.png") as source:
        source = source.convert("RGBA")
        if source.width != source.height or source.width < max(SIZES):
            raise ValueError("Source icon must be square and at least 1024 pixels wide")
        for size in SIZES:
            destination = ROOT / f"assets/icons/{size}x{size}/org.libreomr.libreomr.png"
            destination.parent.mkdir(parents=True, exist_ok=True)
            source.resize((size, size), Image.Resampling.LANCZOS).save(destination, optimize=True)
        source.save(
            ROOT / "assets/icons/libreomr.ico", format="ICO",
            sizes=[(size, size) for size in SIZES if size <= 256],
        )


if __name__ == "__main__":
    main()
