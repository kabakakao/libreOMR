use libreomr::{model::*, musicxml};

fn score() -> Score {
    let mut score = Score::default();
    score.staves.push(Staff {
        lines: [40.0, 50.0, 60.0, 70.0, 80.0],
        left: 10.0,
        right: 500.0,
        header: StaffHeader {
            clef: Clef::Treble,
            key_signature: Some(KeySignature::default()),
            time_signature: Some(TimeSignature::Common),
            ..Default::default()
        },
    });
    score
}

#[test]
fn independent_staff_voices_have_separate_measure_totals_and_xml_timelines() {
    let mut score = score();
    for x in [100.0, 280.0] {
        let i = score.add_note(x, 45.0).unwrap();
        score.notes[i].duration = Duration::Half;
    }
    // Equal onsets and durations in different voices must never become a chord.
    let i = score.add_note(100.0, 75.0).unwrap();
    score.notes[i].duration = Duration::Half;
    score.notes[i].voice = 2;
    for x in [280.0, 370.0] {
        let i = score.add_rest(x, 85.0, Duration::Quarter).unwrap();
        score.rests[i].voice = 2;
    }
    let measure = &score.measure_validities(0)[0];
    assert_eq!(measure.is_valid(), Some(true));
    assert_eq!(measure.voice_ticks.len(), 2);
    let result = musicxml::export(&score, "Two voices").unwrap();
    assert!(
        !result.issues.iter().any(|issue| issue.measure.is_some()),
        "{:?}",
        result.warnings
    );
    assert!(!result.xml.contains("<chord/>"));
    assert_eq!(result.xml.matches("<voice>1</voice>").count(), 2);
    assert_eq!(result.xml.matches("<voice>2</voice>").count(), 3);
    assert!(
        result
            .xml
            .contains("<backup><duration>768</duration></backup>")
    );

    score.rests[1].duration = Duration::Eighth;
    assert_eq!(score.measure_validities(0)[0].is_valid(), Some(false));
    let result = musicxml::export(&score, "Incomplete voice").unwrap();
    let voice = libreomr::i18n::format("Voice {voice}", &[("voice", "2".into())]);
    assert!(
        result
            .issues
            .iter()
            .any(|issue| issue.measure == Some(1) && issue.message.starts_with(&voice)),
        "{:?}",
        result.warnings
    );
}

#[test]
fn whole_measure_rests_and_chords_are_scoped_to_their_voice() {
    let mut score = score();
    let i = score.add_note(100.0, 45.0).unwrap();
    score.notes[i].duration = Duration::Whole;
    let mut chord = score.notes[i].clone();
    chord.pitch += 2;
    chord.y -= 10.0;
    score.notes.push(chord);
    let i = score.add_rest(120.0, 85.0, Duration::Whole).unwrap();
    score.rests[i].voice = 2;
    assert_eq!(score.measure_validities(0)[0].is_valid(), Some(true));
    let xml = musicxml::export(&score, "Chord and rest").unwrap().xml;
    assert_eq!(xml.matches("<chord/>").count(), 1);
    assert!(xml.contains("<rest measure=\"yes\"/><duration>768</duration><voice>2</voice>"));
}

#[test]
fn voice_assignments_roundtrip_and_legacy_projects_default_to_voice_one() {
    let mut score = score();
    let i = score.add_note(100.0, 45.0).unwrap();
    score.notes[i].voice = 3;
    let i = score.add_rest(200.0, 85.0, Duration::Quarter).unwrap();
    score.rests[i].voice = 4;
    let mut json = serde_json::to_value(&score).unwrap();
    let restored: Score = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(restored.notes[0].voice, 3);
    assert_eq!(restored.rests[0].voice, 4);
    restored.validate().unwrap();
    for name in ["notes", "rests"] {
        json[name][0].as_object_mut().unwrap().remove("voice");
    }
    let legacy: Score = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.notes[0].voice, 1);
    assert_eq!(legacy.rests[0].voice, 1);
    for invalid in [0, 5] {
        let mut invalid_score = restored.clone();
        invalid_score.notes[0].voice = invalid;
        assert!(invalid_score.validate().is_err());
        invalid_score.notes[0].voice = 1;
        invalid_score.rests[0].voice = invalid;
        assert!(invalid_score.validate().is_err());
    }
}
