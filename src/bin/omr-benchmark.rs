use std::{path::PathBuf, process::ExitCode};

use libreomr::benchmark::{self, Options, Report};

const HELP: &str = "Offline OMR benchmark\n\nUsage: cargo run --release --bin omr-benchmark -- --manifest FILE --output DIR [OPTIONS]\n\n  --manifest FILE         Frozen sample from scripts/prepare_benchmark.py\n  --output DIR            New/empty report directory (HTML, JSON, marked PNGs)\n  --split dev|test|all    Default: dev; test is the held-out control set\n  --limit N               Evaluate only the first N selected samples\n  --overlays N            Maximum marked error images (default: 20)\n  --straighten            Apply desktop deskewing (PrIMuS only; off by default)\n  --baseline FILE         Compare to a previous report.json on identical inputs\n  --max-regression PP     Allowed F1/SER regression, percentage points (default: 0)\n  --help                  Show this help\n\nExit codes: 0 completed, 1 regression, 2 input/processing error or no evaluated samples.\nUnsupported samples are reported separately, never counted as successes.\n";

fn main() -> ExitCode {
    match execute() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Benchmark error: {error}");
            ExitCode::from(2)
        }
    }
}

fn execute() -> Result<ExitCode, String> {
    let mut args = std::env::args().skip(1);
    let (mut manifest, mut output, mut baseline) = (None, None, None);
    let mut options = Options {
        manifest: PathBuf::new(),
        output: PathBuf::new(),
        split: "dev".into(),
        limit: None,
        overlays: 20,
        straighten: false,
    };
    let mut tolerance = 0.0;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            print!("{HELP}");
            return Ok(ExitCode::SUCCESS);
        }
        if arg == "--straighten" {
            options.straighten = true;
            continue;
        }
        if ![
            "--manifest",
            "--output",
            "--split",
            "--limit",
            "--overlays",
            "--baseline",
            "--max-regression",
        ]
        .contains(&arg.as_str())
        {
            return Err(format!("Unknown option {arg}; use --help"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {arg}"))?;
        match arg.as_str() {
            "--manifest" => manifest = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--split" => options.split = value,
            "--limit" => options.limit = Some(value.parse::<usize>().map_err(|e| e.to_string())?),
            "--overlays" => {
                options.overlays = value
                    .parse()
                    .map_err(|e: std::num::ParseIntError| e.to_string())?
            }
            "--baseline" => baseline = Some(PathBuf::from(value)),
            "--max-regression" => tolerance = value.parse::<f64>().map_err(|e| e.to_string())?,
            _ => unreachable!(),
        }
    }
    options.manifest = manifest.ok_or("--manifest is required; use --help")?;
    options.output = output.ok_or("--output is required; use --help")?;
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err("--max-regression must be finite and non-negative".into());
    }
    let baseline: Option<Report> = baseline
        .map(|path| {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())
        })
        .transpose()?;
    let report = benchmark::run(&options)?;
    println!(
        "{}: {} evaluated / {} selected; {} unsupported, {} errors",
        report.dataset,
        report.coverage.evaluated,
        report.coverage.selected,
        report.coverage.unsupported,
        report.coverage.failed
    );
    if let Some(ser) = report.metrics.symbol_error_rate() {
        println!("Event SER: {:.2}%", ser * 100.0);
    }
    println!("Report: {}", options.output.join("index.html").display());
    if report.coverage.failed > 0 || report.coverage.evaluated == 0 {
        return Ok(ExitCode::from(2));
    }
    if let Some(baseline) = baseline {
        let failures = benchmark::regressions(&report, &baseline, tolerance)?;
        for failure in &failures {
            eprintln!("Regression: {failure}");
        }
        benchmark::write_json(&options.output.join("regressions.json"), &failures)?;
        if !failures.is_empty() {
            return Ok(ExitCode::from(1));
        }
        println!("No regressions beyond {tolerance:.2} percentage points.");
    }
    Ok(ExitCode::SUCCESS)
}
