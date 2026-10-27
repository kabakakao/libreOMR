use libreomr::{model::*, musicxml, score_structure};

fn staff(top: f32, clef: Clef) -> Staff {
    Staff {
        lines: [top, top + 10.0, top + 20.0, top + 30.0, top + 40.0],
        left: 20.0,
        right: 300.0,
        header: StaffHeader {
            clef,
            key_signature: Some(KeySignature { fifths: 2 }),
            time_signature: Some(TimeSignature::Meter {
                beats: 4,
                beat_unit: 2,
            }),
            ..Default::default()
        },
    }
}

fn score() -> Score {
    let mut score = Score::default();
    for (index, clef) in [Clef::Treble, Clef::Treble8vb, Clef::Treble, Clef::Treble8vb]
        .into_iter()
        .enumerate()
    {
        score.staves.push(staff(30.0 + index as f32 * 160.0, clef));
        score.staff_assignments.push(StaffAssignment {
            staff: index,
            part: index % 2,
            system: index / 2,
            name: if index % 2 == 0 { "S & A" } else { "Tenor" }.into(),
        });
        let y = score.staves[index].lines[4];
        score.add_note(100.0, y).unwrap();
        score.notes.last_mut().unwrap().duration = Duration::Whole;
        score.add_note(220.0, y).unwrap();
        score.notes.last_mut().unwrap().duration = Duration::Whole;
        score.barlines.push(Barline {
            staff: index,
            x: 299.0,
            kind: BarlineKind::Final,
            confirmed: true,
        });
    }
    score
}

#[test]
fn parts_continue_across_systems_and_pages_with_lyrics_and_tenor_octave() {
    let mut first = score();
    first.lyrics.push(Lyric {
        placement: Default::default(),
        note: 2,
        verse: 2,
        verse_label: None,
        text: "für & dich".into(),
        syllabic: Syllabic::Begin,
        extend: true,
        bounds: None,
        confirmed: false,
    });
    let mut second = first.clone();
    second.lyrics[0].text = "Ende".into();
    let xml = musicxml::export_pages(&[&first, &second], "Choir")
        .unwrap()
        .xml;
    assert_eq!(xml.matches("<part id=").count(), 2);
    assert_eq!(xml.matches("<part-name>S &amp; A</part-name>").count(), 1);
    assert_eq!(xml.matches("<measure number=\"1\"").count(), 2);
    assert_eq!(xml.matches("<measure number=\"4\"").count(), 2);
    assert!(xml.contains("<clef-octave-change>-1</clef-octave-change>"));
    assert!(xml.contains("<step>E</step><alter>0</alter><octave>3</octave>"));
    assert!(xml.contains(
        "<lyric number=\"2\"><syllabic>begin</syllabic><text>für &amp; dich</text><extend/>"
    ));
    assert_eq!(xml.matches("<text>Ende</text>").count(), 1);
    let restored: Score = serde_json::from_slice(&serde_json::to_vec(&first).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored.lyrics[0].text, "für & dich");
}

#[test]
fn deleting_notes_keeps_lyric_references_and_rejects_duplicates() {
    let mut score = score();
    score.lyrics.push(Lyric {
        placement: Default::default(),
        note: 2,
        verse: 1,
        verse_label: None,
        text: "Wort".into(),
        syllabic: Syllabic::Single,
        extend: false,
        bounds: None,
        confirmed: true,
    });
    score.remove_note(0);
    assert_eq!(score.lyrics[0].note, 1);
    score.validate().unwrap();
    score.lyrics.push(score.lyrics[0].clone());
    assert!(score.validate().is_err());
    score.lyrics.pop();
    score.remove_note(1);
    assert!(score.lyrics.is_empty());
}

#[test]
fn meter_inheritance_stays_in_its_part() {
    let mut score = score();
    score.staves[1].header.time_signature = Some(TimeSignature::Meter {
        beats: 3,
        beat_unit: 4,
    });
    score.staves[2].header.time_signature = None;
    assert_eq!(
        score.time_signature_before(2, 30.0),
        score.staves[0].header.time_signature
    );
}

