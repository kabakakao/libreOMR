#!/usr/bin/env python3
"""Force a renderer failure and verify the real Windows error dialog and log."""

import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def check(binary):
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    user32.EnumWindows.restype = wintypes.BOOL
    user32.EnumChildWindows.argtypes = [wintypes.HWND, callback_type, wintypes.LPARAM]
    user32.EnumChildWindows.restype = wintypes.BOOL
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetWindowThreadProcessId.restype = wintypes.DWORD
    user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user32.GetWindowTextW.restype = ctypes.c_int
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.IsWindowVisible.restype = wintypes.BOOL
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.PostMessageW.restype = wintypes.BOOL

    def window_text(window):
        text = ctypes.create_unicode_buffer(32768)
        user32.GetWindowTextW(window, text, len(text))
        return text.value

    with tempfile.TemporaryDirectory(prefix="libreomr-startup-") as temporary:
        environment = os.environ.copy()
        environment["LOCALAPPDATA"] = temporary
        # An empty backend set deterministically fails before app creation on
        # every Windows runner, regardless of its GPU/driver capabilities.
        environment["WGPU_BACKEND"] = ""
        stderr_path = Path(temporary) / "stderr.txt"
        with stderr_path.open("wb") as stderr:
            process = subprocess.Popen(
                [str(binary)], env=environment,
                stdout=subprocess.DEVNULL, stderr=stderr,
            )
            try:
                dialogs = []

                @callback_type
                def find_dialog(window, _):
                    owner = wintypes.DWORD()
                    user32.GetWindowThreadProcessId(window, ctypes.byref(owner))
                    if (owner.value == process.pid and user32.IsWindowVisible(window)
                            and window_text(window) == "libreOMR - Fehler"):
                        dialogs.append(window)
                    return True

                deadline = time.monotonic() + 45
                while time.monotonic() < deadline and process.poll() is None:
                    user32.EnumWindows(find_dialog, 0)
                    if dialogs:
                        break
                    time.sleep(0.1)
                if not dialogs:
                    raise AssertionError("No visible native error dialog after renderer failure")

                text = []

                @callback_type
                def collect_text(window, _):
                    text.append(window_text(window))
                    return True

                user32.EnumChildWindows(dialogs[0], collect_text, 0)
                description = "\n".join(text)
                for expected in ("Technische Details:", "Fehlerbericht:", "error.log", "Direct3D 12"):
                    if expected not in description:
                        raise AssertionError(f"Error dialog is missing {expected!r}: {description}")
                # Dismiss OK through the dialog's normal command handler.
                if not user32.PostMessageW(dialogs[0], 0x0111, 1, 0):  # WM_COMMAND / IDOK
                    raise ctypes.WinError(ctypes.get_last_error())
                if process.wait(timeout=10) == 0:
                    raise AssertionError("Renderer failure returned a successful exit code")
                report = (Path(temporary) / "libreOMR/error.log").read_text(encoding="utf-8")
                if "libreOMR" not in report or "windows" not in report or "wgpu" not in report:
                    raise AssertionError(f"Incomplete error report: {report}")
            except BaseException:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=10)
                print(stderr_path.read_text(encoding="utf-8", errors="replace"), file=sys.stderr)
                raise
    print("Windows renderer failure: visible dialog, diagnostic log and failure exit code verified")


if __name__ == "__main__":
    if sys.platform != "win32":
        raise SystemExit("This smoke test requires Windows")
    check(Path(sys.argv[1]).resolve(strict=True))
