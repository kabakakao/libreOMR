use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Object;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Counts {
    pub correct: usize,
    pub extra: usize,
    pub missing: usize,
}

impl Counts {
    pub fn precision(&self) -> Option<f64> {
        ratio(self.correct, self.correct + self.extra)
    }

    pub fn recall(&self) -> Option<f64> {
        ratio(self.correct, self.correct + self.missing)
    }

    pub fn f1(&self) -> Option<f64> {
        ratio(
            2 * self.correct,
            2 * self.correct + self.extra + self.missing,
        )
    }
}

pub fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub classes: BTreeMap<String, Counts>,
    pub properties: BTreeMap<String, Counts>,
    pub reference_events: usize,
    pub edit_errors: usize,
    pub sequence_samples: usize,
    pub exact_sequences: usize,
}

impl Metrics {
    pub fn class_totals(&self) -> Counts {
        let mut total = Counts::default();
        for counts in self.classes.values() {
            total.correct += counts.correct;
            total.extra += counts.extra;
            total.missing += counts.missing;
        }
        total
    }

    pub fn merge(&mut self, other: &Self) {
        for (target, source) in [
            (&mut self.classes, &other.classes),
            (&mut self.properties, &other.properties),
        ] {
            for (name, counts) in source {
                let entry = target.entry(name.clone()).or_default();
                entry.correct += counts.correct;
                entry.extra += counts.extra;
                entry.missing += counts.missing;
            }
        }
        self.reference_events += other.reference_events;
        self.edit_errors += other.edit_errors;
        self.sequence_samples += other.sequence_samples;
        self.exact_sequences += other.exact_sequences;
    }

    pub fn symbol_error_rate(&self) -> Option<f64> {
        ratio(self.edit_errors, self.reference_events)
    }
}

/// A missing index represents a deletion/insertion. Indices address original objects.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pair {
    pub reference: Option<usize>,
    pub prediction: Option<usize>,
    pub correct: bool,
}

fn equal(reference: &Object, prediction: &Object) -> bool {
    reference.class == prediction.class
        && reference
            .attributes
            .iter()
            .all(|(key, value)| prediction.attributes.get(key) == Some(value))
}

pub fn measure(reference: &[Object], prediction: &[Object], pairs: &mut [Pair]) -> Metrics {
    let mut metrics = Metrics::default();
    for pair in pairs {
        let expected = pair.reference.map(|i| &reference[i]);
        let actual = pair.prediction.map(|i| &prediction[i]);
        pair.correct = matches!((expected, actual), (Some(a), Some(b)) if equal(a, b));
        match (expected, actual) {
            (Some(a), Some(b)) if a.class == b.class => {
                metrics.classes.entry(a.class.clone()).or_default().correct += 1;
            }
            (a, b) => {
                if let Some(a) = a {
                    metrics.classes.entry(a.class.clone()).or_default().missing += 1;
                }
                if let Some(b) = b {
                    metrics.classes.entry(b.class.clone()).or_default().extra += 1;
                }
            }
        }
        // Only reference-annotated attributes are measurable. A deletion contributes
        // a missing property; insertions contribute extras for applicable properties.
        if let Some(a) = expected {
            for (key, value) in &a.attributes {
                let counts = metrics
                    .properties
                    .entry(format!("{}.{}", a.family, key))
                    .or_default();
                match actual.filter(|b| b.family == a.family) {
                    Some(b) if b.attributes.get(key) == Some(value) => counts.correct += 1,
                    Some(b) => {
                        counts.missing += 1;
                        counts.extra += usize::from(b.attributes.contains_key(key));
                    }
                    None => counts.missing += 1,
                }
            }
        } else if let Some(b) = actual {
            for key in b.attributes.keys() {
                let name = format!("{}.{}", b.family, key);
                if reference
                    .iter()
                    .any(|a| a.family == b.family && a.attributes.contains_key(key))
                {
                    metrics.properties.entry(name).or_default().extra += 1;
                }
            }
        }
    }
    metrics
}

pub fn sequence(
    reference: &[Object],
    prediction: &[Object],
) -> Result<(Metrics, Vec<Pair>), String> {
    let rows = reference.len() + 1;
    let columns = prediction.len() + 1;
    let cells = rows.checked_mul(columns).ok_or("Sequence too large")?;
    if cells > 4_000_000 {
        return Err(
            "Sequence alignment exceeds 4 million cells; split into smaller samples".into(),
        );
    }
    let mut costs = vec![0_usize; cells];
    for i in 0..rows {
        costs[i * columns] = i;
    }
    for (j, cost) in costs.iter_mut().take(columns).enumerate() {
        *cost = j;
    }
    for i in 1..rows {
        for j in 1..columns {
            costs[i * columns + j] = (costs[(i - 1) * columns + j] + 1)
                .min(costs[i * columns + j - 1] + 1)
                .min(
                    costs[(i - 1) * columns + j - 1]
                        + usize::from(!equal(&reference[i - 1], &prediction[j - 1])),
                );
        }
    }
    let errors = costs[cells - 1];
    let (mut i, mut j) = (reference.len(), prediction.len());
    let mut pairs = Vec::new();
    while i > 0 || j > 0 {
        let diagonal = i > 0
            && j > 0
            && costs[i * columns + j]
                == costs[(i - 1) * columns + j - 1]
                    + usize::from(!equal(&reference[i - 1], &prediction[j - 1]));
        if diagonal {
            i -= 1;
            j -= 1;
            pairs.push(Pair {
                reference: Some(i),
                prediction: Some(j),
                correct: false,
            });
        } else if i > 0 && costs[i * columns + j] == costs[(i - 1) * columns + j] + 1 {
            i -= 1;
            pairs.push(Pair {
                reference: Some(i),
                prediction: None,
                correct: false,
            });
        } else {
            j -= 1;
            pairs.push(Pair {
                reference: None,
                prediction: Some(j),
                correct: false,
            });
        }
    }
    pairs.reverse();
    let mut metrics = measure(reference, prediction, &mut pairs);
    metrics.reference_events = reference.len();
    metrics.edit_errors = errors;
    metrics.sequence_samples = 1;
    metrics.exact_sequences = usize::from(errors == 0);
    Ok((metrics, pairs))
}

