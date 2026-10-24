from pathlib import Path
import struct
import tempfile
import unittest

import install_linux_desktop as desktop
import package_release as packaging


class IconTests(unittest.TestCase):
    def test_exported_pngs_have_the_expected_dimensions(self):
        for size in packaging.ICON_SIZES:
            with self.subTest(size=size):
                data = (packaging.ROOT / f"assets/icons/{size}x{size}/{packaging.APP_ID}.png").read_bytes()
                self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
                self.assertEqual(struct.unpack(">II", data[16:24]), (size, size))

    def test_windows_icon_contains_small_and_high_dpi_images(self):
        data = (packaging.ROOT / "assets/icons/libreomr.ico").read_bytes()
        reserved, kind, count = struct.unpack_from("<HHH", data)
        self.assertEqual((reserved, kind), (0, 1))
        sizes = set()
        for index in range(count):
            width, height, _, _, _, _, length, offset = struct.unpack_from("<BBBBHHII", data, 6 + index * 16)
            width, height = width or 256, height or 256
            sizes.add(width)
            image = data[offset:offset + length]
            self.assertEqual(len(image), length)
            self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
            self.assertEqual(struct.unpack(">II", image[16:24]), (width, height))
        self.assertEqual(sizes, {size for size in packaging.ICON_SIZES if size <= 256})

    def test_linux_launcher_installation_uses_current_bundle_location(self):
        with tempfile.TemporaryDirectory() as temporary:
            package = Path(temporary) / "Noten Programme 100%"
            package.mkdir()
            (package / "libreomr").write_bytes(b"binary")
            packaging.copy_linux_desktop(packaging.ROOT, package)
            data_directory = Path(temporary) / "user data"
            entry = desktop.install(package, data_directory)
            text = entry.read_text(encoding="utf-8")
            self.assertIn(f"Icon={desktop.APP_ID}\n", text)
            self.assertIn("Noten Programme 100%%", text)
            self.assertNotIn("Exec=libreomr\n", text)
            self.assertEqual(entry.parent, data_directory / "applications")
            for size in packaging.ICON_SIZES:
                installed = data_directory / f"icons/hicolor/{size}x{size}/apps/{desktop.APP_ID}.png"
                original = packaging.ROOT / f"assets/icons/{size}x{size}/{desktop.APP_ID}.png"
                self.assertEqual(installed.read_bytes(), original.read_bytes())

    def test_linux_launcher_quotes_special_characters_and_rejects_line_injection(self):
        self.assertEqual(
            desktop.quote_executable('/tmp/Musik $noten "Test"/100%/libreomr'),
            r'"/tmp/Musik \\$noten \\"Test\\"/100%%/libreomr"',
        )
        for path in ("/tmp/foo\nExec=bad", "/tmp/foo\rbar", "/tmp/foo=bar"):
            with self.assertRaises(ValueError):
                desktop.quote_executable(path)


if __name__ == "__main__":
    unittest.main()
