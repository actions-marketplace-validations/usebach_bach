//! The built-in styles. Each has a light and a dark palette that hold up
//! on their own (text reads at WCAG AA on its background), fonts every
//! Windows 10 and 11 PC has, and Inter whenever one is missing.
//!
//! The first entry is the default: Bach's own design system, with every
//! token tuned by hand. `apps.rs` holds the styles for apps and tools,
//! `games.rs` those for games.

use super::{Color, Preset, INTER, INTER_DISPLAY};

mod apps;
mod games;

use apps::*;
use games::*;

/// An opaque colour, `0xRRGGBB`.
pub(super) const fn c(value: u32) -> Option<Color> {
    Some(Color::rgb(value))
}

/// A colour with alpha, `0xRRGGBBAA`.
pub(super) const fn a(value: u32) -> Option<Color> {
    Some(Color(value))
}

pub(super) const MONOSPACE: &[&str] = &["Cascadia Mono", "Consolas"];
pub(super) const BAHNSCHRIFT: &[&str] = &["Bahnschrift"];
pub(super) const SANS: &[&str] = &[INTER];
pub(super) const DISPLAY: &[&str] = &[INTER_DISPLAY];

pub static PRESETS: [Preset; 14] =
    [BACH, AURORA, TERMINAL, ARCADE, FORGE, LAGOON, PIXEL, MIDNIGHT, PAPER, EMBER, FOREST, BLOSSOM, MONO, FROST];
