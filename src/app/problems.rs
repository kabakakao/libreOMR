use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Filter {
    #[default]
    Problems,
    Review,
    All,
}

impl Filter {
    fn includes(self, issue: &Issue) -> bool {
        match self {
            Self::Problems => issue.severity != Severity::Review,
            Self::Review => issue.severity == Severity::Review,
            Self::All => true,
        }
    }
}

#[derive(Default)]
pub(super) struct Problems {
    cache: Option<Vec<Issue>>,
    pub(super) show: bool,
    filter: Filter,
    cursor: Option<(u64, Issue)>,
    reveal_row: bool,
    pub(super) open_system: bool,
}

impl Problems {
    pub(super) fn needs_refresh(&self) -> bool {
        self.cache.is_none()
    }

    pub(super) fn reset_document(&mut self) {
        self.invalidate();
        self.cursor = None;
        self.open_system = false;
    }

    pub(super) fn invalidate(&mut self) {
        self.cache = None;
    }
}

impl App {
    pub(super) fn refresh_problems(&mut self) {
        if self.problems.cache.is_some() || self.pending.is_some() {
            return;
        }
        let ready: Vec<_> = (0..self.pages.len())
            .filter(|&page| self.page_at(page).import.ready())
            .collect();
        let scores: Vec<_> = ready
            .iter()
            .map(|&page| &self.page_at(page).score)
            .collect();
        let mut issues = diagnostics::inspect_pages(&scores);
        for issue in &mut issues {
            issue.page = ready[issue.page];
        }
        for page in 0..self.pages.len() {
            if let ImportState::Failed(error) = &self.page_at(page).import {
                issues.push(Issue {
                    page,
                    staff: None,
                    object: None,
                    measure: None,
                    x: None,
                    severity: Severity::Error,
                    message: i18n::format(
                        "Page import failed: {error}",
                        &[("error", error.to_string())],
                    ),
                    help: i18n::tr(
                        "Import the page again or remove it; export requires all pages.",
                    ),
                });
            }
        }
        issues.sort_by_key(|issue| issue.page);
        self.problems.cache = Some(issues);
    }

