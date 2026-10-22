use crate::model::{BarlineKind, Duration, Note, Score, TICKS_PER_QUARTER, TimeSignature};

/// Express a duration as a fraction of a whole note for the scan and preflight.
pub(crate) fn duration_fraction(ticks: u128, divisions: u128) -> String {
    let whole = divisions * 4;
    let (mut a, mut b) = (ticks, whole);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    format!("{}/{}", ticks / a, whole / a)
}

/// Duration of one measure (or one printed multimeasure rest).
#[derive(Clone, Debug)]
pub struct MeasureValidity {
    pub left: f32,
    pub right: f32,
    /// Index of the closing barline; `None` denotes the end of the staff.
    pub barline: Option<usize>,
    pub divisions: u128,
    pub actual_ticks: u128,
    pub expected_ticks: Option<u128>,
}

impl MeasureValidity {
    pub fn is_valid(&self) -> Option<bool> {
        self.expected_ticks
            .map(|expected| self.actual_ticks == expected)
    }

    pub fn label(&self) -> String {
        let Some(expected) = self.expected_ticks else {
            return "Taktart unbekannt: keine Taktprüfung".into();
        };
        format!(
            "Takt {}: Notenwerte {}, erwartet {}",
            if self.actual_ticks == expected {
                "gültig"
            } else {
                "ungültig"
            },
            duration_fraction(self.actual_ticks, self.divisions),
            duration_fraction(expected, self.divisions),
        )
    }
}

#[derive(Clone, Copy)]
enum Event {
    Note(usize),
    Rest(usize),
    Time(usize),
}

impl Score {
    /// Sum rhythmic events per staff in source order. Chords use the same
    /// position tolerance and duration rule as MusicXML export.
    pub fn measure_validities(&self, staff_index: usize) -> Vec<MeasureValidity> {
        self.measure_validities_with_inherited(staff_index, None)
    }

