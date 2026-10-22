//! Structured, navigable findings from the MusicXML export checks.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Object {
    Note(usize),
    Rest(usize),
    Barline(usize),
    Span(usize),
    KeyChange(usize),
    TimeChange(usize),
    Dynamic(usize),
    RepeatMark(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Export is blocked until the score is corrected.
    Error,
    /// Export uses an assumption or omits a symbol.
    Warning,
    /// Recognition proposal that has not been reviewed yet.
    Review,
}

#[derive(Clone, Debug)]
pub struct Issue {
    /// Zero-based page index in document order.
    pub page: usize,
    pub staff: Option<usize>,
    pub object: Option<Object>,
    /// MusicXML measure number, including expanded multimeasure rests.
    pub measure: Option<u32>,
    pub x: Option<f32>,
    pub severity: Severity,
    pub message: String,
    pub help: &'static str,
}

impl Issue {
    pub(crate) fn warning(
        staff: usize,
        object: Option<Object>,
        x: Option<f32>,
        message: impl Into<String>,
        help: &'static str,
    ) -> Self {
        Self {
            page: 0,
            staff: Some(staff),
            object,
            measure: None,
            x,
            severity: Severity::Warning,
            message: message.into(),
            help,
        }
    }

    pub(crate) fn error(page: usize, message: String) -> Self {
        Self {
            page,
            staff: None,
            object: None,
            measure: None,
            x: None,
            severity: Severity::Error,
            message,
            help: "Seite prüfen, fehlende Objekte ergänzen oder erneut erkennen lassen.",
        }
    }
}

