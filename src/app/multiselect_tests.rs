use super::*;

#[test]
fn windows_ctrl_click_and_shift_ranges_keep_and_toggle_selection() {
    let mut app = super::tests::demo_app();
    let objects = app.ordered_objects();
    assert!(objects.len() > 4);
    // Windows integrations can supply ctrl without the logical command flag.
    let ctrl = egui::Modifiers::CTRL;
    assert!(!ctrl.command);
    app.click_object(Some(objects[0]), egui::Modifiers::NONE);
    app.click_object(Some(objects[2]), ctrl);
    assert_eq!(app.selected_objects(), vec![objects[0], objects[2]]);
    app.click_object(Some(objects[0]), ctrl);
    assert_eq!(app.selected_objects(), vec![objects[2]]);
    app.click_object(Some(objects[4]), egui::Modifiers::SHIFT);
    assert_eq!(app.selected_objects(), objects[2..=4]);
    app.click_object(Some(objects[3]), egui::Modifiers::SHIFT);
    assert_eq!(app.selected_objects(), objects[2..=3]);
    app.click_object(None, ctrl);
    assert_eq!(app.selected_objects(), objects[2..=3]);
    app.close_details();
    assert!(app.selected_objects().is_empty());
    assert!(app.page.selection_anchor.is_none());
    app.page.show_object_list = true;
    app.click_object(Some(objects[0]), ctrl);
    app.click_object(Some(objects[2]), ctrl);
    assert!(app.page.show_object_list);
    assert_eq!(app.selected_objects(), vec![objects[0], objects[2]]);
}

#[test]
fn multiselect_voice_and_duration_edits_and_delete_are_single_undo_steps() {
    let mut app = super::tests::demo_app();
    app.page.score.rests.clear();
    let rest = app
        .page
        .score
        .add_rest(350.0, 130.0, Duration::Quarter)
        .unwrap();
    let original = serde_json::to_value(&app.page.score).unwrap();
    app.click_object(Some(Object::Note(0)), egui::Modifiers::NONE);
    app.click_object(Some(Object::Note(1)), egui::Modifiers::CTRL);
    app.click_object(Some(Object::Rest(rest)), egui::Modifiers::CTRL);
    app.assign_voice(2);
    assert_eq!(app.page.undo.len(), 1);
    assert_eq!(app.page.score.notes[0].voice, 2);
    assert_eq!(app.page.score.notes[1].voice, 2);
    assert_eq!(app.page.score.rests[rest].voice, 2);
    app.assign_voice(2);
    assert_eq!(app.page.undo.len(), 1);
    app.apply_edit_shortcut(EditShortcut::Duration(Duration::SixtyFourth));
    assert_eq!(app.page.undo.len(), 2);
    assert_eq!(app.page.score.notes[0].duration, Duration::SixtyFourth);
    assert_eq!(app.page.score.rests[rest].duration, Duration::SixtyFourth);
    let edited = serde_json::to_value(&app.page.score).unwrap();
    let notes = app.page.score.notes.len();
    app.delete_selection();
    assert_eq!(app.page.undo.len(), 3);
    assert_eq!(app.page.score.notes.len(), notes - 2);
    assert!(app.page.score.rests.is_empty());
    app.page.score.validate().unwrap();
    app.undo();
    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), edited);
    assert!(app.selected_objects().is_empty());
    app.undo();
    app.undo();
    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
    app.redo();
    assert_eq!(app.page.score.notes[0].voice, 2);
}

#[test]
fn multiselect_scan_pointer_events_work_at_both_window_sizes() {
    for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
        let mut app = super::tests::demo_app();
        app.page.zoom = 0.6;
        app.page.score.notes.truncate(2);
        app.page.score.notes[0].x = 200.0;
        app.page.score.notes[1].x = 340.0;
        let context = egui::Context::default();
        let render = |app: &mut App, events, modifiers| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    events,
                    modifiers,
                    ..Default::default()
                },
                |context| app.canvas(context),
            )
        };
        for _ in 0..3 {
            render(&mut app, vec![], egui::Modifiers::NONE);
        }
        for (index, modifiers) in [(0, egui::Modifiers::NONE), (1, egui::Modifiers::CTRL)] {
            let note = &app.page.score.notes[index];
            let pos = Pos2::new(16.0, 16.0) + Vec2::new(note.x, note.y) * app.page.zoom;
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        },
                    ],
                    modifiers,
                );
            }
        }
        assert_eq!(
            app.selected_objects(),
            vec![Object::Note(0), Object::Note(1)]
        );
        assert!(app.page.undo.is_empty());
        render(&mut app, vec![], egui::Modifiers::NONE);
    }
}
