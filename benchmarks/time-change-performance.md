# Time-change recognition performance

The previous search compared every prospective change with 194 meters: common
time, cut time, and numerators 1–32 with denominators 1, 2, 4, 8, 16 and 32.
With the existing scale variants, this meant up to 1,351 resized templates at
each staff start and barline. Each template was correlated with every image
position before checking whether the position had the right vertical alignment
and clean borders. Positions that could never pass those checks still incurred
the full pixel comparison. Ordinary notes therefore made this stage expensive
even when the page had no time changes.

The optimized search in `src/symbols/time_changes.rs`:

- Prepares the scaled templates in a bounded, shared cache once per process.
- Checks the existing geometry rules before computing correlations.
- Shares geometric results and source squared sums between templates of the
  same dimensions within each search region.
- Skips zero template pixels while preserving row order and the existing
  floating-point scores and tie breaking.
- Uses integral squared intensities to reject empty windows in constant time.
- Bounds the maximum possible correlation using Cauchy–Schwarz on a 4 × 4
  partition. Only windows that can still reach the acceptance threshold incur
  the full comparison and original floating-point normalization sum. The bound
  uses 0.85, conservatively below the actual 0.86 threshold; rounding cannot
  bridge the margin for these small windows. Windows larger than 4,096 pixels
  bypass this shortcut.
- Stops border checks as soon as they cannot pass.

Supported meters, scales, search regions, minimum quality (0.86), ambiguity
margin (0.025), staff alignment and double-bar handling are unchanged. Other
symbol matching continues to use the existing general matcher.

## Measurements

Local Windows ARM64 measurements on October 27, 2026, using the optimized `ci`
profile. Values are mean milliseconds per staff from three timed repetitions
after warm-up, with the reference and optimized searches in the same executable.

| Case | Exhaustive search | Optimized search | Speedup |
| --- | ---: | ---: | ---: |
| Ordinary notes without a change | 2,761.281 ms | 145.476 ms | 18.98× |
| Printed 12/8 change | 2,987.745 ms | 149.984 ms | 19.92× |
| Printed common-time change | 2,741.992 ms | 133.872 ms | 20.48× |

One-time initialization of all 1,351 scaled templates took 153.288 ms, measured
separately from the warm timings. The cache is shared by subsequent searches and
pages in the same process. Actual page gains depend on the number of search
anchors and time spent in the other recognition stages.

## Reproduce

```sh
cargo test --profile ci --locked --lib time_change
cargo test --profile ci --locked --lib meter
cargo test --profile ci --locked --lib time_change_benchmark -- --ignored --nocapture --test-threads=1
```

The timing test runs the preserved exhaustive implementation and optimized
implementation in the same executable. Each uses the same synthetic 1,120 × 220
pixel staff with four barlines and ordinary notes. The other two cases replace
one note with a printed 12/8 or common-time change. Both paths are warmed before
three timed repetitions. Template-cache initialization is printed separately.
The benchmark also checks identical returned score objects.

The regression matrix compares the full serialized time-change objects across
three image scales (0.75, 1.0, 1.4), common/cut time, single- and double-digit
meters, ordinary notes and an incomplete symbol. A separate test compares the
correlation values and accepted bounds directly with the original dense matcher,
including antialiased gray pixels and rejected maxima. Existing regressions
cover curved staff ends, tight double bars, incomplete/misaligned digits, and
changes at later staff starts without a repeated clef.

Validation: 21 distinct functional tests passed across the `time_change` and
`meter` filters, as did the manual timing test. Clippy with `-D warnings` and
the formatting check passed. The test-only exhaustive reference is excluded
from application builds.

These measurements isolate this recognition stage; they are not end-to-end
page timings or an accuracy benchmark on arbitrary scans.
