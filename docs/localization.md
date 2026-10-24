# Localization and settings

The desktop app supports English and German. The gear button opens Settings,
where language and theme can be changed independently without restarting.
On first launch both follow the system. German system locales (`de`, `de-DE`,
`de_AT`, `de-CH`, and equivalent variants) use German; all other locales use English.
The system locale is obtained through `sys-locale`, including the native Windows
and macOS locale APIs rather than relying on `LANG` on those platforms.

## Language files

`locales/en.json` and `locales/de.json` are UTF-8 JSON dictionaries. Keys are
English source messages shared by every language. Values contain the translation:

```json
{
  "Settings": "Einstellungen",
  "Page {0}": "Seite {0}"
}
```

Use `libreomr::i18n::tr("Settings")` for fixed text and
`libreomr::i18n::format("Page {0}", &[("0", page.to_string())])` for messages
with values. Named placeholders such as `{error}` are supported and can be
reordered. Values are inserted once, so braces in filenames and error details
remain literal. Format numbers before passing them to the translator.

Catalogs are embedded at compile time with `include_str!`; installed packages
work without additional resource files. Edit the JSON files and rebuild to
update translations. Missing translations fall back to the English catalog,
then to the English message key. Project serialization and MusicXML element
names remain independent of the display language.

To add a language, copy `en.json`, translate its values while preserving every
key and placeholder, then register the catalog and locale mapping in `src/i18n.rs`
and the preference and native language name in `src/settings.rs`. Use the
language's own name in the selector so users can recover from an accidental choice.

Recognition progress carries stable English stage IDs and is translated when
displayed. Export diagnostics are regenerated when the language changes. Native
file dialogs and system-provided buttons use the operating system's language.
Existing page names and already reported import errors remain document data.

## Persistence

The preferences are stored outside projects in a per-user `libreOMR/settings.json`:

| Platform | Base directory |
| --- | --- |
| Windows | `%APPDATA%` |
| macOS | `~/Library/Application Support` |
| Linux | `$XDG_CONFIG_HOME`, falling back to `~/.config` |

```json
{
  "language": "system",
  "theme": "system"
}
```

Language values are `system`, `english` and `german`; theme values are `system`,
`light` and `dark`. Missing fields use defaults. An absent or invalid file uses
system defaults. Saving creates the directory and atomically replaces the file
through a temporary file in the same directory. A save error is shown in the
status bar; the selected preferences still apply to the current session.

## Verification

```sh
cargo test --profile ci --locked i18n
cargo test --profile ci --locked settings
cargo test --profile ci --locked theme
cargo test --profile ci --locked
```

Catalog tests check matching keys and placeholders, locale variants, fallbacks,
placeholder reordering and literal user data. Settings tests cover defaults,
explicit overrides, repeated saves, missing/invalid files and partial settings.
The existing editor, project, import and export regression tests remain applicable.
For native captures and manual language checks, see the
[screenshot instructions](../assets/screenshots/README.md).

For interactive acceptance on each target platform:

| Action | Expected result |
| --- | --- |
| Start without a settings file on German and English systems | Language follows the system; theme follows system appearance. |
| Select English, then Deutsch | Menus, inspector, musical labels and refreshed export diagnostics change immediately. |
| Select light or dark appearance, then restart | Both language and appearance choices are restored. |
| Select Follow system and change system appearance | The open application follows light/dark changes. |
| Open Settings at 760 × 520 and 1280 × 820 | The gear and all controls remain visible and usable. |
| Change preferences while reviewing an unsaved score | Notes, corrections, undo history and project save state are preserved. |
| Save and export the same score in both languages | Project data and MusicXML notation are identical; diagnostic text is translated. |
