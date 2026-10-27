# Development and technical background

This document covers architecture, recognition limitations and local regression
tests. Run shell examples from the project root. See the [README](../README.md)
for usage and [Builds and releases](releases.md) for artifacts and publishing.
Language files and user preferences are described in [Localization](localization.md).

## CI runtime

Image and template matching require optimized dependencies. The workflow uses
the `ci` profile with `opt-level = 2` for tests and builds. Project code keeps
debug assertions and overflow checks enabled; dependencies use release settings
for those checks. Reduced debug information and disabled incremental compilation
limit build and cache size. Run the same checks locally with:

```sh
cargo test --profile ci --locked
cargo build --profile ci --locked
```

All Rust tests and binaries are checked on Linux, Windows x86_64 and ARM64, and
macOS Apple Silicon. Formatting, Clippy, Python tests and the benchmark without
desktop features run once in a separate Linux job. Dependencies are cached per
job/platform, including failed runs. A new profile initially needs a new cache.

The matrix separates test compilation from execution, with limits of 30 and
10 minutes respectively and a 45-minute job limit. A new run cancels older runs
for the same Git ref. Successful regular builds upload development artifacts
for 14 days. Release tags reuse CI and prepare publication in a separate workflow.

## Public-domain regression example

The [BWV 1013 Partita](../examples/public-domain/README.md) is a public clef
recognition regression. Its treble clefs are narrower than the original Bravura
template. Matching considers additional widths and heights. If normal matching
fails, a larger crop and staff-line cleanup preserve thin clef contours better.
Quality thresholds and treble/bass discrimination remain unchanged.

`cargo test --locked public_bach_partita` checks all 64 clefs across five pages,
and checks that the first staff's clef and common-time symbol do not become
notes or rests while the first real sixteenth note is retained. No private scans
are required. See [Screenshots](../assets/screenshots/README.md) for captures.

## Architecture

- `src/model.rs`: notes, rests, measure boundaries, measure-scoped accidentals,
  staff key signatures, part assignments, lyrics, slurs/ties/wedges and the
  versioned project format.
- `src/score_structure.rs`: bracketed system grouping, persistent part proposals,
  positioned lyric alignment and safe lyric references when notes are deleted.
- `src/pdf_text.rs`: native positioned Unicode extraction using the PDF
  rasterizer's crop, rotation and scale transform.
- `src/rhythm.rs`: sums note values per measure and compares them with the active
  time signature for validity overlays in the scan and preview.
- `src/recognition.rs`: conservative deskewing, binarization and connected
  components with `imageproc`, staff/ledger-line checks, barlines, stem/beam
  analysis, dots and a generated demo image.
- `src/symbols.rs`: notehead templates and cavity checks, scaled template
  matching for staff attributes, flags and rests with `imageproc`. SMuFL templates
  are generated from the bundled Bravura font using `ab_glyph`.
- `src/app.rs`: native `eframe/egui` UI, background processing, image overlay,
  inspector, undo/redo and local project files.
- `src/app/problems.rs`: cached export diagnostics and navigation to corrections.
- `src/i18n.rs` and `locales/*.json`: embedded translations, locale resolution,
  English fallback and named-placeholder substitution.
- `src/settings.rs`: per-user language/theme preferences, atomic persistence
  and the Settings window.
- `src/spans.rs`: geometric proposals for crescendo/decrescendo wedges and
  slurs/ties based on image contours and note pairs.
- `src/preview.rs`: optional staff-based preview of the current model with
  Bravura symbols, scrolling and note/rest selection; transparent accidental
  glyphs and shared slur/tie/wedge drawing for preview and scan overlay.
- `src/document_import.rs`: page discovery and page-by-page PDF/TIFF/image import;
  PDF rasterization with `hayro` at 300 dpi on white, with image-size validation.
  PDF pages share one loaded document without rasterizing all pages up front.
- `src/tiff_import.rs`: TIFF image-directory enumeration and selection, with a
  Fax3 fallback using `hayro-ccitt` and `tiff` metadata. Other compression types
  continue to use the `image` decoder, including on later pages.
- `src/image_limits.rs`: shared image limits for normal/Fax3 import and embedded
  project-image restoration.
- `src/main.rs`: desktop entry point.

This separation allows a future Rust-compatible model runner, such as ONNX,
to replace heuristic recognition without rewriting the correction editor.
Model, vocabulary, license and dataset must be evaluated first. No model weights
are currently included.

