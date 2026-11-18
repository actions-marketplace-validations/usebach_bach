//! `installer.style` and `[style]`: which built-in style the window wears,
//! and the developer's own fonts, colours, corners and background on top.
//!
//! Colours can be set for both modes at once (`[style] accent = ...`) or
//! per mode (`[style.dark] accent = ...`), which wins. A font is a family
//! Windows has or a `.ttf`/`.otf` file in the project, which ships in the
//! setup file.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::{existing_file, has_extension, icon::PNG_SIGNATURE};
use crate::manifest::{LookFile, LookFileKind, Theme};
use crate::style::{self, font, Color, Colors, Custom};

/// The largest font file a style may ship.
const MAX_FONT: u64 = 4 << 20;
/// The largest background picture.
const MAX_PICTURE: u64 = 8 << 20;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StyleSection {
    font: Option<toml::Value>,
    heading_font: Option<toml::Value>,
    heading_weight: Option<u16>,
    radius: Option<u8>,
    button_radius: Option<u8>,
    background_image: Option<String>,
    background_image_opacity: Option<f32>,
    background: Option<toml::Value>,
    card: Option<String>,
    text: Option<String>,
    muted_text: Option<String>,
    accent: Option<String>,
    accent_text: Option<String>,
    border: Option<String>,
    error: Option<String>,
    mark: Option<String>,
    light: Option<ColorSection>,
    dark: Option<ColorSection>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColorSection {
    background: Option<toml::Value>,
    card: Option<String>,
    text: Option<String>,
    muted_text: Option<String>,
    accent: Option<String>,
    accent_text: Option<String>,
    border: Option<String>,
    error: Option<String>,
    mark: Option<String>,
}

/// The style as the manifest carries it, and the files it ships.
#[derive(Debug, Default)]
pub(super) struct Styled {
    pub name: Option<String>,
    /// The theme the style looks best in, for when `installer.theme` is unset.
    pub theme: Option<Theme>,
    pub custom: Option<Custom>,
    pub files: Vec<LookFile>,
    pub bytes: Vec<Vec<u8>>,
}

/// Checks `installer.style` and `[style]`. `accent` is `installer.accent`,
/// which may not be set twice.
pub(super) fn style(name: Option<&str>, section: Option<&StyleSection>, accent: Option<&str>, root: &Path) -> Result<Styled> {
    let preset = match name.map(str::trim) {
        None => None,
        Some(name) => Some(style::preset(name).with_context(|| unknown(name))?),
    };
    let mut styled =
        Styled { name: preset.map(|preset| preset.name.to_string()), theme: preset.and_then(|preset| preset.theme), ..Styled::default() };
    let Some(section) = section else {
        return Ok(styled);
    };
    if accent.is_some()
        && (section.accent.is_some()
            || [&section.light, &section.dark].iter().any(|mode| mode.as_ref().is_some_and(|mode| mode.accent.is_some())))
    {
        bail!("installer.accent and [style] both set the accent; keep it in [style] and remove installer.accent");
    }

    let mut custom = Custom::default();
    let body = fonts(section.font.as_ref(), "style.font", root, &mut styled)?;
    let heading = fonts(section.heading_font.as_ref(), "style.heading_font", root, &mut styled)?;
    custom.font = body.as_ref().map(|(family, _)| family.clone());
    // Headings follow the text's font unless they have their own, in its boldest cut.
    custom.heading_font = heading.as_ref().or(body.as_ref()).map(|(family, _)| family.clone());
    custom.heading_weight = section.heading_weight.or(heading.as_ref().or(body.as_ref()).and_then(|(_, weight)| *weight));
    if let Some(weight) = section.heading_weight {
        if !(100..=900).contains(&weight) {
            bail!("style.heading_weight is {weight}; use 100 (thin) to 900 (black), like 600 for semibold");
        }
    }
    if let Some(radius) = section.radius.filter(|radius| *radius > 24) {
        bail!("style.radius is {radius}; use 0 (square) to 24 points");
    }
    if let Some(radius) = section.button_radius.filter(|radius| *radius > 16) {
        bail!("style.button_radius is {radius}; use 0 (square) to 16 points (16 makes pill buttons)");
    }
    custom.radius = section.radius;
    custom.button_radius = section.button_radius;

    if let Some(opacity) = section.background_image_opacity {
        if !(0.0..=1.0).contains(&opacity) {
            bail!("style.background_image_opacity is {opacity}; use 0 (hidden) to 1 (fully shown)");
        }
        if section.background_image.is_none() {
            bail!("style.background_image_opacity is set, but style.background_image isn't");
        }
    }
    custom.background_image_opacity = section.background_image_opacity;
    if let Some(picture) = &section.background_image {
        let (kind, bytes) = background_image(picture, root)?;
        styled.files.push(LookFile { kind, len: bytes.len() as u64 });
        styled.bytes.push(bytes);
    }

    let shared = ColorSection {
        background: section.background.clone(),
        card: section.card.clone(),
        text: section.text.clone(),
        muted_text: section.muted_text.clone(),
        accent: section.accent.clone(),
        accent_text: section.accent_text.clone(),
        border: section.border.clone(),
        error: section.error.clone(),
        mark: section.mark.clone(),
    };
    let shared = colors(&shared, "style")?;
    for (mode, own, out) in [("light", &section.light, &mut custom.light), ("dark", &section.dark, &mut custom.dark)] {
        let own = match own {
            Some(own) => colors(own, &format!("style.{mode}"))?,
            None => Colors::default(),
        };
        *out = merge(&shared, own);
    }
    styled.custom = Some(custom);
    Ok(styled)
}

fn unknown(name: &str) -> String {
    let all: Vec<&str> = style::PRESETS.iter().map(|preset| preset.name).collect();
    match style::closest(name) {
        Some(near) => format!("installer.style is \"{name}\"; did you mean \"{near}\"? (`bach styles` lists them all)"),
        None => format!("installer.style is \"{name}\"; use one of {} (`bach styles` shows what each looks like)", all.join(", ")),
    }
}

/// `font` or `heading_font`: a family Windows has, or font files to ship.
/// Returns the family, and the boldest file's weight when they are files.
fn fonts(value: Option<&toml::Value>, key: &str, root: &Path, styled: &mut Styled) -> Result<Option<(String, Option<u16>)>> {
    let entries: Vec<&str> = match value {
        None => return Ok(None),
        Some(toml::Value::String(value)) => vec![value.as_str()],
        Some(toml::Value::Array(values)) if !values.is_empty() => values
            .iter()
            .map(|value| value.as_str().with_context(|| format!("{key} is a list; every entry must be a font file in quotes")))
            .collect::<Result<_>>()?,
        Some(_) => bail!("{key} is a font family like \"Segoe UI\", a font file like \"fonts/Nunito.ttf\", or a list of font files"),
    };
    let is_file =
        |entry: &str| [".ttf", ".otf", ".ttc", ".woff", ".woff2"].iter().any(|ext| entry.trim().to_ascii_lowercase().ends_with(ext));
    if entries.len() == 1 && !is_file(entries[0]) {
        let family = entries[0].trim();
        if family.is_empty() || family.chars().count() > 64 || family.chars().any(char::is_control) {
            bail!("{key} is \"{family}\"; give a font family like \"Segoe UI\" (up to 64 characters) or a .ttf file");
        }
        return Ok(Some((family.to_string(), None)));
    }
    let mut family: Option<String> = None;
    let mut heaviest = 0;
    for entry in entries {
        if !is_file(entry) {
            bail!("{key} lists \"{entry}\"; a list holds only .ttf or .otf files");
        }
        let path = existing_file(root, entry, key)?;
        if !(has_extension(&path, "ttf") || has_extension(&path, "otf")) {
            bail!("{key} is \"{entry}\"; Windows loads .ttf and .otf fonts, so use one of those");
        }
        let bytes = read_capped(&path, MAX_FONT, entry, key)?;
        let info = font::read(&bytes).with_context(|| format!("{key} is \"{entry}\", but"))?;
        match &family {
            Some(first) if !first.eq_ignore_ascii_case(&info.family) => {
                bail!("{key} lists fonts of two families (\"{first}\" and \"{}\"); give the cuts of one family", info.family)
            }
            _ => family = Some(info.family),
        }
        heaviest = heaviest.max(info.weight);
        styled.files.push(LookFile { kind: LookFileKind::Font, len: bytes.len() as u64 });
        styled.bytes.push(bytes);
    }
    Ok(family.map(|family| (family, Some(heaviest))))
}

fn background_image(picture: &str, root: &Path) -> Result<(LookFileKind, Vec<u8>)> {
    let key = "style.background_image";
    let path = existing_file(root, picture, key)?;
    let bytes = read_capped(&path, MAX_PICTURE, picture, key)?;
    if bytes.starts_with(PNG_SIGNATURE) {
        Ok((LookFileKind::BackgroundPng, bytes))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok((LookFileKind::BackgroundJpeg, bytes))
    } else {
        bail!("{key} is \"{picture}\", but it isn't a PNG or JPEG picture inside")
    }
}

fn read_capped(path: &Path, limit: u64, shown: &str, key: &str) -> Result<Vec<u8>> {
    let size = std::fs::metadata(path).with_context(|| format!("couldn't read {}", path.display()))?.len();
    if size > limit {
        bail!("{key} is \"{shown}\", which is {} MB; keep it under {} MB", size.div_ceil(1 << 20), limit >> 20);
    }
    std::fs::read(path).with_context(|| format!("couldn't read {}", path.display()))
}

/// One mode's colours, checked.
fn colors(section: &ColorSection, prefix: &str) -> Result<Colors> {
    let color = |value: &Option<String>, name: &str| -> Result<Option<String>> {
        let Some(value) = value else {
            return Ok(None);
        };
        let parsed = Color::parse(value).with_context(|| {
            format!("{prefix}.{name} is \"{value}\"; use a colour like \"#2563eb\" (or \"#2563eb80\" to let what's behind show through)")
        })?;
        Ok(Some(parsed.hex()))
    };
    let (background, background_end) = match &section.background {
        None => (None, None),
        Some(toml::Value::String(value)) => (Some(value.clone()), None),
        Some(toml::Value::Array(values)) if values.len() == 2 && values.iter().all(toml::Value::is_str) => {
            (values[0].as_str().map(String::from), values[1].as_str().map(String::from))
        }
        Some(_) => bail!(
            "{prefix}.background is a colour like \"#ffffff\", or two for a gradient from top to bottom, like [\"#1d0b36\", \"#0a0416\"]"
        ),
    };
    let background = color(&background, "background")?;
    let background_end = color(&background_end, "background")?;
    for value in [&background, &background_end].into_iter().flatten() {
        if Color::parse(value).is_some_and(|color| color.alpha() < 1.0) {
            bail!("{prefix}.background is \"{value}\"; the window can't be see-through, so leave out the last two digits");
        }
    }
    Ok(Colors {
        background,
        background_end,
        card: color(&section.card, "card")?,
        text: color(&section.text, "text")?,
        muted_text: color(&section.muted_text, "muted_text")?,
        accent: color(&section.accent, "accent")?,
        accent_text: color(&section.accent_text, "accent_text")?,
        border: color(&section.border, "border")?,
        error: color(&section.error, "error")?,
        mark: color(&section.mark, "mark")?,
    })
}

/// `own` where it says something, `shared` elsewhere. A mode's own
/// background replaces the shared one whole, gradient and all.
fn merge(shared: &Colors, own: Colors) -> Colors {
    let pick = |own: Option<String>, shared: &Option<String>| own.or_else(|| shared.clone());
    let (background, background_end) = match own.background {
        Some(background) => (Some(background), own.background_end),
        None => (shared.background.clone(), shared.background_end.clone()),
    };
    Colors {
        background,
        background_end,
        card: pick(own.card, &shared.card),
        text: pick(own.text, &shared.text),
        muted_text: pick(own.muted_text, &shared.muted_text),
        accent: pick(own.accent, &shared.accent),
        accent_text: pick(own.accent_text, &shared.accent_text),
        border: pick(own.border, &shared.border),
        error: pick(own.error, &shared.error),
        mark: pick(own.mark, &shared.mark),
    }
}
