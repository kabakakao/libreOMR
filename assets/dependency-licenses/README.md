# Supplementary license texts

Some crates pinned by `Cargo.lock` do not include their own license text in the
published Cargo archive. This directory contains supplementary notices from
their upstream repositories, each associated with a specific package version.

`sources.json` records each file's source and SHA-256 checksum. Git sources
point to fixed commits from Cargo VCS metadata or the corresponding version
tags. Files are copied unchanged. `.gitattributes` disables automatic line-ending
conversion for this directory so checksums remain valid in Windows checkouts.

The older `block`, `dispatch` and `malloc_buf` packages declare MIT in their
manifests but include no separate license text. Their original manifests with
author/license declarations and the standard SPDX License List text are included.
For `hexf-parse`, the CC0 declaration and standard CC0 text are included.
Placeholders in these standard texts come from the SPDX template and are not
presented as additional copyright claims. For `r-efi`, `AUTHORS` contains the
license texts.

The FlatBuffers and RTen crates used by OCR omit the repository-level license
files from their published archives. Their supplemental files come from the
exact commits recorded in each crate's `.cargo_vcs_info.json`.

Packaging copies these files and their source index into `licenses/` and
verifies their checksums. Dependency updates require missing notices for new
versions to be copied from the corresponding source and recorded in
`sources.json`. The project license does not change these components' licenses.
Run `python scripts/package_release.py --check-licenses` to validate the complete
locked dependency graph without building application binaries; CI also runs this
check in the quality job.
