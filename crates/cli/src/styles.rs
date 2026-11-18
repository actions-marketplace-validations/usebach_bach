//! `bach styles`: the built-in styles of the installer window, what each
//! looks like and what kind of app it suits, so picking one for
//! `installer.style` takes a glance.

use anyhow::Result;
use bach_pack::manifest::Theme;
use bach_pack::style::PRESETS;
use serde_json::json;

use crate::output;

pub fn command() -> Result<()> {
    let list: Vec<serde_json::Value> = PRESETS
        .iter()
        .map(|preset| {
            json!({
                "name": preset.name,
                "description": preset.description,
                "suits": preset.suits,
                "theme": match preset.theme { Some(Theme::Dark) => "dark", Some(Theme::Light) => "light", _ => "system" },
            })
        })
        .collect();
    output::result("styles", list);
    say!("{} built-in styles for installer.style:", PRESETS.len());
    for preset in &PRESETS {
        let dark = if preset.theme == Some(Theme::Dark) { " Dark by default." } else { "" };
        say!("");
        say!("  {:<10} {}", preset.name, preset.description);
        say!("  {:<10} For {}.{dark}", "", preset.suits);
    }
    say!("");
    say!("Try one without changing bach.toml: bach preview --style arcade");
    say!("Change its fonts, colours, corners and background in [style] (see docs/config.md).");
    Ok(())
}
