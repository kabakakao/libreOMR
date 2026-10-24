use eframe::egui::{self, ThemePreference};
use libreomr::i18n::{self, Language};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguagePreference {
    #[default]
    System,
    English,
    German,
}

impl LanguagePreference {
    pub fn resolve(self, locale: Option<&str>) -> Language {
        match self {
            Self::System => Language::from_locale(locale.unwrap_or("en")),
            Self::English => Language::English,
            Self::German => Language::German,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: LanguagePreference,
    pub theme: Theme,
}

pub fn path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|directory| directory.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.filter(|base| base.is_absolute())
        .map(|base| base.join("libreOMR/settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        path()
            .map(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn apply_language(self) {
        i18n::set_language(self.language.resolve(sys_locale::get_locale().as_deref()));
    }

    pub fn apply(self, context: &egui::Context) {
        self.apply_language();
        context.set_theme(match self.theme {
            Theme::System => ThemePreference::System,
            Theme::Light => ThemePreference::Light,
            Theme::Dark => ThemePreference::Dark,
        });
    }

    pub fn save(self) -> Result<(), String> {
        let path = path()
            .ok_or_else(|| i18n::tr("No user configuration directory available").to_owned())?;
        self.save_to(&path).map_err(|error| error.to_string())
    }

    fn save_to(self, path: &Path) -> std::io::Result<()> {
        let directory = path.parent().expect("settings parent directory");
        std::fs::create_dir_all(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(&mut file, &self)?;
        file.write_all(b"\n")?;
        file.as_file_mut().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    /// Returns true only when a preference actually changed.
    pub fn show(&mut self, context: &egui::Context, open: &mut bool) -> bool {
        let previous = *self;
        egui::Window::new(i18n::tr("Settings"))
            .id(egui::Id::new("settings"))
            .default_pos(context.content_rect().center() - egui::vec2(180.0, 120.0))
            .open(open)
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .show(context, |ui| {
                ui.label(i18n::tr("Language"));
                egui::ComboBox::from_id_salt("settings_language")
                    .width(230.0)
                    .selected_text(match self.language {
                        LanguagePreference::System => i18n::tr("System default"),
                        LanguagePreference::English => "English",
                        LanguagePreference::German => "Deutsch",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.language,
                            LanguagePreference::System,
                            i18n::tr("System default"),
                        );
                        ui.selectable_value(
                            &mut self.language,
                            LanguagePreference::English,
                            "English",
                        );
                        ui.selectable_value(
                            &mut self.language,
                            LanguagePreference::German,
                            "Deutsch",
                        );
                    });
                ui.add_space(8.0);
                ui.label(i18n::tr("Theme"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.theme, Theme::System, i18n::tr("Follow system"));
                    ui.selectable_value(&mut self.theme, Theme::Light, i18n::tr("Light"));
                    ui.selectable_value(&mut self.theme, Theme::Dark, i18n::tr("Dark"));
                });
                ui.add_space(8.0);
                ui.weak(i18n::tr("Settings are saved automatically."));
            });
        *self != previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_default_to_system_and_explicit_language_overrides_locale() {
        let settings = Settings::default();
        assert_eq!(settings.theme, Theme::System);
        assert_eq!(settings.language, LanguagePreference::System);
        assert_eq!(settings.language.resolve(Some("de-AT")), Language::German);
        assert_eq!(settings.language.resolve(Some("en-US")), Language::English);
        assert_eq!(settings.language.resolve(Some("ja-JP")), Language::English);
        assert_eq!(settings.language.resolve(None), Language::English);
        assert_eq!(
            LanguagePreference::English.resolve(Some("de-DE")),
            Language::English
        );
        assert_eq!(
            LanguagePreference::German.resolve(Some("en-GB")),
            Language::German
        );
    }

    #[test]
    fn settings_survive_restart_and_repeated_atomic_saves() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new/settings.json");
        assert_eq!(Settings::load_from(&path), Settings::default());
        let settings = Settings {
            language: LanguagePreference::English,
            theme: Theme::Dark,
        };
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), settings);
        Settings::default().save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        std::fs::write(&path, "invalid JSON").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        std::fs::write(&path, r#"{"theme":"light"}"#).unwrap();
        assert_eq!(Settings::load_from(&path).theme, Theme::Light);
        assert_eq!(
            Settings::load_from(&path).language,
            LanguagePreference::System
        );
    }

    #[test]
    fn settings_controls_select_languages_and_all_theme_modes_at_both_sizes() {
        fn render(
            settings: &mut Settings,
            context: &egui::Context,
            size: egui::Vec2,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |context| {
                    settings.show(context, &mut true);
                },
            )
        }
        fn click(settings: &mut Settings, context: &egui::Context, size: egui::Vec2, label: &str) {
            render(settings, context, size, vec![]);
            let output = render(settings, context, size, vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing settings control: {label}"));
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains(position));
            for pressed in [true, false] {
                render(
                    settings,
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
        for size in [egui::vec2(760.0, 520.0), egui::vec2(1280.0, 820.0)] {
            let context = egui::Context::default();
            crate::theme::configure(&context);
            let mut settings = Settings::default();
            for (label, theme) in [
                ("Dark", Theme::Dark),
                ("Light", Theme::Light),
                ("Follow system", Theme::System),
            ] {
                click(&mut settings, &context, size, i18n::tr(label));
                assert_eq!(settings.theme, theme);
            }
            click(&mut settings, &context, size, i18n::tr("System default"));
            click(&mut settings, &context, size, "English");
            assert_eq!(settings.language, LanguagePreference::English);
            click(&mut settings, &context, size, "English");
            click(&mut settings, &context, size, "Deutsch");
            assert_eq!(settings.language, LanguagePreference::German);
            click(&mut settings, &context, size, "Deutsch");
            click(&mut settings, &context, size, i18n::tr("System default"));
            assert_eq!(settings.language, LanguagePreference::System);
        }
    }
}