#[test]
fn bracket_groups_and_lyrics_are_not_inferred_from_text_or_spacing_alone() {
    let mut score = score();
    let mut image = image::GrayImage::from_pixel(400, 700, image::Luma([255]));
    score_structure::assign_parts(&image, &mut score);
    assert!(score.staff_assignments.is_empty());
    for (top, bottom) in [(30, 230), (350, 550)] {
        for y in top..=bottom {
            image.put_pixel(20, y, image::Luma([0]));
        }
    }
    score_structure::assign_parts(&image, &mut score);
    assert_eq!(
        score
            .staff_assignments
            .iter()
            .map(|a| a.part)
            .collect::<Vec<_>>(),
        vec![0, 1, 0, 1]
    );
}

#[test]
fn positioned_text_produces_verses_and_syllables_without_consuming_hyphens() {
    let mut score = score();
    let mut text = Vec::new();
    // Three syllables on three notes, two separate verse baselines.
    let y = score.staves[0].lines[4];
    score.add_note(160.0, y).unwrap();
    for (row, words) in [
        (90.0, ["Hel", "lo", "Welt"]),
        (110.0, ["Lie", "be", "Leut"]),
    ] {
        for (x, word) in [100.0, 160.0, 220.0].into_iter().zip(words) {
            text.push(TextGlyph {
                text: word.into(),
                baseline: row,
                bounds: SymbolBounds {
                    left: x - 10.0,
                    right: x + 10.0,
                    top: row - 10.0,
                    bottom: row,
                },
            });
        }
        text.push(TextGlyph {
            text: "-".into(),
            baseline: row,
            bounds: SymbolBounds {
                left: 125.0,
                right: 130.0,
                top: row - 5.0,
                bottom: row - 3.0,
            },
        });
    }
    score_structure::attach_text(&mut score, text);
    assert_eq!(score.lyrics.len(), 6);
    assert_eq!(score.lyrics[0].syllabic, Syllabic::Begin);
    assert_eq!(score.lyrics[1].syllabic, Syllabic::End);
    assert_eq!(score.lyrics[3].verse, 2);
    score.validate().unwrap();
}

#[test]
fn sparse_and_above_staff_lyrics_stay_separate_from_stanza_blocks() {
    let mut score = score();
    score.add_note(160.0, score.staves[0].lines[4]).unwrap();
    let mut text = Vec::new();
    for (baseline, words) in [
        (
            130.0,
            vec![(100.0, "Come"), (160.0, "back"), (220.0, "Liza")],
        ),
        (80.0, vec![(100.0, "P"), (160.0, "V")]),
        (170.0, vec![(100.0, "du"), (220.0, "ah")]),
        (260.0, vec![(220.0, "oh")]),
        (570.0, vec![(100.0, "end")]),
        (
            610.0,
            vec![(15.0, "2."), (70.0, "When"), (100.0, "I"), (130.0, "go")],
        ),
        (
            630.0,
            vec![(15.0, "3."), (70.0, "When"), (100.0, "I"), (130.0, "see")],
        ),
    ] {
        for (x, word) in words {
            text.push(TextGlyph {
                text: word.into(),
                baseline,
                bounds: SymbolBounds {
                    left: x - 7.0,
                    right: x + 7.0,
                    top: baseline - 8.0,
                    bottom: baseline,
                },
            });
        }
    }
    score_structure::attach_text(&mut score, text);
    let above: Vec<_> = score
        .lyrics
        .iter()
        .filter(|l| l.placement == LyricPlacement::Above)
        .collect();
    assert_eq!(above.len(), 2);
    assert!(above.iter().all(|l| score.notes[l.note].staff == 1));
    for word in ["Come", "back", "Liza"] {
        assert!(score.lyrics.iter().any(|l| l.text == word));
    }
    assert!(
        score
            .lyrics
            .iter()
            .any(|l| l.text == "oh" && l.placement == LyricPlacement::Below)
    );
    assert!(score.lyrics.iter().any(|l| l.text == "end"));
    assert!(
        !score
            .lyrics
            .iter()
            .any(|l| matches!(l.text.as_str(), "When" | "go" | "see" | "P" | "V"))
    );
    score.validate().unwrap();
    let xml = musicxml::export_pages(&[&score], "Lyrics").unwrap().xml;
    assert!(xml.contains("placement=\"above\"><syllabic>single</syllabic><text>du</text>"));
    let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
    assert_eq!(
        restored
            .lyrics
            .iter()
            .filter(|l| l.placement == LyricPlacement::Above)
            .count(),
        2
    );
}

