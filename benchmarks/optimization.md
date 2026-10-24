# Recognition optimization, October 19, 2026

These changes affect recognition in [recognition.rs](../src/recognition.rs) and
[symbols.rs](../src/symbols.rs). Benchmark adapters, reference files, samples,
exclusions and scoring rules remained unchanged.

## Causes and changes

- **Key signatures:** Symbols following a confidently recognized first sharp/flat
  may have weaker matches when tightly adjacent and at the prescribed position.
  Staff-line removal damages the same shape differently by position. Missing or
  unreadable first symbols are still not accepted as a valid signature.
- **No accidentals:** Only an actually empty gap between recognized clef and
  meter confirms zero sharps/flats. Foreign/unreadable ink and missing attributes
  remain unknown.
- **Beams:** Long vertical strokes no longer count as beams. A connection only
  through a staff line is insufficient; that previously blurred the distinction
  between nearby barlines/arpeggios and real beams.
- **Flags:** Both side placements are checked for downward stems. A match must
  lie at its own stem end; multiple flags retain stricter minimum quality. A
  clearly continuous beam connection excludes additional flags.
- **Extra heads:** Confirmed flags help reject apparent heads at the opposite
  end of the same stem. Flag-search results are reused per stem.
- **Tall chords:** Stem search can exceed five spacings so the highest head
  reaches the shared beam. Whitespace still terminates the search.

## Procedure

Development used `dev`. After changes were complete, the same code ran on the
previously held-out `test` set. The preserved original benchmark executable also
ran on `test` to establish the previous state. Held-out images were not used
for further adjustments.

Original development reports remain in `target/omr-benchmarks/primus-baseline/`
and `doremi-baseline/`. Final reports are in `primus-optimized-dev/`,
`doremi-optimized-dev/`, `primus-optimized-test/` and `doremi-optimized-test/`
under the same directory. Comparisons use `--baseline` without increased
regression tolerance; individual declines remain visible in `regressions.json`.

Additional regressions check adjacent bars/ornaments, long chord stems, both flag
positions, damaged following key symbols and empty versus unknown signatures.
Local Cassiopeia tests for note values, accidentals and second-row triplets
run separately.

## Measurements

Complete aggregates and source checksums are in
[optimization-summary.json](optimization-summary.json). F1 and SER apply to the
supported subset rather than all notation. DoReMi duration F1 evaluates sounding
duration including dots and triplets.

| Measurement | Before | After |
| --- | ---: | ---: |
| PrIMuS development: event SER ↓ | 21.99% | 6.96% |
| PrIMuS held-out: event SER ↓ | 19.67% | 7.18% |
| PrIMuS development: key-signature F1 ↑ | 47.69% | 95.08% |
| PrIMuS held-out: key-signature F1 ↑ | 55.56% | 97.50% |
| PrIMuS development: note-value F1 ↑ | 83.72% | 94.97% |
| PrIMuS held-out: note-value F1 ↑ | 81.19% | 93.70% |
| DoReMi development: duration F1 ↑ | 57.20% | 67.51% |
| DoReMi held-out: duration F1 ↑ | 60.95% | 71.77% |

PrIMuS retained 159 evaluable development samples and 41 held-out samples;
232 and 68 respectively were outside adapter scope. DoReMi had 30 development
and 10 held-out pages, all evaluable. All four final runs had no processing errors.

Both held-out runs and PrIMuS development pass strict comparison without metric
declines. In DoReMi development, correctly recognized durations increase from
902 to 1,065. One additional false half-note match appears (98 → 99), reducing
class F1 slightly from 88.47% to 88.45%. Pitch F1 falls from 73.62% to 73.60%,
and alteration F1 from 83.45% to 83.36%. These remain explicitly reported
regressions; neither tolerance nor scoring scope was relaxed.

Results show improvement on these fixed excerpts/pages, not general accuracy
on other fonts, old scans, photos or excluded notation.

## Final checks recorded for this optimization

- `cargo test --locked`: 95 library and 49 desktop tests passed; 27 local
  original-scan tests ignored in normal runs.
- Three original tests also ran with the Cassiopeia baritone scan: note values,
  accidentals and triplets; all passed.
- `cargo clippy --all-targets --locked -- -D warnings`, `cargo fmt --all --check`
  and the desktop build passed.

The full log is `target/omr-benchmarks/tests-final-verified.log`; separate
original tests are in `original-note-tests-optimized.log` and
`original-triplet-test-optimized.log` in the same directory.