/// Uses the exporter's checks and timeline, including meter inheritance across pages.
/// Findings are recomputed from the score; confirming a symbol never hides an error.
pub fn inspect_pages(pages: &[&crate::model::Score]) -> Vec<Issue> {
    crate::musicxml::inspect_pages(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Barline, BarlineKind, Clef, Duration, KeySignature, RepeatMark, RepeatMarkKind, Score,
        Span, SpanKind, Staff, StaffHeader, TimeChange, TimeSignature,
    };

    fn score() -> Score {
        let mut score = Score {
            staves: vec![Staff {
                left: 20.0,
                right: 500.0,
                lines: [40.0, 50.0, 60.0, 70.0, 80.0],
                header: StaffHeader {
                    clef: Clef::Treble,
                    key_signature: Some(KeySignature { fifths: 0 }),
                    time_signature: Some(TimeSignature::Common),
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        score.add_note(80.0, 60.0).unwrap();
        score.notes[0].duration = Duration::Whole;
        score.notes[0].confirmed = true;
        score.barlines.push(Barline {
            staff: 0,
            x: 400.0,
            kind: BarlineKind::Final,
            confirmed: true,
        });
        score
    }

    fn problems(score: &Score) -> Vec<Issue> {
        inspect_pages(&[score])
            .into_iter()
            .filter(|issue| issue.severity != Severity::Review)
            .collect()
    }

    #[test]
    fn corrections_remove_measure_findings_but_confirmation_does_not() {
        let mut score = score();
        assert!(problems(&score).is_empty());
        score.notes[0].duration = Duration::Quarter;
        score.notes[0].confirmed = false;
        let findings = inspect_pages(&[&score]);
        let measure = findings
            .iter()
            .find(|issue| issue.measure == Some(1))
            .unwrap();
        assert_eq!(measure.staff, Some(0));
        assert_eq!(measure.x, Some(20.0));
        assert!(findings.iter().any(
            |issue| issue.object == Some(Object::Note(0)) && issue.severity == Severity::Review
        ));
        score.notes[0].confirmed = true;
        assert_eq!(problems(&score).len(), 1);
        score.notes[0].duration = Duration::Whole;
        assert!(inspect_pages(&[&score]).is_empty());
    }

    #[test]
    fn all_missing_headers_open_the_system_location() {
        let mut score = score();
        score.staves[0].header.clef = Clef::Unknown;
        score.staves[0].header.key_signature = None;
        score.staves[0].header.time_signature = None;
        let issues = problems(&score);
        assert_eq!(issues.len(), 3);
        assert!(issues.iter().all(|issue| issue.staff == Some(0)
            && issue.object.is_none()
            && issue.measure.is_none()));
    }

    #[test]
    fn bows_wedges_and_endings_have_specific_editable_targets() {
        let mut score = score();
        score.spans.push(Span {
            staff: 0,
            kind: SpanKind::Slur,
            start: [220.0, 60.0],
            end: [320.0, 60.0],
            height: -10.0,
            confirmed: true,
        });
        score.repeat_marks.push(RepeatMark {
            staff: 0,
            x: 20.0,
            y: 20.0,
            end_x: 400.0,
            kind: RepeatMarkKind::Ending,
            numbers: String::new(),
            closed: true,
            bounds: None,
            confirmed: true,
        });
        let mut empty = score.staves[0].clone();
        empty.lines = empty.lines.map(|y| y + 140.0);
        score.staves.push(empty);
        score.spans.push(Span {
            staff: 1,
            kind: SpanKind::Crescendo,
            start: [80.0, 240.0],
            end: [320.0, 240.0],
            height: 10.0,
            confirmed: true,
        });
        let issues = problems(&score);
        for object in [Object::Span(0), Object::Span(1), Object::RepeatMark(0)] {
            assert!(
                issues.iter().any(|issue| issue.object == Some(object)),
                "{object:?}: {issues:?}"
            );
        }
        score.spans.clear();
        score.repeat_marks[0].numbers = "1,2".into();
        assert!(problems(&score).is_empty());
        score.repeat_marks[0].end_x = 300.0;
        assert!(
            problems(&score)
                .iter()
                .any(|issue| issue.object == Some(Object::RepeatMark(0)))
        );
    }

    #[test]
    fn preflight_keeps_later_findings_after_a_blocking_multirest_conflict() {
        let mut score = score();
        score.add_rest(140.0, 60.0, Duration::Whole).unwrap();
        score.rests[0].measures = Some(3);
        score.add_note(450.0, 60.0).unwrap();
        let issues = problems(&score);
        assert!(issues.iter().any(
            |issue| issue.severity == Severity::Error && issue.object == Some(Object::Rest(0))
        ));
        assert!(
            issues
                .iter()
                .any(|issue| issue.measure.is_some() && issue.x == Some(400.0))
        );
        assert!(crate::musicxml::export(&score, "Test").is_err());
        score.notes.remove(0);
        assert!(
            problems(&score)
                .iter()
                .all(|issue| issue.severity != Severity::Error)
        );
    }

    #[test]
    fn cross_page_meter_and_object_indices_match_export_without_mutating_scores() {
        let mut first = score();
        first.time_changes.push(TimeChange {
            staff: 0,
            x: 470.0,
            time_signature: TimeSignature::Meter {
                beats: 3,
                beat_unit: 4,
            },
            bounds: vec![],
            confirmed: true,
        });
        first.spans.push(Span {
            staff: 0,
            kind: SpanKind::Crescendo,
            start: [80.0, 100.0],
            end: [300.0, 100.0],
            height: 10.0,
            confirmed: true,
        });
        let mut second = score();
        second.staves[0].header.time_signature = None;
        second.notes[0].duration = Duration::Half;
        second.notes[0].dots = 1;
        second.spans.push(Span {
            staff: 0,
            kind: SpanKind::Tie,
            start: [220.0, 60.0],
            end: [320.0, 60.0],
            height: -10.0,
            confirmed: true,
        });
        let before = serde_json::to_string(&[&first, &second]).unwrap();
        let issues = inspect_pages(&[&first, &second]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].page, 1);
        assert_eq!(issues[0].staff, Some(0));
        assert_eq!(issues[0].object, Some(Object::Span(0)));
        assert_eq!(
            crate::musicxml::export_pages(&[&first, &second], "Test")
                .unwrap()
                .warnings
                .len(),
            1
        );
        assert_eq!(serde_json::to_string(&[&first, &second]).unwrap(), before);
        second.spans.clear();
        second.notes[0].dots = 0;
        let issues = inspect_pages(&[&first, &second]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].measure, Some(2));
    }

    #[test]
    fn dangling_repeats_and_overlap_limits_are_reported() {
        let mut score = score();
        score.barlines[0].kind = BarlineKind::RepeatStart;
        for _ in 0..17 {
            score.spans.push(Span {
                staff: 0,
                kind: SpanKind::Decrescendo,
                start: [80.0, 100.0],
                end: [300.0, 100.0],
                height: 10.0,
                confirmed: true,
            });
        }
        let issues = problems(&score);
        assert!(
            issues
                .iter()
                .any(|issue| issue.object == Some(Object::Barline(0)))
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.object == Some(Object::Span(16)))
        );
    }

    #[test]
    fn invalid_and_empty_pages_do_not_hide_other_pages() {
        let mut invalid = score();
        invalid.notes[0].staff = 99;
        let empty = Score::default();
        let mut third = score();
        third.notes[0].duration = Duration::Quarter;
        let issues = inspect_pages(&[&invalid, &empty, &third]);
        for page in [0, 1] {
            assert!(
                issues
                    .iter()
                    .any(|issue| issue.page == page && issue.severity == Severity::Error)
            );
        }
        assert!(
            issues
                .iter()
                .any(|issue| issue.page == 2 && issue.measure.is_some())
        );
    }
}