#[test]
fn performance_directions_do_not_create_lyric_verses() {
    let mut score = score();
    score.add_note(160.0, 70.0).unwrap();
    let mut text = Vec::new();
    for (baseline, words) in [
        (12.0, vec!["dolce"]),
        (100.0, vec!["Come", "back", "Liza"]),
        (130.0, vec!["dim.", "e", "rit."]),
    ] {
        for (i, word) in words.into_iter().enumerate() {
            let x = 100.0 + i as f32 * 60.0;
            text.push(TextGlyph {
                text: word.into(),
                baseline,
                bounds: SymbolBounds {
                    left: x - 8.0,
                    right: x + 8.0,
                    top: baseline - 8.0,
                    bottom: baseline,
                },
            });
        }
    }
    score_structure::attach_text(&mut score, text);
    assert_eq!(score.lyrics.len(), 3);
    assert!(
        score
            .lyrics
            .iter()
            .all(|l| l.verse == 1 && l.placement == LyricPlacement::Below)
    );
    assert!(score.source_text.iter().any(|t| t.text == "dolce"));
    score.validate().unwrap();
}

fn text_word(x: f32, baseline: f32, text: &str) -> TextGlyph {
    TextGlyph {
        text: text.into(),
        baseline,
        bounds: SymbolBounds {
            left: x - 8.0,
            right: x + 8.0,
            top: baseline - 8.0,
            bottom: baseline,
        },
    }
}

#[test]
fn ocr_direction_phrases_are_excluded_above_below_and_beside_lyrics() {
    for direction in [
        vec!["marcato"],
        vec!["ALLARGANDO"],
        vec!["poco", "rit."],
        vec!["molto", "ritardando"],
        vec!["a", "tempo"],
        vec!["dim.e.rit."],
        vec!["(sempre", "più", "marcato)"],
    ] {
        let mut score = score();
        score.add_note(160.0, 70.0).unwrap();
        let mut text = vec![
            text_word(100.0, 100.0, "Sing"),
            text_word(160.0, 100.0, "a"),
            text_word(220.0, 100.0, "song"),
        ];
        for baseline in [12.0, 130.0] {
            text.extend(
                direction
                    .iter()
                    .enumerate()
                    .map(|(i, word)| text_word(100.0 + i as f32 * 60.0, baseline, word)),
            );
        }
        let source = text.clone();
        score_structure::attach_text(&mut score, text);
        assert_eq!(
            score
                .lyrics
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            ["Sing", "a", "song"],
            "{direction:?}"
        );
        assert!(score.lyrics.iter().all(|l| l.verse == 1));
        assert_eq!(score.source_text.len(), source.len());
        score.validate().unwrap();
    }
    let mut score = score();
    score.add_note(160.0, 70.0).unwrap();
    score_structure::attach_text(
        &mut score,
        vec![
            text_word(100.0, 100.0, "Hear"),
            text_word(160.0, 100.0, "me"),
            text_word(220.0, 100.0, "marcato"),
        ],
    );
    assert_eq!(score.lyrics.len(), 2);
    assert!(score.lyrics.iter().all(|l| l.text != "marcato"));
}

