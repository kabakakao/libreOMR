# Polyphonic scores and lyrics

Experimental implementation on `feature/polyphonic-scores-and-lyrics`,
2026-10-25. This extends the recognizer and editor; it is no longer only a plan.

## What is implemented

- Staff boundaries use agreement between at least four staff lines, so part
  labels to the left no longer move the clef search into the text.
- A continuous left bracket/bar groups simultaneous staves into systems.
  Staff positions within successive systems become persistent part identities.
  Positioned PDF labels supply names such as Soprano, Alto, Tenor and Bass.
  Part assignments and names can be corrected in the Details panel.
- The header recognizer supports 4/2 and octave-down treble clefs. The octave
  digit needs both a shape match and two enclosed counters. Pitches use the
  sounding octave; preview and MusicXML retain the octave-displaced clef.
- Matching meter proposals within a bracketed system can complete an unreadable
  header. Translated meter bounds are accepted only with matching printed ink.
  Meter/key inheritance follows each part across systems/pages.
- Hollow-head contours are inspected at native image resolution to avoid
  opening thin rings during downsampling. A filled-center check guards against
  interpreting small stem/staff enclosures as half notes.
- The existing hayro interpreter supplies Unicode characters, baselines and
  bounds with the same crop/rotation/scale transform as PDF rasterization.
  This adds no external PDF-text executable or downloaded language model.
- Text rows below each staff become separate lyric verses. Syllables are linked
  monotonically to notes, with single/begin/middle/end and extender state.
  Titles, labels and remote footer text are excluded geometrically.
- The preview places every system of the current page on one shared canvas,
  with aligned notes and barlines across its voices and all lyric verses.
  Fit mode scales only to the available width, preserving readable notation
  on short displays. Vertical scrolling follows the selected object across
  voices and systems; manual zoom also follows horizontally.
  Manual scrolling stays in place until the selection changes.
  Click a syllable to select its note. The Details panel uses one compact text
  row per verse, with additional options in the row's menu. Enter/Shift+Enter
  and the arrow buttons move between syllables of the active verse, continuing
  in the same part across systems on the current page. Changes support undo/redo.
- Printed numeric verse labels (e.g. `1.` or `3)`) are preserved separately
  from syllables and determine verse identity, including nonconsecutive numbers.
  They appear only at the printed verse start in the preview and MusicXML text.
  The syllable menu can correct or remove a printed label. Older projects that
  lack this optional field remain readable.
- MusicXML exports separate named parts, continuing measure numbers in each
  part, octave clefs and numbered lyric elements. Unreviewed/empty lyrics appear
  in export diagnostics with navigation to the corresponding note.
- Projects preserve assignments, lyrics and source text through optional model
  fields. Existing version 1/2 projects remain readable. Deleting a note removes
  its lyrics and remaps later references; undo restores the complete score.

## Local SATB example and baseline

Input: `ach-lieber-herre-jesu-christ-satb.pdf`, Johannes Brahms,
*Ach lieber Herre Jesu Christ*, CPDL edition dated 1999-09-01.
SHA-256: `0961bb26ba600ef14a5c3a023433cfec409a23aedd780ce6fb2c2a9ce1164764`.
The input and generated inspection artifacts remain ignored, outside the public
fixtures. The two US Letter pages render at 300 dpi to 2550 x 3300 pixels.
There are three four-staff systems, four lyric verses per part, two sharps,
printed 4/2 and an opening half-note pickup. The Tenor clef sounds an octave down.
The editorial footer's cut-time remark does not override the printed 4/2.

Before this change (`93241c6`), the first page had four unknown clefs and missing
keys/meters. The baseline proposed 53 and 117 notes on the two pages; those
counts included false proposals and omitted real notes. Export produced one
continuous part with 52 measures and no lyrics. All twelve staff instances were
already detected. Counts alone are not recognition accuracy.

The final local feature run produces:

| Page | Staves | Note proposals | Lyric syllables |
| --- | ---: | ---: | ---: |
| 1 | 4 | 52 | 208 |
| 2 | 8 | 140 | 496 |

All twelve headers have the two-sharp key and 4/2; all three Tenor staves use
the octave-down clef. Names continue across the page break. MusicXML contains
four named parts with 13 measures and 176 lyric elements each. Each part has
four distinct verses. Rhythmic export warnings are confined to the complementary
pickup and short final measure; proposals still need musical review, including
articulations and other symbols. There is no full manually corrected reference
transcription, so these totals are not an accuracy percentage.

The updated local ARM64 preview binary is `target/ci/libreomr-lyrics-preview.exe`.
The checked project, MusicXML and initial native screenshot are under
`local/satb-improved/`; the shared-canvas screenshot is under
`local/preview-refresh/`.
The updated project and MusicXML with printed verse labels are under
`local/lyrics-improved/`; `local/preview-refresh/lyrics-compact.png` shows the
compact editor. Reattaching the original positioned text preserves all 704
syllables and recovers 16 printed labels on page 1 (four per part), with no
spurious labels on page 2.
The core regression suite passed 142 tests (two external-score tests ignored),
with seven additional integration tests. All 21 preview tests passed, including
a GUI test checking that lyrics render, retain the octave clef and select their
note when clicked, and shared-canvas tests covering voice alignment, automatic
selection following, resizing and manual scrolling. Clippy checks
all targets, including screenshot support, with warnings denied.
The shared-canvas update also passed the three application preview tests
(24 tests in total), including span-anchor dragging with undo/redo at both scales.

## Scanned score: Come Back, Liza

