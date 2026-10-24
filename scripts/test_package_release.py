import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import unittest
from unittest.mock import patch
import zipfile

import package_release as packaging


class PackagingTests(unittest.TestCase):
    def test_manifest_binary_names_match_packaging_contract(self):
        manifest = tomllib.loads((packaging.ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        binaries = {target["name"] for target in manifest["bin"]}
        self.assertIn("libreomr", binaries)
        self.assertIn("omr-benchmark", binaries)
        self.assertEqual(manifest["package"]["default-run"], "libreomr")

    def fixture(self, parent):
        root = parent / "project"
        root.mkdir()
        files = {
            "README.md": "Usage", "LICENSE": "Project license", "COPYRIGHT": "Project notice",
            "assets/Bravura-LICENSE.txt": "OFL", "assets/libreomr-logo.png": "logo",
            "assets/screenshots/overview.png": "public screenshot",
            "examples/public-domain/README.md": "Public Domain source",
            "examples/public-domain/bach.pdf": "public score",
            "local/scores/private.pdf": "DO NOT DISTRIBUTE",
            "target/ci/unrelated.json": "DO NOT DISTRIBUTE",
        }
        for name, content in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        for directory in ("assets/icons", "assets/linux"):
            shutil.copytree(packaging.ROOT / directory, root / directory)
        (root / "scripts").mkdir()
        shutil.copy2(packaging.ROOT / "scripts/install_linux_desktop.py", root / "scripts")
        for name in ("libreomr", "omr-benchmark", "libreomr.exe", "omr-benchmark.exe"):
            binary = root / "target/ci" / name
            binary.write_bytes(b"native binary")
            binary.chmod(0o755)
        app_binary = root / "target/ci/libreOMR.app/Contents/MacOS/libreomr"
        app_binary.parent.mkdir(parents=True)
        app_binary.write_bytes(b"native binary")
        app_binary.chmod(0o755)
        (app_binary.parent.parent / "Resources").mkdir()
        (app_binary.parent.parent / "Info.plist").write_bytes(plistlib.dumps({
            "CFBundleIdentifier": packaging.APP_ID,
            "CFBundleExecutable": "libreomr",
            "CFBundleIconFile": "libreomr.icns",
        }))
        (app_binary.parent.parent / "Resources/libreomr.icns").write_bytes(
            b"icns\x00\x00\x00\x10icp4\x00\x00\x00\x08"
        )
        dependency = parent / "dependency"
        (dependency / "fonts").mkdir(parents=True)
        (dependency / "LICENSE-MIT").write_text("dependency license")
        os.utime(dependency / "LICENSE-MIT", (0, 0))
        (dependency / "fonts/Hack-Regular.txt").write_text("embedded font notice")
        data = {
            "workspace_members": ["project"],
            "packages": [
                {"id": "project", "name": "libreomr", "version": "0.1.0", "manifest_path": str(root / "Cargo.toml")},
                {"id": "dependency", "name": "dependency", "version": "1.0.0", "license": "MIT", "manifest_path": str(dependency / "Cargo.toml")},
            ],
        }
        return root, data

    def test_binary_archives_include_notices_and_public_example_only(self):
        for target in ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc", "aarch64-apple-darwin"):
            with self.subTest(target=target), tempfile.TemporaryDirectory() as temporary:
                parent = Path(temporary)
                root, data = self.fixture(parent)
                stage = parent / "bundle"
                stage.mkdir()
                packaging.assemble(root, stage, data, "ci", target, "a" * 40, True)
                archive = packaging.archive_package(stage, parent, target)
                if archive.suffix == ".zip":
                    with zipfile.ZipFile(archive) as bundle:
                        names = bundle.namelist()
                        if target.endswith("apple-darwin"):
                            executable = "bundle/libreOMR.app/Contents/MacOS/libreomr"
                            self.assertEqual(bundle.getinfo(executable).external_attr >> 16 & 0o777, 0o755)
                            self.assertIn("bundle/libreOMR.app/Contents/Resources/libreomr.icns", names)
                            for notice in ("LICENSE", "COPYRIGHT", "THIRD_PARTY_NOTICES.md", "licenses/dependency-1.0.0/LICENSE-MIT", "licenses/Bravura-LICENSE.txt"):
                                self.assertIn(f"bundle/libreOMR.app/Contents/Resources/{notice}", names)
                        else:
                            self.assertIn("bundle/libreomr.exe", names)
                            self.assertIn("bundle/omr-benchmark.exe", names)
                else:
                    with tarfile.open(archive) as bundle:
                        names = bundle.getnames()
                        self.assertEqual(bundle.getmember("bundle/libreomr").mode & 0o777, 0o755)
                        self.assertEqual(bundle.getmember("bundle/omr-benchmark").mode & 0o777, 0o755)
                        self.assertIn("bundle/install-desktop.py", names)
                        self.assertIn(f"bundle/share/applications/{packaging.APP_ID}.desktop", names)
                        for size in packaging.ICON_SIZES:
                            self.assertIn(f"bundle/share/icons/hicolor/{size}x{size}/apps/{packaging.APP_ID}.png", names)
                for expected in ("LICENSE", "COPYRIGHT", "THIRD_PARTY_NOTICES.md", "examples/public-domain/bach.pdf", "licenses/dependency-1.0.0/LICENSE-MIT", "licenses/dependency-1.0.0/fonts/Hack-Regular.txt"):
                    self.assertIn(f"bundle/{expected}", names)
                self.assertFalse(any("private.pdf" in name or "/target/" in name or "/local/" in name for name in names))
                checksum = archive.with_name(archive.name + ".sha256").read_text()
                self.assertEqual(checksum, f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
                info = json.loads((stage / "build-info.json").read_text())
                self.assertTrue(info["dirty"])
                self.assertEqual(info["profile"], "ci")
                self.assertEqual(info["target"], target)

    def test_macos_packaging_rejects_missing_or_corrupt_bundle_icons(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, _ = self.fixture(Path(temporary))
            bundle = root / "target/ci/libreOMR.app"
            packaging.validate_macos_bundle(bundle)
            icon = bundle / "Contents/Resources/libreomr.icns"
            icon.write_bytes(b"not an icon")
            with self.assertRaisesRegex(ValueError, "ICNS"):
                packaging.validate_macos_bundle(bundle)
            icon.unlink()
            with self.assertRaises(FileNotFoundError):
                packaging.validate_macos_bundle(bundle)

    def test_release_rejects_version_mismatch_dirty_tree_and_ci_profile(self):
        packaging.check_tag("0.1.0-alpha.1", "v0.1.0-alpha.1")
        for tag in ("0.1.0", "v0.2.0", "v0.1.0/other"):
            with self.assertRaises(ValueError):
                packaging.check_tag("0.1.0", tag)
        with tempfile.TemporaryDirectory() as temporary:
            root, data = self.fixture(Path(temporary))
            for profile, dirty in (("ci", False), ("release", True)):
                with self.assertRaises(ValueError):
                    packaging.assemble(root, root, data, profile, "x86_64-unknown-linux-gnu", "a" * 40, dirty, "v0.1.0")
            with self.assertRaisesRegex(ValueError, "Unsupported"):
                packaging.assemble(root, root, data, "release", "x86_64-apple-darwin", "a" * 40, False)

    def test_upstream_notices_are_included_and_checksum_changes_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root, data = self.fixture(parent)
            dependency = parent / "without-notice"
            dependency.mkdir()
            data["packages"].append({"id": "missing", "name": "missing", "version": "1.0.0", "license": "MIT", "manifest_path": str(dependency / "Cargo.toml")})
            stage = parent / "missing-stage"
            stage.mkdir()
            with self.assertRaisesRegex(ValueError, "Missing license text"):
                packaging.copy_notices(root, stage, data)
            supplemental = root / "assets/dependency-licenses"
            original = supplemental / "missing-1.0.0/LICENSE"
            original.parent.mkdir(parents=True)
            original.write_bytes(b"Original MIT license")
            sources = {"missing-1.0.0": [{"path": "LICENSE", "url": "https://example.invalid/commit/LICENSE", "sha256": hashlib.sha256(original.read_bytes()).hexdigest()}]}
            (supplemental / "sources.json").write_text(json.dumps(sources))
            stage = parent / "included-stage"
            stage.mkdir()
            packaging.copy_notices(root, stage, data)
            self.assertEqual((stage / "licenses/missing-1.0.0/upstream/LICENSE").read_bytes(), original.read_bytes())
            self.assertEqual(json.loads((stage / "licenses/upstream-sources.json").read_text()), sources)
            original.write_bytes(b"Changed")
            stage = parent / "changed-stage"
            stage.mkdir()
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                packaging.copy_notices(root, stage, data)

    def test_pinned_upstream_notices_survive_windows_checkout(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root = parent / "repository"
            root.mkdir()
            shutil.copy2(packaging.ROOT / ".gitattributes", root / ".gitattributes")
            shutil.copytree(packaging.ROOT / "assets/dependency-licenses", root / "assets/dependency-licenses")
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            subprocess.run(["git", "-c", "core.autocrlf=false", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "-c", "user.name=Packaging test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture"], cwd=root, check=True)
            checkout = parent / "checkout"
            subprocess.run([
                "git", "clone", "--quiet", "--no-local",
                "--config", "core.autocrlf=true", "--config", "core.eol=crlf",
                str(root), str(checkout),
            ], check=True)
            supplemental = checkout / "assets/dependency-licenses"
            sources = json.loads((supplemental / "sources.json").read_text(encoding="utf-8"))
            for package, entries in sources.items():
                for entry in entries:
                    source = supplemental / package / entry["path"]
                    self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), entry["sha256"], str(source))

    def test_source_archive_contains_committed_files_and_relative_vendor_config(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root, data = self.fixture(parent)
            (root / ".gitignore").write_text("/local/\n/target/\n/dist/\n")
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "-c", "user.name=Packaging test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture"], cwd=root, check=True)
            original_command = packaging.command

            def command(args, cwd):
                if args[:2] == ["cargo", "vendor"]:
                    vendor = Path(args[-1])
                    (vendor / "dependency-1.0.0").mkdir(parents=True)
                    (vendor / "dependency-1.0.0/LICENSE").write_text("MIT")
                    return f'[source.crates-io]\nreplace-with = "vendored"\n[source.vendored]\ndirectory = "{vendor}"\n'
                return original_command(args, cwd)

            with patch.object(packaging, "metadata", return_value=data), patch.object(packaging, "command", side_effect=command):
                archive = packaging.source_package(root, root / "dist", "v0.1.0")
            with tarfile.open(archive) as bundle:
                names = bundle.getnames()
                prefix = "libreomr-0.1.0-source/"
                config = bundle.extractfile(prefix + ".cargo/config.toml").read().decode()
                self.assertIn('directory = "vendor"', config)
                self.assertNotIn(str(parent), config)
                self.assertIn(prefix + "vendor/dependency-1.0.0/LICENSE", names)
                self.assertFalse(any("/local/" in name or "/target/" in name or "/.git/" in name for name in names))


if __name__ == "__main__":
    unittest.main()
