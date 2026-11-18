//! Styles for apps and tools.

use super::{a, c, BAHNSCHRIFT, DISPLAY, MONOSPACE, SANS, SEGOE_DISPLAY, SEGOE_TEXT};
use crate::manifest::Theme;
use crate::style::{Palette, Preset};

pub const BACH: Preset = Preset {
    name: "bach",
    description: "Bach's own look: white or graphite, black buttons, Inter.",
    suits: "any app; the default",
    theme: None,
    fonts: SANS,
    heading_fonts: DISPLAY,
    heading_weight: 500,
    accent_mark: false,
    radius: 12,
    control_radius: 12,
    light: Palette {
        background: c(0xffffff),
        text: c(0x141414),
        muted_text: c(0x696969),
        accent: c(0x0f0f0f),
        accent_text: c(0xfafafa),
        accent_hover: c(0x292929),
        border: a(0x00000012),
        error: c(0xe7000b),
        well: c(0xf3f3f3),
        track: c(0xe1e1e1),
        input: c(0xdedede),
        ring: c(0x808080),
        hover: a(0x0000000d),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x181818),
        text: c(0xf0f0f0),
        muted_text: c(0xa4a4a4),
        accent: c(0xdedede),
        accent_text: c(0x0f0f0f),
        accent_hover: c(0xeeeeee),
        border: a(0xffffff0f),
        error: c(0xff6467),
        well: c(0x242424),
        track: c(0x303030),
        input: a(0xffffff26),
        ring: c(0x747474),
        hover: a(0xffffff0f),
        ..Palette::EMPTY
    },
};

pub const FLUENT: Preset = Preset {
    name: "fluent",
    description: "Like part of Windows 11: Segoe UI, a soft grey window, white cards, Windows blue.",
    suits: ".NET, WinUI, WPF, Qt and Tauri apps that should feel native",
    theme: None,
    fonts: SEGOE_TEXT,
    heading_fonts: SEGOE_DISPLAY,
    heading_weight: 600,
    accent_mark: false,
    radius: 8,
    control_radius: 4,
    light: Palette {
        background: c(0xf3f3f3),
        card: c(0xfbfbfb),
        text: c(0x1b1b1b),
        muted_text: c(0x5c5c5c),
        accent: c(0x005fb8),
        accent_text: c(0xffffff),
        border: a(0x00000014),
        error: c(0xc42b1c),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x202020),
        card: c(0x2b2b2b),
        text: c(0xffffff),
        muted_text: c(0xcfcfcf),
        accent: c(0x60cdff),
        accent_text: c(0x000000),
        border: a(0xffffff14),
        error: c(0xff99a4),
        ..Palette::EMPTY
    },
};

pub const AURORA: Preset = Preset {
    name: "aurora",
    description: "A soft violet-to-blue wash, frosted cards and pill buttons.",
    suits: "Tauri, Electron, Wails and Flutter desktop apps",
    theme: None,
    fonts: SANS,
    heading_fonts: DISPLAY,
    heading_weight: 500,
    accent_mark: true,
    radius: 16,
    control_radius: 16,
    light: Palette {
        background: c(0xf6f3ff),
        background_end: c(0xeaf3ff),
        card: a(0xffffffc7),
        text: c(0x17152b),
        muted_text: c(0x5d5a78),
        accent: c(0x6d5dfc),
        border: a(0x1e1b4b17),
        error: c(0xe5484d),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x1c1733),
        background_end: c(0x0d1526),
        card: a(0xffffff0b),
        text: c(0xf3f1ff),
        muted_text: c(0xaaa6c8),
        accent: c(0x8b80ff),
        accent_text: c(0x120d2b),
        border: a(0xffffff17),
        error: c(0xff6b7a),
        ..Palette::EMPTY
    },
};

