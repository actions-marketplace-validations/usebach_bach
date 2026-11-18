//! Styles for games.

use super::{a, c, BAHNSCHRIFT, MONOSPACE, SANS};
use crate::manifest::Theme;
use crate::style::{Palette, Preset};

pub const ARCADE: Preset = Preset {
    name: "arcade",
    description: "Hot pink on deep violet, with a bold condensed heading.",
    suits: "indie and web-tech games (Electron, Phaser, LÖVE, MonoGame)",
    theme: Some(Theme::Dark),
    fonts: SANS,
    heading_fonts: BAHNSCHRIFT,
    heading_weight: 700,
    accent_mark: true,
    radius: 12,
    control_radius: 10,
    light: Palette {
        background: c(0xfff3fa),
        background_end: c(0xf1ebff),
        card: a(0xffffffd9),
        text: c(0x2a0f3d),
        muted_text: c(0x6b5780),
        accent: c(0xe0186a),
        border: a(0x2a0f3d1c),
        error: c(0xd92d20),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x1d0b36),
        background_end: c(0x0a0416),
        card: a(0xffffff0b),
        text: c(0xfbf4ff),
        muted_text: c(0xbba7d6),
        accent: c(0xe11d6a),
        accent_text: c(0xffffff),
        border: a(0xffffff1c),
        error: c(0xff6b6b),
        ..Palette::EMPTY
    },
};

pub const FORGE: Preset = Preset {
    name: "forge",
    description: "Brushed steel and molten orange, with tight corners.",
    suits: "3D games made with Unity or Unreal",
    theme: Some(Theme::Dark),
    fonts: SANS,
    heading_fonts: BAHNSCHRIFT,
    heading_weight: 600,
    accent_mark: true,
    radius: 6,
    control_radius: 6,
    light: Palette {
        background: c(0xf3f4f6),
        background_end: c(0xe8eaed),
        card: c(0xffffff),
        text: c(0x16181c),
        muted_text: c(0x5b616b),
        accent: c(0xc2410c),
        accent_text: c(0xffffff),
        border: a(0x0000001c),
        error: c(0xd92d20),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x1d1f23),
        background_end: c(0x101113),
        card: c(0x22252a),
        text: c(0xeceef1),
        muted_text: c(0x9399a3),
        accent: c(0xff7a1a),
        accent_text: c(0x1a0d02),
        border: a(0xffffff14),
        error: c(0xff5d5d),
        ..Palette::EMPTY
    },
};

pub const LAGOON: Preset = Preset {
    name: "lagoon",
    description: "Calm sea blue on a pale wash, rounded and friendly.",
    suits: "games and tools made with Godot",
    theme: None,
    fonts: SANS,
    heading_fonts: SANS,
    heading_weight: 600,
    accent_mark: true,
    radius: 14,
    control_radius: 10,
    light: Palette {
        background: c(0xf3f9fd),
        background_end: c(0xe6f1f9),
        card: c(0xffffff),
        text: c(0x0f2233),
        muted_text: c(0x52697c),
        accent: c(0x336f9e),
        accent_text: c(0xffffff),
        border: a(0x0f22331a),
        error: c(0xd64545),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x1b2b3a),
        background_end: c(0x101a24),
        card: a(0xffffff0b),
        text: c(0xe9f2f9),
        muted_text: c(0x93aabd),
        accent: c(0x6fb2e6),
        accent_text: c(0x06121c),
        border: a(0xffffff17),
        error: c(0xff7b7b),
        ..Palette::EMPTY
    },
};

pub const PIXEL: Preset = Preset {
    name: "pixel",
    description: "Square corners, a monospaced face and a warm retro palette.",
    suits: "pixel-art and retro games",
    theme: Some(Theme::Dark),
    fonts: MONOSPACE,
    heading_fonts: MONOSPACE,
    heading_weight: 700,
    accent_mark: true,
    radius: 0,
    control_radius: 0,
    light: Palette {
        background: c(0xfff8e7),
        card: c(0xfffdf6),
        text: c(0x2b2340),
        muted_text: c(0x655b7a),
        accent: c(0xc73a24),
        accent_text: c(0xffffff),
        border: a(0x2b234038),
        error: c(0xa61b1b),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x1e1b2e),
        card: c(0x27233c),
        text: c(0xfff8e7),
        muted_text: c(0xb4abc8),
        accent: c(0xffcd3c),
        accent_text: c(0x1e1b2e),
        border: a(0xfff8e733),
        error: c(0xff6b57),
        ..Palette::EMPTY
    },
};
