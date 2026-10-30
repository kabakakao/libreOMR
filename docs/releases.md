# Builds and releases

Regular CI produces downloadable development builds. Versioned releases start
from a Git tag and are initially created as drafts. Supported targets are
Windows x86_64 and ARM64, Linux x86_64 and macOS Apple Silicon.

## Development builds

After successful tests and builds, `Build and Test` uploads an artifact for each
platform containing a native archive and its SHA-256 checksum. Artifacts remain
available in the Actions run for 14 days. These packages use the optimized `ci`
profile. The filename and `build-info.json` identify the commit. Pull requests
also receive artifacts without requiring write access to repository contents.

The matrix uses Ubuntu 22.04, `windows-latest` for Windows x86_64,
`windows-11-arm` for Windows ARM64 and `macos-15` for Apple Silicon.
Windows ARM64 is built and tested with the native
`stable-aarch64-pc-windows-msvc` Rust toolchain. Both Windows variants receive
separate ZIP archives whose filenames identify the architecture, for example
`libreomr-0.2.0-aarch64-pc-windows-msvc.zip` for ARM64.

The fixed Ubuntu version limits the required glibc version; the application is
still dynamically linked against desktop system libraries. Packages are portable
archives rather than installers or complete Linux bundles. The macOS bundle is
not signed or notarized.

## Prepare a release

1. Set the version in `Cargo.toml`, for example `0.2.0` or `0.2.0-beta.1`.
2. Run `cargo check --locked`. After changing the version, first run `cargo check`
   to update the project's version in `Cargo.lock`.
3. Prepare `docs/release-notes/<version>.md`, for example
   `docs/release-notes/0.2.0.md`. The workflow verifies that it exists and is not
   empty, then uses its contents for the release draft.
4. Commit the changes, including `Cargo.lock` and release notes, and push to
   `main`. Wait for the regular CI run to succeed.
5. Tag the intended commit with the exact matching version and push the tag:

```sh
git tag -a v0.2.0 -m "libreOMR 0.2.0"
git push origin v0.2.0
```

For a prerelease, use a tag such as `v0.2.0-beta.1`. A tag alone does not publish
a release. The `Release` workflow first checks the version against `Cargo.toml`,
then runs all quality checks and tests again for that exact commit. Only then
does it build the four platform packages with `cargo build --release --locked`.

A source archive is also created from tracked project files and Cargo
dependencies pinned by `Cargo.lock`. `cargo vendor` places dependencies in
`vendor/`; an included Cargo configuration allows offline builds with an
installed Rust toolchain and system dependencies. This archive accompanies the
binary packages in the GPL release. Private and ignored files are not taken
from the working directory.

After successful builds, the workflow creates a **GitHub release draft** with
the prepared notes, five archives and `SHA256SUMS`. Tags containing a hyphen
are marked as prereleases. Only the final draft-creation job has write access
to repository contents. Other jobs need read access only; a personal token is
not required.

Open the draft under **Releases**, complete the notes and try the packages on
their target platforms. Publish it there when ready. README download links
point to the release overview.

Rerunning the workflow does not overwrite an existing release. If only draft
creation failed, rerun that job. If the draft already exists, inspect its files;
use a new version for changed code and never move tags of published versions.

## Package contents and licenses

`scripts/package_release.py` uses an explicit file selection for binary packages:
the application, benchmark executable, user guide, documentation, public sample score, guide
images, and license/copyright notices. Language catalogs are embedded in the
application; the package needs no separate locale directory. User settings
remain outside the package and score projects.

Linux packages also contain a desktop entry, icons under `share/icons/hicolor`
and `install-desktop.py` for the current user's application menu. Before packaging
Windows builds, the scripts check that embedded icons can be read from the EXE.
macOS bundles are checked for their icon reference and a valid ICNS file.
Private scans, local exports, test data and build directories are excluded.
`build-info.json` records version, commit, platform and build profile.

Project code is licensed under `GPL-3.0-or-later`. `LICENSE` contains the full
GPLv3 text; `COPYRIGHT` grants use under version 3 or later. Declared licenses
from the resolved Cargo dependency graph appear in `THIRD_PARTY_NOTICES.md`;
available original notices are included under `licenses/`. Missing texts in
Cargo archives are supplied from the
[supplementary notices](../assets/dependency-licenses/README.md), preserving
source URLs and checksums. Dependency updates may require updated copies for
the new versions.

The list also includes build, test and other-platform dependencies; it does not
claim that every component is used in every executable. Fonts and the sample
score retain their own license or public-domain notices. In macOS bundles,
license texts also appear under `Contents/Resources` so they remain with the
application when it is copied.

## Local verification

Python tests require Python 3.11 or newer; the packaging script itself requires
Python 3.9 or newer. After building, create a local package with:

```sh
cargo build --profile ci --locked
python3 scripts/package_release.py --profile ci
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Output is written to the ignored `dist/` directory. Untagged build filenames
include `dev`, the commit ID and, if there are local changes, `dirty`.
Official packages require a clean checkout, the release profile and a matching
version:

```sh
python3 scripts/package_release.py --check-tag v0.2.0
cargo build --release --locked
python3 scripts/package_release.py --profile release --release-tag v0.2.0
python3 scripts/package_release.py --source-tag v0.2.0
```

The Apple Silicon app bundle is created automatically on macOS. To create a
bundle without an archive, use `sh scripts/build-macos-app.sh`.
