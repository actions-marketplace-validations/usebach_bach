//! `[layout]`: the window's size and format, where the header sits, how
//! text aligns, spacing and sizes, how the rows, buttons and footer are
//! drawn, and how pages animate. Each value is checked against the bounds
//! in `style::layout::bounds`, and every mistake names its setting.

use std::path::Path;

use anyhow::{bail, Result};
use serde::Deserialize;

use super::existing_file;
use super::icon::PNG_SIGNATURE;
use crate::manifest::LookFileKind;
use crate::style::layout::{bounds, Align, Animation, Buttons, Footer, Header, Layout, LogoPlace, Primary, Rows};

/// The largest panel picture.
const MAX_PICTURE: u64 = 8 << 20;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LayoutSection {
    width: Option<i64>,
    height: Option<i64>,
    header: Option<String>,
    header_width: Option<i64>,
    panel: Option<bool>,
    panel_image: Option<String>,
    align: Option<String>,
    logo: Option<String>,
    padding: Option<toml::Value>,
    gap: Option<i64>,
    heading_size: Option<i64>,
    text_size: Option<i64>,
    row_padding: Option<i64>,
    rows: Option<String>,
    icons: Option<bool>,
    dividers: Option<bool>,
    button_height: Option<i64>,
    buttons: Option<String>,
    primary: Option<String>,
    footer: Option<String>,
    animation: Option<String>,
    animation_speed: Option<f64>,
}

/// A picture the layout ships, and what kind it is.
type Picture = (LookFileKind, Vec<u8>);

/// The checked layout, and the panel picture it ships.
pub(super) fn layout(section: &LayoutSection, root: &Path) -> Result<(Layout, Option<Picture>)> {
    let layout = Layout {
        width: number(section.width, "width", bounds::WIDTH, "points wide")?,
        height: number(section.height, "height", bounds::HEIGHT, "points tall")?,
        header: choice(&section.header, "header", &[("top", Header::Top), ("left", Header::Left), ("right", Header::Right)])?,
        header_width: number(section.header_width, "header_width", bounds::HEADER_WIDTH, "points")?,
        panel: section.panel,
        align: choice(&section.align, "align", &[("left", Align::Left), ("center", Align::Center), ("right", Align::Right)])?,
        logo: choice(&section.logo, "logo", &[("above", LogoPlace::Above), ("beside", LogoPlace::Beside)])?,
        padding: padding(section.padding.as_ref())?,
        gap: number(section.gap, "gap", bounds::GAP, "points")?,
        heading_size: number(section.heading_size, "heading_size", bounds::HEADING_SIZE, "points")?,
        text_size: number(section.text_size, "text_size", bounds::TEXT_SIZE, "points")?,
        row_padding: number(section.row_padding, "row_padding", bounds::ROW_PADDING, "points")?,
        rows: choice(&section.rows, "rows", &[("joined", Rows::Joined), ("separate", Rows::Separate)])?,
        icons: section.icons,
        dividers: section.dividers,
        button_height: number(section.button_height, "button_height", bounds::BUTTON_HEIGHT, "points")?,
        buttons: choice(
            &section.buttons,
            "buttons",
            &[("right", Buttons::Right), ("left", Buttons::Left), ("center", Buttons::Center), ("stretch", Buttons::Stretch)],
        )?,
        primary: choice(&section.primary, "primary", &[("last", Primary::Last), ("first", Primary::First)])?,
        footer: choice(&section.footer, "footer", &[("beside", Footer::Beside), ("above", Footer::Above), ("hidden", Footer::Hidden)])?,
        animation: choice(
            &section.animation,
            "animation",
            &[
                ("rise", Animation::Rise),
                ("fade", Animation::Fade),
                ("slide", Animation::Slide),
                ("drop", Animation::Drop),
                ("none", Animation::None),
            ],
        )?,
        animation_speed: match section.animation_speed {
            None => None,
            Some(speed) if (f64::from(bounds::ANIMATION_SPEED.0)..=f64::from(bounds::ANIMATION_SPEED.1)).contains(&speed) => {
                Some(speed as f32)
            }
            Some(speed) => bail!("layout.animation_speed is {speed}; use 0.25 (four times slower) to 4 (four times faster)"),
        },
    };
    if let (Some(header_width), Some(width)) = (layout.header_width, layout.width) {
        if u32::from(header_width) + 220 > u32::from(width) {
            bail!("layout.header_width is {header_width}, which leaves the choices less than 220 points of a {width}-point window");
        }
    }
    let side = matches!(layout.header, Some(Header::Left | Header::Right));
    if !side {
        for (set, key) in [(layout.header_width.is_some(), "header_width"), (section.panel_image.is_some(), "panel_image")] {
            if set {
                bail!("layout.{key} is set, but the header isn't at the side; add header = \"left\" or \"right\"");
            }
        }
    }
    if section.panel == Some(false) && section.panel_image.is_some() {
        bail!("layout.panel_image is set, but layout.panel is false");
    }
    let picture = match &section.panel_image {
        None => None,
        Some(picture) => Some(panel_image(picture, root)?),
    };
    Ok((layout, picture))
}

