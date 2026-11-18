//! A style's palette turned into the window's tokens. A preset names the
//! colours people see (window, text, accent...); the quieter ones (wells,
//! hover washes, the progress track, checkbox edges) are mixed from those,
//! unless the preset tuned them by hand, as Bach's own style does.

use std::sync::Arc;

use bach_pack::style::layout::Arrangement;
use bach_pack::style::{Color, Palette, Resolved, PRESETS};
use gpui::{black, px, rgb, white, FontWeight, Hsla, Image, Rgba, SharedString};

use super::{Emblem, Mode, Theme};
use crate::ui::color::mix;

/// The window's style in both modes, worked out once at startup.
#[derive(Clone, Debug)]
pub struct Looks {
    light: Theme,
    dark: Theme,
}

/// The font families the style asked for that this PC has.
#[derive(Clone, Debug)]
pub struct Faces {
    pub text: SharedString,
    pub heading: SharedString,
}

impl Faces {
    /// Inter, which every setup file carries.
    #[cfg(test)]
    pub fn inter() -> Self {
        Self { text: super::FONT_SANS.into(), heading: super::FONT_DISPLAY.into() }
    }
}

impl Looks {
    pub fn new(style: &Resolved, faces: &Faces, pictures: Pictures, layout: Arrangement) -> Self {
        let make = |mode| theme(mode, style, faces, &pictures, layout);
        Self { light: make(Mode::Light), dark: make(Mode::Dark) }
    }

    /// Bach's own style.
    #[cfg(test)]
    pub fn bach() -> Self {
        Self::new(
            &bach_pack::style::resolve(&Default::default(), None),
            &Faces::inter(),
            Pictures::default(),
            Arrangement::default_window(),
        )
    }

    pub fn get(&self, mode: Mode) -> Theme {
        match mode {
            Mode::Light => self.light.clone(),
            Mode::Dark => self.dark.clone(),
        }
    }
}

fn hsla(color: Color) -> Hsla {
    let channel = |shift: u32| ((color.0 >> shift) & 0xff) as f32 / 255.0;
    Rgba { r: channel(24), g: channel(16), b: channel(8), a: channel(0) }.into()
}

/// The pictures a look ships: behind the window, and in a side panel.
#[derive(Clone, Debug, Default)]
pub struct Pictures {
    pub background: Option<Arc<Image>>,
    pub panel: Option<Arc<Image>>,
    /// The logo, for a panel with no picture.
    pub emblem: Emblem,
}