    /// Check a page with the final meter inherited from preceding pages.
    pub fn measure_validities_with_inherited(
        &self,
        staff_index: usize,
        inherited: Option<TimeSignature>,
    ) -> Vec<MeasureValidity> {
        let Some(staff) = self.staves.get(staff_index) else {
            return vec![];
        };
        let divisions = self.rhythm_divisions();
        let factor = divisions / u128::from(TICKS_PER_QUARTER);
        let mut events: Vec<_> = self
            .notes
            .iter()
            .enumerate()
            .filter(|(_, note)| note.staff == staff_index && !note.grace)
            .map(|(index, note)| (note.x, Event::Note(index)))
            .chain(
                self.rests
                    .iter()
                    .enumerate()
                    .filter(|(_, rest)| rest.staff == staff_index)
                    .map(|(index, rest)| (rest.x, Event::Rest(index))),
            )
            .chain(
                self.time_changes
                    .iter()
                    .enumerate()
                    .filter(|(_, change)| change.staff == staff_index)
                    .map(|(index, change)| (change.x, Event::Time(index))),
            )
            .collect();
        events.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then_with(|| matches!(b.1, Event::Time(_)).cmp(&matches!(a.1, Event::Time(_))))
        });
        let mut bars: Vec<_> = self
            .barlines
            .iter()
            .enumerate()
            .filter(|(_, bar)| bar.staff == staff_index)
            .collect();
        bars.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        let mut meter =
            self.time_signature_before_with_inherited(staff_index, staff.left, inherited);
        let mut cursor = 0;
        let mut left = staff.left;
        let mut result = vec![];
        for boundary in bars.into_iter().map(Some).chain(std::iter::once(None)) {
            let right = boundary.map_or(staff.right, |(_, bar)| bar.x);
            let end = if boundary.is_some() {
                events.partition_point(|event| event.0 < right)
            } else {
                events.len()
            };
            let segment = &events[cursor..end];
            let count = segment
                .iter()
                .filter(|(_, event)| !matches!(event, Event::Time(_)))
                .count();
            let mut previous: Option<&Note> = None;
            let mut actual = 0;
            let mut measures = 1;
            for &(_, event) in segment {
                match event {
                    Event::Time(index) => {
                        meter = Some(self.time_changes[index].time_signature);
                        previous = None;
                    }
                    Event::Note(index) => {
                        let note = &self.notes[index];
                        if !previous.is_some_and(|prior| {
                            (note.x - prior.x).abs() <= staff.spacing() * 0.35
                                && note.ticks(divisions) == prior.ticks(divisions)
                        }) {
                            actual += note.ticks(divisions);
                            previous = Some(note);
                        }
                    }
                    Event::Rest(index) => {
                        let rest = &self.rests[index];
                        let full_measure = rest.measures.is_some()
                            || (rest.duration == Duration::Whole && rest.dots == 0 && count == 1);
                        if let Some(rest_measures) = rest.measures {
                            measures = u32::from(rest_measures);
                        }
                        actual += if full_measure {
                            u128::from(meter.and_then(|meter| meter.ticks()).unwrap_or(0))
                                * u128::from(rest.measures.unwrap_or(1))
                                * factor
                        } else {
                            rest.ticks(divisions)
                        };
                        previous = None;
                    }
                }
            }
            // A bar at the row start closes no measure; attributes after the
            // last bar are courtesy changes for the next row, not an empty bar.
            let opening_repeat = result.is_empty()
                && count == 0
                && boundary.is_some_and(|(_, bar)| {
                    matches!(bar.kind, BarlineKind::RepeatStart | BarlineKind::RepeatBoth)
                });
            if right > left && (boundary.is_some() || count > 0) && !opening_repeat {
                result.push(MeasureValidity {
                    left,
                    right,
                    barline: boundary.map(|(index, _)| index),
                    divisions,
                    actual_ticks: actual,
                    expected_ticks: meter
                        .and_then(|meter| meter.ticks())
                        .map(|ticks| u128::from(ticks) * u128::from(measures) * factor),
                });
            }
            cursor = end;
            left = right;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Barline, Staff, StaffHeader, TimeChange, TimeSignature};

    fn score() -> Score {
        Score {
            staves: vec![Staff {
                lines: [60.0, 72.0, 84.0, 96.0, 108.0],
                left: 20.0,
                right: 800.0,
                header: StaffHeader {
                    time_signature: Some(TimeSignature::Common),
                    ..Default::default()
                },
            }],
            ..Default::default()
        }
    }

    fn bar(score: &mut Score, x: f32) {
        score.barlines.push(Barline {
            staff: 0,
            x,
            kind: Default::default(),
            confirmed: false,
        });
    }

    #[test]
    fn sums_notes_and_rests_and_updates_after_edits() {
        let mut score = score();
        score.add_note(100.0, 84.0);
        score.add_rest(200.0, 84.0, Duration::Half);
        score.add_note(300.0, 84.0);
        bar(&mut score, 400.0);
        let measure = &score.measure_validities(0)[0];
        assert_eq!(measure.is_valid(), Some(true));
        assert_eq!(measure.actual_ticks, 768);
        assert_eq!(measure.barline, Some(0));
        score.notes[1].duration = Duration::Eighth;
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(false));
        score.notes[1].duration = Duration::Half;
        let measure = &score.measure_validities(0)[0];
        assert_eq!(measure.is_valid(), Some(false));
        assert!(measure.label().contains("5/4"));
        assert!(measure.label().contains("1/1"));
    }

    #[test]
    fn dotted_and_triplet_values_stay_exact_down_to_sixty_fourths() {
        for (duration, base) in Duration::ALL
            .into_iter()
            .zip([768, 384, 192, 96, 48, 24, 12])
        {
            let mut score = score();
            score.add_note(100.0, 84.0);
            score.notes[0].duration = duration;
            for (dots, numerator, denominator) in [(0, 1, 1), (1, 3, 2), (2, 7, 4)] {
                score.notes[0].dots = dots;
                for triplet in [false, true] {
                    score.notes[0].triplet = triplet;
                    assert_eq!(
                        score.measure_validities(0)[0].actual_ticks,
                        base * numerator / denominator * if triplet { 2 } else { 3 } / 3
                    );
                }
            }
        }
        let mut score = score();
        score.staves[0].header.time_signature = Some(TimeSignature::Meter {
            beats: 1,
            beat_unit: 4,
        });
        for x in [100.0, 200.0, 300.0] {
            let index = score.add_note(x, 84.0).unwrap();
            score.notes[index].duration = Duration::Eighth;
            score.notes[index].triplet = true;
        }
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(true));
    }

    #[test]
    fn general_tuplets_remain_exact_for_all_values_and_dots() {
        use crate::model::Tuplet;
        for actual in 2..=64 {
            for duration in Duration::ALL {
                for dots in 0..=2 {
                    let mut score = score();
                    for index in 0..actual {
                        score.add_note(100.0 + f32::from(index) * 8.0, 84.0);
                        let note = score.notes.last_mut().unwrap();
                        note.duration = duration;
                        note.dots = dots;
                        note.tuplet = Some(Tuplet { actual, normal: 4 });
                    }
                    let measure = &score.measure_validities(0)[0];
                    let (numerator, denominator) = match dots {
                        1 => (3, 2),
                        2 => (7, 4),
                        _ => (1, 1),
                    };
                    assert_eq!(
                        measure.actual_ticks,
                        u128::from(duration.ticks()) * (measure.divisions / 192) * 4 * numerator
                            / denominator,
                        "{actual}:4 {duration:?} dots={dots}"
                    );
                }
            }
        }
    }

    #[test]
    fn grace_notes_do_not_change_measure_rest_or_chord_duration() {
        let mut score = score();
        score.add_rest(200.0, 84.0, Duration::Whole);
        for x in [100.0, 120.0, 140.0] {
            score.add_note(x, 84.0);
            score.notes.last_mut().unwrap().grace = true;
        }
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(true));
        score.rests.clear();
        score.add_note(300.0, 84.0);
        score.add_note(302.0, 78.0);
        score.add_rest(400.0, 84.0, Duration::Half);
        score.add_note(500.0, 84.0);
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(true));
    }

    #[test]
    fn chords_count_once_and_do_not_cross_bars_or_rests() {
        let mut score = score();
        for (x, pitch) in [(103.0, 90.0), (100.0, 84.0), (102.0, 78.0)] {
            score.add_note(x, pitch);
        }
        score.add_rest(200.0, 84.0, Duration::Half);
        score.add_note(300.0, 84.0);
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(true));
        // Near notes with different durations remain separate events.
        score.notes[0].duration = Duration::Half;
        assert_eq!(score.measure_validities(0)[0].actual_ticks, 1152);
        score.notes.clear();
        score.rests.clear();
        score.add_note(399.0, 84.0);
        score.add_note(401.0, 90.0);
        bar(&mut score, 400.0);
        assert_eq!(
            score
                .measure_validities(0)
                .iter()
                .map(|measure| measure.actual_ticks)
                .collect::<Vec<_>>(),
            vec![192, 192]
        );
    }

    #[test]
    fn dotted_rests_count_exactly_and_dotted_whole_rests_keep_their_value() {
        let mut score = score();
        score.add_rest(100.0, 84.0, Duration::Quarter);
        for (duration, base) in Duration::ALL
            .into_iter()
            .zip([768, 384, 192, 96, 48, 24, 12])
        {
            score.rests[0].duration = duration;
            for (dots, numerator) in [(0, 4), (1, 6), (2, 7)] {
                score.rests[0].dots = dots;
                assert_eq!(
                    score.measure_validities(0)[0].actual_ticks,
                    base * numerator / 4,
                    "{duration:?}, {dots} dots"
                );
            }
        }
        score.staves[0].header.time_signature = Some(TimeSignature::Meter {
            beats: 3,
            beat_unit: 4,
        });
        score.rests[0].duration = Duration::Half;
        score.rests[0].dots = 1;
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(true));
        score.rests[0].duration = Duration::Whole;
        assert_eq!(score.measure_validities(0)[0].actual_ticks, 1152);
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(false));
        score.staves[0].header.time_signature = Some(TimeSignature::Meter {
            beats: 6,
            beat_unit: 4,
        });
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(true));
    }

    #[test]
    fn whole_measure_and_multimeasure_rests_follow_the_meter() {
        let mut score = score();
        score.staves[0].header.time_signature = Some(TimeSignature::Meter {
            beats: 3,
            beat_unit: 4,
        });
        score.add_rest(100.0, 84.0, Duration::Whole);
        let measure = &score.measure_validities(0)[0];
        assert_eq!(measure.actual_ticks, 576);
        assert_eq!(measure.is_valid(), Some(true));
        score.rests[0].measures = Some(8);
        let measure = &score.measure_validities(0)[0];
        assert_eq!(measure.actual_ticks, 8 * 576);
        assert_eq!(measure.expected_ticks, Some(8 * 576));
        assert_eq!(measure.is_valid(), Some(true));
        score.add_note(200.0, 84.0);
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(false));
        score.notes.clear();
        score.add_rest(200.0, 84.0, Duration::Whole);
        score.rests[1].measures = Some(2);
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(false));
    }

    #[test]
    fn sorted_boundaries_keep_start_bars_empty_measures_and_final_segments_distinct() {
        let mut score = score();
        bar(&mut score, 600.0);
        bar(&mut score, 20.0);
        bar(&mut score, 400.0);
        let measures = score.measure_validities(0);
        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].barline, Some(2));
        assert_eq!(measures[1].barline, Some(0));
        assert!(
            measures
                .iter()
                .all(|measure| measure.is_valid() == Some(false))
        );
        score.add_note(600.0, 84.0);
        let measures = score.measure_validities(0);
        assert_eq!(measures.len(), 3);
        assert_eq!(measures[2].left, 600.0);
        assert_eq!(measures[2].right, 800.0);
        assert_eq!(measures[2].barline, None);
        assert_eq!(measures[2].actual_ticks, 192);
        assert!(score.measure_validities(10).is_empty());
    }

    #[test]
    fn meter_changes_and_courtesy_meters_carry_into_later_staves() {
        let mut score = score();
        score.add_note(100.0, 84.0);
        score.notes[0].duration = Duration::Whole;
        bar(&mut score, 400.0);
        score.time_changes.push(TimeChange {
            staff: 0,
            x: 400.0,
            time_signature: TimeSignature::Meter {
                beats: 3,
                beat_unit: 4,
            },
            bounds: vec![],
            confirmed: false,
        });
        score.add_rest(500.0, 84.0, Duration::Half);
        score.add_note(600.0, 84.0);
        bar(&mut score, 700.0);
        score.time_changes.push(TimeChange {
            staff: 0,
            x: 750.0,
            time_signature: TimeSignature::Cut,
            bounds: vec![],
            confirmed: false,
        });
        let measures = score.measure_validities(0);
        assert_eq!(measures.len(), 2);
        assert_eq!(measures[0].expected_ticks, Some(768));
        assert_eq!(measures[1].expected_ticks, Some(576));
        assert!(
            measures
                .iter()
                .all(|measure| measure.is_valid() == Some(true))
        );
        let mut next = score.staves[0].clone();
        next.lines = next.lines.map(|y| y + 200.0);
        next.header.time_signature = None;
        score.staves.push(next);
        score.add_rest(100.0, 284.0, Duration::Whole);
        assert_eq!(score.measure_validities(1)[0].expected_ticks, Some(768));
        assert_eq!(score.measure_validities(1)[0].is_valid(), Some(true));
        assert_eq!(score.measure_validities(0).len(), 2);
    }

    #[test]
    fn inherited_meter_checks_later_pages_and_yields_to_local_changes() {
        let mut first = score();
        // Storage order need not match the printed order. The courtesy change
        // on the bottom staff must win over the header on the top staff.
        let mut bottom = first.staves[0].clone();
        bottom.lines = bottom.lines.map(|y| y + 200.0);
        bottom.header.time_signature = None;
        first.staves.insert(0, bottom);
        let three_four = TimeSignature::Meter {
            beats: 3,
            beat_unit: 4,
        };
        first.time_changes.push(TimeChange {
            staff: 0,
            x: 750.0,
            time_signature: three_four,
            bounds: vec![],
            confirmed: false,
        });
        let inherited = first.ending_time_signature();
        assert_eq!(inherited, Some(three_four));

        let mut next = score();
        next.staves[0].header.time_signature = None;
        next.add_rest(100.0, 84.0, Duration::Half);
        next.rests[0].dots = 1;
        bar(&mut next, 400.0);
        assert_eq!(next.measure_validities(0)[0].is_valid(), None);
        assert_eq!(
            next.measure_validities_with_inherited(0, inherited)[0].is_valid(),
            Some(true)
        );
        next.rests[0].dots = 0;
        assert_eq!(
            next.measure_validities_with_inherited(0, inherited)[0].is_valid(),
            Some(false)
        );
        // Whole-measure and multimeasure rests also need the inherited meter.
        next.rests[0].duration = Duration::Whole;
        next.rests[0].measures = Some(8);
        let measures = next.measure_validities_with_inherited(0, inherited);
        assert_eq!(measures[0].actual_ticks, 8 * 576);
        assert_eq!(measures[0].is_valid(), Some(true));

        next.time_changes.push(TimeChange {
            staff: 0,
            x: 400.0,
            time_signature: TimeSignature::Common,
            bounds: vec![],
            confirmed: false,
        });
        next.add_note(500.0, 84.0);
        next.notes[0].duration = Duration::Whole;
        bar(&mut next, 700.0);
        let measures = next.measure_validities_with_inherited(0, inherited);
        assert_eq!(measures[0].expected_ticks, Some(8 * 576));
        assert_eq!(measures[1].expected_ticks, Some(768));
        assert!(
            measures
                .iter()
                .all(|measure| measure.is_valid() == Some(true))
        );
        assert_eq!(next.ending_time_signature(), Some(TimeSignature::Common));

        next.staves[0].header.time_signature = Some(TimeSignature::Cut);
        assert_eq!(
            next.time_signature_before_with_inherited(0, 100.0, inherited),
            Some(TimeSignature::Cut)
        );
        assert_eq!(Score::default().ending_time_signature(), None,);
    }

    #[test]
    fn unknown_and_invalid_meters_do_not_report_a_false_result() {
        let mut score = score();
        score.add_note(100.0, 84.0);
        for meter in [
            None,
            Some(TimeSignature::Meter {
                beats: 4,
                beat_unit: 0,
            }),
        ] {
            score.staves[0].header.time_signature = meter;
            let measure = &score.measure_validities(0)[0];
            assert_eq!(measure.is_valid(), None);
            assert!(measure.label().contains("Taktart unbekannt"));
        }
    }

    #[test]
    fn opening_repeat_after_the_header_does_not_create_an_empty_measure() {
        let mut score = score();
        bar(&mut score, 90.0);
        score.barlines[0].kind = BarlineKind::RepeatStart;
        score.add_rest(200.0, 84.0, Duration::Whole);
        bar(&mut score, 400.0);
        let measures = score.measure_validities(0);
        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].barline, Some(1));
        assert_eq!(measures[0].is_valid(), Some(true));
    }
}