fn number<T: TryFrom<i64> + Copy + Into<i64>>(value: Option<i64>, key: &str, (low, high): (T, T), unit: &str) -> Result<Option<T>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value < low.into() || value > high.into() {
        bail!("layout.{key} is {value}; use {} to {} {unit}", low.into(), high.into());
    }
    match T::try_from(value) {
        Ok(value) => Ok(Some(value)),
        Err(_) => bail!("layout.{key} is {value}; use {} to {} {unit}", low.into(), high.into()),
    }
}

fn choice<T: Copy>(value: &Option<String>, key: &str, options: &[(&str, T)]) -> Result<Option<T>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let wanted = value.trim().to_ascii_lowercase();
    match options.iter().find(|(name, _)| *name == wanted) {
        Some((_, choice)) => Ok(Some(*choice)),
        None => {
            let names: Vec<String> = options.iter().map(|(name, _)| format!("\"{name}\"")).collect();
            let list = match names.split_last() {
                Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
                _ => names.join(""),
            };
            bail!("layout.{key} is \"{value}\"; use {list}")
        }
    }
}

/// One number for every side, two for top and bottom then the sides, or
/// four for top, right, bottom and left, like CSS.
fn padding(value: Option<&toml::Value>) -> Result<Option<[u8; 4]>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let numbers: Vec<i64> = match value {
        toml::Value::Integer(number) => vec![*number],
        toml::Value::Array(items) => items.iter().filter_map(toml::Value::as_integer).collect(),
        _ => Vec::new(),
    };
    let complete = matches!(value, toml::Value::Integer(_)) || matches!(value, toml::Value::Array(items) if items.len() == numbers.len());
    let [top, right, bottom, left] = match numbers.as_slice() {
        [all] if complete => [*all; 4],
        [vertical, horizontal] if complete => [*vertical, *horizontal, *vertical, *horizontal],
        [top, right, bottom, left] if complete => [*top, *right, *bottom, *left],
        _ => bail!(
            "layout.padding is one number (every side), two ([top and bottom, the sides]) or four ([top, right, bottom, left]), in points"
        ),
    };
    let mut out = [0u8; 4];
    for (slot, value) in out.iter_mut().zip([top, right, bottom, left]) {
        *slot = number(Some(value), "padding", bounds::PADDING, "points on each side")?.unwrap_or(0);
    }
    Ok(Some(out))
}

fn panel_image(picture: &str, root: &Path) -> Result<Picture> {
    let key = "layout.panel_image";
    let path = existing_file(root, picture, key)?;
    let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    if size > MAX_PICTURE {
        bail!("{key} is \"{picture}\", which is {} MB; keep it under {} MB", size.div_ceil(1 << 20), MAX_PICTURE >> 20);
    }
    let bytes = std::fs::read(&path).map_err(|err| anyhow::anyhow!("couldn't read {}: {err}", path.display()))?;
    if bytes.starts_with(PNG_SIGNATURE) {
        Ok((LookFileKind::PanelPng, bytes))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok((LookFileKind::PanelJpeg, bytes))
    } else {
        bail!("{key} is \"{picture}\", but it isn't a PNG or JPEG picture inside")
    }
}
