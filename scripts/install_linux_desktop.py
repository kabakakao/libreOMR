#!/usr/bin/env python3
"""Add the unpacked Linux application to the current user's application menu."""

import os
from pathlib import Path
import shutil
import sys


APP_ID = "org.libreomr.libreomr"


def quote_executable(path):
    value = str(path)
    if any(character in value for character in "\r\n\t\0="):
        raise ValueError("The application path contains characters unsupported by desktop launchers")
    # Desktop entries apply string unescaping before command-line unescaping.
    value = value.replace("\\", "\\\\\\\\")
    for character in ('"', "`", "$"):
        value = value.replace(character, "\\\\" + character)
    return '"' + value.replace("%", "%%") + '"'


def install(package, data_directory):
    package = package.resolve()
    binary = package / "libreomr"
    if not binary.is_file():
        raise ValueError("Keep this installer next to the unpacked libreomr executable")
    desktop_name = f"{APP_ID}.desktop"
    template = (package / "share/applications" / desktop_name).read_text(encoding="utf-8")
    desktop = template.replace("Exec=libreomr\n", f"Exec={quote_executable(binary)}\n")
    if desktop == template:
        raise ValueError("Unexpected desktop entry: missing Exec=libreomr")
    source_icons = package / "share/icons/hicolor"
    icons = sorted(source_icons.glob(f"*/apps/{APP_ID}.png"))
    if not icons:
        raise ValueError("The application package contains no desktop icons")
    for icon in icons:
        destination = data_directory / "icons/hicolor" / icon.relative_to(source_icons)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(icon, destination)
    destination = data_directory / "applications" / desktop_name
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(desktop, encoding="utf-8")
    return destination


def main():
    if not sys.platform.startswith("linux"):
        raise SystemExit("This installer is for the Linux release package")
    data_directory = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
    destination = install(Path(__file__).resolve().parent, data_directory)
    print(f"Application menu entry installed: {destination}")
    print("Keep the unpacked application folder in place; run this installer again after moving it.")


if __name__ == "__main__":
    main()
