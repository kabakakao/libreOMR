//! Embedded language files. English message IDs double as the fallback text.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    German,
}

impl Language {
    pub fn from_locale(locale: &str) -> Self {
        match locale
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "de" => Self::German,
            _ => Self::English,
        }
    }
}

// Call set_language during desktop startup. The library retains its original
// German default for existing command-line consumers until configured.
static LANGUAGE: AtomicU8 = AtomicU8::new(1);
type Catalog = BTreeMap<String, String>;
static ENGLISH: OnceLock<Catalog> = OnceLock::new();
static GERMAN: OnceLock<Catalog> = OnceLock::new();

pub fn set_language(language: Language) {
    LANGUAGE.store(
        if language == Language::German { 1 } else { 0 },
        Ordering::Relaxed,
    );
}

pub fn language() -> Language {
    if LANGUAGE.load(Ordering::Relaxed) == 1 {
        Language::German
    } else {
        Language::English
    }
}

pub fn translate(language: Language, key: &str) -> &str {
    let english = ENGLISH.get_or_init(|| {
        serde_json::from_str(include_str!("../locales/en.json"))
            .expect("valid English language file")
    });
    let catalog = match language {
        Language::English => english,
        Language::German => GERMAN.get_or_init(|| {
            serde_json::from_str(include_str!("../locales/de.json"))
                .expect("valid German language file")
        }),
    };
    catalog
        .get(key)
        .or_else(|| english.get(key))
        .map(String::as_str)
        .unwrap_or(key)
}

pub fn tr(key: &str) -> &str {
    translate(language(), key)
}

/// Substitute named placeholders in one pass: user data containing braces is
/// copied verbatim and can never be interpreted as another placeholder.
pub fn interpolate(template: &str, arguments: &[(&str, String)]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start + 1..].find('}') else {
            rest = &rest[start..];
            break;
        };
        let end = start + 1 + end;
        let name = &rest[start + 1..end];
        if let Some((_, value)) = arguments.iter().find(|(key, _)| *key == name) {
            output.push_str(value);
        } else {
            output.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

pub fn format(key: &str, arguments: &[(&str, String)]) -> String {
    interpolate(tr(key), arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(text: &str) -> Vec<&str> {
        let mut result: Vec<_> = text
            .split('{')
            .skip(1)
            .map(|part| part.split('}').next().unwrap())
            .collect();
        result.sort_unstable();
        result
    }

    #[test]
    fn catalogs_have_identical_keys_and_placeholders() {
        let en: Catalog = serde_json::from_str(include_str!("../locales/en.json")).unwrap();
        let de: Catalog = serde_json::from_str(include_str!("../locales/de.json")).unwrap();
        assert_eq!(en.keys().collect::<Vec<_>>(), de.keys().collect::<Vec<_>>());
        for (key, value) in &en {
            assert!(!value.is_empty());
            assert!(!de[key].is_empty());
            assert_eq!(placeholders(value), placeholders(&de[key]), "{key}");
        }
    }

    #[test]
    fn locale_variants_and_unknown_languages_have_predictable_fallbacks() {
        for locale in ["de", "de-DE", "de_AT.UTF-8", "DE-ch", "de_DE@euro"] {
            assert_eq!(Language::from_locale(locale), Language::German);
        }
        for locale in ["en-US", "fr-FR", "", "C", "POSIX", "deu"] {
            assert_eq!(Language::from_locale(locale), Language::English);
        }
        assert_eq!(translate(Language::German, "Settings"), "Einstellungen");
        assert_eq!(translate(Language::English, "Settings"), "Settings");
        assert_eq!(
            translate(Language::German, "New untranslated message"),
            "New untranslated message"
        );
    }

    #[test]
    fn interpolation_supports_reordering_and_preserves_user_data() {
        assert_eq!(
            interpolate("{b} / {a} / {b}", &[("a", "{b}".into()), ("b", "✓".into())]),
            "✓ / {b} / ✓"
        );
        assert_eq!(
            interpolate("{unknown} {unfinished", &[]),
            "{unknown} {unfinished"
        );
    }
}
