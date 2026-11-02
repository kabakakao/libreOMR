use super::*;

fn app() -> App {
    let mut app = super::tests::demo_app();
    app.page.image = GrayImage::new(600, 500);
    app.page.score = Score::default();
    for top in [80.0, 280.0] {
        app.page.score.staves.push(Staff {
            lines: [top, top + 20.0, top + 40.0, top + 60.0, top + 80.0],
            left: 20.0,
            right: 580.0,
            header: Default::default(),
        });
    }
    for (x, y) in [(150.0, 130.0), (450.0, 130.0), (150.0, 330.0)] {
        app.page.score.add_note(x, y).unwrap();
    }
    app.select_object(Some(Object::Note(0)));
    app.add_span(SpanKind::Slur);
    app.page.undo.clear();
    app
}

#[test]
fn bow_anchor_snaps_across_rows_and_is_undoable() {
    let mut app = app();
    let original = serde_json::to_value(&app.page.score).unwrap();
    let span = app.page.score.resolved_span(&app.page.score.spans[0]);
    app.snapshot();
    app.page.object_drag = Some(ObjectDrag {
        target: DragTarget::Anchor(0, true),
        origin: Vec2::from(span.end),
        score: app.page.score.clone(),
    });
    app.move_object(Vec2::new(150.0, 330.0));
    assert_eq!(app.page.score.spans[0].end_anchor.as_ref().unwrap().note, 2);
    assert_eq!(
        app.page.score.span_segments(&app.page.score.spans[0]).len(),
        2
    );
    assert!(app.page.score.validate().is_ok());
    assert_eq!(app.page.undo.len(), 1);
    app.page.object_drag = None;
    app.undo();
    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
    app.redo();
    assert_eq!(app.page.score.spans[0].end_anchor.as_ref().unwrap().note, 2);
}