/// Maximum-cardinality bipartite matching inside each symbol family. A wrong
/// subtype stays a substitution. Distances order candidates but do not override
/// the one-to-one constraint (a duplicate prediction always remains an extra).
pub fn spatial(reference: &[Object], prediction: &[Object]) -> (Metrics, Vec<Pair>) {
    let edges: Vec<Vec<usize>> = reference
        .iter()
        .map(|a| {
            let mut candidates: Vec<_> = prediction
                .iter()
                .enumerate()
                .filter_map(|(j, b)| {
                    let (x, y) = b.center()?;
                    let bounds = a.bounds.as_ref()?;
                    let tolerance = ((bounds.bottom - bounds.top).min(bounds.right - bounds.left)
                        * 0.25)
                        .max(2.0);
                    if a.family == b.family
                        && x >= bounds.left - tolerance
                        && x <= bounds.right + tolerance
                        && y >= bounds.top - tolerance
                        && y <= bounds.bottom + tolerance
                    {
                        let (ax, ay) = a.center()?;
                        Some((j, (x - ax).hypot(y - ay)))
                    } else {
                        None
                    }
                })
                .collect();
            candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
            candidates.into_iter().map(|(j, _)| j).collect()
        })
        .collect();
    fn augment(
        i: usize,
        edges: &[Vec<usize>],
        owners: &mut [Option<usize>],
        seen: &mut [bool],
    ) -> bool {
        for &j in &edges[i] {
            if seen[j] {
                continue;
            }
            seen[j] = true;
            if owners[j].is_none_or(|previous| augment(previous, edges, owners, seen)) {
                owners[j] = Some(i);
                return true;
            }
        }
        false
    }
    let mut owners = vec![None; prediction.len()];
    for i in 0..reference.len() {
        augment(i, &edges, &mut owners, &mut vec![false; prediction.len()]);
    }
    let mut assigned = vec![None; reference.len()];
    for (j, owner) in owners.iter().enumerate() {
        if let Some(i) = owner {
            assigned[*i] = Some(j);
        }
    }
    let mut pairs: Vec<_> = assigned
        .into_iter()
        .enumerate()
        .map(|(i, prediction)| Pair {
            reference: Some(i),
            prediction,
            correct: false,
        })
        .collect();
    pairs.extend(
        owners
            .iter()
            .enumerate()
            .filter(|(_, owner)| owner.is_none())
            .map(|(j, _)| Pair {
                reference: None,
                prediction: Some(j),
                correct: false,
            }),
    );
    let metrics = measure(reference, prediction, &mut pairs);
    (metrics, pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SymbolBounds;

    fn note(pitch: &str, x: f32) -> Object {
        Object {
            class: "note.black".into(),
            family: "note".into(),
            attributes: BTreeMap::from([("pitch".into(), pitch.into())]),
            bounds: Some(SymbolBounds {
                left: x,
                top: 10.0,
                right: x + 10.0,
                bottom: 20.0,
            }),
        }
    }

    #[test]
    fn insertion_does_not_shift_the_rest_of_the_sequence() {
        let reference = vec![note("C4", 0.0), note("D4", 20.0), note("E4", 40.0)];
        let prediction = vec![
            note("C4", 0.0),
            note("G4", 10.0),
            note("D4", 20.0),
            note("E4", 40.0),
        ];
        let (metrics, _) = sequence(&reference, &prediction).unwrap();
        assert_eq!(metrics.edit_errors, 1);
        assert_eq!(metrics.classes["note.black"].extra, 1);
        assert_eq!(metrics.properties["note.pitch"].correct, 3);
        assert_eq!(metrics.properties["note.pitch"].extra, 1);
    }

    #[test]
    fn spatial_matching_counts_duplicates_and_wrong_properties() {
        let reference = vec![note("C4", 0.0)];
        let prediction = vec![note("D4", 0.0), note("C4", 1.0)];
        let (metrics, pairs) = spatial(&reference, &prediction);
        assert_eq!(metrics.classes["note.black"].correct, 1);
        assert_eq!(metrics.classes["note.black"].extra, 1);
        assert_eq!(metrics.properties["note.pitch"].missing, 1);
        assert!(!pairs[0].correct);
    }

    #[test]
    fn spatial_matching_recovers_cardinality_in_ambiguous_neighborhood() {
        let mut a = note("C4", 0.0);
        a.bounds.as_mut().unwrap().right = 30.0;
        let b = note("D4", 0.0);
        let prediction = vec![note("C4", 5.0), note("D4", 20.0)];
        let (metrics, _) = spatial(&[a, b], &prediction);
        assert_eq!(metrics.classes["note.black"].correct, 2);
        assert_eq!(metrics.classes["note.black"].missing, 0);
    }

    #[test]
    fn empty_reference_has_no_defined_error_rate() {
        let (metrics, _) = sequence(&[], &[note("C4", 0.0)]).unwrap();
        assert_eq!(metrics.symbol_error_rate(), None);
        assert_eq!(Counts::default().f1(), None);
    }
}