fn theme(mode: Mode, style: &Resolved, faces: &Faces, pictures: &Pictures, layout: Arrangement) -> Theme {
    let (own, fallback) = match mode {
        Mode::Light => (&style.light, &PRESETS[0].light),
        Mode::Dark => (&style.dark, &PRESETS[0].dark),
    };
    // Every resolved palette has the core colours; Bach's fill any gap.
    let palette = fallback.with(own);
    let core = |pick: fn(&Palette) -> Option<Color>| pick(&palette).or(pick(fallback)).map(hsla).unwrap_or_else(black);
    let surface = core(|p| p.background);
    let text = core(|p| p.text);
    let accent = core(|p| p.accent);
    let card = palette.card.map(hsla).unwrap_or(surface.opacity(0.0));
    // What rows sit on: the card over the window.
    let base = mix(surface, card.opacity(1.0), card.a);
    let tuned = |pick: fn(&Palette) -> Option<Color>, derived: Hsla| pick(&palette).map(hsla).unwrap_or(derived);
    let white = white();
    let ink = if relative_luminance(accent) < 0.45 { white } else { rgb(0x0f0f0f).into() };
    let toward = if mode == Mode::Dark { white } else { black() };
    let muted = tuned(|p| p.muted_text, mix(base, text, 0.6));
    // A panel without a picture is a light tint of the accent, so it reads
    // as its own surface in both modes.
    let panel = tuned(|p| p.panel, mix(surface, accent, if mode == Mode::Dark { 0.1 } else { 0.08 }));
    // On a panel the header reads on the panel, not the window: light on a
    // picture (under a shade, see pages), else whatever reads on its colour.
    let (header_text, header_muted) = match (layout.header_on_panel(), palette.panel_text.map(hsla)) {
        (true, Some(own)) => (own, own.opacity(0.75)),
        (true, None) if pictures.panel.is_some() => (rgb(0xfafafa).into(), white.opacity(0.78)),
        (true, None) if (relative_luminance(panel) < 0.4) != (relative_luminance(text) < 0.4) => (text, muted),
        (true, None) => {
            let ink: Hsla = if relative_luminance(panel) < 0.4 { rgb(0xfafafa).into() } else { rgb(0x141414).into() };
            (ink, ink.opacity(0.72))
        }
        (false, _) => (text, muted),
    };
    Theme {
        surface,
        surface_end: palette.background_end.map(hsla),
        picture: pictures.background.clone().map(|picture| (picture, style.image_opacity)),
        // A card that hides the window makes a good panel; else a soft wash of the text colour.
        panel,
        // The panel's own picture shows fully; the developer dims it in the file.
        panel_picture: pictures.panel.clone().map(|picture| (picture, 1.0)),
        header_text,
        header_muted,
        emblem: pictures.emblem.clone(),
        layout,
        card,
        well: tuned(|p| p.well, mix(base, text, 0.05)),
        well_outline: text.opacity(0.05),
        well_highlight: white.opacity(if mode == Mode::Dark { 0.12 } else { 0.9 }),
        border: tuned(|p| p.border, text.opacity(0.08)),
        foreground: text,
        muted_foreground: muted,
        primary: accent,
        primary_hover: tuned(|p| p.accent_hover, mix(accent, toward, 0.12)),
        primary_foreground: tuned(|p| p.accent_text, ink),
        muted: tuned(|p| p.track, mix(base, text, 0.12)),
        destructive: core(|p| p.error),
        input: tuned(|p| p.input, text.opacity(0.18)),
        ring: tuned(|p| p.ring, mix(base, text, 0.5)),
        hover: tuned(|p| p.hover, text.opacity(0.05)),
        // The mark sits in the header, so it follows the header's ink.
        mark: tuned(|p| p.mark, header_text),
        font: faces.text.clone(),
        heading_font: faces.heading.clone(),
        heading_weight: FontWeight(f32::from(style.heading_weight)),
        radius: px(style.radius),
        control_radius: px(style.control_radius),
    }
}

/// WCAG relative luminance, 0 (black) to 1 (white).
fn relative_luminance(color: Hsla) -> f32 {
    let color = Rgba::from(color);
    let linear = |channel: f32| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

#[cfg(test)]
mod tests {
    use bach_pack::manifest::Look;

    use super::*;

    fn looks(look: &Look) -> Looks {
        Looks::new(&bach_pack::style::resolve(look, None), &Faces::inter(), Pictures::default(), Arrangement::default_window())
    }

    fn hex(value: u32) -> Hsla {
        rgb(value).into()
    }

    #[test]
    fn bach_keeps_its_hand_tuned_tokens() {
        let light = Looks::bach().get(Mode::Light);
        assert_eq!(light.surface, hex(0xffffff));
        assert_eq!(light.well, hex(0xf3f3f3));
        assert_eq!(light.primary_hover, hex(0x292929));
        assert_eq!(light.card.a, 0.0);
        assert_eq!(light.radius, px(12.));
        let dark = Looks::bach().get(Mode::Dark);
        assert_eq!(dark.surface, hex(0x181818));
        assert_eq!(dark.muted, hex(0x303030));
        assert_eq!(dark.mark, dark.foreground);
    }

    #[test]
    fn accents_get_readable_ink() {
        let blue = looks(&Look { accent: Some("#2563eb".into()), ..Look::default() }).get(Mode::Light);
        assert_eq!(blue.primary, hex(0x2563eb));
        assert_eq!(blue.primary_foreground, white());
        let yellow = looks(&Look { accent: Some("#facc15".into()), ..Look::default() }).get(Mode::Dark);
        assert_eq!(yellow.primary_foreground, hex(0x0f0f0f));
    }

    #[test]
    fn a_style_brings_its_gradient_card_and_corners() {
        let arcade = looks(&Look { style: Some("arcade".into()), ..Look::default() }).get(Mode::Dark);
        assert!(arcade.surface_end.is_some());
        assert!(arcade.card.a > 0.0 && arcade.card.a < 0.1);
        assert_eq!(arcade.mark, arcade.primary);
        let pixel = looks(&Look { style: Some("pixel".into()), ..Look::default() }).get(Mode::Light);
        assert_eq!(pixel.radius, px(0.));
        assert_eq!(pixel.bar_radius(px(6.)), px(0.));
        assert_eq!(pixel.checkbox_radius(), px(0.));
    }
}