#[test]
fn publication_footers_and_continuation_lines_are_never_attached_to_notes() {
    for marker in [
        "©",
        "Copyright",
        "Musikverlag",
        "Verlag",
        "Publishing",
        "ISBN",
        "www.example.org",
        "All rights reserved",
        "Alle Rechte vorbehalten",
    ] {
        // The identifying marker sits outside the staff crop, but identifies
        // the entire row and following lines as a publication block.
        let mut score = Score {
            staves: vec![staff(30.0, Clef::Treble)],
            ..Default::default()
        };
        score.add_note(100.0, 70.0).unwrap();
        score.add_note(220.0, 70.0).unwrap();
        let text = vec![
            text_word(100.0, 90.0, "Sing"),
            text_word(220.0, 90.0, "on"),
            text_word(100.0, 110.0, "Früh"),
            text_word(220.0, 110.0, "auf"),
            text_word(340.0, 130.0, marker),
            text_word(100.0, 130.0, "Example"),
            text_word(220.0, 130.0, "Music"),
            text_word(100.0, 150.0, "Berlin"),
            text_word(220.0, 150.0, "2026"),
        ];
        let source_len = text.len();
        score_structure::attach_text(&mut score, text);
        assert_eq!(
            score
                .lyrics
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            ["Sing", "on", "Früh", "auf"],
            "{marker}"
        );
        assert_eq!(score.source_text.len(), source_len);
        score.validate().unwrap();
        let xml = musicxml::export_pages(&[&score], "Footer regression")
            .unwrap()
            .xml;
        assert!(!xml.contains("<text>Example</text>"));
        assert!(!xml.contains("<text>Berlin</text>"));
    }
}

#[test]
fn distant_unmarked_text_does_not_start_a_lyric_verse() {
    let mut score = Score {
        staves: vec![staff(30.0, Clef::Treble)],
        ..Default::default()
    };
    score.add_note(100.0, 70.0).unwrap();
    score_structure::attach_text(&mut score, vec![text_word(100.0, 180.0, "Example")]);
    assert!(score.lyrics.is_empty());
    assert_eq!(score.source_text.len(), 1);
}

#[test]
fn syllable_ten_is_not_mistaken_for_a_tenuto_abbreviation() {
    let mut score = score();
    score.add_note(160.0, 70.0).unwrap();
    score_structure::attach_text(
        &mut score,
        vec![
            text_word(100.0, 100.0, "gu"),
            text_word(130.0, 100.0, "-"),
            text_word(160.0, 100.0, "ten"),
            text_word(220.0, 100.0, "Tag"),
        ],
    );
    assert_eq!(score.lyrics.len(), 3);
    assert_eq!(score.lyrics[0].syllabic, Syllabic::Begin);
    assert_eq!(score.lyrics[1].text, "ten");
    assert_eq!(score.lyrics[1].syllabic, Syllabic::End);
    assert_eq!(score.lyrics[2].text, "Tag");
}

#[test]
fn shifted_short_syllables_share_the_available_notes_without_phantom_verses() {
    let mut score = Score {
        staves: vec![staff(30.0, Clef::Treble)],
        ..Default::default()
    };
    score.add_note(100.0, 70.0).unwrap();
    score.add_note(130.0, 70.0).unwrap();
    let text = [
        (1.0, 60.0, "Refrain:du"),
        (12.0, 117.0, "du"),
        (12.0, 145.0, "ah"),
    ]
    .into_iter()
    .map(|(baseline, x, text)| TextGlyph {
        text: text.into(),
        baseline,
        bounds: SymbolBounds {
            left: x - 7.0,
            right: x + 7.0,
            top: (baseline - 8.0_f32).max(0.0),
            bottom: baseline + 1.0,
        },
    })
    .collect();
    score_structure::attach_text(&mut score, text);
    assert_eq!(score.lyrics.len(), 2);
    assert_eq!(
        (score.lyrics[0].note, score.lyrics[0].text.as_str()),
        (0, "du")
    );
    assert_eq!(
        (score.lyrics[1].note, score.lyrics[1].text.as_str()),
        (1, "ah")
    );
    assert!(
        score
            .lyrics
            .iter()
            .all(|l| l.verse == 1 && l.placement == LyricPlacement::Above)
    );
    score.validate().unwrap();
}

