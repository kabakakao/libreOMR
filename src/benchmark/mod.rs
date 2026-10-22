//! Offline evaluation against frozen, externally annotated dataset samples.
//!
//! Run `python3 scripts/prepare_benchmark.py --help` to obtain a manifest,
//! then `cargo run --release --bin omr-benchmark -- --help`.
mod doremi;
pub mod metrics;
mod primus;
mod report;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Instant,
};

use serde::{Deserialize, Serialize};

use crate::{image_import::decode_image, model::SymbolBounds, recognition};
use metrics::{Metrics, Pair};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Object {
    pub family: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub bounds: Option<SymbolBounds>,
}

impl Object {
    fn new(family: &str, class: &str) -> Self {
        Self {
            family: family.into(),
            class: class.into(),
            attributes: BTreeMap::new(),
            bounds: None,
        }
    }

    fn center(&self) -> Option<(f32, f32)> {
        self.bounds
            .as_ref()
            .map(|b| ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0))
    }
}

#[derive(Default)]
struct Reference {
    objects: Vec<Object>,
    headers: Vec<Object>,
    ignored: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub version: u32,
    pub dataset: String,
    pub source: String,
    pub data_root: PathBuf,
    pub seed: u64,
    pub test_percent: u8,
    pub candidate_count: usize,
    pub samples: Vec<Sample>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sample {
    pub id: String,
    pub group: String,
    pub split: String,
    pub image: PathBuf,
    pub annotation: PathBuf,
    pub image_fingerprint: String,
    pub annotation_fingerprint: String,
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !["primus", "camera-primus", "doremi"].contains(&self.dataset.as_str())
        {
            return Err("Unsupported manifest version/dataset".into());
        }
        if self.samples.is_empty() {
            return Err("Manifest contains no samples".into());
        }
        let mut ids = BTreeSet::new();
        let mut groups = BTreeMap::new();
        for sample in &self.samples {
            if sample.id.is_empty() || sample.group.is_empty() || !ids.insert(&sample.id) {
                return Err(format!("Empty/duplicate sample identity: {}", sample.id));
            }
            if !["dev", "test"].contains(&sample.split.as_str()) {
                return Err(format!("Invalid split for {}", sample.id));
            }
            if groups
                .insert(&sample.group, &sample.split)
                .is_some_and(|split| split != &sample.split)
            {
                return Err(format!(
                    "Source group {} leaks across dev/test",
                    sample.group
                ));
            }
            for hash in [&sample.image_fingerprint, &sample.annotation_fingerprint] {
                if hash.len() != 16 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(format!("Invalid FNV-1a fingerprint for {}", sample.id));
                }
            }
        }
        Ok(())
    }
}

pub fn fingerprint(bytes: &[u8]) -> String {
    let value = bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    format!("{value:016x}")
}

fn checked_read(path: &Path, expected: &str) -> Result<Vec<u8>, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if fingerprint(&bytes) != expected {
        return Err(format!(
            "Content fingerprint changed: {}. Prepare a new manifest for changed data.",
            path.display()
        ));
    }
    Ok(bytes)
}

