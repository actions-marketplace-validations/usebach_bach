//! Colour tokens, fonts and corner radii for light and dark, worked out
//! from the app's style (`installer.style` and `[style]`, see
//! `bach_pack::style`). No view spells a colour or a radius of its own.
//!
//! - `derive.rs`: a style's palette turned into the full set of tokens.

use std::sync::Arc;

use gpui::{px, App, FontWeight, Global, Hsla, Image, Pixels, SharedString, WindowAppearance};

mod derive;

pub use derive::{Faces, Looks};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    /// The window, or the top of its gradient.
    pub surface: Hsla,
    /// The bottom of the window's gradient, when it has one.
    pub surface_end: Option<Hsla>,
    /// A picture over the window's colour, and how strongly it shows.
    pub picture: Option<(Arc<Image>, f32)>,
    /// The fill of the card of choices, the license box and the key field;
    /// clear when the window shows through.
    pub card: Hsla,
    /// Sunken icon wells.
    pub well: Hsla,
    pub well_outline: Hsla,
    pub well_highlight: Hsla,
    pub border: Hsla,
    pub foreground: Hsla,
    pub muted_foreground: Hsla,
    /// Buttons, the progress fill and ticked boxes.
    pub primary: Hsla,
    pub primary_hover: Hsla,
    pub primary_foreground: Hsla,
    /// The progress track.
    pub muted: Hsla,
    pub destructive: Hsla,
    /// A checkbox's resting edge.
    pub input: Hsla,
    /// A checkbox's hovered edge.
    pub ring: Hsla,
    /// Hover wash on outline buttons.
    pub hover: Hsla,
    /// The Bach mark, one-colour logos and the page icons.
    pub mark: Hsla,
    /// The text's font family.
    pub font: SharedString,
    pub heading_font: SharedString,
    pub heading_weight: FontWeight,
    /// Cards, the license box and the key field.
    pub radius: Pixels,
    /// Buttons, icon wells and checkboxes.
    pub control_radius: Pixels,
}

impl Global for Theme {}

/// Inter, embedded as static instances (see assets.rs). If it can't be
/// registered GPUI falls back to Segoe UI on its own.
pub const FONT_SANS: &str = bach_pack::style::INTER;
/// Inter at its display optical size, for headings.
pub const FONT_DISPLAY: &str = bach_pack::style::INTER_DISPLAY;

impl Theme {
    /// A checkbox's corners: the controls' radius, kept small on a 16px box.
    pub fn checkbox_radius(&self) -> Pixels {
        self.control_radius.min(px(5.))
    }

    /// The progress bar's ends: round unless the style is square.
    pub fn bar_radius(&self, height: Pixels) -> Pixels {
        if self.control_radius > px(0.) {
            height / 2.
        } else {
            px(0.)
        }
    }

    /// The corners of something `inset` inside an element with `radius` corners.
    pub fn inner_radius(radius: Pixels, inset: f32) -> Pixels {
        (radius - px(inset)).max(px(0.))
    }
}

/// `cx.theme()` anywhere an `App` is reachable.
pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}
