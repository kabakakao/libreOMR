use libreomr::{model::*, musicxml};

fn score() -> Score {
    let mut score = Score::default();
    for y in [40.0, 180.0, 320.0] {
        score.staves.push(Staff {
            lines: [y, y + 10.0, y + 20.0, y + 30.0, y + 40.0],
            left: 10.0,
            right: 500.0,
            header: StaffHeader {
                clef: Clef::Treble,
                key_signature: Some(KeySignature::default()),
                time_signature: Some(TimeSignature::Common),
                ..Default::default()
            },
        });
    }
    score.add_note(100.0, 65.0).unwrap();
    score.add_note(400.0, 65.0).unwrap();
    score.add_note(100.0, 205.0).unwrap();
    score.add_note(400.0, 205.0).unwrap();
    score.add_note(100.0, 345.0).unwrap();
    score
}

fn bow(score: &Score, first: usize, last: usize) -> Span {
    let a = &score.notes[first];
    let b = &score.notes[last];
    Span {
        staff: a.staff,
        kind: SpanKind::Slur,
        start_anchor: Some(NoteAnchor {
            note: first,
            offset: [0.0, -8.0],
        }),
        end_anchor: Some(NoteAnchor {
            note: last,
            offset: [0.0, -8.0],
        }),
        continuation: None,
        wedge_openings: None,
        start: [a.x, a.y - 8.0],
        end: [b.x, b.y - 8.0],
        height: -15.0,
        confirmed: true,
    }
}

#[test]
fn anchored_bows_follow_notes_and_round_trip_with_both_directions() {
    let mut score = score();
    score.spans.push(bow(&score, 0, 1));
    score.notes[1].x += 25.0;
    score.notes[1].y += 5.0;
    let resolved = score.resolved_span(&score.spans[0]);
    assert_eq!(resolved.end, [425.0, 62.0]);
    score.spans[0].flip();
    let flipped = score.resolved_span(&score.spans[0]);
    assert_eq!(flipped.end, [425.0, 78.0]);
    assert_eq!(flipped.height, 15.0);
    score.spans[0].flip();
    assert_eq!(score.resolved_span(&score.spans[0]).end, resolved.end);
    let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
    assert!(restored.validate().is_ok());
    assert_eq!(restored.resolved_span(&restored.spans[0]).end, resolved.end);
}

