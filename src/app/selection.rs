use super::*;

#[derive(Clone, Copy)]
pub(super) enum BatchEdit {
    Voice(u8),
    Duration(Duration),
    Dots(u8),
    Tuplet(Option<Tuplet>),
    Confirmed(bool),
    Small(bool),
    Grace(bool),
    Articulation(Articulation),
    Accidental(i8),
    Delete,
}

fn common<T: Copy + PartialEq>(values: impl IntoIterator<Item = T>) -> Option<T> {
    let mut values = values.into_iter();
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

fn choice<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    current: Option<T>,
    options: impl IntoIterator<Item = (T, String)>,
) -> Option<T> {
    let options: Vec<_> = options.into_iter().collect();
    let text = options
        .iter()
        .find(|(value, _)| Some(*value) == current)
        .map_or(i18n::tr("Mixed"), |(_, label)| label.as_str());
    let mut result = None;
    App::inspector_combo(ui, label)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for (value, label) in options {
                if ui.selectable_label(Some(value) == current, label).clicked() {
                    result = Some(value);
                }
            }
        });
    result
}

fn checkbox(ui: &mut egui::Ui, label: &str, current: Option<bool>) -> Option<bool> {
    let mut value = current.unwrap_or(false);
    ui.add(egui::Checkbox::new(&mut value, label).indeterminate(current.is_none()))
        .changed()
        .then_some(value)
}

impl App {
    /// Plain clicks replace, command clicks toggle, shift clicks select a printed range.
    pub(super) fn select_with_modifiers(
        &mut self,
        object: Option<Object>,
        modifiers: egui::Modifiers,
    ) {
        let Some(object) =
            object.filter(|object| matches!(object, Object::Note(_) | Object::Rest(_)))
        else {
            if !modifiers.shift && !modifiers.command && !modifiers.ctrl {
                self.select_object(object);
            }
            return;
        };
        let anchor = self.page.selection_anchor;
        let mut selection = self.page.selection.clone();
        if modifiers.shift {
            let mut objects = self.ordered_objects();
            objects.retain(|object| matches!(object, Object::Note(_) | Object::Rest(_)));
            // Storage order can differ from the printed order of staves.
            objects.sort_by(|a, b| {
                let position = |object| match object {
                    Object::Note(i) => (
                        self.page.score.staves[self.page.score.notes[i].staff].lines[0],
                        self.page.score.notes[i].x,
                    ),
                    Object::Rest(i) => (
                        self.page.score.staves[self.page.score.rests[i].staff].lines[0],
                        self.page.score.rests[i].x,
                    ),
                    _ => unreachable!(),
                };
                let (ay, ax) = position(*a);
                let (by, bx) = position(*b);
                ay.total_cmp(&by).then(ax.total_cmp(&bx))
            });
            if let Some((start, end)) = anchor
                .and_then(|anchor| objects.iter().position(|item| *item == anchor))
                .zip(objects.iter().position(|item| *item == object))
            {
                if !modifiers.command && !modifiers.ctrl {
                    selection.clear();
                }
                for &item in &objects[start.min(end)..=start.max(end)] {
                    if !selection.contains(&item) {
                        selection.push(item);
                    }
                }
            } else {
                selection = vec![object];
            }
        } else if modifiers.command || modifiers.ctrl {
            if selection.contains(&object) {
                selection.retain(|item| *item != object);
            } else {
                selection.push(object);
            }
        } else {
            self.select_object(Some(object));
            return;
        }
        let primary = if selection.contains(&object) {
            Some(object)
        } else {
            selection.last().copied()
        };
        self.select_object(primary);
        self.page.selection = selection;
        self.page.selection_anchor = if modifiers.shift {
            anchor.or(primary)
        } else {
            primary
        };
    }

