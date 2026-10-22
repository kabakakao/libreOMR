use eframe::egui::{self, Color32, Stroke, Theme, ThemePreference, Vec2};
use egui_phosphor::regular as icons;

pub struct Palette {
    pub canvas: Color32,
    pub confirmed: Color32,
    pub confirmed_bg: Color32,
    pub pending: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            canvas: Color32::from_rgb(24, 29, 35),
            confirmed: Color32::from_rgb(105, 218, 162),
            confirmed_bg: Color32::from_rgb(30, 61, 49),
            pending: Color32::from_rgb(237, 183, 99),
        }
    } else {
        Palette {
            canvas: Color32::from_rgb(222, 227, 233),
            confirmed: Color32::from_rgb(0, 120, 75),
            confirmed_bg: Color32::from_rgb(228, 244, 234),
            pending: Color32::from_rgb(185, 115, 30),
        }
    }
}

pub fn configure(context: &egui::Context) {
    // System is the default preference. Use light if the platform supplies no
    // theme; keep both styles ready for live system changes and manual overrides.
    context.options_mut(|options| options.fallback_theme = Theme::Light);
    for theme in [Theme::Light, Theme::Dark] {
        context.style_mut_of(theme, |style| {
            let dark = theme == Theme::Dark;
            style.visuals = if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            let visuals = &mut style.visuals;
            if dark {
                visuals.panel_fill = Color32::from_rgb(31, 37, 44);
                visuals.window_fill = Color32::from_rgb(36, 43, 52);
                visuals.faint_bg_color = Color32::from_rgb(40, 48, 57);
                visuals.extreme_bg_color = Color32::from_rgb(24, 29, 35);
                visuals.override_text_color = Some(Color32::from_rgb(229, 235, 241));
                visuals.selection.bg_fill = Color32::from_rgb(34, 78, 87);
                visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(85, 194, 208));
                visuals.widgets.inactive.bg_fill = Color32::from_rgb(44, 53, 63);
                visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(44, 53, 63);
                visuals.widgets.inactive.bg_stroke =
                    Stroke::new(1.0_f32, Color32::from_rgb(69, 81, 94));
                visuals.error_fg_color = Color32::from_rgb(255, 138, 148);
                visuals.warn_fg_color = palette(true).pending;
            } else {
                visuals.panel_fill = Color32::from_rgb(249, 250, 252);
                visuals.window_fill = Color32::WHITE;
                visuals.faint_bg_color = Color32::from_rgb(237, 240, 244);
                visuals.extreme_bg_color = Color32::WHITE;
                visuals.override_text_color = Some(Color32::from_rgb(36, 43, 52));
                visuals.selection.bg_fill = Color32::from_rgb(210, 234, 238);
                visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(15, 95, 112));
                visuals.widgets.inactive.bg_fill = Color32::from_rgb(235, 239, 243);
                visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(235, 239, 243);
                visuals.widgets.inactive.bg_stroke =
                    Stroke::new(1.0_f32, Color32::from_rgb(210, 217, 225));
                visuals.error_fg_color = Color32::from_rgb(168, 36, 46);
            }
            style.spacing.item_spacing = Vec2::new(8.0, 8.0);
            style.spacing.button_padding = Vec2::new(10.0, 6.0);
            style.spacing.interact_size.y = 30.0;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        });
    }
}

pub fn toggle(ui: &mut egui::Ui) {
    let preference = ui.ctx().options(|options| options.theme_preference);
    // The containing toolbar flows right to left, so emit Auto first.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().button_padding.x = 5.0;
        for (value, label, tooltip) in [
            (
                ThemePreference::System,
                "Auto",
                "Automatisch: System-Theme folgen",
            ),
            (ThemePreference::Dark, icons::MOON, "Dunkles Theme"),
            (ThemePreference::Light, icons::SUN, "Helles Theme"),
        ] {
            if ui
                .selectable_label(preference == value, label)
                .on_hover_text(tooltip)
                .clicked()
            {
                ui.ctx().set_theme(value);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(
        context: &egui::Context,
        system: Theme,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(760.0, 520.0),
                )),
                system_theme: Some(system),
                events,
                ..Default::default()
            },
            |context| {
                egui::TopBottomPanel::top("toolbar").show(context, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), toggle);
                });
            },
        )
    }

    #[test]
    fn automatic_theme_tracks_system_and_manual_modes_override_it() {
        let context = egui::Context::default();
        configure(&context);
        assert_eq!(
            context.options(|options| options.theme_preference),
            ThemePreference::System
        );
        for (preference, system, expected) in [
            (ThemePreference::System, Theme::Light, Theme::Light),
            (ThemePreference::System, Theme::Dark, Theme::Dark),
            (ThemePreference::Light, Theme::Dark, Theme::Light),
            (ThemePreference::Dark, Theme::Light, Theme::Dark),
            (ThemePreference::System, Theme::Light, Theme::Light),
            (ThemePreference::System, Theme::Dark, Theme::Dark),
        ] {
            context.set_theme(preference);
            render(&context, system, vec![]);
            assert_eq!(context.theme(), expected);
            let visuals = &context.style().visuals;
            assert_eq!(visuals.dark_mode, expected == Theme::Dark);
            let foreground = visuals.text_color().r() as i16;
            let background = visuals.window_fill.r() as i16;
            assert!((foreground - background).abs() > 150);
        }
    }

    #[test]
    fn theme_toggle_clicks_select_all_three_modes() {
        let context = egui::Context::default();
        configure(&context);
        for (label, preference, expected) in [
            (icons::MOON, ThemePreference::Dark, Theme::Dark),
            (icons::SUN, ThemePreference::Light, Theme::Light),
            ("Auto", ThemePreference::System, Theme::Dark),
        ] {
            render(&context, Theme::Dark, vec![]);
            let output = render(&context, Theme::Dark, vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .expect(label);
            for pressed in [true, false] {
                render(
                    &context,
                    Theme::Dark,
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
            render(&context, Theme::Dark, vec![]);
            assert_eq!(
                context.options(|options| options.theme_preference),
                preference
            );
            assert_eq!(context.theme(), expected);
        }
    }
}