#[test]
fn system_breaks_are_drawn_as_separate_curves_and_export_as_one_bow() {
    let mut score = score();
    score.spans.push(bow(&score, 1, 4));
    assert!(score.validate().is_ok());
    let segments = score.span_segments(&score.spans[0]);
    assert_eq!(segments.len(), 3);
    assert_eq!(
        segments.iter().map(|s| s.staff).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(segments[0].end[0], 500.0);
    assert_eq!(segments[1].start[0], 10.0);
    assert_eq!(segments[1].end[0], 500.0);
    assert_eq!(segments[2].end, [100.0, 337.0]);
    let exported = musicxml::export(&score, "System break").unwrap();
    assert_eq!(exported.xml.matches("<slur type=\"start\"").count(), 1);
    assert_eq!(exported.xml.matches("<slur type=\"stop\"").count(), 1);
    assert!(
        exported.xml.find("<slur type=\"start\"").unwrap()
            < exported.xml.find("<slur type=\"stop\"").unwrap()
    );
}

#[test]
fn ties_cross_rows_but_require_adjacent_notes_in_the_same_voice() {
    let mut score = score();
    let mut span = bow(&score, 1, 2);
    span.kind = SpanKind::Tie;
    score.spans.push(span);
    let exported = musicxml::export(&score, "Tie").unwrap();
    assert_eq!(exported.xml.matches("<tie type=\"start\"/>").count(), 1);
    assert_eq!(exported.xml.matches("<tie type=\"stop\"/>").count(), 1);
    score.add_rest(470.0, 60.0, Duration::Quarter).unwrap();
    let exported = musicxml::export(&score, "Rest interrupts tie").unwrap();
    assert!(!exported.xml.contains("<tie "));
}

#[test]
fn deleting_notes_remaps_anchors_and_removes_attached_bows() {
    let mut score = score();
    score.spans.push(bow(&score, 1, 2));
    score.remove_note(0);
    assert_eq!(score.spans[0].start_anchor.as_ref().unwrap().note, 0);
    assert_eq!(score.spans[0].end_anchor.as_ref().unwrap().note, 1);
    assert!(score.validate().is_ok());
    score.remove_note(1);
    assert!(score.spans.is_empty());
}

#[test]
fn multipage_export_remaps_note_anchors_without_attaching_to_the_first_page() {
    let first = score();
    let mut second = score();
    second.spans.push(bow(&second, 1, 2));
    let exported = musicxml::export_pages(&[&first, &second], "Two pages").unwrap();
    let page_two = exported.xml.find("new-page=\"yes\"").unwrap();
    assert!(!exported.xml[..page_two].contains("<slur "));
    assert_eq!(exported.xml[page_two..].matches("<slur ").count(), 2);
}

#[test]
fn invalid_and_reversed_references_are_rejected_and_legacy_bows_still_load() {
    let mut score = score();
    score.spans.push(bow(&score, 1, 2));
    score.spans[0].end_anchor.as_mut().unwrap().note = 99;
    assert!(score.validate().is_err());
    score.spans[0] = bow(&score, 2, 1);
    assert!(score.validate().is_err());
    score.spans[0] = bow(&score, 0, 1);
    let mut value = serde_json::to_value(&score).unwrap();
    for key in [
        "start_anchor",
        "end_anchor",
        "continuation",
        "wedge_openings",
    ] {
        value["spans"][0].as_object_mut().unwrap().remove(key);
    }
    let legacy: Score = serde_json::from_value(value).unwrap();
    assert!(legacy.validate().is_ok());
    assert!(legacy.spans[0].start_anchor.is_none());
}

fn wedge(score: &Score, first: usize, last: usize, kind: SpanKind) -> Span {
    let mut span = bow(score, first, last);
    span.kind = kind;
    span.height = 10.0;
    for (point, anchor) in [
        (&mut span.start, &mut span.start_anchor),
        (&mut span.end, &mut span.end_anchor),
    ] {
        let anchor = anchor.as_mut().unwrap();
        let note = &score.notes[anchor.note];
        let staff = &score.staves[note.staff];
        point[1] = staff.lines[4] + staff.spacing() * 2.0;
        anchor.offset[1] = point[1] - note.y;
    }
    span
}

#[test]
fn wedge_anchors_follow_notes_and_continue_with_an_open_gap_across_systems() {
    for kind in [SpanKind::Crescendo, SpanKind::Decrescendo] {
        let mut score = score();
        score.staves[1].lines = [180.0, 200.0, 220.0, 240.0, 260.0];
        let mut span = wedge(&score, 0, 1, kind);
        assert!(score.attach_span_note(&mut span, true, 4));
        score.spans.push(span);
        score.validate().unwrap();
        let pieces = score.span_segments(&score.spans[0]);
        assert_eq!(pieces.len(), 3);
        for pair in pieces.windows(2) {
            assert!(pair[0].wedge_opening(1.0) > 0.0);
            assert!(pair[1].wedge_opening(0.0) > 0.0);
            assert!(
                (pair[0].wedge_opening(1.0) / score.staves[pair[0].staff].spacing()
                    - pair[1].wedge_opening(0.0) / score.staves[pair[1].staff].spacing())
                .abs()
                    < 0.001
            );
        }
        let closed = if kind == SpanKind::Crescendo {
            pieces[0].wedge_opening(0.0)
        } else {
            pieces[2].wedge_opening(1.0)
        };
        assert_eq!(closed, 0.0);
        score.notes[4].x += 20.0;
        score.notes[4].y += 5.0;
        let resolved = score.resolved_span(&score.spans[0]);
        assert_eq!(resolved.end, [120.0, 385.0]);
        let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.span_segments(&restored.spans[0]).len(), 3);
        score.remove_note(4);
        assert!(score.spans.is_empty());
    }
}

