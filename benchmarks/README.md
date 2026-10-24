# Public recognition benchmarks

The benchmark processes a frozen selection of public score images without the
desktop UI. It compares recognized objects with independent reference annotations
and writes a locally viewable HTML report, JSON, score models and marked error
images. It does not modify recognition itself.

## Prepared local datasets

The initial samples were prepared with seed 42:

| Dataset | Sample | Development (`dev`) | Held-out (`test`) |
| --- | ---: | ---: | ---: |
| PrIMuS | 500 of 87,678 excerpts | 391 | 109 |
| DoReMi v1.0 | 40 of 5,218 pages | 30 | 10 |

Manifests are `target/omr-benchmarks/primus.json` and `doremi.json`. Initial
reports are in `target/omr-benchmarks/primus-baseline/` and `doremi-baseline/`.
The default invocation does not evaluate the held-out set.

The first PrIMuS development run evaluated **159/391** samples; **232** were
outside adapter scope and **0** had processing errors. Event SER was **21.99%**.
Selected property F1 scores were pitch **96.28%**, note value **83.72%** and key
signature (`key.fifths`) **47.69%**. These are separate measures on the supported
subset, not an overall recognition rate. C clefs and `tie` tokens were the most
common exclusions. The DoReMi run evaluated **30/30** pages without processing
errors; micro class F1 across evaluated symbols was **88.47%**. The compact
[baseline summary](baseline-summary.json) preserves this measurement outside
`target/`; it is a summary, not a `--baseline` input report.

Subsequent recognition changes and their comparison on the held-out set are
documented in [Recognition optimization](optimization.md). Adapters, samples
and scoring rules were retained.

## Download and freeze a sample

Python **3.9+** without additional packages and the project's Rust toolchain are
required. Downloads and extracted data default to `target/omr-data/`, outside
the tracked source. Both archives total about 366 MiB downloaded and several
GiB extracted.

```sh
python3 scripts/prepare_benchmark.py fetch primus
python3 scripts/prepare_benchmark.py fetch doremi

python3 scripts/prepare_benchmark.py prepare primus \
  --input target/omr-data/primus \
  --output target/omr-benchmarks/primus.json --limit 500 --seed 42

python3 scripts/prepare_benchmark.py prepare doremi \
  --input target/omr-data/doremi \
  --output target/omr-benchmarks/doremi.json --limit 40 --seed 42
```

Use `prepare` directly with already extracted data. For Camera-PrIMuS, replace
the dataset name with `camera-primus` for both commands and use separate input
and output directories. The Camera-PrIMuS download has not been tested here.

Selection is deterministic and independent of filesystem order. DoReMi pages
are distributed evenly across source projects so large synthetic exercise
projects do not dominate a small sample. The dev/test split is determined using
SHA-256 of the seed and source group, with 20% of groups assigned to `test` by
default. DoReMi groups all pages of a project; PrIMuS groups incipit IDs including
recognized font variants. This does not guarantee separation of different
incipits from the same musical work.

The manifest records selected files and FNV-1a/64 checksums, rechecked on every
run. Missing or changed files produce processing errors. FNV provides
repeatability rather than a cryptographic signature. Use a new seed only for a
deliberately new sample; retain the manifest when changing recognition.

DoReMi images and page annotations have different filenames; `prepare` matches
them by page suffix. The official page export removes musical `DataItem`
metadata. The script supplements available metadata from `OMR_XML` by object ID
in separate reference copies beside the manifest. Without original XML, spatial
class evaluation still works but additional properties remain unevaluated.
Original files are unchanged. Keep the manifest and its `*-references` directory
together.

## Run

```sh
cargo build --release --locked --no-default-features --bin omr-benchmark

target/release/omr-benchmark \
  --manifest target/omr-benchmarks/primus.json \
  --output target/omr-benchmarks/primus-run-01

target/release/omr-benchmark \
  --manifest target/omr-benchmarks/doremi.json \
  --output target/omr-benchmarks/doremi-run-01 --overlays 10
```

On Windows the executable is `target/release/omr-benchmark.exe`. `cargo run --locked`
still starts the desktop app. With `--no-default-features`, the benchmark needs
no GUI dependencies.

`--split dev` is the default. Use `--limit 10` for a short trial, `--split test`
to explicitly evaluate the held-out set, and `--split all` to inspect the full
selection. Keep development and held-out results separate and do not use held-out
images to tune heuristics.

