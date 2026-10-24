#!/usr/bin/env python3
"""Check that Windows can extract application icons from the built executable."""

import ctypes
from ctypes import wintypes
from pathlib import Path
import sys


def check(binary):
    # ExtractIconEx reads PE resources; it does not start the application.
    shell32 = ctypes.WinDLL("shell32", use_last_error=True)
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    icon_pointer = ctypes.POINTER(wintypes.HICON)
    shell32.ExtractIconExW.argtypes = [
        wintypes.LPCWSTR, ctypes.c_int, icon_pointer, icon_pointer, wintypes.UINT,
    ]
    shell32.ExtractIconExW.restype = wintypes.UINT
    user32.DestroyIcon.argtypes = [wintypes.HICON]
    user32.DestroyIcon.restype = wintypes.BOOL
    large, small = wintypes.HICON(), wintypes.HICON()
    try:
        count = shell32.ExtractIconExW(str(binary), 0, ctypes.byref(large), ctypes.byref(small), 1)
        if count in (0, 0xFFFFFFFF) or not large.value or not small.value:
            raise ValueError(f"Windows cannot extract large and small application icons from {binary}")
    finally:
        for icon in (large, small):
            if icon.value:
                user32.DestroyIcon(icon)
    print(f"Embedded Windows application icon verified: {binary}")


if __name__ == "__main__":
    if sys.platform != "win32":
        raise SystemExit("This check requires Windows")
    check(Path(sys.argv[1]).resolve(strict=True))