#[test]
fn wedge_export_matches_endpoints_across_systems_and_keeps_overlap_numbers_unique() {
    for kind in [SpanKind::Crescendo, SpanKind::Decrescendo] {
        let mut score = score();
        score.spans.push(wedge(&score, 1, 4, kind));
        score.spans.push(wedge(&score, 2, 3, kind));
        let result = musicxml::export(&score, "Continued wedges").unwrap();
        let name = if kind == SpanKind::Crescendo {
            "crescendo"
        } else {
            "diminuendo"
        };
        for number in [1, 2] {
            assert_eq!(
                result
                    .xml
                    .matches(&format!("<wedge type=\"{name}\" number=\"{number}\"/>"))
                    .count(),
                1
            );
            assert_eq!(
                result
                    .xml
                    .matches(&format!("<wedge type=\"stop\" number=\"{number}\"/>"))
                    .count(),
                1
            );
        }
        let last_system = result.xml.rfind("new-system=\"yes\"").unwrap();
        assert!(
            result
                .xml
                .find("<wedge type=\"stop\" number=\"1\"/>")
                .unwrap()
                > last_system
        );
    }
}

#[test]
fn wedge_note_anchors_define_time_independently_of_visual_offsets_including_other_voices() {
    for voice in [1, 2] {
        let mut score = score();
        score.notes.truncate(2);
        score.staves.truncate(1);
        for note in &mut score.notes {
            note.voice = voice;
        }
        if voice == 2 {
            let index = score.add_note(100.0, 45.0).unwrap();
            score.notes[index].duration = Duration::Whole;
        }
        score.spans.push(wedge(&score, 0, 1, SpanKind::Crescendo));
        let original = musicxml::export(&score, "Anchored wedge").unwrap();
        score.spans[0].start_anchor.as_mut().unwrap().offset[0] = 80.0;
        score.spans[0].end_anchor.as_mut().unwrap().offset[0] = -80.0;
        let shifted = musicxml::export(&score, "Anchored wedge").unwrap();
        assert_eq!(original.xml, shifted.xml);
        assert!(shifted.xml.contains(
            "<wedge type=\"crescendo\" number=\"1\"/></direction-type><offset>-384</offset>"
        ));
        assert!(
            shifted.xml.contains(
                "<wedge type=\"stop\" number=\"1\"/></direction-type><offset>-192</offset>"
            )
        );
    }
}

#[test]
fn wedge_anchors_cannot_cross_parts_and_opening_data_must_be_valid() {
    let mut score = score();
    score.staff_assignments = vec![
        StaffAssignment {
            staff: 0,
            system: 0,
            part: 0,
            name: "A".into(),
        },
        StaffAssignment {
            staff: 1,
            system: 0,
            part: 1,
            name: "B".into(),
        },
        StaffAssignment {
            staff: 2,
            system: 1,
            part: 0,
            name: "A".into(),
        },
    ];
    let mut span = wedge(&score, 0, 1, SpanKind::Decrescendo);
    assert!(!score.attach_span_note(&mut span, true, 2));
    assert!(!score.attach_span_note(&mut span, true, 0));
    assert!(score.attach_span_note(&mut span, true, 4));
    score.spans.push(span);
    score.validate().unwrap();
    score.spans[0].wedge_openings = Some([f32::NAN, 0.0]);
    assert!(score.validate().is_err());
    score.spans[0].wedge_openings = Some([1.0, -0.1]);
    assert!(score.validate().is_err());
}

#[test]
fn anchoring_cannot_join_different_parts_or_reverse_the_bow() {
    let mut score = score();
    score.staff_assignments = vec![
        StaffAssignment {
            staff: 0,
            system: 0,
            part: 0,
            name: "A".into(),
        },
        StaffAssignment {
            staff: 1,
            system: 0,
            part: 1,
            name: "B".into(),
        },
        StaffAssignment {
            staff: 2,
            system: 1,
            part: 0,
            name: "A".into(),
        },
    ];
    let mut span = bow(&score, 0, 1);
    assert!(!score.attach_span_note(&mut span, true, 2));
    assert!(!score.attach_span_note(&mut span, true, 0));
    assert!(score.attach_span_note(&mut span, true, 4));
    score.spans.push(span);
    assert!(score.validate().is_ok());
    assert_eq!(score.span_segments(&score.spans[0]).len(), 2);
}

#[test]
fn explicit_anchors_select_the_intended_chord_note_in_musicxml() {
    let mut score = score();
    let high = score.add_note(400.0, 45.0).unwrap();
    score.notes[high].pitch = 35; // C5, sharing an onset with another note.
    score.spans.push(bow(&score, 0, high));
    let exported = musicxml::export(&score, "Chord anchor").unwrap();
    let stopped = exported
        .xml
        .split("</note>")
        .find(|note| note.contains("<slur type=\"stop\""))
        .unwrap();
    assert!(stopped.contains("<step>C</step>"));
    assert!(stopped.contains("<octave>5</octave>"));
    // Reposition the note after attaching it; export must still use its reference.
    score.notes[high].x = 420.0;
    assert_eq!(score.resolved_span(&score.spans[0]).end[0], 420.0);
    let exported = musicxml::export(&score, "Moved anchor").unwrap();
    let stopped = exported
        .xml
        .split("</note>")
        .find(|note| note.contains("<slur type=\"stop\""))
        .unwrap();
    assert!(stopped.contains("<step>C</step>"));
}