pub const TERMINAL: Preset = Preset {
    name: "terminal",
    description: "A monospaced face, near-black, square-ish edges and a green prompt.",
    suits: "command-line tools and developer utilities",
    theme: Some(Theme::Dark),
    fonts: MONOSPACE,
    heading_fonts: MONOSPACE,
    heading_weight: 600,
    accent_mark: true,
    radius: 4,
    control_radius: 4,
    light: Palette {
        background: c(0xfafaf7),
        text: c(0x1c1f1d),
        muted_text: c(0x5b635d),
        accent: c(0x15803d),
        accent_text: c(0xffffff),
        border: a(0x0000001c),
        error: c(0xd92d20),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x0c0f0d),
        text: c(0xe4ebe6),
        muted_text: c(0x8b968f),
        accent: c(0x3ddc84),
        accent_text: c(0x04130a),
        border: a(0xffffff1c),
        error: c(0xff5f56),
        ..Palette::EMPTY
    },
};

pub const MIDNIGHT: Preset = Preset {
    name: "midnight",
    description: "Deep navy and electric blue, crisp and technical.",
    suits: "developer tools, servers and anything technical",
    theme: None,
    fonts: SANS,
    heading_fonts: SANS,
    heading_weight: 600,
    accent_mark: false,
    radius: 10,
    control_radius: 8,
    light: Palette {
        background: c(0xf5f7fc),
        card: c(0xffffff),
        text: c(0x0b1020),
        muted_text: c(0x56617c),
        accent: c(0x2563eb),
        accent_text: c(0xffffff),
        border: a(0x0b10201a),
        error: c(0xe5484d),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x0b1020),
        card: c(0x111830),
        text: c(0xe7ecf7),
        muted_text: c(0x8d98b3),
        accent: c(0x2f66e8),
        accent_text: c(0xffffff),
        border: a(0xffffff14),
        error: c(0xff6b81),
        ..Palette::EMPTY
    },
};

pub const PAPER: Preset = Preset {
    name: "paper",
    description: "Warm paper, a serif heading and a terracotta accent.",
    suits: "writing, reading and note-taking apps",
    theme: None,
    fonts: SANS,
    heading_fonts: &["Georgia"],
    heading_weight: 400,
    accent_mark: false,
    radius: 10,
    control_radius: 8,
    light: Palette {
        background: c(0xfaf7f2),
        card: c(0xfffdf9),
        text: c(0x2b2722),
        muted_text: c(0x6c645a),
        accent: c(0xb4532a),
        accent_text: c(0xffffff),
        border: a(0x2b27221c),
        error: c(0xc0392b),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x1f1c18),
        card: c(0x27231e),
        text: c(0xf3ede4),
        muted_text: c(0xb0a698),
        accent: c(0xe08a5f),
        accent_text: c(0x1f1c18),
        border: a(0xffffff14),
        error: c(0xff7a6b),
        ..Palette::EMPTY
    },
};

pub const EMBER: Preset = Preset {
    name: "ember",
    description: "Charcoal with a warm amber glow.",
    suits: "audio, video, photo and other creative tools",
    theme: Some(Theme::Dark),
    fonts: SANS,
    heading_fonts: DISPLAY,
    heading_weight: 500,
    accent_mark: true,
    radius: 12,
    control_radius: 10,
    light: Palette {
        background: c(0xfffaf4),
        card: c(0xffffff),
        text: c(0x2a1d14),
        muted_text: c(0x76624f),
        accent: c(0xb85a0b),
        accent_text: c(0xffffff),
        border: a(0x2a1d141c),
        error: c(0xd92d20),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x1f1915),
        background_end: c(0x110d0b),
        card: a(0xffffff09),
        text: c(0xf7efe8),
        muted_text: c(0xaa9a8c),
        accent: c(0xffb02e),
        accent_text: c(0x1a1206),
        border: a(0xffffff14),
        error: c(0xff6b5b),
        ..Palette::EMPTY
    },
};