    pub(super) fn apply_batch_edit(&mut self, edit: BatchEdit) {
        let mut score = self.page.score.clone();
        for &object in &self.page.selection {
            match object {
                Object::Note(i) => {
                    let Some(note) = score.notes.get_mut(i) else {
                        continue;
                    };
                    match edit {
                        BatchEdit::Voice(value) => note.voice = value,
                        BatchEdit::Duration(value) => note.duration = value,
                        BatchEdit::Dots(value) => note.dots = value,
                        BatchEdit::Tuplet(value) => {
                            note.tuplet = value;
                            note.triplet = false;
                        }
                        BatchEdit::Confirmed(value) => note.confirmed = value,
                        BatchEdit::Small(value) => note.small = value,
                        BatchEdit::Grace(value) => {
                            note.grace = value;
                            if value {
                                note.small = true;
                            }
                        }
                        BatchEdit::Articulation(value) => note.articulation = value,
                        BatchEdit::Accidental(value) => {
                            note.accidental = value;
                            note.accidental_explicit = true;
                        }
                        BatchEdit::Delete => {}
                    }
                }
                Object::Rest(i) => {
                    let Some(rest) = score.rests.get_mut(i) else {
                        continue;
                    };
                    match edit {
                        BatchEdit::Voice(value) => rest.voice = value,
                        BatchEdit::Confirmed(value) => rest.confirmed = value,
                        BatchEdit::Duration(value) if rest.measures.is_none() => {
                            rest.duration = value
                        }
                        BatchEdit::Dots(value) if rest.measures.is_none() => rest.dots = value,
                        BatchEdit::Tuplet(value) if rest.measures.is_none() => rest.tuplet = value,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if matches!(edit, BatchEdit::Delete) {
            let mut index = 0;
            score.notes.retain(|_| {
                let keep = !self.page.selection.contains(&Object::Note(index));
                index += 1;
                keep
            });
            index = 0;
            score.rests.retain(|_| {
                let keep = !self.page.selection.contains(&Object::Rest(index));
                index += 1;
                keep
            });
        }
        score.recompute_accidentals();
        if serde_json::to_value(&score).unwrap() != serde_json::to_value(&self.page.score).unwrap()
        {
            self.snapshot();
            self.page.score = score;
            if matches!(edit, BatchEdit::Delete) {
                self.select_object(None);
            }
        }
    }

    pub(super) fn voice_editor(ui: &mut egui::Ui, voice: &mut u8) -> bool {
        ui.add(
            egui::DragValue::new(voice)
                .range(1..=255)
                .prefix(i18n::tr("Voice: ")),
        )
        .on_hover_text(i18n::tr(
            "Each voice starts at the measure boundary. Add rests in this voice for later entries.",
        ))
        .changed()
    }

    pub(super) fn rest_tuplet_editor(
        ui: &mut egui::Ui,
        current: Option<Option<Tuplet>>,
    ) -> Option<Option<Tuplet>> {
        let mut result = choice(
            ui,
            i18n::tr("Tuplet"),
            current,
            [
                (None, i18n::tr("None").into()),
                (
                    Some(Tuplet {
                        actual: 3,
                        normal: 2,
                    }),
                    i18n::tr("Triplet 3:2").into(),
                ),
                (
                    Some(Tuplet {
                        actual: 5,
                        normal: 4,
                    }),
                    i18n::tr("Quintuplet 5:4").into(),
                ),
                (
                    Some(Tuplet {
                        actual: 7,
                        normal: 4,
                    }),
                    i18n::tr("Septuplet 7:4").into(),
                ),
            ]
            .into_iter()
            .chain(
                current
                    .flatten()
                    .filter(|ratio| {
                        !matches!((ratio.actual, ratio.normal), (3, 2) | (5, 4) | (7, 4))
                    })
                    .map(|ratio| (Some(ratio), format!("{}:{}", ratio.actual, ratio.normal))),
            ),
        );
        if let Some(mut ratio) = current.flatten() {
            ui.horizontal_wrapped(|ui| {
                let a = ui
                    .add(
                        egui::DragValue::new(&mut ratio.actual)
                            .range(2..=64)
                            .prefix(i18n::tr("Notes: ")),
                    )
                    .changed();
                ui.label(i18n::tr("in the time of"));
                let b = ui
                    .add(egui::DragValue::new(&mut ratio.normal).range(1..=64))
                    .changed();
                if a || b {
                    result = Some(Some(ratio));
                }
            });
        }
        result
    }

    pub(super) fn selection_editor(&mut self, ui: &mut egui::Ui) {
        ui.heading(i18n::format(
            "{count} objects selected",
            &[("count", self.page.selection.len().to_string())],
        ));
        ui.small(i18n::tr(
            "Ctrl/Cmd-click: toggle selection. Shift-click: select a range.",
        ));
        let notes: Vec<_> = self
            .page
            .selection
            .iter()
            .filter_map(|object| match *object {
                Object::Note(i) => self.page.score.notes.get(i).cloned(),
                _ => None,
            })
            .collect();
        let rests: Vec<_> = self
            .page
            .selection
            .iter()
            .filter_map(|object| match *object {
                Object::Rest(i) => self.page.score.rests.get(i).cloned(),
                _ => None,
            })
            .collect();
        let regular: Vec<_> = rests
            .iter()
            .filter(|rest| rest.measures.is_none())
            .collect();
        let voice = common(
            notes
                .iter()
                .map(|note| note.voice)
                .chain(rests.iter().map(|rest| rest.voice)),
        );
        if let Some(value) = choice(
            ui,
            i18n::tr("Voice"),
            voice,
            (1..=255).map(|voice| {
                (
                    voice,
                    i18n::format("Voice {voice}", &[("voice", voice.to_string())]),
                )
            }),
        ) {
            self.apply_batch_edit(BatchEdit::Voice(value));
        }
        if !notes.is_empty() || !regular.is_empty() {
            if regular.len() != rests.len() {
                ui.small(i18n::tr(
                    "Note value, dots and tuplets apply only to notes and regular rests.",
                ));
            }
            if let Some(value) = choice(
                ui,
                i18n::tr("Note/rest value"),
                common(
                    notes
                        .iter()
                        .map(|note| note.duration)
                        .chain(regular.iter().map(|rest| rest.duration)),
                ),
                Duration::ALL.map(|duration| (duration, duration_shortcut_label(duration))),
            ) {
                self.apply_batch_edit(BatchEdit::Duration(value));
            }
            if let Some(value) = choice(
                ui,
                i18n::tr("Dots"),
                common(
                    notes
                        .iter()
                        .map(|note| note.dots)
                        .chain(regular.iter().map(|rest| rest.dots)),
                ),
                [
                    (0, i18n::tr("No dots").into()),
                    (1, i18n::tr("Single").into()),
                    (2, i18n::tr("Double dots").into()),
                ],
            ) {
                self.apply_batch_edit(BatchEdit::Dots(value));
            }
            if let Some(value) = Self::rest_tuplet_editor(
                ui,
                common(
                    notes
                        .iter()
                        .map(|note| note.tuplet_ratio())
                        .chain(regular.iter().map(|rest| rest.tuplet)),
                ),
            ) {
                self.apply_batch_edit(BatchEdit::Tuplet(value));
            }
        }
        if !notes.is_empty() {
            ui.separator();
            ui.label(i18n::tr("For selected notes"));
            if let Some(value) = checkbox(
                ui,
                i18n::tr("Small noteheads"),
                common(notes.iter().map(|note| note.small)),
            ) {
                self.apply_batch_edit(BatchEdit::Small(value));
            }
            if let Some(value) = checkbox(
                ui,
                i18n::tr("Grace notes"),
                common(notes.iter().map(|note| note.grace)),
            ) {
                self.apply_batch_edit(BatchEdit::Grace(value));
            }
            if let Some(value) = choice(
                ui,
                i18n::tr("Articulation"),
                common(notes.iter().map(|note| note.articulation)),
                Articulation::ALL.map(|value| (value, value.label().into())),
            ) {
                self.apply_batch_edit(BatchEdit::Articulation(value));
            }
        }
        ui.separator();
        if let Some(value) = checkbox(
            ui,
            i18n::tr("Reviewed"),
            common(
                notes
                    .iter()
                    .map(|note| note.confirmed)
                    .chain(rests.iter().map(|rest| rest.confirmed)),
            ),
        ) {
            self.apply_batch_edit(BatchEdit::Confirmed(value));
        }
        if ui.button(i18n::tr("Delete selection")).clicked() {
            self.apply_batch_edit(BatchEdit::Delete);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = super::super::tests::demo_app();
        let staff = app.page.score.staves[0].clone();
        let y = staff.lines[2];
        app.page.score = Score {
            staves: vec![staff],
            ..Default::default()
        };
        app.page.score.add_note(200.0, y);
        app.page.score.add_rest(250.0, y, Duration::Eighth);
        app.page.score.add_note(300.0, y);
        app
    }

    #[test]
    fn range_and_toggle_selection_preserve_score_and_anchor() {
        let mut app = app();
        let before = serde_json::to_value(&app.page.score).unwrap();
        app.select_with_modifiers(Some(Object::Note(0)), egui::Modifiers::NONE);
        app.select_with_modifiers(Some(Object::Note(1)), egui::Modifiers::SHIFT);
        assert_eq!(
            app.page.selection,
            vec![Object::Note(0), Object::Rest(0), Object::Note(1)]
        );
        app.select_with_modifiers(Some(Object::Rest(0)), egui::Modifiers::CTRL);
        assert!(!app.page.selection.contains(&Object::Rest(0)));
        app.select_with_modifiers(Some(Object::Rest(0)), egui::Modifiers::COMMAND);
        assert_eq!(app.page.selection.len(), 3);
        app.select_with_modifiers(Some(Object::Note(0)), egui::Modifiers::NONE);
        app.select_with_modifiers(Some(Object::Note(1)), egui::Modifiers::SHIFT);
        app.select_with_modifiers(Some(Object::Rest(0)), egui::Modifiers::SHIFT);
        assert_eq!(app.page.selection, vec![Object::Note(0), Object::Rest(0)]);
        assert!(app.page.undo.is_empty());
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
        app.select_object(None);
        assert!(app.page.selection.is_empty());
    }

    #[test]
    fn mixed_batch_edits_and_deletion_are_atomic_and_undoable() {
        let mut app = app();
        app.page.score.notes[0].dots = 1;
        app.page.score.notes[1].small = true;
        app.select_with_modifiers(Some(Object::Note(0)), egui::Modifiers::NONE);
        app.select_with_modifiers(Some(Object::Note(1)), egui::Modifiers::SHIFT);
        let original = serde_json::to_value(&app.page.score).unwrap();
        app.apply_batch_edit(BatchEdit::Voice(3));
        assert!(app.page.score.notes.iter().all(|note| note.voice == 3));
        assert_eq!(app.page.score.rests[0].voice, 3);
        assert_eq!(app.page.score.notes[0].dots, 1);
        assert!(app.page.score.notes[1].small);
        assert_eq!(app.page.undo.len(), 1);
        app.apply_batch_edit(BatchEdit::Voice(3));
        assert_eq!(app.page.undo.len(), 1); // No-op does not consume undo history.
        app.undo();
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
        assert!(app.page.selection.is_empty());
        app.redo();
        app.select_with_modifiers(Some(Object::Note(0)), egui::Modifiers::NONE);
        app.select_with_modifiers(Some(Object::Note(1)), egui::Modifiers::SHIFT);
        app.apply_batch_edit(BatchEdit::Tuplet(Some(Tuplet {
            actual: 7,
            normal: 4,
        })));
        assert!(app.page.score.notes.iter().all(|note| note.tuplet_ratio()
            == Some(Tuplet {
                actual: 7,
                normal: 4
            })));
        assert_eq!(
            app.page.score.rests[0].tuplet,
            Some(Tuplet {
                actual: 7,
                normal: 4
            })
        );
        let before_delete = serde_json::to_value(&app.page.score).unwrap();
        app.apply_edit_shortcut(EditShortcut::Delete);
        assert!(app.page.score.notes.is_empty() && app.page.score.rests.is_empty());
        assert!(app.page.selection.is_empty());
        app.undo();
        assert_eq!(
            serde_json::to_value(&app.page.score).unwrap(),
            before_delete
        );
    }

    #[test]
    fn modifier_clicks_in_object_list_reach_mixed_batch_controls() {
        let mut app = app();
        app.page.score.notes[0].voice = 2;
        app.page.score.notes[1].dots = 1;
        app.select_object(Some(Object::Note(0)));
        let context = egui::Context::default();
        App::configure_style(&context);
        context.style_mut(|style| style.animation_time = 0.0);
        let size = Vec2::new(360.0, 1000.0);
        let render = |app: &mut App, events, modifiers| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    events,
                    modifiers,
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        if app.page.show_object_list {
                            app.object_row(ui, Object::Note(0));
                            app.object_row(ui, Object::Rest(0));
                            app.object_row(ui, Object::Note(1));
                        } else {
                            app.selection_editor(ui);
                        }
                    });
                },
            )
        };
        let find = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!(
                        "missing {label}, texts: {:?}",
                        output
                            .shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Text(text) => Some(text.galley.text()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    )
                })
        };
        app.page.show_object_list = true;
        let mut output = render(&mut app, vec![], egui::Modifiers::NONE);
        for _ in 0..2 {
            output = render(&mut app, vec![], egui::Modifiers::NONE);
        }
        let label = format!(
            "{}  {}   {}",
            icons::MUSIC_NOTE,
            app.page.score.notes[1].pitch_label(),
            app.page.score.notes[1].duration_label()
        );
        let target = find(&output, &label);
        for pressed in [true, false] {
            render(
                &mut app,
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::SHIFT,
                    },
                ],
                egui::Modifiers::SHIFT,
            );
        }
        assert_eq!(app.page.selection.len(), 3);
        assert!(app.page.show_object_list);
        app.page.show_object_list = false;
        for _ in 0..3 {
            output = render(&mut app, vec![], egui::Modifiers::NONE);
        }
        // Choose voice 1 explicitly from a mixed selection, without changing other fields.
        let target = find(&output, "Unterschiedlich");
        for pressed in [true, false] {
            output = render(
                &mut app,
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                egui::Modifiers::NONE,
            );
        }
        for _ in 0..3 {
            output = render(&mut app, vec![], egui::Modifiers::NONE);
        }
        let target = find(&output, "Stimme 1");
        for pressed in [true, false] {
            render(
                &mut app,
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                egui::Modifiers::NONE,
            );
        }
        assert!(app.page.score.notes.iter().all(|note| note.voice == 1));
        assert_eq!(app.page.score.rests[0].voice, 1);
        assert_eq!(app.page.score.notes[1].dots, 1);
        assert_eq!(app.page.undo.len(), 1);
        app.undo();
        assert_eq!(app.page.score.notes[0].voice, 2);
    }

    #[test]
    fn mixed_selection_editor_preserves_values_and_fits_compact_panel() {
        for width in [200.0, 360.0] {
            let mut app = app();
            app.page.score.notes[0].voice = 2;
            app.page.score.notes[1].dots = 1;
            app.select_with_modifiers(Some(Object::Note(0)), egui::Modifiers::NONE);
            app.select_with_modifiers(Some(Object::Note(1)), egui::Modifiers::SHIFT);
            let original = serde_json::to_value(&app.page.score).unwrap();
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut output = None;
            for _ in 0..3 {
                output = Some(context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(width, 1000.0),
                        )),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| app.selection_editor(ui));
                    },
                ));
            }
            let output = output.unwrap();
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Unterschiedlich")));
            for shape in output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    let bounds = shape.shape.visual_bounding_rect();
                    assert!(
                        bounds.max.x <= width + 1.0,
                        "{} exceeds {width}: {bounds:?}",
                        text.galley.text()
                    );
                }
            }
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
            assert!(app.page.undo.is_empty());
        }
    }
}