#[derive(Clone, Debug)]
pub struct Options {
    pub manifest: PathBuf,
    pub output: PathBuf,
    pub split: String,
    pub limit: Option<usize>,
    pub overlays: usize,
    pub straighten: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub selected: usize,
    pub evaluated: usize,
    pub unsupported: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SampleResult {
    pub sample: Sample,
    pub status: String,
    pub reason: Option<String>,
    pub elapsed_ms: u128,
    pub metrics: Metrics,
    pub ignored_classes: BTreeMap<String, usize>,
    pub reference: Vec<Object>,
    pub prediction: Vec<Object>,
    pub pairs: Vec<Pair>,
    pub overlay: Option<String>,
    pub score: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub evaluator: String,
    pub dataset: String,
    pub source: String,
    pub manifest_fingerprint: String,
    pub split: String,
    pub straighten: bool,
    pub coverage: Coverage,
    pub metrics: Metrics,
    pub ignored_classes: BTreeMap<String, usize>,
    pub samples: Vec<SampleResult>,
}

fn align_headers(
    reference: &mut Reference,
    prediction: &mut Vec<Object>,
    headers: Vec<Object>,
    pairs: &mut Vec<Pair>,
    metrics: &mut Metrics,
) {
    let reference_start = reference.objects.len();
    let prediction_start = prediction.len();
    let mut header_pairs = Vec::new();
    let mut used = BTreeSet::new();
    for (i, a) in reference.headers.iter().enumerate() {
        let found = headers
            .iter()
            .enumerate()
            .find(|(j, b)| !used.contains(j) && a.family == b.family)
            .map(|(j, _)| j);
        if let Some(j) = found {
            used.insert(j);
        }
        header_pairs.push(Pair {
            reference: Some(i),
            prediction: found,
            correct: false,
        });
    }
    header_pairs.extend(
        (0..headers.len())
            .filter(|j| !used.contains(j))
            .map(|j| Pair {
                reference: None,
                prediction: Some(j),
                correct: false,
            }),
    );
    metrics.merge(&metrics::measure(
        &reference.headers,
        &headers,
        &mut header_pairs,
    ));
    pairs.extend(header_pairs.into_iter().map(|pair| Pair {
        reference: pair.reference.map(|i| i + reference_start),
        prediction: pair.prediction.map(|j| j + prediction_start),
        correct: pair.correct,
    }));
    reference.objects.append(&mut reference.headers);
    prediction.extend(headers);
}

pub fn run(options: &Options) -> Result<Report, String> {
    if !["dev", "test", "all"].contains(&options.split.as_str()) || options.limit == Some(0) {
        return Err("Use --split dev|test|all and a positive --limit".into());
    }
    let manifest_bytes = fs::read(&options.manifest).map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
    manifest.validate()?;
    if options.straighten && manifest.dataset == "doremi" {
        return Err("DoReMi pixel annotations require original coordinates; --straighten is supported only for PrIMuS".into());
    }
    let root = options
        .manifest
        .parent()
        .unwrap_or(Path::new("."))
        .join(&manifest.data_root);
    let selected: Vec<_> = manifest
        .samples
        .iter()
        .filter(|sample| options.split == "all" || sample.split == options.split)
        .take(options.limit.unwrap_or(usize::MAX))
        .collect();
    if selected.is_empty() {
        return Err("No samples selected for this split".into());
    }
    if options.output.exists()
        && fs::read_dir(&options.output)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err(format!(
            "Output directory is not empty: {}. Use a fresh directory for each run.",
            options.output.display()
        ));
    }
    fs::create_dir_all(&options.output).map_err(|e| e.to_string())?;
    let mut report = Report {
        version: 1,
        evaluator: "libreomr-benchmark-v1".into(),
        dataset: manifest.dataset.clone(),
        source: manifest.source,
        manifest_fingerprint: fingerprint(&manifest_bytes),
        split: options.split.clone(),
        straighten: options.straighten,
        coverage: Coverage {
            selected: selected.len(),
            ..Default::default()
        },
        metrics: Metrics::default(),
        ignored_classes: BTreeMap::new(),
        samples: Vec::new(),
    };
    let mut overlays = 0;
    for (index, sample) in selected.into_iter().enumerate() {
        eprintln!("[{}/{}] {}", index + 1, report.coverage.selected, sample.id);
        let started = Instant::now();
        let mut result = SampleResult {
            sample: sample.clone(),
            status: "error".into(),
            reason: None,
            elapsed_ms: 0,
            metrics: Metrics::default(),
            ignored_classes: BTreeMap::new(),
            reference: Vec::new(),
            prediction: Vec::new(),
            pairs: Vec::new(),
            overlay: None,
            score: None,
        };
        let evaluated = (|| -> Result<(), String> {
            let annotation = checked_read(
                &root.join(&sample.annotation),
                &sample.annotation_fingerprint,
            )?;
            let bytes = checked_read(&root.join(&sample.image), &sample.image_fingerprint)?;
            let annotation = std::str::from_utf8(&annotation).map_err(|e| e.to_string())?;
            let mut reference = if manifest.dataset == "doremi" {
                doremi::parse(annotation)?
            } else {
                match primus::parse(annotation) {
                    Ok(reference) => reference,
                    Err(reason) => {
                        result.status = "unsupported".into();
                        result.reason = Some(reason);
                        return Ok(());
                    }
                }
            };
            let image = decode_image(image::ImageReader::new(Cursor::new(bytes)))?;
            let image = if options.straighten {
                recognition::straighten(image)
            } else {
                image
            };
            let score = recognition::recognize(&image);
            let (mut prediction, metrics, mut pairs) = if manifest.dataset == "doremi" {
                let prediction = doremi::predict(&score);
                let (metrics, pairs) = metrics::spatial(&reference.objects, &prediction);
                (prediction, metrics, pairs)
            } else {
                let (mut prediction, headers) = primus::predict(&score);
                let (mut metrics, mut pairs) = metrics::sequence(&reference.objects, &prediction)?;
                align_headers(
                    &mut reference,
                    &mut prediction,
                    headers,
                    &mut pairs,
                    &mut metrics,
                );
                (prediction, metrics, pairs)
            };
            let filename = format!("sample-{index:05}");
            let score_path = format!("{filename}.score.json");
            write_json(&options.output.join(&score_path), &score)?;
            result.score = Some(score_path);
            if pairs.iter().any(|pair| !pair.correct) && overlays < options.overlays {
                let path = format!("{filename}.png");
                report::overlay(
                    &image,
                    &reference.objects,
                    &prediction,
                    &pairs,
                    &options.output.join(&path),
                )?;
                result.overlay = Some(path);
                overlays += 1;
            }
            result.status = "evaluated".into();
            result.metrics = metrics;
            result.ignored_classes = reference.ignored;
            result.reference = reference.objects;
            result.prediction.append(&mut prediction);
            result.pairs.append(&mut pairs);
            Ok(())
        })();
        if let Err(error) = evaluated {
            result.status = "error".into();
            result.reason = Some(error);
        }
        result.elapsed_ms = started.elapsed().as_millis();
        eprintln!(
            "  {} ({} ms){}",
            result.status,
            result.elapsed_ms,
            result
                .reason
                .as_ref()
                .map_or_else(String::new, |reason| format!(": {reason}"))
        );
        match result.status.as_str() {
            "evaluated" => {
                report.coverage.evaluated += 1;
                report.metrics.merge(&result.metrics);
                for (class, count) in &result.ignored_classes {
                    *report.ignored_classes.entry(class.clone()).or_default() += count;
                }
            }
            "unsupported" => report.coverage.unsupported += 1,
            _ => report.coverage.failed += 1,
        }
        report.samples.push(result);
        if report.samples.len().is_multiple_of(10) {
            write_json(&options.output.join("report.json"), &report)?;
        }
    }
    write_json(&options.output.join("report.json"), &report)?;
    report::write_html(&report, &options.output.join("index.html"))?;
    Ok(report)
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    {
        let mut writer = std::io::BufWriter::new(file.as_file_mut());
        serde_json::to_writer_pretty(&mut writer, value).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
    }
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Compare only identical inputs/settings; report every class/property regression
/// and any loss of coverage. Metrics are fractions, tolerance is percentage points.
pub fn regressions(
    current: &Report,
    baseline: &Report,
    tolerance_pp: f64,
) -> Result<Vec<String>, String> {
    let identities = |report: &Report| {
        report
            .samples
            .iter()
            .map(|r| {
                (
                    r.sample.id.clone(),
                    r.sample.image_fingerprint.clone(),
                    r.sample.annotation_fingerprint.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    if current.version != baseline.version
        || current.evaluator != baseline.evaluator
        || current.dataset != baseline.dataset
        || current.manifest_fingerprint != baseline.manifest_fingerprint
        || current.split != baseline.split
        || current.straighten != baseline.straighten
        || identities(current) != identities(baseline)
    {
        return Err("Baseline has different inputs, manifest or evaluator settings".into());
    }
    for report in [current, baseline] {
        if report.samples.len() != report.coverage.selected
            || report.coverage.evaluated + report.coverage.unsupported + report.coverage.failed
                != report.coverage.selected
        {
            return Err("Cannot compare an incomplete benchmark run".into());
        }
    }
    if !tolerance_pp.is_finite() || tolerance_pp < 0.0 {
        return Err("Regression tolerance must be finite and non-negative".into());
    }
    let mut failures = Vec::new();
    for (new, old) in current.samples.iter().zip(&baseline.samples) {
        if old.status == "evaluated" && new.status != "evaluated" {
            failures.push(format!("Lost coverage: {}", new.sample.id));
        }
    }
    let tolerance = tolerance_pp / 100.0;
    if let Some(previous) = baseline.metrics.class_totals().f1() {
        let actual = current.metrics.class_totals().f1().unwrap_or(0.0);
        if previous - actual > tolerance + 1e-12 {
            failures.push(format!(
                "All classes: F1 {:.2}% -> {:.2}%",
                previous * 100.0,
                actual * 100.0
            ));
        }
    }
    for (label, new, old) in [
        ("class", &current.metrics.classes, &baseline.metrics.classes),
        (
            "property",
            &current.metrics.properties,
            &baseline.metrics.properties,
        ),
    ] {
        for (name, counts) in old {
            if let Some(previous) = counts.f1() {
                let actual = new.get(name).and_then(metrics::Counts::f1).unwrap_or(0.0);
                if previous - actual > tolerance + 1e-12 {
                    failures.push(format!(
                        "{label} {name}: F1 {:.2}% -> {:.2}%",
                        previous * 100.0,
                        actual * 100.0
                    ));
                }
            }
        }
        for (name, counts) in new.iter().filter(|(name, _)| !old.contains_key(*name)) {
            let reference_count =
                baseline.metrics.class_totals().correct + baseline.metrics.class_totals().missing;
            if counts.extra as f64 > tolerance * reference_count as f64 + 1e-12 {
                failures.push(format!(
                    "{label} {name}: {} new false positives",
                    counts.extra
                ));
            }
        }
    }
    if let (Some(actual), Some(previous)) = (
        current.metrics.symbol_error_rate(),
        baseline.metrics.symbol_error_rate(),
    ) && actual - previous > tolerance + 1e-12
    {
        failures.push(format!(
            "SER {:.2}% -> {:.2}%",
            previous * 100.0,
            actual * 100.0
        ));
    }
    Ok(failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_match_standard_fnv1a_vectors() {
        assert_eq!(fingerprint(b""), "cbf29ce484222325");
        assert_eq!(fingerprint(b"hello"), "a430d84680aabd0b");
    }

    #[test]
    fn manifest_rejects_source_leakage_and_duplicate_ids() {
        let sample = Sample {
            id: "a".into(),
            group: "score".into(),
            split: "dev".into(),
            image: "a.png".into(),
            annotation: "a.semantic".into(),
            image_fingerprint: fingerprint(b"a"),
            annotation_fingerprint: fingerprint(b"b"),
        };
        let mut manifest = Manifest {
            version: 1,
            dataset: "primus".into(),
            source: "test".into(),
            data_root: ".".into(),
            seed: 42,
            test_percent: 20,
            candidate_count: 2,
            samples: vec![sample.clone()],
        };
        assert!(manifest.validate().is_ok());
        manifest.samples.push(sample);
        assert!(manifest.validate().unwrap_err().contains("duplicate"));
        manifest.samples[1].id = "b".into();
        manifest.samples[1].split = "test".into();
        assert!(manifest.validate().unwrap_err().contains("leaks"));
    }

    #[test]
    fn offline_run_preserves_coverage_integrity_and_reports_actual_misses() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let image = image::GrayImage::from_pixel(100, 80, image::Luma([255]));
        image.save(root.join("blank.png")).unwrap();
        let image_hash = fingerprint(&fs::read(root.join("blank.png")).unwrap());
        let mut samples = Vec::new();
        for (id, text) in [
            ("supported", "clef-G2 note-C4_quarter barline"),
            ("unsupported", "clef-C3 note-C4_quarter"),
        ] {
            fs::write(root.join(format!("{id}.semantic")), text).unwrap();
            samples.push(Sample {
                id: id.into(),
                group: id.into(),
                split: "dev".into(),
                image: "blank.png".into(),
                annotation: format!("{id}.semantic").into(),
                image_fingerprint: image_hash.clone(),
                annotation_fingerprint: fingerprint(text.as_bytes()),
            });
        }
        let manifest = Manifest {
            version: 1,
            dataset: "primus".into(),
            source: "synthetic test".into(),
            data_root: ".".into(),
            seed: 42,
            test_percent: 20,
            candidate_count: 2,
            samples,
        };
        write_json(&root.join("manifest.json"), &manifest).unwrap();
        let mut options = Options {
            manifest: root.join("manifest.json"),
            output: root.join("first"),
            split: "dev".into(),
            limit: None,
            overlays: 1,
            straighten: false,
        };
        let baseline = run(&options).unwrap();
        assert_eq!(baseline.coverage.evaluated, 1);
        assert_eq!(baseline.coverage.unsupported, 1);
        assert_eq!(baseline.coverage.failed, 0);
        assert_eq!(baseline.metrics.reference_events, 2);
        assert_eq!(baseline.metrics.edit_errors, 2);
        assert_eq!(baseline.metrics.classes["note"].missing, 1);
        assert!(root.join("first/index.html").exists());
        assert!(root.join("first/sample-00000.png").exists());
        assert!(run(&options).is_err(), "never overwrite a previous report");
        options.output = root.join("second");
        let current = run(&options).unwrap();
        assert!(regressions(&current, &baseline, 0.0).unwrap().is_empty());
        let mut worse = current.clone();
        worse.metrics.classes.insert(
            "spurious".into(),
            metrics::Counts {
                correct: 0,
                extra: 1,
                missing: 0,
            },
        );
        assert!(
            regressions(&worse, &baseline, 0.0)
                .unwrap()
                .iter()
                .any(|s| s.contains("new false positives"))
        );
        worse = current.clone();
        worse.metrics.edit_errors += 1;
        assert!(
            regressions(&worse, &baseline, 0.0)
                .unwrap()
                .iter()
                .any(|s| s.starts_with("SER"))
        );
        worse.straighten = true;
        assert!(regressions(&worse, &baseline, 0.0).is_err());
        fs::write(root.join("supported.semantic"), "note-D4_quarter").unwrap();
        options.output = root.join("changed");
        let changed = run(&options).unwrap();
        assert_eq!(changed.coverage.failed, 1);
        assert!(
            changed.samples[0]
                .reason
                .as_ref()
                .unwrap()
                .contains("fingerprint")
        );
        assert!(
            regressions(&changed, &baseline, 0.0)
                .unwrap()
                .iter()
                .any(|s| s.starts_with("Lost coverage"))
        );
    }
}