#[test]
fn bow_flip_button_updates_both_note_anchors_in_one_history_entry() {
    let context = egui::Context::default();
    let mut app = app();
    let before = serde_json::to_value(&app.page.score).unwrap();
    let render = |app: &mut App, events| {
        context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 800.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_max_width(260.0);
                    app.span_editor(ui);
                });
            },
        )
    };
    let mut position = None;
    for _ in 0..3 {
        let output = render(&mut app, vec![]);
        position = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == i18n::tr("Flip bow") => {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        });
    }
    let position = position.expect("flip button");
    for pressed in [true, false] {
        render(
            &mut app,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert!(app.page.score.spans[0].height > 0.0);
    for anchor in [
        &app.page.score.spans[0].start_anchor,
        &app.page.score.spans[0].end_anchor,
    ] {
        assert!(anchor.as_ref().unwrap().offset[1] > 0.0);
    }
    assert_eq!(app.page.undo.len(), 1);
    app.undo();
    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
}

#[test]
fn deleting_a_selected_note_and_bow_handles_cascading_removal() {
    let mut app = app();
    app.page.score.spans.push(app.page.score.spans[0].clone());
    let before = serde_json::to_value(&app.page.score).unwrap();
    app.click_object(Some(Object::Note(1)), egui::Modifiers::NONE);
    app.click_object(Some(Object::Span(1)), egui::Modifiers::CTRL);
    app.delete_selection();
    assert!(app.page.score.spans.is_empty());
    assert_eq!(app.page.score.notes.len(), 2);
    app.undo();
    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
}

#[test]
fn two_selected_notes_can_create_one_bow_across_a_system_break() {
    let mut app = app();
    app.page.score.spans.clear();
    app.click_object(Some(Object::Note(0)), egui::Modifiers::NONE);
    app.click_object(Some(Object::Note(2)), egui::Modifiers::CTRL);
    app.add_span(SpanKind::Slur);
    let span = &app.page.score.spans[0];
    assert_eq!(span.start_anchor.as_ref().unwrap().note, 0);
    assert_eq!(span.end_anchor.as_ref().unwrap().note, 2);
    assert!(app.page.score.validate().is_ok());
}

#[test]
fn changing_a_tie_anchor_recomputes_the_held_accidental_and_supports_undo() {
    let mut app = app();
    app.page.score.barlines.push(libreomr::model::Barline {
        staff: 0,
        x: 300.0,
        kind: libreomr::model::BarlineKind::Single,
        confirmed: true,
    });
    app.page.score.notes[0].accidental = -1;
    app.page.score.notes[0].accidental_explicit = true;
    app.page.score.spans[0].kind = SpanKind::Tie;
    app.page.score.recompute_accidentals();
    assert_eq!(app.page.score.notes[1].accidental, -1);
    let before = serde_json::to_value(&app.page.score).unwrap();
    let end = app.page.score.resolved_span(&app.page.score.spans[0]).end;
    app.snapshot();
    app.page.object_drag = Some(ObjectDrag {
        target: DragTarget::Anchor(0, true),
        origin: Vec2::from(end),
        score: app.page.score.clone(),
    });
    app.move_object(Vec2::new(150.0, 330.0));
    assert_eq!(app.page.score.notes[1].accidental, 0);
    // The intervening note makes the moved tie invalid, so it carries no alteration.
    assert_eq!(app.page.score.notes[2].accidental, 0);
    app.page.object_drag = None;
    app.undo();
    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
}

#[test]
fn both_wedge_kinds_support_selected_notes_system_breaks_and_anchor_dragging() {
    for kind in [SpanKind::Crescendo, SpanKind::Decrescendo] {
        let mut app = app();
        app.page.score.spans.clear();
        app.click_object(Some(Object::Note(0)), egui::Modifiers::NONE);
        app.click_object(Some(Object::Note(2)), egui::Modifiers::CTRL);
        app.add_span(kind);
        assert_eq!(
            app.page.score.spans[0].start_anchor.as_ref().unwrap().note,
            0
        );
        assert_eq!(app.page.score.spans[0].end_anchor.as_ref().unwrap().note, 2);
        assert_eq!(
            app.page.score.span_segments(&app.page.score.spans[0]).len(),
            2
        );
        app.page.score.validate().unwrap();
        let before = serde_json::to_value(&app.page.score).unwrap();
        app.page.undo.clear();
        app.snapshot();
        let end = app.page.score.resolved_span(&app.page.score.spans[0]).end;
        app.page.object_drag = Some(ObjectDrag {
            target: DragTarget::Anchor(0, true),
            origin: Vec2::from(end),
            score: app.page.score.clone(),
        });
        app.move_object(Vec2::new(450.0, 130.0));
        assert_eq!(app.page.score.spans[0].end_anchor.as_ref().unwrap().note, 1);
        app.page.score.validate().unwrap();
        app.page.object_drag = None;
        app.undo();
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
    }
}

#[test]
fn moving_a_recognized_wedge_moves_both_printed_halves_vertically() {
    let mut app = app();
    app.page.score.spans.clear();
    app.click_object(Some(Object::Note(0)), egui::Modifiers::NONE);
    app.click_object(Some(Object::Note(2)), egui::Modifiers::CTRL);
    app.add_span(SpanKind::Crescendo);
    let pieces = app.page.score.span_segments(&app.page.score.spans[0]);
    let upper = &app.page.score.staves[0];
    let lower = &app.page.score.staves[1];
    app.page.score.spans[0].continuation = Some(libreomr::model::BowContinuation {
        exit_x: 0.0,
        entry_x: 0.0,
        exit_y: (pieces[0].end[1] - upper.lines[0]) / upper.spacing(),
        entry_y: (pieces[1].start[1] - lower.lines[0]) / lower.spacing(),
        exit_height: pieces[0].wedge_opening(1.0) / upper.spacing(),
        entry_height: pieces[1].wedge_opening(0.0) / lower.spacing(),
    });
    app.page.object_drag = Some(ObjectDrag {
        target: DragTarget::Body(Object::Span(0)),
        origin: Vec2::new(150.0, 200.0),
        score: app.page.score.clone(),
    });
    app.move_object(Vec2::new(150.0, 210.0));
    let moved = app.page.score.span_segments(&app.page.score.spans[0]);
    assert_eq!(moved[0].end[1], pieces[0].end[1] + 10.0);
    assert_eq!(moved[1].start[1], pieces[1].start[1] + 10.0);
    app.page.score.validate().unwrap();
}
