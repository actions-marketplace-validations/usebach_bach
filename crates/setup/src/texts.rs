//! The window's words: the built-in texts in the chosen language, with the
//! developer's own texts from `[text]` on top and the app's name, version
//! and publisher filled in.

use std::collections::HashMap;

use bach_pack::i18n::{self, Key, Language};
use bach_pack::manifest::{Component, Manifest, Platform};

pub struct Texts {
    pub language: &'static Language,
    platform: Platform,
    custom: HashMap<Key, String>,
    values: Vec<(&'static str, String)>,
}

impl Texts {
    /// Texts in the language the user prefers (or `forced`, from `--lang`),
    /// among those the app offers.
    pub fn new(manifest: &Manifest, forced: Option<&str>) -> Self {
        let preferred = match forced {
            Some(code) => vec![code.to_string()],
            None => crate::system::host::ui_languages(),
        };
        let language = i18n::pick(&manifest.locale.languages, &preferred);
        Self::in_language(manifest, language)
    }

    pub fn in_language(manifest: &Manifest, language: &'static Language) -> Self {
        let custom =
            Key::ALL.iter().filter_map(|key| manifest.custom_text(*key, language.code).map(|text| (*key, text.to_string()))).collect();
        let app = &manifest.app;
        let values = vec![
            ("name", app.name.clone()),
            ("version", app.version.clone()),
            ("publisher", app.publisher.clone()),
            ("description", app.description.clone().unwrap_or_default()),
        ];
        Self { language, platform: manifest.platform, custom, values }
    }

    /// The macOS wording of a text that names a Windows place (the Start menu, Settings › Apps), when
    /// this is a macOS setup file. The developer's own text for the Windows key still wins over the built-in one.
    fn mac_variant(key: Key) -> Option<Key> {
        Some(match key {
            Key::start_menu => Key::mac_start_menu,
            Key::start_menu_description => Key::mac_start_menu_description,
            Key::start_with_windows => Key::mac_start_with_windows,
            Key::start_with_windows_description => Key::mac_start_with_windows_description,
            Key::installed_subtitle => Key::mac_installed_subtitle,
            Key::error_register => Key::mac_error_register,
            Key::error_unregister => Key::mac_error_unregister,
            Key::error_system => Key::mac_error_system,
            _ => return None,
        })
    }

    /// The text for `key`, with the app's details filled in.
    pub fn get(&self, key: Key) -> String {
        self.with(key, &[])
    }

    /// The text for `key`, with `extra` placeholders filled in as well.
    pub fn with(&self, key: Key, extra: &[(&str, &str)]) -> String {
        let variant = Some(self.platform).filter(Platform::is_macos).and_then(|_| Self::mac_variant(key));
        let template = self
            .custom
            .get(&key)
            .or_else(|| variant.and_then(|mac| self.custom.get(&mac)))
            .map(String::as_str)
            .unwrap_or_else(|| self.language.strings.get(variant.unwrap_or(key)));
        let mut values: Vec<(&str, &str)> = extra.to_vec();
        values.extend(self.values.iter().map(|(name, value)| (*name, value.as_str())));
        i18n::fill(template, &values)
    }

    /// A component's title or description in this language.
    pub fn component(&self, component: &Component, manifest: &Manifest) -> (String, Option<String>) {
        let code = self.language.code;
        let fallbacks = &manifest.locale.languages;
        let fill = |text: &str| i18n::fill(text, &self.values.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
        let title = i18n::resolve(&component.title, code, fallbacks).map(fill).unwrap_or_else(|| component.id.clone());
        let description = component.description.as_ref().and_then(|text| i18n::resolve(text, code, fallbacks)).map(fill);
        // A component that is downloaded when it is ticked says how much.
        let description = match &component.download {
            Some(download) => {
                let line = self.with(Key::component_download, &[("size", &size(download.size))]);
                Some(match description {
                    Some(text) if !text.trim().is_empty() => format!("{text} \u{b7} {line}"),
                    _ => line,
                })
            }
            None => description,
        };
        (title, description)
    }
}

/// A byte count people can read: "1.5 MB".
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_macos_setup_file_speaks_of_applications_not_the_start_menu() {
        let mut manifest = crate::package::Package::sample().manifest;
        let english = &i18n::LANGUAGES[0];
        manifest.platform = Platform::Windows;
        assert_eq!(Texts::in_language(&manifest, english).get(Key::start_menu), "Start menu");
        manifest.platform = Platform::Macos;
        let texts = Texts::in_language(&manifest, english);
        assert_eq!(texts.get(Key::start_menu), "Applications");
        assert_eq!(texts.get(Key::start_with_windows), "Open at login");
        assert!(texts.get(Key::installed_subtitle).contains("Applications"));
        assert!(!texts.get(Key::error_register).contains("Settings"));
        // A text with no macOS wording is the same everywhere.
        assert_eq!(texts.get(Key::cancel), "Cancel");
        // The developer's own text for the Windows key still applies.
        manifest.locale.text.insert("start_menu".into(), [(i18n::ANY_LANGUAGE.to_string(), "Launchpad".to_string())].into());
        assert_eq!(Texts::in_language(&manifest, english).get(Key::start_menu), "Launchpad");
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(size(3 << 30), "3.0 GB");
    }
}