    pub(super) fn show_export_problems(&mut self) {
        self.refresh_problems();
        self.problems.show = true;
        self.problems.filter = if self
            .problems
            .cache
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|issue| issue.severity != Severity::Review)
        {
            Filter::Problems
        } else {
            Filter::Review
        };
    }

    fn visible_problems(&self) -> Vec<usize> {
        self.problems
            .cache
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .filter(|(_, issue)| self.problems.filter.includes(issue))
            .map(|(index, _)| index)
            .collect()
    }

    fn is_current_problem(&self, issue: &Issue) -> bool {
        self.problems
            .cursor
            .as_ref()
            .is_some_and(|(page_id, current)| {
                self.page_at(issue.page).id == *page_id
                    && issue.staff == current.staff
                    && issue.object == current.object
                    // Measure numbers and message details can change when pages
                    // move or a duration is edited; the correction rule stays stable.
                    && issue.measure.is_some() == current.measure.is_some()
                    && issue.x == current.x
                    && issue.help == current.help
                    && issue.severity == current.severity
            })
    }

    pub(super) fn select_problem(&mut self, index: usize) {
        let Some(issue) = self
            .problems
            .cache
            .as_ref()
            .and_then(|issues| issues.get(index))
            .cloned()
        else {
            return;
        };
        if self.pending.is_some() {
            return;
        }
        self.switch_page(issue.page);
        self.page.canvas_context = None;
        self.page.object_drag = None;
        // A measure opens its first rhythmic object, so duration shortcuts and
        // the inspector can be used immediately. An empty measure opens its bar.
        let object = issue.object.or_else(|| {
            issue.measure.and_then(|_| {
                let staff = issue.staff?;
                let left = issue.x?;
                let right = self
                    .page
                    .score
                    .barlines
                    .iter()
                    .filter(|bar| bar.staff == staff && bar.x > left)
                    .map(|bar| bar.x)
                    .min_by(f32::total_cmp)
                    .unwrap_or(f32::INFINITY);
                self.page
                    .score
                    .notes
                    .iter()
                    .enumerate()
                    .filter(|(_, note)| note.staff == staff && note.x >= left && note.x < right)
                    .map(|(index, note)| (note.x, Object::Note(index)))
                    .chain(
                        self.page
                            .score
                            .rests
                            .iter()
                            .enumerate()
                            .filter(|(_, rest)| {
                                rest.staff == staff && rest.x >= left && rest.x < right
                            })
                            .map(|(index, rest)| (rest.x, Object::Rest(index))),
                    )
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, object)| object)
                    .or_else(|| {
                        self.page
                            .score
                            .barlines
                            .iter()
                            .position(|bar| bar.staff == staff && bar.x == right)
                            .map(Object::Barline)
                    })
            })
        });
        self.select_object(object);
        if object.is_none() {
            self.page.reveal_x = issue.x;
        }
        if let Some(staff) = issue.staff {
            self.page.selected_staff = staff;
        }
        self.page.show_object_list = false;
        self.page.reveal_selection = true;
        self.problems.open_system =
            issue.staff.is_some() && issue.object.is_none() && issue.measure.is_none();
        self.problems.cursor = Some((self.page.id, issue));
        self.problems.reveal_row = true;
    }

    pub(super) fn next_problem(&mut self, backwards: bool) {
        self.refresh_problems();
        let visible = self.visible_problems();
        if visible.is_empty() || self.pending.is_some() {
            return;
        }
        let issues = self.problems.cache.as_ref().unwrap();
        let position = visible
            .iter()
            .position(|&index| self.is_current_problem(&issues[index]));
        let next = if let Some(position) = position {
            if backwards {
                (position + visible.len() - 1) % visible.len()
            } else {
                (position + 1) % visible.len()
            }
        } else if let Some((page_id, current)) = &self.problems.cursor {
            // The corrected finding disappeared: continue at that location,
            // without skipping the row that moved into its former position.
            let page = (0..self.pages.len())
                .find(|&page| self.page_at(page).id == *page_id)
                .unwrap_or(0);
            let location = |issue: &Issue| {
                (
                    issue.page,
                    issue.staff.map_or(f32::NEG_INFINITY, |staff| {
                        self.page_at(issue.page).score.staves[staff].lines[0]
                    }),
                    issue.x.unwrap_or(0.0),
                )
            };
            let anchor = (
                page,
                current
                    .staff
                    .and_then(|staff| self.page_at(page).score.staves.get(staff))
                    .map_or(f32::NEG_INFINITY, |staff| staff.lines[0]),
                current.x.unwrap_or(0.0),
            );
            if backwards {
                visible
                    .iter()
                    .rposition(|&index| location(&issues[index]) < anchor)
                    .unwrap_or(visible.len() - 1)
            } else {
                visible
                    .iter()
                    .position(|&index| location(&issues[index]) >= anchor)
                    .unwrap_or(0)
            }
        } else if backwards {
            visible.len() - 1
        } else {
            0
        };
        self.select_problem(visible[next]);
    }

    pub(super) fn problem_status(&mut self, context: &egui::Context) {
        self.refresh_problems();
        let issues = self.problems.cache.as_deref().unwrap_or_default();
        let errors = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .count();
        let warnings = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Warning)
            .count();
        let review = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Review)
            .count();
        let complete = self.document_ready();
        egui::TopBottomPanel::bottom("status").show(context, |ui| {
            ui.horizontal(|ui| {
                let text = if errors + warnings > 0 {
                    i18n::format("{0}  {1} problems", &[("0", icons::WARNING.to_string()), ("1", format!("{}", errors + warnings))])
                } else if complete { i18n::format("{0}  No export problems", &[("0", icons::CHECK_CIRCLE.to_string())]) }
                else { i18n::tr("Export check pending").into() };
                let color = if errors > 0 { ui.visuals().error_fg_color }
                    else if warnings > 0 { ui.visuals().warn_fg_color } else { ui.visuals().text_color() };
                if ui.add_enabled(self.pending.is_none(), egui::Button::new(egui::RichText::new(text).color(color))
                    .selected(self.problems.show && self.problems.filter == Filter::Problems))
                    .on_hover_text(i18n::format("{errors} export errors, {warnings} warnings. Click to open the affected locations and correction tips.{0}", &[("errors", format!("{}", errors)), ("warnings", format!("{}", warnings)), ("0", (if complete { "" } else { i18n::tr(" More pages are still being processed.") }).to_string())])).clicked()
                {
                    self.problems.show = !self.problems.show || self.problems.filter != Filter::Problems;
                    self.problems.filter = Filter::Problems;
                }
                if ui.add_enabled(self.pending.is_none(), egui::Button::new(i18n::format("{review} unreviewed", &[("review", format!("{}", review))]))
                    .selected(self.problems.show && self.problems.filter == Filter::Review)).clicked()
                {
                    self.problems.show = !self.problems.show || self.problems.filter != Filter::Review;
                    self.problems.filter = Filter::Review;
                }
                if self.pending.is_some() || self.running_import.is_some() || self.discovery.is_some() { ui.spinner(); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(if self.dirty { i18n::tr("Unsaved") } else if self.project_path.is_some() { i18n::tr("Saved") } else { i18n::tr("Demo") });
                    ui.add(egui::Label::new(egui::RichText::new(&self.status).color(if self.status_error {
                        ui.visuals().error_fg_color
                    } else { ui.visuals().weak_text_color() })).truncate()).on_hover_text(&self.status);
                });
            });
        });
    }

    pub(super) fn problem_panel(&mut self, context: &egui::Context) {
        if !self.problems.show {
            return;
        }
        self.refresh_problems();
        egui::TopBottomPanel::bottom("export_problems").resizable(true)
            .default_height(180.0).min_height(120.0)
            .max_height((context.content_rect().height() * 0.35).max(120.0))
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(i18n::tr("Export check"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(icons::X).on_hover_text(i18n::tr("Close problem list")).clicked() { self.problems.show = false; }
                        let enabled = !self.visible_problems().is_empty() && self.pending.is_none();
                        if ui.add_enabled(enabled, egui::Button::new(icons::ARROW_RIGHT)).on_hover_text(i18n::tr("Next location")).clicked() { self.next_problem(false); }
                        if ui.add_enabled(enabled, egui::Button::new(icons::ARROW_LEFT)).on_hover_text(i18n::tr("Previous location")).clicked() { self.next_problem(true); }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    let issues = self.problems.cache.as_deref().unwrap_or_default();
                    let problems = issues.iter().filter(|issue| issue.severity != Severity::Review).count();
                    let review = issues.len() - problems;
                    ui.selectable_value(&mut self.problems.filter, Filter::Problems, i18n::format("Problems ({problems})", &[("problems", format!("{}", problems))]));
                    ui.selectable_value(&mut self.problems.filter, Filter::Review, i18n::format("Unreviewed ({review})", &[("review", format!("{}", review))]));
                    ui.selectable_value(&mut self.problems.filter, Filter::All, i18n::tr("All"));
                });
                let visible = self.visible_problems();
                let current = self.problems.cache.as_ref().and_then(|issues| issues.iter().find(|issue| self.is_current_problem(issue)));
                let help = current.map(|issue| issue.help).unwrap_or(if self.problems.cursor.is_some() {
                    i18n::tr("Location fixed or changed. Use the arrow to move to the next location.")
                } else { i18n::tr("Click a location; make corrections on the right or directly in the scan.") });
                ui.add(egui::Label::new(help).truncate()).on_hover_text(help);
                if !self.document_ready() { ui.weak(i18n::tr("Partial check: more pages are pending; export is available after all analyses finish.")); }
                ui.separator();
                if visible.is_empty() {
                    ui.label(if self.problems.filter == Filter::Review { i18n::tr("No unreviewed symbols.") }
                        else if self.document_ready() { i18n::tr("No locations match this filter.") }
                        else { i18n::tr("No locations yet. The check runs after recognition.") });
                }
                let mut selected = None;
                let reveal = std::mem::take(&mut self.problems.reveal_row);
                // Virtual rows keep long scans and documents responsive.
                let mut scroll = egui::ScrollArea::vertical().id_salt("export_problem_rows").auto_shrink([false, false]);
                if reveal && let Some(position) = visible.iter().position(|&index| self.is_current_problem(&self.problems.cache.as_ref().unwrap()[index])) {
                    scroll = scroll.vertical_scroll_offset(position as f32 * (30.0 + ui.spacing().item_spacing.y));
                }
                scroll.show_rows(ui, 30.0, visible.len(), |ui, range| {
                        for position in range {
                            let index = visible[position];
                            let issue = &self.problems.cache.as_ref().unwrap()[index];
                            let mut location = i18n::format("Page {0}", &[("0", format!("{}", issue.page + 1))]);
                            if let Some(staff) = issue.staff {
                                let score = &self.page_at(issue.page).score;
                                let row = score.staves.iter().filter(|other| other.lines[0] < score.staves[staff].lines[0]).count() + 1;
                                location.push_str(&i18n::format(" · Staff {row}", &[("row", format!("{}", row))]));
                            }
                            if let Some(measure) = issue.measure { location.push_str(&i18n::format(" · Measure {measure}", &[("measure", format!("{}", measure))])); }
                            let (icon, color) = match issue.severity {
                                Severity::Error => (icons::X_CIRCLE, ui.visuals().error_fg_color),
                                Severity::Warning => (icons::WARNING, ui.visuals().warn_fg_color),
                                Severity::Review => (icons::CIRCLE, ui.visuals().weak_text_color()),
                            };
                            let message = issue.measure.and_then(|measure| issue.message.strip_prefix(&i18n::format("Measure {measure}: ", &[("measure", format!("{}", measure))]))).unwrap_or(&issue.message);
                            let current = self.is_current_problem(issue);
                            let response = ui.add_enabled(self.pending.is_none(),
                                egui::Button::new(egui::RichText::new(format!("{icon}  {location}: {message}")).color(color))
                                    .selected(current).wrap_mode(egui::TextWrapMode::Truncate)
                                    .min_size(Vec2::new(ui.available_width(), 30.0)))
                                .on_hover_text(format!("{location} · {}\n{}\n{}", self.page_at(issue.page).name, issue.message, issue.help));
                            if current && reveal { response.scroll_to_me(Some(egui::Align::Center)); }
                            if response.clicked() { selected = Some(index); }
                        }
                    });
                if let Some(index) = selected { self.select_problem(index); }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = crate::app::tests::demo_app();
        app.page.score = Score {
            staves: vec![Staff {
                left: 30.0,
                right: 900.0,
                lines: [100.0, 112.0, 124.0, 136.0, 148.0],
                header: libreomr::model::StaffHeader {
                    clef: Clef::Treble,
                    key_signature: Some(KeySignature { fifths: 0 }),
                    time_signature: Some(TimeSignature::Common),
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        for x in [100.0, 600.0] {
            app.page.score.add_note(x, 124.0).unwrap();
        }
        for x in [450.0, 850.0] {
            app.page.score.barlines.push(Barline {
                staff: 0,
                x,
                kind: BarlineKind::Single,
                confirmed: true,
            });
        }
        app.problems.invalidate();
        app
    }

    fn render(
        app: &mut App,
        context: &egui::Context,
        size: Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |context| {
                app.toolbar(context);
                app.page_tabs(context);
                app.problem_status(context);
                app.inspector(context);
                app.problems.open_system = false;
                app.problem_panel(context);
                app.preview(context);
                app.canvas(context);
            },
        )
    }

    fn click(app: &mut App, context: &egui::Context, size: Vec2, needle: &str) {
        let output = render(app, context, size, vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                    Some(shape.shape.visual_bounding_rect().center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing clickable text: {needle}"));
        assert!(Rect::from_min_size(Pos2::ZERO, size).contains(position));
        for pressed in [true, false] {
            render(
                app,
                context,
                size,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn status_opens_findings_and_clicks_select_editors_at_both_window_sizes() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let mut app = app();
            let context = egui::Context::default();
            App::configure_style(&context);
            for _ in 0..3 {
                render(&mut app, &context, size, vec![]);
            }
            click(&mut app, &context, size, "2 Probleme");
            assert!(app.problems.show);
            for _ in 0..3 {
                render(&mut app, &context, size, vec![]);
            }
            click(&mut app, &context, size, "Seite 1 · System 1 · Takt 1:");
            assert_eq!(app.page.selected_object, Some(Object::Note(0)));
            assert!(!app.page.show_object_list);
            app.apply_edit_shortcut(EditShortcut::Duration(Duration::Whole));
            render(&mut app, &context, size, vec![]);
            assert_eq!(app.visible_problems().len(), 1);
            app.undo();
            render(&mut app, &context, size, vec![]);
            assert_eq!(app.visible_problems().len(), 2);
            app.redo();
            render(&mut app, &context, size, vec![]);
            assert_eq!(app.visible_problems().len(), 1);
            app.next_problem(false);
            assert_eq!(app.page.selected_object, Some(Object::Note(1)));
        }
    }

    #[test]
    fn system_findings_expand_the_header_and_review_is_separate() {
        let mut app = app();
        app.page.score.staves[0].header.clef = Clef::Unknown;
        app.page.score.notes[0].confirmed = false;
        app.refresh_problems();
        let issues = app.problems.cache.as_ref().unwrap();
        assert_eq!(
            issues
                .iter()
                .filter(|issue| issue.severity == Severity::Review)
                .count(),
            1
        );
        let header = issues
            .iter()
            .position(|issue| issue.object.is_none() && issue.measure.is_none())
            .unwrap();
        app.select_problem(header);
        assert!(app.problems.open_system);
        assert_eq!(app.page.reveal_x, Some(30.0));
        assert_eq!(app.page.selected_object, None);
        let context = egui::Context::default();
        App::configure_style(&context);
        let output = render(&mut app, &context, Vec2::new(760.0, 520.0), vec![]);
        let expected = i18n::tr("Pitch reference: treble (assumed)");
        assert!(output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) => text.galley.text().contains(expected),
            _ => false,
        }));
        app.problems.filter = Filter::Review;
        app.next_problem(false);
        assert_eq!(app.page.selected_object, Some(Object::Note(0)));
        app.snapshot();
        app.page.score.notes[0].confirmed = true;
        app.refresh_problems();
        assert!(app.visible_problems().is_empty());
        app.problems.filter = Filter::Problems;
        assert_eq!(
            app.visible_problems().len(),
            3,
            "confirming must not hide structural warnings"
        );
    }

    #[test]
    fn navigation_follows_pages_after_reordering_and_recovers_from_deletion() {
        let mut app = app();
        let score = app.page.score.clone();
        app.apply_loaded(
            Loaded {
                pages: ["one", "two"]
                    .into_iter()
                    .map(|name| LoadedPage {
                        image: recognition::demo_image(),
                        score: score.clone(),
                        name: name.into(),
                    })
                    .collect(),
                path: None,
            },
            JobMode::ReplaceDocument,
        );
        app.refresh_problems();
        let second = app
            .problems
            .cache
            .as_ref()
            .unwrap()
            .iter()
            .position(|issue| issue.page == 1)
            .unwrap();
        app.select_problem(second);
        assert_eq!(app.active_page, 1);
        assert_eq!(app.page.selected_object, Some(Object::Note(0)));
        let id = app.page.id;
        app.move_page(1, 0);
        app.refresh_problems();
        app.next_problem(false);
        assert_eq!(app.page.id, id);
        assert_eq!(app.active_page, 0);
        assert_eq!(app.page.selected_object, Some(Object::Note(1)));
        app.delete_object(Object::Note(1));
        app.refresh_problems();
        app.next_problem(false);
        assert!(app.page.selected_object.is_some());
        assert_eq!(app.active_page, 1);
    }
}
