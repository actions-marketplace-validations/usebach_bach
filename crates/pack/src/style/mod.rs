//! Styles for the setup window: a set of ready-made looks (`installer.style`)
//! and the developer's own changes on top (`[style]`), as plain data the
//! builder and the runtime share.
//!
//! A style is colours for light and dark, the fonts to try, the corner radii
//! and, for some, the theme it looks best in. The runtime turns the resolved
//! palette into its full set of colour tokens; nothing here knows about GPUI.
//!
//! - [`presets`]: the built-in styles.
//! - [`font`]: the family name and weight inside a font file.
//! - [`layout`]: `[layout]`, the window's size, arrangement and motion.

use serde::{Deserialize, Serialize};

use crate::manifest::{Look, Theme};

pub mod font;
pub mod layout;
mod presets;

pub use presets::PRESETS;

/// The style used when `installer.style` isn't set: the Bach design system.
pub const DEFAULT: &str = "bach";

/// The family the runtime always has: Inter, embedded in every setup file.
pub const INTER: &str = "Inter";
/// Inter at its display optical size, the default heading face.
pub const INTER_DISPLAY: &str = "Inter Display";

/// A colour as `0xRRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    /// An opaque colour from `0xRRGGBB`.
    pub const fn rgb(value: u32) -> Self {
        Self((value << 8) | 0xff)
    }

    /// `#rrggbb` or `#rrggbbaa` (the `#` is optional).
    pub fn parse(value: &str) -> Option<Self> {
        let hex = value.trim().trim_start_matches('#');
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let number = u32::from_str_radix(hex, 16).ok()?;
        match hex.len() {
            6 => Some(Self::rgb(number)),
            8 => Some(Self(number)),
            _ => None,
        }
    }

    /// `0xRRGGBB`, without the alpha.
    pub const fn rgb_value(self) -> u32 {
        self.0 >> 8
    }

    /// The alpha, 0 (clear) to 1 (opaque).
    pub fn alpha(self) -> f32 {
        (self.0 & 0xff) as f32 / 255.0
    }

    /// `#rrggbb`, or `#rrggbbaa` when it isn't opaque.
    pub fn hex(self) -> String {
        if self.0 & 0xff == 0xff {
            format!("#{:06x}", self.rgb_value())
        } else {
            format!("#{:08x}", self.0)
        }
    }
}

/// The colours of one mode. A preset sets the core ones; the rest are
/// worked out from them unless a preset tunes them by hand.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Palette {
    /// The window, or the top of its gradient.
    pub background: Option<Color>,
    /// The bottom of the window's gradient; a solid window when None.
    pub background_end: Option<Color>,
    /// The fill of the card of choices and the license box; the window
    /// shows through when None.
    pub card: Option<Color>,
    pub text: Option<Color>,
    pub muted_text: Option<Color>,
    /// Buttons, the progress bar and ticked boxes.
    pub accent: Option<Color>,
    /// Text on the accent; black or white, whichever reads, when None.
    pub accent_text: Option<Color>,
    /// The accent under the pointer.
    pub accent_hover: Option<Color>,
    pub border: Option<Color>,
    /// Remove buttons and failure icons.
    pub error: Option<Color>,
    /// The Bach mark and one-colour (SVG) logos; the text colour when None.
    pub mark: Option<Color>,
    /// A side header's panel (`[layout] header`); derived when None.
    pub panel: Option<Color>,
    /// Hand-tuned tokens the runtime otherwise derives.
    pub well: Option<Color>,
    pub track: Option<Color>,
    pub input: Option<Color>,
    pub ring: Option<Color>,
    pub hover: Option<Color>,
}

impl Palette {
    pub const EMPTY: Self = Self {
        background: None,
        background_end: None,
        card: None,
        text: None,
        muted_text: None,
        accent: None,
        accent_text: None,
        accent_hover: None,
        border: None,
        error: None,
        mark: None,
        panel: None,
        well: None,
        track: None,
        input: None,
        ring: None,
        hover: None,
    };

    /// `other`'s colours where it has them, this palette's elsewhere.
    pub fn with(mut self, other: &Palette) -> Self {
        macro_rules! take {
            ($($field:ident),*) => { $( if other.$field.is_some() { self.$field = other.$field; } )* };
        }
        take!(background, background_end, card, text, muted_text, accent, accent_text, accent_hover, border, error, mark, panel);
        take!(well, track, input, ring, hover);
        // A new background without its own gradient end is a solid window.
        if other.background.is_some() && other.background_end.is_none() {
            self.background_end = None;
        }
        // A new accent drops the preset's hand-tuned hover and ink for it.
        if other.accent.is_some() {
            if other.accent_hover.is_none() {
                self.accent_hover = None;
            }
            if other.accent_text.is_none() {
                self.accent_text = None;
            }
        }
        // Derived tokens follow the colours they come from.
        if other.background.is_some() || other.card.is_some() || other.text.is_some() {
            self.well = other.well;
            self.track = other.track;
            self.input = other.input;
            self.ring = other.ring;
            self.hover = other.hover;
        }
        self
    }
}