The unmodified Bravura font comes from
[steinbergmedia/bravura, commit 37b1943](https://github.com/steinbergmedia/bravura/tree/37b194378b710cc40e406ab6c4b07608bb9548ae)
under SIL Open Font License 1.1. Include
[assets/Bravura-LICENSE.txt](../assets/Bravura-LICENSE.txt) when distributing the
font or a package containing it. Templates do not store crops from user scores.

## Manual voices and multiple selection

Notes and rests store a voice number from 1 to 4, defaulting to 1 when an older
project omits the field. The editor supports Ctrl/Cmd toggles and Shift ranges
across the scan, preview and object list. Shared voice/duration edits and
selection deletion create one history entry; note deletion retains lyric
reference cleanup. Measure validity checks every voice separately, and
MusicXML emits voice elements and backs up the cursor between voices in each
measure. Onsets within a voice still follow source position and written values;
missing leading events must be represented by rests. Recognition initially
assigns voice 1, so independent voices require manual assignment.

Regression coverage: `cargo test --profile ci --locked --test staff_voices`
and `cargo test --profile ci --locked --bin libreomr multiselect`.

## Limitations and next steps

The baseline may fail or produce false notes on skewed scans, short staves,
dense beams, chords, damaged/open hollow shapes, text and other symbols.
Separate heads sharing a stem are detected, but full chord/voice assignment is
not reconstructed. The preview displays the model rather than validated
engraving. Spacing is adjusted for readability; eighth through sixty-fourth
notes use flags instead of reconstructed beam groups. Full voice assignment,
rhythmic completeness and engraving/line-breaking logic remain incomplete.

Staff detection expects lines spanning at least about 45% of image width.
Clef recognition is limited to staff starts and can fail with unfamiliar fonts
or damaged symbols. Weak/ambiguous matches remain unknown, though false
classifications are still possible. Preview and MusicXML carry the time signature
forward until another printed or manually inserted signature. Staves form one
continuous part when unassigned; bracketed systems can carry separate parts
across pages. Independent voices sharing a staff are not reconstructed.
An empty key signature is confirmed only when a clear
gap is observed between recognized clef and meter; otherwise it stays unknown.

Rest recognition targets upright, largely isolated standard shapes. Short rests
are also searched up to two staff spacings lower and at several print sizes.
At comparable quality, complete rests with more hooks take precedence over
partial matches with fewer hooks, preserving sixteenth rests. Automatic dotted-rest
recognition and polyphonic rhythmic assignment are not implemented.

Multimeasure rests need a centered long bar, two end strokes and a readable
centered number from 2 to 99 above. Other shapes, missing numbers and automatic
temporal splitting of the rested measures are not implemented. Number recognition
is heuristic and editable. Gaps up to 0.08 staff spacings in the bar are bridged;
larger gaps are rejected. Digits need complete contours; multi-hole shapes also
have cavity-area checks. Template quality thresholds remain unchanged.

Automatic triplets are restricted to rest-free groups of three equal base
values sharing a beam and a visible 3. Bracketed triplets, mixed values, dotted
triplet notes and other tuplets are not assigned automatically. Values shorter
than sixty-fourths and full temporal measure/voice reconstruction are absent.

Wedges and slurs/ties are heuristic and incomplete. Bows between adjacent notes
with the same pitch and accidental and no intervening rest are proposed as ties;
ambiguous types remain editable. Nested, heavily overlapping and cross-staff bows
and voice assignment are not specifically supported. Text such as `cresc.` or
`dim.` is not read. Symbols store image coordinates rather than fixed note links;
check endpoints after correcting notes. Tie playback and rhythmic merging are
not implemented.

Articulations are searched near the note axis using shape, size and Bravura
templates. Separate fermata components and combined symbols are compared
together. Cleanup checks actual continuous lines to preserve marks near heads.
Stem/slur overlap, close barlines, text and unfamiliar fonts can still cause
missing or false proposals. Each note has one articulation selection, including
combinations. Independent voices sharing a staff are not assigned.

Dynamic letters must be largely isolated above/below the staff, within six
spacings. Connected groups are matched together so `pp` is not reduced to `p`.
Additional text, collisions, damage and other fonts can prevent recognition.
Assignment between staves considers distance to the staff edge, preferring the
usual position below a staff. Proposals remain unreviewed. MusicXML places them
by horizontal position within each exported part; this does not reconstruct
independent rhythmic timing.

Clef changes within a staff are not implemented. Printed meter changes are
searched after detected barlines and at staff starts. Common time, alla breve
and numeric meters with numerators 1–32 and denominators 1, 2, 4, 8, 16 or 32 are
supported. Boundary checks account for narrow double bars next to meter signs;
other digits remain excluded. Anchors are checked even when a meter digit first
resembles a note. Weak clef matches are realigned to local staff lines, and
slightly curved row ends are tracked across multiple lines to retain courtesy
changes at the edge.

Internal key changes are proposed at detected measure boundaries as compact,
ordered groups with at least two symbols. Flats, sharps and matching leading
cancellation groups are supported, including changes to zero accidentals.
Every symbol must match clef-relative positions without relaxed quality
thresholds. Single symbols, incomplete groups, displaced printing and changes
without a detected boundary may be missed. Ordinary note accidentals must not
become key changes. Changes apply only within their staff; automatic inheritance
into subsequent staves without a printed signature is absent.

Barlines need largely continuous, straight vertical strokes across the full staff
height. Broken, short, skewed or note-adjacent strokes may be missed and are
editable. Repeat classification requires two small aligned dots in the middle
spaces; one dot is insufficient. Stroke widths distinguish equal double bars
from thin/thick final bars. Damaged dots and ambiguous widths may misclassify.
Repeat playback and repeat counts are not implemented.

Segno/coda use Bravura templates, including upright print variants. Voltas need
a long, thin, nearly horizontal roof, a left downstroke and detached labeling.
Digits are matched against music-font templates; other text fonts may need
manual number corrections. Endings continued across line breaks and written
jump instructions such as D.S. al Coda are not reconstructed.

Note accidentals apply within a staff at the exact diatonic pitch and octave,
not to other octaves/staves. Scope resets at each staff start. Measures split
over rows and conflicting simultaneous accidentals in different voices are not
resolved. Double accidentals, parenthesized courtesy signs and tightly displaced
chord accidentals are not specifically supported. Matches may be missed or
misclassified and remain unreviewed. Articulations belong to notes; dynamics
are separate freely movable image objects.

Priorities for further development:

1. Real, licensed test scores and measurable recognition metrics.
2. Dewarping, perspective correction and layout analysis for multicolumn scans.
3. Model-based symbol recognition with uncertain candidates for review.
4. A richer model for clef changes, keys, measures, voices, chords, polyphonic
   rests, semantic note links and rhythmic plausibility.
5. Same-staff polyphonic MusicXML export and MIDI playback. Current part-based
   export does not replace rhythmic/voice reconstruction within a staff.
6. Installable packages for Linux, Windows and macOS.

See [Polyphonic scores and lyrics](polyphonic-scores-and-lyrics.md) for the
SATB sample analysis, model/export gaps and proposed implementation milestones.

## Existing OMR engines

Research recorded on October 14, 2026 found no established, directly usable Rust
OMR engine in crates.io searches for `optical music recognition` and `omr`.
This is not a complete market survey or proof that none exists. `imageproc` and
`ab_glyph` provide image processing and font rasterization, without OMR semantics.

- [Audiveris](https://github.com/Audiveris/audiveris): established OMR with
  interactive correction and MusicXML on Linux, Windows and macOS; Java,
  AGPL-3.0. An optional integration is possible but would not be a pure Rust
  backend and is not implemented here.
- [homr](https://github.com/liebharc/homr): model-based segmentation/transformer
  pipeline producing MusicXML; Python, AGPL-3.0. The project describes image
  positions as approximate. It is not a directly embeddable Rust library.
- [ort](https://github.com/pykeio/ort): Rust ONNX inference interface, normally
  using native ONNX Runtime. It is not an OMR engine and includes no OMR weights.
  A suitable model, license, vocabulary and mapping to editable image positions
  need separate evaluation.

## Optional original-scan regressions

Private/local score scans are not committed. Tests using them are ignored in
normal runs and need the corresponding environment variables. Commands below
use macOS/Linux shell syntax; on Windows set variables in PowerShell.

For the Cassiopeia baritone scan, note-value, accidental and triplet tests check
the original notes, measure-scoped accidental behavior and the second row's
triplets. Repeat/barline checks distinguish the double bar before measure 38
and both repeats. The multirest regression checks the eight-measure rest at 38
without a false note on its digit. Synthetic tests cover counts 2, 3, 5, 7, 8,
9, 12 and 16 at three scales, short/large bar gaps and missing end strokes/digits.
Model/editor tests cover old rests without counts, JSON round trips, invalid
counts and multirest editing with undo/redo at both window sizes.

```sh
LIBREOMR_TEST_IMAGE=/path/to/Cassiopeia-file.tif cargo test external_double_before_38_and_both_repeats -- --ignored
LIBREOMR_TEST_IMAGE=/path/to/Cassiopeia-file.tif cargo test external_multimeasure_rest_at_38_is_not_a_note -- --ignored
LIBREOMR_TEST_IMAGE=/path/to/Cassiopeia-file.tif cargo test external_hairpins_and_bows -- --ignored
```

`external_hairpins_and_bows` checks three wedges, selected bows and four rows
without proposals, rather than complete bow recognition. Synthetic tests cover
all four types at three scales, upper/lower bows, and negatives with staff lines,
beams and diagonal strokes. Tests also cover tie discrimination, old projects,
JSON, invalid geometry, undo/redo and staff previews at both sizes.

Known failure in this optional test: the first row's bow near x ≈ 908 is classified
as a slur rather than a tie because its two notes currently receive different
pitches. This also occurs without dynamics recognition. The test remains strict
and fails there; dynamics/articulation changes do not fix that pitch discrepancy.

`Cassiopeia_-_BassCf-1.tif` has regressions for the lower eighth rest after two
eighth notes in measure 30, the decrescendo in measure 37 outside the previous
crop, and the three-measure rest at 62 with a small bar gap. Synthetic tests check
displaced eighth rests and both wedge directions five spacings outside the
staff. Wedge search extends six spacings above/below without relaxed contour checks.

```sh
LIBREOMR_BASS_TEST_IMAGE=local/scores/Cassiopeia_-_BassCf-1.tif cargo test external_bass_ -- --ignored
```

In `Atlantic_Avenue_02_Oboe.tif`, treble clefs extend beyond the estimated left
edge. The normal header crop is unchanged; unknown clefs trigger a retry with
0.75 spacings of extra left margin. Finer treble scales close size gaps without
lowering thresholds. Tests require all ten treble clefs, initial 12/8 and two
flats; row one has 17 notes and 14 eighth rests without false header notes. Five
multirests have counts 4, 8, 8, 8 and 8.

False filled heads at beam/stem intersections are removed before duration
analysis when opposing stem interpretations and a long thick beam continuation
support rejection. True eighth notes remain and are not reduced to quarters
by false head masks. The original regression checks three locations and their
real eighths; synthetic tests cover clef overhang/12-8 at multiple scales and
beam continuations versus thin lines and isolated heads.

The oboe scan has four internal key changes: four flats in staff 3, one flat
in staff 5, four in staff 6 and two in staff 8. Change symbols, including
cancellations, are excluded before renewed note analysis so they do not become
heads or ordinary accidentals. Tests check all four changes, positions and
following accidental scope. Neither Cassiopeia page may gain internal changes.
Synthetic tests cover both clefs, three scales, flat/sharp/zero signatures and
negatives with individual signs, wrong ordering and large gaps. These checks
do not prove complete transcription or musical bow linking.

```sh
LIBREOMR_OBOE_TEST_IMAGE=local/scores/Atlantic_Avenue_02_Oboe.tif \
LIBREOMR_SAX_TEST_IMAGE=local/scores/Atlantic_Avenue_10_AltoSax2_Es-1.tif \
cargo test external_atlantic_ -- --ignored
```

Skew in the alto-sax scan previously left only four of seven staves detected;
`a_Little_Concert_Suite_-_BaritoneB-1.tif` lost all eleven. Alignment restores
seven/eleven staves with note proposals. Tests emphasize upper rows and initial
attributes: treble, 12/8 and one sharp for saxophone; treble, 4/4 and one flat
for the suite. Synthetic tests cover both rotation directions at three sizes,
note coordinates, unchanged straight/incomplete pages and pixel-exact loading
of project images. This does not prove full note/voice/bow reconstruction.

For `A_Holy_Night_-_BassCf-1.tif`, tests check all ten 2/4–4/4 changes, including
both courtesy changes at row ends, all twelve bass clefs and no barline before
them. True double bars before changes remain and meter digits must not become
notes. Widely spaced heads sharing a stem prefer the stem direction at the
attachment side, so the upper head of a beamed octave is not followed toward
the lower head and misclassified as a quarter. Tests cover both heads of all
eight sixteenth pairs plus eighths, dotted values, quarters and halves.
Synthetic tests cover both directions and up to four beams at three scales.
False hollow heads between short beams and staff lines are rejected before beam
counting. Staff 5's first measure is checked as 16/16 rather than 17/16, including
chord grouping and MusicXML durations.

```sh
LIBREOMR_HOLY_NIGHT_TEST_IMAGE=local/scores/A_Holy_Night_-_BassCf-1.tif cargo test external_holy_night -- --ignored
```

To run the complete suite including local originals:

```sh
LIBREOMR_TEST_IMAGE=local/scores/Cassiopeia_-_BaritoneBf-1.tif \
LIBREOMR_BASS_TEST_IMAGE=local/scores/Cassiopeia_-_BassCf-1.tif \
LIBREOMR_OBOE_TEST_IMAGE=local/scores/Atlantic_Avenue_02_Oboe.tif \
LIBREOMR_SAX_TEST_IMAGE=local/scores/Atlantic_Avenue_10_AltoSax2_Es-1.tif \
LIBREOMR_SUITE_TEST_IMAGE=local/scores/a_Little_Concert_Suite_-_BaritoneB-1.tif \
LIBREOMR_HOLY_NIGHT_TEST_IMAGE=local/scores/A_Holy_Night_-_BassCf-1.tif \
LIBREOMR_TEST_PDF=local/scores/80er_KULTtour_2_-_Bass2Cf.pdf \
LIBREOMR_KULTTOUR_TEST_PDF=local/scores/80er_KULTtour_2_-_Bass2Cf.pdf \
cargo test --locked -- --include-ignored
```

`external_dynamics_and_marcato` checks nine dynamics in the Cassiopeia baritone
scan and the marcato in measure 37, including positions, note exclusion and JSON.
Holy Night additionally checks mf, f, mf and ff in the relevant staves; ff next
to an accent on the next row remains a separate letter group.

```sh
LIBREOMR_TEST_IMAGE=local/scores/Cassiopeia_-_BaritoneBf-1.tif cargo test external_dynamics_and_marcato -- --ignored
```

The six original scans complement synthetic tests and do not prove error-free
recognition of arbitrary scores.

## Other regressions and native rendering

Short-note tests cover 1/32 and 1/64 with isolated flags and slanted beams in
both stem directions. Rest tests cover 1/16, 1/32 and 1/64 at three image scales,
three print sizes and two vertical positions. MusicXML uses 192 divisions per
quarter to represent double-dotted sixty-fourth triplets exactly. Manually
assigned tuplets extend resolution exactly, including septuplets. Model, rhythm,
export and UI tests cover ratios, grace notes without measure duration, independent
head sizes, old projects and undo/redo.

Meter-change tests cover common time, alla breve, 3/4, 6/8, 5/4, 7/8 and 12/8
at three scales, incomplete/displaced signatures, preview selection, project
round trips, undo/redo and MusicXML measure lengths across row breaks. UI tests
also cover long image names at narrow and wide sizes.

Preview tests simulate egui input at 760 × 520 and 1280 × 820, covering glyphs,
note/rest selection, active staff, live edits, undo/redo, visibility and empty
results. Pixel checks verify stem attachments for all eight flags from 1/8 to
1/64, raster size and stem alignment at 1×, 1.5× and 2× density. Additional tests
check stem direction around the middle line in bass/treble clefs and consistent
head/flag/accidental proportions. Bravura outlines are rendered with tiny-skia
as transparent native-resolution masks, avoiding ab_glyph_rasterizer artifacts
at certain fractional sizes. A regression checks actual pixel transparency and
opacity, including the size that triggered the artifact in row one of the
KULTtour PDF. An optional original test switches to row two and back:

```sh
LIBREOMR_TEST_PDF=/path/to/80er_KULTtour_2_-_Bass2Cf.pdf cargo test --locked external_pdf_first_staff_accidentals -- --ignored
```

Native screenshot checks require a graphical session:

```sh
LIBREOMR_SCREENSHOT_TO=target/native-smoke.png cargo run --locked --features ui-screenshot
LIBREOMR_COMPACT_SCREENSHOT=1 LIBREOMR_SCREENSHOT_TO=target/native-compact.png cargo run --locked --features ui-screenshot
LIBREOMR_SCREENSHOT_THEME=dark LIBREOMR_SCREENSHOT_TO=target/native-dark.png cargo run --locked --features ui-screenshot
LIBREOMR_SCREENSHOT_IMAGE=/path/to/scan.tif LIBREOMR_SCREENSHOT_TO=target/original-scan.png cargo run --locked --features ui-screenshot
LIBREOMR_PREVIEW_ACCIDENTALS_SCREENSHOT=1 LIBREOMR_SCREENSHOT_THEME=dark LIBREOMR_SCREENSHOT_TO=target/preview-accidentals.png cargo run --locked --features ui-screenshot
LIBREOMR_PROBLEMS_SCREENSHOT=1 LIBREOMR_SCREENSHOT_TO=target/export-problems.png cargo run --locked --features ui-screenshot
LIBREOMR_SETTINGS_SCREENSHOT=1 LIBREOMR_SCREENSHOT_LANGUAGE=en LIBREOMR_SCREENSHOT_TO=target/settings.png cargo run --locked --features ui-screenshot
```

The screenshot mode uses egui's native framebuffer capture on both wgpu and
OpenGL, then exits after saving the image. `ui-screenshot`
recognizes the demo synchronously before the first image so correction tools and
preview are visible. Captures default to English and can explicitly request
German with `LIBREOMR_SCREENSHOT_LANGUAGE=de`. Native rendering checks complement
interactive file-dialog, mouse and correction testing.

GitHub Actions runs tests, Clippy and builds across platforms. Platform support
depends on the underlying Rust libraries; a local build on one platform alone
does not establish behavior on the others. Interactive GUI acceptance is still
required.

Segno/coda and volta tests cover three scales, open/closed endings, note
exclusion, project round trips, export and selection at both preview sizes.
Original tests check both segnos and both codas in Holy Night. Negatives reject
boxed rehearsal/measure numbers in Cassiopeia and A Little Concert Suite.
Voltas currently have synthetic coverage.

```sh
LIBREOMR_HOLY_NIGHT_TEST_IMAGE=local/scores/A_Holy_Night_-_BassCf-1.tif \
cargo test --locked external_repeat_marks -- --ignored
```

For negatives, also set `LIBREOMR_TEST_IMAGE` and `LIBREOMR_SUITE_TEST_IMAGE`.

PDF/TIFF tests (`cargo test --locked document_import`) use a generated two-page
PDF with different page sizes, crop and rotation; multipage TIFFs with varied
bit depth, transparency and compression; BigTIFF; both byte orders; and multipage
Fax3. Further tests cover discovery during analysis, tab/queue order, per-page
retries, project round trips and combined export after reordering tabs.

On page 2 of `80er_KULTtour_2_-_Bass2Cf.pdf`, the estimated left edge fell inside
the bass clef. After normal clef searches fail, the nearby edge is retried using
at least four of five staff lines with vertical tolerance. Search extends at
most three spacings and preserves note-recognition geometry. Synthetic tests
cover strongly clipped bass/treble clefs at three scales. The optional PDF test
checks all 43 row starts across four pages, including changes between three
flats and one flat:

```sh
LIBREOMR_KULTTOUR_TEST_PDF=local/scores/80er_KULTtour_2_-_Bass2Cf.pdf \
cargo test --release --locked --no-default-features external_kulttour_headers_on_all_pages -- --ignored
```

Scans without positioned PDF text use `rusto-rs` with its default features
disabled and RTen for inference in Rust. RTen requires Rust 1.94 or newer.
The multilingual PP-OCRv6 Tiny
models and dictionary are embedded from `assets/ocr/`; that directory records
provenance, checksums and the upstream license. Model files are materialized
in a temporary directory during initialization, then removed. No page data
is sent to external services. Native text layers retain their original bounds.
OCR first reads the page, then refines the spaces between staves at higher
resolution. It runs in the import worker and reports progress/errors through
the existing job interface. Neither platform OCR APIs nor external executables
are required. The same implementation is compiled on all supported targets.

Staff detection supplements the full-width projection with agreement between
several local strips, then estimates horizontal boundaries from support across
four staff lines. This recovers gently curved scan rows and excludes isolated
page borders. Global deskew acceptance still uses the full-width projection.
Lyrics can be attached above or below the same note; placement is part of their
identity and survives project serialization, editing, preview and MusicXML.