Each run needs a new or empty output directory. Outputs include:

- `index.html`: class/property metrics and expandable per-sample comparisons.
- `report.json`: assignments, metrics, checksums, unsupported samples, error
  reasons and excluded DoReMi classes.
- `sample-*.score.json`: unmodified recognition results before correction.
- `sample-*.png`: marked discrepancies, at most 20 images by default.

Green marks correct objects, red missing objects, magenta extras and orange
misclassified objects or incorrect properties. PrIMuS has no reference pixel
coordinates; missing events appear in the comparison list without invented
positions in overlays. DoReMi supplies spatially located reference objects.

Image decoding, including transparency, size limits and Fax3 TIFF, is shared
with the app. The benchmark does not deskew by default. `--straighten` enables
the desktop alignment step for PrIMuS. DoReMi rejects that flag because rotating
the image without transforming reference boxes would invalidate the comparison.

## Interpreting metrics

**PrIMuS:** Notes, rests and barlines are compared as a sequence. Levenshtein
alignment counts insertions, deletions and substitutions without letting one
extra note shift every later event. Sequence error rate **SER** is their sum
divided by the reference event count. An event is fully correct only when its
annotated properties also match. Many extra events can make SER exceed 100%.
Clefs, keys and meters are assigned separately and excluded from event SER.
Common time/4-4 and alla breve/2-2 are treated as musically equivalent.

The adapter supports G2/F4 clefs, ordinary notes/rests down to 1/64, up to two
dots, and simple pitch alterations. Samples with C/other clefs, grace notes,
`tie`/`multirest` tokens, unrepresentable values or internal staff attributes are
reported as **unsupported**. They count neither as successful recognition nor
toward the SER denominator. This is an adapter restriction even where libreOMR
can partially recognize those symbols.

**DoReMi:** Ordinary filled/half/whole heads, seven rest values, treble/bass
clefs and mappable dynamics classes are compared spatially. A predicted center
must lie within the reference box expanded by 25% of its shorter side, at least
2 pixels. Matching maximizes unique pairs within the same symbol family;
distance orders candidates. Wrong subclasses remain misclassifications and
duplicate matches count as extras. This is a diagnostic for current recognition,
not a COCO-mAP benchmark.

Supplemented original metadata enables diatonic pitch, alteration and sounding
duration checks. Duration is normalized to 192 units per quarter to absorb
rounding in DoReMi decimals. Voice assignment and onset timing are not evaluated.
Unknown classes are counted in the report. Separate accidental objects,
articulations, bows, wedges and numeric meters do not yet have spatial evaluation.

**Both:** Precision, recall and F1 are reported per class and musical property.
Class matches measure detection/classification independently of separately scored
properties. Missing objects also cause missing properties. Unannotated properties
are excluded. Values without a denominator are `null` or “—”. Errors and
unsupported samples are always shown beside the evaluated count. Runs with no
evaluated sample fail.

Most images are typeset notation. Good results here do not establish robustness
on photos or old scans. The small initial DoReMi selection is for debugging,
not a general quality claim.

## Compare with a reference run

```sh
target/release/omr-benchmark \
  --manifest target/omr-benchmarks/primus.json \
  --output target/omr-benchmarks/primus-run-02 \
  --baseline target/omr-benchmarks/primus-run-01/report.json
```

Comparison requires identical manifests, selected files, splits and alignment
settings. Fewer evaluable images, lower class/property F1 and higher SER are
regressions. `--max-regression 0.5` permits a decline of 0.5 percentage points.
Small classes deliberately remain visible and may be sensitive. Comparisons
are also saved as `regressions.json`.

Exit codes: **0** completed, **1** regression, **2** input/file/processing error
or no evaluable sample. Unsupported samples alone do not cause failure if other
samples were evaluated.

## Automated tests

```sh
cargo test --locked
cargo test --locked --no-default-features --lib benchmark
python3 -m unittest discover -s scripts -p 'test_*.py'
```

CI checks offline logic without public downloads: sequence alignment, spatial
ambiguity and duplicates, parsers, reference supplementation, reproducible
selection, split separation, integrity and regressions. Large corpora run locally.

Sources: [PrIMuS and Camera-PrIMuS](https://grfia.dlsi.ua.es/primus/),
[DoReMi documentation](https://github.com/steinbergmedia/DoReMi) and
[DoReMi v1.0](https://github.com/steinbergmedia/DoReMi/releases/tag/v1.0).