#[test]
fn printed_verse_labels_survive_recognition_projects_and_export() {
    let mut score = score();
    score.add_note(160.0, score.staves[0].lines[4]).unwrap();
    let mut text = Vec::new();
    for (baseline, prefix) in [(90.0, "3."), (110.0, "7)"), (130.0, "")] {
        for (x, word) in [
            (60.0, prefix),
            (100.0, "Ein"),
            (160.0, "neues"),
            (220.0, "Lied"),
        ] {
            if word.is_empty() {
                continue;
            }
            text.push(TextGlyph {
                text: word.into(),
                baseline,
                bounds: SymbolBounds {
                    left: x - 8.0,
                    right: x + 8.0,
                    top: baseline - 8.0,
                    bottom: baseline,
                },
            });
        }
    }
    let note_count = score.notes.len();
    score_structure::attach_text(&mut score, text);
    assert_eq!(score.notes.len(), note_count);
    assert_eq!(score.lyrics.len(), 9);
    for (row, verse, label) in [(0, 3, Some("3.")), (1, 7, Some("7)")), (2, 8, None)] {
        let lyrics = &score.lyrics[row * 3..row * 3 + 3];
        assert!(lyrics.iter().all(|l| l.verse == verse));
        assert_eq!(lyrics[0].verse_label.as_deref(), label);
        assert_eq!(lyrics[0].text, "Ein");
        assert!(lyrics[1..].iter().all(|l| l.verse_label.is_none()));
    }
    score.validate().unwrap();
    let xml = musicxml::export_pages(&[&score], "Verses").unwrap().xml;
    assert!(xml.contains("<lyric number=\"3\"><syllabic>single</syllabic><text>3. Ein</text>"));
    assert!(xml.contains("<text>7) Ein</text>"));
    assert_eq!(xml.matches("<text>3. Ein</text>").count(), 1);
    let mut saved = serde_json::to_value(&score).unwrap();
    let restored: Score = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(restored.lyrics[0].verse_label.as_deref(), Some("3."));
    for lyric in saved["lyrics"].as_array_mut().unwrap() {
        lyric.as_object_mut().unwrap().remove("verse_label");
    }
    let old_project: Score = serde_json::from_value(saved).unwrap();
    assert!(old_project.lyrics.iter().all(|l| l.verse_label.is_none()));
    old_project.validate().unwrap();
    score.lyrics[0].verse_label = Some("\0".into());
    assert!(score.validate().is_err());
}

#[test]
fn pdf_unicode_bounds_follow_raster_rotation() {
    use libreomr::document_import::open_document;
    for rotation in [0, 90] {
        let stream = "BT /F1 12 Tf 40 100 Td (f\\374r Test) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 240 180] /Rotate {rotation} /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            ),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                stream.len(),
                stream
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>"
                .to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0];
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in &offsets[1..] {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text.pdf");
        std::fs::write(&path, pdf).unwrap();
        let pages = open_document(&path).unwrap();
        let text = pages[0].text();
        assert_eq!(
            text.iter()
                .map(|g| g.text.as_str())
                .collect::<String>()
                .replace(' ', ""),
            "fürTest"
        );
        let image = pages[0].render().unwrap();
        for glyph in text.iter().filter(|g| !g.text.trim().is_empty()) {
            let ink = (glyph.bounds.top.max(0.0) as u32
                ..glyph.bounds.bottom.ceil().min(image.height() as f32) as u32)
                .any(|y| {
                    (glyph.bounds.left.max(0.0) as u32
                        ..glyph.bounds.right.ceil().min(image.width() as f32) as u32)
                        .any(|x| image.get_pixel(x, y)[0] < 170)
                });
            assert!(ink, "{rotation}: {}", glyph.text);
        }
    }
}