The private `012_Come-back-Liza.pdf` renders to 2484 x 3509 pixels and has no
PDF text layer. Its four paired systems contain eight regular five-line
staves, plus a separate one-line Bongo example. Local projection agreement
recovers the opening pair missed by the previous full-width detector. Staff
boundaries now follow the notation rather than isolated scan borders.

| Local run | Regular staves | Note proposals | Attached lyric syllables |
| --- | ---: | ---: | ---: |
| Before scan update | 6 | 154 | 0 |
| With Rust OCR and layout update | 8 | 203 | 73 |

Of these syllables, 63 are below staves and ten are above them. The opening
printed `1.`, hyphenated words, the single bass `oh`, both `Hear me crying`
phrases and the two above-staff `du/ah` rows are represented. The five free
stanzas numbered 2–6 remain positioned source text, without invented note
associations. The saved project and MusicXML are under `local/liza-improved/`;
the native screenshot is `local/liza-analysis/preview.png`. These private
inspection artifacts are ignored by Git.

Long extenders are removed from OCR crops only. Higher-resolution crop results
replace overlapping coarse words while preserving words missed on the second
pass. Whole-row ordered alignment prevents a short shifted syllable from taking
the final note before its successor. Performance directions such as `dolce`
and `dim. e rit.` are excluded from lyric proposals. The original SATB check
retains its 704 syllables and 16 printed labels.

These counts are proposals, not an accuracy score. Four treble clefs in this
scan still need correction, as do some note values and OCR spellings. The
one-line percussion example is not recognized. Independent voices sharing a
staff still need rhythmic reconstruction. The local Windows ARM64 executable
is `target/ci/libreomr-scan-preview.exe`; other supported platforms use the same
Rust implementation but were not run in this local check. The scan update is
covered by targeted staff tests, offline OCR and lyric integration tests,
preview/editor regressions, packaging tests and Clippy.

## Try it

Build/run the desktop app normally and open the PDF again. Previously saved
recognition results do not acquire the new proposals until re-recognized.
Select a note to open Details; its lyrics are below the note controls. The
preview fits the score to the available width. Its height can overflow. Drag
the panel divider to give it more room, or use its plus/minus buttons for
details. Selecting an object in the recognition image automatically brings its
preview into view. The fit button restores horizontal fitting.
For lyrics, edit the text directly in its verse row. Press Enter for the next
syllable or Shift+Enter for the previous one; the active verse retains focus.
The row's `...` menu contains word position, extension, review, printed verse
label, placement above/below the staff and deletion. Previously saved results need a fresh import to acquire
the newly recognized printed labels; existing lyric corrections are preserved
when an older project is opened.

For a reproducible headless import with the same text/recognition pipeline:

```sh
cargo run --profile ci --locked --example inspect_score -- INPUT.pdf local/inspection
```

The output directory contains `score.libreomr.json`, `score.musicxml` and
`warnings.txt`. Open the generated project in the app to inspect the result
without repeating recognition. Output may contain private score content; keep
it under `local/` rather than adding it to public fixtures.

Tests include synthetic multi-part/lyric examples and an in-memory PDF with
German text and page rotation:

```sh
cargo test --profile ci --locked --no-default-features --lib --test polyphonic_scores
```

## Remaining limitations

This is a foundation for printed part scores, not complete arbitrary polyphonic
OMR. Several independent voices sharing one staff still need explicit onset,
voice and chord reconstruction. Braces, divided staves, changing system size,
complex orchestral groups and absent brackets require richer layout analysis;
current proposals can be wrong and must be reviewed.

PDF text layers retain their original text and coordinates. Scanned PDFs and
raster images use bundled multilingual RustO/RTen OCR, including German umlauts.
The implementation uses the same Rust backend on all supported platforms,
with no OS OCR service, external program or runtime model download. It reads
the full page and then refines the spaces between staves. Model provenance and
license notices are in [assets/ocr](../assets/ocr/README.md).
Type 3 musical glyphs without Unicode remain handled by image recognition,
not by guessing their character codes.

OCR words are converted to positioned tokens with explicit word boundaries,
hyphens and printed verse prefixes. Simultaneous noteheads at one onset share
one lyric anchor; below-staff text uses the lower note, above-staff text the
upper note. Short rows, including a single syllable, are accepted. Separated
above-staff refrain text can coexist with below-staff lyrics. Standalone
left-aligned numbered stanza blocks below the final system are retained as
source text rather than attached to unrelated accompaniment notes. They do
not yet have a separate stanza-block editor.

Single-line percussion staves and percussion crossheads are not supported by
the current five-line staff and pitch model. OCR and note/syllable associations
remain proposals requiring review. Cross-platform design has been exercised
locally on Windows ARM64; Linux, macOS and Windows x86_64 still require their
normal platform CI and interactive acceptance checks.

Lyric assignment filters common performance directions (including `marcato`,
`allargando`, `ritardando`, `poco rit.` and `a tempo`), also when OCR joins
punctuation into a single token. Modifiers are filtered with a direction;
standalone sung words such as `a` and `e` remain eligible. Copyright, publisher,
rights and URL markers below the final staff identify footer blocks across the
full page width, including markers outside the staff margins. Following footer
lines remain source text as well. A distant first text row cannot start a lyric
verse; later verses still use their spacing relative to the preceding verse.
These heuristics apply equally to positioned PDF text and OCR, and retain the
original source text. Unusual wording and ambiguous sung performance terms can
still require correction. Reimport the source to update saved proposals.

Missing notes can shift syllable associations. Short lyric rows, ambiguous
verse numbers, overlapping performance text, cross-system melisma/extender
continuation and historical spelling need further examples and review.
General pickup inference and exact engraving/layout remain incomplete. These
limitations are not resolved by a clear-looking input alone; recognition must
preserve the relationships between the printed objects.
