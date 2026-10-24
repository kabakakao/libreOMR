//! This separate test process can switch the application-wide language without
//! changing the locale of concurrently running editor/library regression tests.
use libreomr::{
    i18n::{self, Language},
    model::{Clef, Duration, KeySignature, Score, Staff, StaffHeader, TimeSignature},
    musicxml,
};

#[test]
fn changing_language_translates_labels_and_diagnostics_but_preserves_score_data() {
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
    score.add_note(100.0, 60.0).unwrap();
    score.notes[0].duration = Duration::Quarter;
    score.notes[0].confirmed = false;
    let serialized = serde_json::to_string(&score).unwrap();

    i18n::set_language(Language::English);
    assert_eq!(Clef::Treble.label(), "Treble clef");
    assert_eq!(KeySignature { fifths: -1 }.label(), "F major / D minor");
    let english = musicxml::export(&score, "Score {0}").unwrap();
    assert!(
        english
            .warnings
            .iter()
            .any(|warning| warning.contains("unreviewed"))
    );
    assert!(
        english
            .warnings
            .iter()
            .any(|warning| warning.contains("Measure"))
    );

    i18n::set_language(Language::German);
    assert_eq!(Clef::Treble.label(), "Violinschluessel");
    assert_eq!(KeySignature { fifths: -1 }.label(), "F-Dur / d-Moll");
    let german = musicxml::export(&score, "Score {0}").unwrap();
    assert!(
        german
            .warnings
            .iter()
            .any(|warning| warning.contains("ungeprueft"))
    );
    assert!(
        german
            .warnings
            .iter()
            .any(|warning| warning.contains("Takt"))
    );
    assert_eq!(english.xml, german.xml);
    assert_eq!(serialized, serde_json::to_string(&score).unwrap());
}
