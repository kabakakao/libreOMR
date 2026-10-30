#!/usr/bin/env python3
"""Package native builds from an explicit list of distributable files."""

import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import tarfile
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parents[1]
PROJECT_URL = "https://github.com/kabakakao/libreOMR"
TARGETS = {
    "x86_64-unknown-linux-gnu": "tar.gz",
    "x86_64-pc-windows-msvc": "zip",
    "aarch64-pc-windows-msvc": "zip",
    "aarch64-apple-darwin": "zip",
}
APP_ID = "org.libreomr.libreomr"
ICON_SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)


def command(args, root=ROOT):
    return subprocess.check_output(args, cwd=root, text=True).strip()


def metadata(root=ROOT, dependencies=False):
    args = ["cargo", "metadata", "--locked", "--format-version", "1"]
    if not dependencies:
        args.append("--no-deps")
    return json.loads(command(args, root))


def project_package(data):
    return next(p for p in data["packages"] if p["id"] in data["workspace_members"])


def check_tag(version, tag):
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.+-]+)?", version):
        raise ValueError(f"Unexpected package version: {version}")
    if tag != f"v{version}":
        raise ValueError(f"Tag {tag!r} does not match Cargo.toml version v{version}")


def native_target():
    version = command(["rustc", "-vV"])
    return next(line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: "))


def license_files(package):
    directory = Path(package["manifest_path"]).parent
    found = set()
    explicit = package.get("license_file")
    if explicit and (directory / explicit).is_file():
        found.add(directory / explicit)
    for path in directory.rglob("*"):
        if not path.is_file() or path.is_symlink():
            continue
        name = path.name.lower()
        legal_name = any(word in name for word in ("license", "licence", "copying", "copyright", "notice")) or name == "authors"
        font_notice = name in ("ofl.txt", "ufl.txt") or ("fonts" in path.parts and path.suffix == ".txt")
        if legal_name or font_notice:
            found.add(path)
    return sorted(path for path in found if path.resolve().is_relative_to(directory.resolve()))


def copy_notices(root, stage, data):
    destination = stage / "licenses"
    destination.mkdir()
    shutil.copy2(root / "assets/Bravura-LICENSE.txt", destination)
    ocr_notices = root / "assets/ocr"
    if ocr_notices.exists():
        for name in ("LICENSE.txt", "README.md"):
            shutil.copy2(ocr_notices / name, destination / f"ocr-models-{name}")
    supplemental = root / "assets/dependency-licenses"
    sources_path = supplemental / "sources.json"
    sources = json.loads(sources_path.read_text(encoding="utf-8")) if sources_path.exists() else {}
    if sources:
        shutil.copy2(sources_path, destination / "upstream-sources.json")
        explanation = supplemental / "README.md"
        if explanation.exists():
            shutil.copy2(explanation, destination / "upstream-README.md")
    rows = [
        "# Third-party components", "",
        "Declared licenses from Cargo.lock's resolved dependency graph, including",
        "build/test and platform-specific dependencies that may not be in every binary.",
        "Original notices are included under licenses/. For texts omitted from crate",
        "archives, upstream-sources.json records pinned upstream URLs and checksums.", "",
        "| Package | Version | Declared license |", "| --- | --- | --- |",
    ]
    for package in sorted(data["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] in data["workspace_members"]:
            continue
        rows.append(f'| {package["name"]} | {package["version"]} | {package.get("license") or "See license file"} |')
        directory = Path(package["manifest_path"]).parent
        package_name = f'{package["name"]}-{package["version"]}'
        originals = license_files(package)
        extra = sources.get(package_name, [])
        if not any(path.name.lower() != "authors" for path in originals) and not extra:
            raise ValueError(f"Missing license text for {package_name}; add a pinned upstream copy")
        for source in originals:
            relative = source.relative_to(directory)
            target = destination / package_name / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        for entry in extra:
            source = supplemental / package_name / entry["path"]
            if hashlib.sha256(source.read_bytes()).hexdigest() != entry["sha256"]:
                raise ValueError(f"Upstream license checksum mismatch: {source}")
            target = destination / package_name / "upstream" / entry["path"]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
    (stage / "THIRD_PARTY_NOTICES.md").write_text("\n".join(rows) + "\n", encoding="utf-8")


def validate_macos_bundle(bundle):
    with (bundle / "Contents/Info.plist").open("rb") as source:
        info = plistlib.load(source)
    if info.get("CFBundleIdentifier") != APP_ID or info.get("CFBundleExecutable") != "libreomr":
        raise ValueError("Unexpected macOS bundle identifier or executable")
    icon_name = info.get("CFBundleIconFile", "")
    if not icon_name or Path(icon_name).name != icon_name:
        raise ValueError("Missing or invalid macOS bundle icon reference")
    icon = (bundle / "Contents/Resources" / icon_name).read_bytes()
    if len(icon) <= 8 or icon[:4] != b"icns" or struct.unpack(">I", icon[4:8])[0] != len(icon):
        raise ValueError("Invalid macOS ICNS icon")
    if not (bundle / "Contents/MacOS/libreomr").is_file():
        raise ValueError("Missing macOS application executable")


def copy_linux_desktop(root, stage):
    applications = stage / "share/applications"
    applications.mkdir(parents=True)
    shutil.copy2(root / f"assets/linux/{APP_ID}.desktop", applications)
    for size in ICON_SIZES:
        destination = stage / f"share/icons/hicolor/{size}x{size}/apps"
        destination.mkdir(parents=True)
        shutil.copy2(root / f"assets/icons/{size}x{size}/{APP_ID}.png", destination)
    shutil.copy2(root / "scripts/install_linux_desktop.py", stage / "install-desktop.py")


def assemble(root, stage, data, profile, target, commit, dirty, tag=None):
    version = project_package(data)["version"]
    if target not in TARGETS:
        raise ValueError(f"Unsupported native target: {target}")
    if tag:
        check_tag(version, tag)
        if profile != "release" or dirty:
            raise ValueError("Tagged packages require a clean checkout and the release profile")
    binary_dir = root / "target" / profile
    extension = ".exe" if target.endswith("windows-msvc") else ""
    for name in ("README.md", "LICENSE", "COPYRIGHT"):
        shutil.copy2(root / name, stage / name)
    shutil.copytree(root / "docs", stage / "docs")
    # Only public example material and the README's images are distributed.
    shutil.copytree(root / "examples/public-domain", stage / "examples/public-domain")
    (stage / "assets").mkdir()
    shutil.copy2(root / "assets/libreomr-logo.png", stage / "assets/libreomr-logo.png")
    shutil.copy2(root / "assets/Bravura-LICENSE.txt", stage / "assets/Bravura-LICENSE.txt")
    shutil.copytree(root / "assets/screenshots", stage / "assets/screenshots")
    copy_notices(root, stage, data)
    if target.endswith("apple-darwin"):
        validate_macos_bundle(binary_dir / "libreOMR.app")
        shutil.copytree(binary_dir / "libreOMR.app", stage / "libreOMR.app")
        resources = stage / "libreOMR.app/Contents/Resources"
        shutil.copytree(stage / "licenses", resources / "licenses")
        for name in ("LICENSE", "COPYRIGHT", "THIRD_PARTY_NOTICES.md"):
            shutil.copy2(stage / name, resources / name)
    else:
        shutil.copy2(binary_dir / f"libreomr{extension}", stage)
        if target.endswith("linux-gnu"):
            copy_linux_desktop(root, stage)
    shutil.copy2(binary_dir / f"omr-benchmark{extension}", stage)
    (stage / "build-info.json").write_text(json.dumps({
        "version": version, "commit": commit, "dirty": dirty,
        "target": target, "profile": profile, "tag": tag,
        "source": f"{PROJECT_URL}/tree/{commit}",
    }, indent=2) + "\n", encoding="utf-8")


def archive_package(stage, output, target):
    extension = "tar.gz" if target == "source" else TARGETS[target]
    archive = output / f"{stage.name}.{extension}"
    executables = {"libreomr", "omr-benchmark", "libreOMR.app/Contents/MacOS/libreomr"}
    if extension == "tar.gz":
        def permissions(info):
            if info.isfile() and info.name.removeprefix(stage.name + "/") in executables:
                info.mode = 0o755
            return info

        with tarfile.open(archive, "w:gz") as bundle:
            bundle.add(stage, arcname=stage.name, filter=permissions)
    else:
        # Registry crates can have normalized timestamps predating ZIP's 1980 epoch.
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, strict_timestamps=False) as bundle:
            for path in sorted(stage.rglob("*")):
                relative = path.relative_to(stage.parent)
                if path.is_file() and path.relative_to(stage).as_posix() in executables:
                    info = zipfile.ZipInfo.from_file(path, relative, strict_timestamps=False)
                    info.create_system = 3  # Unix mode bits, including on a Windows packaging host.
                    info.external_attr = (stat.S_IFREG | 0o755) << 16
                    bundle.writestr(info, path.read_bytes(), compress_type=zipfile.ZIP_DEFLATED)
                else:
                    bundle.write(path, relative)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (output / f"{archive.name}.sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
    return archive


def source_package(root, output, tag):
    version = project_package(metadata(root))["version"]
    check_tag(version, tag)
    if command(["git", "status", "--porcelain"], root):
        raise ValueError("Tagged source packages require a clean checkout")
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="source-", dir=output) as temporary:
        temporary = Path(temporary)
        stage = temporary / f"libreomr-{version}-source"
        stage.mkdir()
        # git archive includes only committed files, so private/ignored data stays out.
        source_tar = temporary / "tracked.tar"
        subprocess.run(["git", "archive", "--format=tar", f"--output={source_tar}", "HEAD"], cwd=root, check=True)
        subprocess.run(["tar", "-xf", str(source_tar), "-C", str(stage)], check=True)
        vendor = stage / "vendor"
        config = command(["cargo", "vendor", "--locked", "--versioned-dirs", str(vendor)], root)
        config = config.replace(str(vendor), "vendor")
        config_path = stage / ".cargo/config.toml"
        config_path.parent.mkdir(exist_ok=True)
        existing = config_path.read_text(encoding="utf-8") if config_path.exists() else ""
        config_path.write_text(existing + "\n" + config + "\n", encoding="utf-8")
        (stage / "BUILD-SOURCE.txt").write_text(
            "This archive includes the tagged source and the locked Cargo dependencies.\n"
            "With Rust 1.92+ and the platform's native build tools/libraries installed:\n"
            "  cargo build --release --locked --offline\n"
            "Dependency notices are in vendor/ and assets/dependency-licenses/.\n",
            encoding="utf-8",
        )
        return archive_package(stage, output, "source")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("ci", "release"), default="release")
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    parser.add_argument("--release-tag")
    parser.add_argument("--check-tag", help="Only check tag/version consistency; do not build/package")
    parser.add_argument("--check-licenses", action="store_true", help="Check license texts for the complete locked dependency graph without built binaries")
    parser.add_argument("--source-tag", help="Package the tagged source with vendored dependencies")
    args = parser.parse_args()
    version = project_package(metadata())["version"]
    if args.check_tag:
        check_tag(version, args.check_tag)
        print(f"Validated {args.check_tag}")
        return
    if args.source_tag:
        print(source_package(ROOT, args.output, args.source_tag))
        return
    if args.check_licenses:
        data = metadata(dependencies=True)
        with tempfile.TemporaryDirectory(prefix="license-check-") as temporary:
            copy_notices(ROOT, Path(temporary), data)
        print("Validated license texts for the complete locked dependency graph")
        return
    target = native_target()
    if target not in TARGETS:
        raise ValueError(f"Unsupported native target: {target}")
    commit = command(["git", "rev-parse", "HEAD"])
    dirty = bool(command(["git", "status", "--porcelain"]))
    if args.release_tag:
        check_tag(version, args.release_tag)
        if args.profile != "release" or dirty:
            raise ValueError("Tagged packages require a clean checkout and the release profile")
    if target.endswith("apple-darwin"):
        subprocess.run(["sh", "scripts/build-macos-app.sh", "--profile", args.profile, "--no-build"], cwd=ROOT, check=True)
    if target.endswith("windows-msvc"):
        from check_windows_icon import check
        check(ROOT / "target" / args.profile / "libreomr.exe")
    # Metadata resolves the complete graph and makes the crate license files available.
    data = metadata(dependencies=True)
    args.output.mkdir(parents=True, exist_ok=True)
    label = version if args.release_tag else f"{version}-dev-{commit[:8]}" + ("-dirty" if dirty else "")
    with tempfile.TemporaryDirectory(prefix="package-", dir=args.output) as temporary:
        stage = Path(temporary) / f"libreomr-{label}-{target}"
        stage.mkdir()
        assemble(ROOT, stage, data, args.profile, target, commit, dirty, args.release_tag)
        print(archive_package(stage, args.output, target))


if __name__ == "__main__":
    main()