#[test]
fn chord_tones_at_the_same_onsets_do_not_interrupt_ties() {
    let mut score = score();
    score.add_note(102.0, 45.0).unwrap();
    score.add_note(402.0, 45.0).unwrap();
    assert!(score.tie_compatible(0, 1));
    let mut span = bow(&score, 0, 1);
    span.kind = SpanKind::Tie;
    score.spans.push(span);
    assert!(
        musicxml::export(&score, "Chord tie")
            .unwrap()
            .xml
            .contains("<tie type=\"start\"/>")
    );
    score.add_note(250.0, 45.0).unwrap();
    assert!(!score.tie_compatible(0, 1));
    assert!(
        !musicxml::export(&score, "Interrupted chord tie")
            .unwrap()
            .xml
            .contains("<tie ")
    );
}

#[test]
fn tied_accidentals_survive_barlines_and_system_breaks_without_changing_later_notes() {
    let mut score = score();
    for note in &mut score.notes {
        note.pitch = 28;
    }
    score.notes[0].accidental = -1;
    score.notes[0].accidental_explicit = true;
    score.barlines.push(Barline {
        staff: 0,
        x: 250.0,
        kind: BarlineKind::Single,
        confirmed: true,
    });
    for (a, b) in [(1, 2), (0, 1)] {
        // Deliberately store the chain out of order.
        let mut span = bow(&score, a, b);
        span.kind = SpanKind::Tie;
        score.spans.push(span);
    }
    assert!(score.tie_compatible(0, 1));
    let exported = musicxml::export(&score, "Tied alteration").unwrap();
    assert_eq!(exported.xml.matches("<tie type=\"start\"/>").count(), 2);
    assert_eq!(exported.xml.matches("<alter>-1</alter>").count(), 3);
    assert_eq!(score.notes[1].accidental, 0); // Export does not mutate the score.
    score.recompute_accidentals();
    assert_eq!(
        score.notes.iter().map(|n| n.accidental).collect::<Vec<_>>(),
        [-1, -1, -1, 0, 0]
    );
    for note in &mut score.notes {
        note.confirmed = true;
    }
    score.recompute_accidentals();
    assert!(score.notes.iter().all(|note| note.confirmed));
    score.spans.clear();
    score.recompute_accidentals();
    assert_eq!(score.notes[1].accidental, 0);
    assert_eq!(score.notes[2].accidental, 0);
    assert!(!score.notes[1].confirmed);
    assert!(!score.notes[2].confirmed);
}

#[test]
fn an_explicit_different_accidental_prevents_a_tie() {
    let mut score = score();
    score.barlines.push(Barline {
        staff: 0,
        x: 250.0,
        kind: BarlineKind::Single,
        confirmed: true,
    });
    score.notes[0].accidental = -1;
    score.notes[0].accidental_explicit = true;
    score.notes[1].accidental_explicit = true;
    assert!(!score.tie_compatible(0, 1));
    let mut span = bow(&score, 0, 1);
    span.kind = SpanKind::Tie;
    score.spans.push(span);
    score.recompute_accidentals();
    assert_eq!(score.notes[1].accidental, 0);
    assert!(
        !musicxml::export(&score, "Explicit natural")
            .unwrap()
            .xml
            .contains("<tie ")
    );
}

#[test]
fn a_different_implicit_accidental_without_a_boundary_is_not_silently_rewritten() {
    let mut score = score();
    score.notes[0].accidental = -1;
    let mut span = bow(&score, 0, 1);
    span.kind = SpanKind::Tie;
    score.spans.push(span);
    assert!(!score.tie_compatible(0, 1));
    let exported = musicxml::export(&score, "Unresolved alteration").unwrap();
    assert!(!exported.xml.contains("<tie "));
    assert_eq!(exported.xml.matches("<alter>-1</alter>").count(), 1);
}
