//! Icons and fonts compiled into the runtime, plus the app's own SVG logo
//! and its style's fonts read from the payload, all served to GPUI from
//! memory.

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Result;
use gpui::{App, AssetSource, SharedString};
use rust_embed::RustEmbed;

use bach_pack::style::Resolved;

use crate::theme::{Faces, FONT_DISPLAY, FONT_SANS};

#[derive(RustEmbed)]
#[folder = "assets"]
struct Embedded;

/// The asset path the app's SVG logo is served under.
pub const LOGO_PATH: &str = "payload/logo.svg";

/// The runtime's assets, and the logo when the payload has an SVG one.
pub struct Assets {
    pub logo_svg: Option<Arc<[u8]>>,
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO_PATH {
            return Ok(self.logo_svg.as_ref().map(|bytes| Cow::Owned(bytes.to_vec())));
        }
        Ok(Embedded::get(path).map(|file| file.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Embedded::iter().filter(|name| name.starts_with(path)).map(SharedString::from).collect())
    }
}

/// The SVG path for an exported Tabler icon, e.g. `icon("folder-filled")`.
pub fn icon(name: &str) -> SharedString {
    format!("icons/{name}.svg").into()
}

/// Inter as static TTF instances (400/500/600 at the text optical size,
/// plus a 500 display cut for headings). DirectWrite's in-memory loader
/// can't read WOFF2, so they ship as plain TTF.
const FONT_FILES: [&str; 4] =
    ["fonts/Inter-Regular.ttf", "fonts/Inter-Medium.ttf", "fonts/Inter-SemiBold.ttf", "fonts/InterDisplay-Medium.ttf"];

/// Registers the embedded fonts. On failure GPUI's own fallback (Segoe UI)
/// renders everything, so this only explains why.
pub fn load_fonts(cx: &mut App) -> Result<(), String> {
    let fonts: Vec<Cow<'static, [u8]>> = FONT_FILES.iter().filter_map(|path| Embedded::get(path).map(|file| file.data)).collect();
    if fonts.len() != FONT_FILES.len() {
        return Err("some font files are missing from the build".into());
    }
    cx.text_system().add_fonts(fonts).map_err(|err| format!("{err:#}"))?;
    let names = cx.text_system().all_font_names();
    let has = |family: &str| names.iter().any(|name| name == family);
    if has(FONT_SANS) && has(FONT_DISPLAY) {
        Ok(())
    } else {
        Err("the fonts loaded but their families didn't register".into())
    }
}

/// Registers the fonts a style ships and picks the families to draw with:
/// the first the style names that this PC has (the developer's own font
/// first), else Inter. Returns them with a sentence for the log when a
/// family the style asked for isn't there.
pub fn style_faces(cx: &mut App, style: &Resolved, shipped: &[Vec<u8>]) -> (Faces, Option<String>) {
    let before = cx.text_system().all_font_names();
    if !shipped.is_empty() {
        let fonts = shipped.iter().map(|bytes| Cow::Owned(bytes.clone())).collect();
        if let Err(err) = cx.text_system().add_fonts(fonts) {
            eprintln!("bach-setup: the style's fonts didn't load ({err:#})");
        }
    }
    let names = cx.text_system().all_font_names();
    // A shipped family Windows names differently than its file did.
    let added: Vec<&String> = names.iter().filter(|name| !before.contains(name)).collect();
    let find = |wanted: &[String], fallback: &'static str| -> (SharedString, bool) {
        for (index, family) in wanted.iter().enumerate() {
            if let Some(name) = names.iter().find(|name| name.eq_ignore_ascii_case(family)) {
                return (name.clone().into(), index == 0);
            }
            if index == 0 && added.len() == 1 {
                return (added[0].clone().into(), true);
            }
        }
        (fallback.into(), false)
    };
    let (text, text_found) = find(&style.fonts, FONT_SANS);
    let (heading, heading_found) = find(&style.heading_fonts, FONT_DISPLAY);
    let missing = (!text_found || !heading_found).then(|| {
        format!("Style {}: wanted fonts {:?} and {:?}; drawing with {text} and {heading}", style.name, style.fonts, style.heading_fonts)
    });
    (Faces { text, heading }, missing)
}
