//! `bach styles` and `bach layouts`: the built-in styles and layouts of the
//! installer window, what each looks like and what kind of app it suits,
//! so picking one for `installer.style` or `[layout] preset` takes a glance.

use anyhow::Result;
use bach_pack::manifest::Theme;
use bach_pack::style::{Palette, PRESETS};
use serde_json::{json, Value};

use crate::output;

/// A palette as `{ "background": "#rrggbb", ... }`, with the colours it sets (the rest the window derives).
fn palette(palette: &Palette) -> Value {
    let mut colours = serde_json::Map::new();
    for (name, colour) in [
        ("background", palette.background),
        ("background_end", palette.background_end),
        ("card", palette.card),
        ("text", palette.text),
        ("muted_text", palette.muted_text),
        ("accent", palette.accent),
        ("accent_text", palette.accent_text),
        ("accent_hover", palette.accent_hover),
        ("border", palette.border),
        ("error", palette.error),
        ("mark", palette.mark),
        ("panel", palette.panel),
        ("panel_text", palette.panel_text),
    ] {
        if let Some(colour) = colour {
            let alpha = colour.0 & 0xff;
            let text =
                if alpha == 0xff { format!("#{:06x}", colour.rgb_value()) } else { format!("#{:06x}{alpha:02x}", colour.rgb_value()) };
            colours.insert(name.to_string(), Value::String(text));
        }
    }
    Value::Object(colours)
}

pub fn command() -> Result<()> {
    let list: Vec<serde_json::Value> = PRESETS
        .iter()
        .map(|preset| {
            json!({
                "name": preset.name,
                "description": preset.description,
                "suits": preset.suits,
                "theme": match preset.theme { Some(Theme::Dark) => "dark", Some(Theme::Light) => "light", _ => "system" },
                "fonts": preset.fonts,
                "heading_fonts": preset.heading_fonts,
                "heading_weight": preset.heading_weight,
                "accent_mark": preset.accent_mark,
                "radius": preset.radius,
                "control_radius": preset.control_radius,
                "light": palette(&preset.light),
                "dark": palette(&preset.dark),
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

pub fn layouts() -> Result<()> {
    use bach_pack::style::layout::LAYOUT_PRESETS;
    let list: Vec<serde_json::Value> = LAYOUT_PRESETS
        .iter()
        .map(|preset| {
            // What the preset sets, as `[layout]` writes it: the designer on usebach.app draws the window from it.
            let settings = bach_pack::style::layout::preset(preset.name).and_then(|(_, layout)| serde_json::to_value(layout).ok());
            json!({
                "name": preset.name,
                "description": preset.description,
                "suits": preset.suits,
                "picture": preset.picture,
                "settings": settings,
            })
        })
        .collect();
    output::result("layouts", list);
    say!("{} built-in layouts for [layout] preset:", LAYOUT_PRESETS.len());
    for preset in &LAYOUT_PRESETS {
        let picture = match preset.picture {
            Some(key) => format!(" Best with a picture ({key})."),
            None => String::new(),
        };
        say!("");
        say!("  {:<10} {}", preset.name, preset.description);
        say!("  {:<10} For {}.{picture}", "", preset.suits);
    }
    say!("");
    say!("Try one without changing bach.toml: bach preview --layout showcase");
    say!("Start from one and change any setting: [layout] preset = \"showcase\" (see docs/config.md).");
    Ok(())
}