pub const FOREST: Preset = Preset {
    name: "forest",
    description: "Quiet greens, white cards and pill buttons.",
    suits: "productivity, health and outdoor apps",
    theme: None,
    fonts: SANS,
    heading_fonts: DISPLAY,
    heading_weight: 500,
    accent_mark: true,
    radius: 12,
    control_radius: 16,
    light: Palette {
        background: c(0xf4f7f2),
        card: c(0xffffff),
        text: c(0x17241b),
        muted_text: c(0x56665a),
        accent: c(0x2f7d4f),
        accent_text: c(0xffffff),
        border: a(0x17241b17),
        error: c(0xc2412d),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x121a15),
        card: c(0x18221b),
        text: c(0xe8f0ea),
        muted_text: c(0x94a698),
        accent: c(0x5bc287),
        accent_text: c(0x07140c),
        border: a(0xffffff14),
        error: c(0xff7b6b),
        ..Palette::EMPTY
    },
};

pub const BLOSSOM: Preset = Preset {
    name: "blossom",
    description: "Soft pink, generous corners and pill buttons.",
    suits: "friendly consumer apps",
    theme: None,
    fonts: SANS,
    heading_fonts: SANS,
    heading_weight: 600,
    accent_mark: true,
    radius: 18,
    control_radius: 16,
    light: Palette {
        background: c(0xfff6f9),
        background_end: c(0xfcedf4),
        card: c(0xffffff),
        text: c(0x2d1520),
        muted_text: c(0x7a5767),
        accent: c(0xc92f62),
        accent_text: c(0xffffff),
        border: a(0x2d152017),
        error: c(0xc92a2a),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x24141d),
        background_end: c(0x150c11),
        card: a(0xffffff0b),
        text: c(0xfdeef3),
        muted_text: c(0xc49cad),
        accent: c(0xff7aa2),
        accent_text: c(0x2a0b17),
        border: a(0xffffff17),
        error: c(0xff7b7b),
        ..Palette::EMPTY
    },
};

pub const MONO: Preset = Preset {
    name: "mono",
    description: "Pure black and white, hard edges and a heavy heading.",
    suits: "utilities that should look strict and plain",
    theme: None,
    fonts: SANS,
    heading_fonts: BAHNSCHRIFT,
    heading_weight: 700,
    accent_mark: false,
    radius: 0,
    control_radius: 0,
    light: Palette {
        background: c(0xffffff),
        text: c(0x000000),
        muted_text: c(0x525252),
        accent: c(0x000000),
        accent_text: c(0xffffff),
        accent_hover: c(0x2b2b2b),
        border: c(0x000000),
        error: c(0xd70000),
        well: c(0xefefef),
        track: c(0xe5e5e5),
        input: c(0x000000),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x000000),
        text: c(0xffffff),
        muted_text: c(0xa3a3a3),
        accent: c(0xffffff),
        accent_text: c(0x000000),
        accent_hover: c(0xdcdcdc),
        border: c(0xffffff),
        error: c(0xff4d4d),
        well: c(0x1a1a1a),
        track: c(0x262626),
        input: c(0xffffff),
        ..Palette::EMPTY
    },
};

pub const FROST: Preset = Preset {
    name: "frost",
    description: "Cool blue-grey, calm and quiet.",
    suits: "system utilities, backup and sync tools",
    theme: None,
    fonts: SANS,
    heading_fonts: DISPLAY,
    heading_weight: 500,
    accent_mark: false,
    radius: 10,
    control_radius: 8,
    light: Palette {
        background: c(0xedf0f5),
        card: c(0xfafbfc),
        text: c(0x1f2633),
        muted_text: c(0x566070),
        accent: c(0x4c6a92),
        accent_text: c(0xffffff),
        border: a(0x1f26331a),
        error: c(0xbf3b4b),
        ..Palette::EMPTY
    },
    dark: Palette {
        background: c(0x1e232c),
        card: c(0x252b36),
        text: c(0xe5e9f0),
        muted_text: c(0x9aa3b2),
        accent: c(0x8cc4d6),
        accent_text: c(0x10161c),
        border: a(0xffffff14),
        error: c(0xec6f7d),
        ..Palette::EMPTY
    },
};