/// A built-in style.
#[derive(Debug)]
pub struct Preset {
    /// What `installer.style` names it by.
    pub name: &'static str,
    /// One line: what it looks like.
    pub description: &'static str,
    /// What kind of app it suits, for `bach styles` and `bach init`.
    pub suits: &'static str,
    /// The theme it looks best in, used when `installer.theme` isn't set.
    pub theme: Option<Theme>,
    /// Families to try for the text, in order; Inter when none is installed.
    pub fonts: &'static [&'static str],
    /// Families to try for headings; Inter Display when none is installed.
    pub heading_fonts: &'static [&'static str],
    pub heading_weight: u16,
    /// Whether the Bach mark and one-colour logos take the accent colour
    /// (and keep following it when the developer changes the accent).
    pub accent_mark: bool,
    /// Cards and the license box, in points.
    pub radius: u8,
    /// Buttons, icon wells and boxes, in points (16 makes pill buttons).
    pub control_radius: u8,
    pub light: Palette,
    pub dark: Palette,
}

/// The built-in style called `name` (any case).
pub fn preset(name: &str) -> Option<&'static Preset> {
    let name = name.trim();
    PRESETS.iter().find(|preset| preset.name.eq_ignore_ascii_case(name))
}

/// The built-in style whose name is closest to a misspelt one, if any is close.
pub fn closest(name: &str) -> Option<&'static str> {
    let name = name.trim().to_ascii_lowercase();
    PRESETS
        .iter()
        .map(|preset| (preset.name, edit_distance(&name, preset.name)))
        .filter(|(_, distance)| *distance <= 2)
        .min_by_key(|(_, distance)| *distance)
        .map(|(name, _)| name)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let replaced = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = replaced.min(row[j] + 1).min(previous + 1);
        }
    }
    row[b.len()]
}

/// The colours of one mode as written in the manifest: `#rrggbb` or
/// `#rrggbbaa`, each optional.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Colors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<String>,
}

impl Colors {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// As a palette; a colour that doesn't parse is left out.
    pub fn palette(&self) -> Palette {
        let color = |value: &Option<String>| value.as_deref().and_then(Color::parse);
        Palette {
            background: color(&self.background),
            background_end: color(&self.background_end),
            card: color(&self.card),
            text: color(&self.text),
            muted_text: color(&self.muted_text),
            accent: color(&self.accent),
            accent_text: color(&self.accent_text),
            border: color(&self.border),
            error: color(&self.error),
            mark: color(&self.mark),
            panel: color(&self.panel),
            ..Palette::EMPTY
        }
    }
}

/// The developer's own changes to the chosen style, from `[style]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Custom {
    /// The text's font family: one Windows has, or one shipped in the look section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_weight: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button_radius: Option<u8>,
    /// How strongly the background (and panel) picture shows, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image_opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Colors::is_empty")]
    pub light: Colors,
    #[serde(default, skip_serializing_if = "Colors::is_empty")]
    pub dark: Colors,
}

/// How strongly a background image shows when `[style]` doesn't say.
pub const DEFAULT_IMAGE_OPACITY: f32 = 0.35;

/// A style worked out for the runtime: the preset with the developer's
/// changes on top.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// The preset's name.
    pub name: &'static str,
    /// Families to try for the text, the developer's first.
    pub fonts: Vec<String>,
    pub heading_fonts: Vec<String>,
    pub heading_weight: u16,
    pub radius: f32,
    pub control_radius: f32,
    pub image_opacity: f32,
    pub light: Palette,
    pub dark: Palette,
}

/// The look's style, or `style` instead of its preset (a preview trying
/// another one). An unknown name falls back to the default style.
pub fn resolve(look: &Look, style: Option<&str>) -> Resolved {
    let preset = style.or(look.style.as_deref()).and_then(preset).unwrap_or(&PRESETS[0]);
    let custom = look.custom.clone().unwrap_or_default();
    let accent = Palette { accent: look.accent.as_deref().and_then(Color::parse), ..Palette::EMPTY };
    let families = |own: &Option<String>, preset: &[&str]| -> Vec<String> {
        own.iter().cloned().chain(preset.iter().map(|family| family.to_string())).collect()
    };
    let palette = |base: &Palette, own: &Colors| {
        let mut palette = base.with(&accent).with(&own.palette());
        if preset.accent_mark && palette.mark.is_none() {
            palette.mark = palette.accent;
        }
        palette
    };
    Resolved {
        name: preset.name,
        fonts: families(&custom.font, preset.fonts),
        heading_fonts: families(&custom.heading_font, preset.heading_fonts),
        heading_weight: custom.heading_weight.unwrap_or(preset.heading_weight),
        radius: f32::from(custom.radius.unwrap_or(preset.radius)),
        control_radius: f32::from(custom.button_radius.unwrap_or(preset.control_radius)),
        image_opacity: custom.background_image_opacity.unwrap_or(DEFAULT_IMAGE_OPACITY).clamp(0.0, 1.0),
        light: palette(&preset.light, &custom.light),
        dark: palette(&preset.dark, &custom.dark),
    }
}

#[cfg(test)]
mod tests;
