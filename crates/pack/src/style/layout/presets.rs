//! The built-in layouts: a starting point for `[layout] preset`, each a
//! set of fields the developer can still change one by one. `classic` is
//! the window without `[layout]`.

use super::{Align, Animation, Buttons, Footer, HeaderPlace, Layout, LogoPlace, Panel, Rows, Surface};

/// A built-in layout.
#[derive(Debug)]
pub struct LayoutPreset {
    /// What `[layout] preset` names it by.
    pub name: &'static str,
    /// One line: what it looks like.
    pub description: &'static str,
    /// What it suits, for `bach layouts`.
    pub suits: &'static str,
    /// Whether it shows a picture best: `[layout] panel_image`, or
    /// `[style] background_image` for `floating`. It works without one.
    pub picture: Option<&'static str>,
}

pub static LAYOUT_PRESETS: [LayoutPreset; 11] = [
    LayoutPreset {
        name: "classic",
        description: "One calm centred column: logo, heading, the card of choices, buttons. The default.",
        suits: "any app",
        picture: None,
    },
    LayoutPreset {
        name: "showcase",
        description: "The choices on the left, a large rounded picture on the right, floating in the window.",
        suits: "apps and games with a strong picture",
        picture: Some("panel_image"),
    },
    LayoutPreset {
        name: "spotlight",
        description: "Key art down the left with the heading on it, the choices as tiles beside it.",
        suits: "games and launchers",
        picture: Some("panel_image"),
    },
    LayoutPreset {
        name: "banner",
        description: "A picture across the top with the heading on it, the choices below.",
        suits: "games, creative tools and anything with a wide picture",
        picture: Some("panel_image"),
    },
    LayoutPreset {
        name: "split",
        description: "The heading in a coloured panel on the left, the choices on the right.",
        suits: "business apps and tools",
        picture: None,
    },
    LayoutPreset {
        name: "editorial",
        description: "A picture edge to edge on the right, the heading set large on the left.",
        suits: "creative, photo and writing apps",
        picture: Some("panel_image"),
    },
    LayoutPreset {
        name: "floating",
        description: "Everything on one card floating over the window's picture or colour.",
        suits: "games and apps with a background picture",
        picture: Some("background_image"),
    },
    LayoutPreset {
        name: "native",
        description: "Like a Windows dialog: aligned left, the logo beside the heading, the main button first.",
        suits: "utilities and business software",
        picture: None,
    },
    LayoutPreset {
        name: "compact",
        description: "A narrow, tall window with tiles and full-width buttons.",
        suits: "small tools and consumer apps",
        picture: None,
    },
    LayoutPreset {
        name: "tiles",
        description: "A wide window with the choices as tiles in two columns.",
        suits: "apps with many options or components",
        picture: None,
    },
    LayoutPreset {
        name: "minimal",
        description: "Aligned left, no icons or dividers, no footer line: only what matters.",
        suits: "developer tools and plain utilities",
        picture: None,
    },
];

/// The built-in layout called `name` (any case), as fields to start from.
pub fn preset(name: &str) -> Option<(&'static LayoutPreset, Layout)> {
    let name = name.trim();
    let found = LAYOUT_PRESETS.iter().find(|preset| preset.name.eq_ignore_ascii_case(name))?;
    let base = Layout { preset: Some(found.name.to_string()), ..Layout::default() };
    let layout = match found.name {
        "showcase" => Layout {
            width: Some(940),
            panel: Some(Panel::Right),
            panel_inset: Some(14),
            panel_radius: Some(26),
            header: Some(HeaderPlace::Content),
            content_width: Some(400),
            align: Some(Align::Left),
            padding: Some([48, 44, 32, 44]),
            gap: Some(24),
            heading_size: Some(30),
            rows: Some(Rows::Separate),
            row_padding: Some(13),
            buttons: Some(Buttons::Stretch),
            button_height: Some(38),
            footer: Some(Footer::Above),
            animation: Some(Animation::Fade),
            ..base
        },
        "spotlight" => Layout {
            panel: Some(Panel::Left),
            header: Some(HeaderPlace::Panel),
            heading_size: Some(30),
            rows: Some(Rows::Separate),
            buttons: Some(Buttons::Stretch),
            button_height: Some(38),
            footer: Some(Footer::Above),
            animation: Some(Animation::Slide),
            ..base
        },
        "banner" => Layout {
            panel: Some(Panel::Top),
            header: Some(HeaderPlace::Panel),
            panel_size: Some(200),
            align: Some(Align::Left),
            padding: Some([28, 32, 24, 32]),
            gap: Some(0),
            heading_size: Some(28),
            animation: Some(Animation::Drop),
            ..base
        },
        "split" => Layout {
            width: Some(820),
            panel: Some(Panel::Left),
            header: Some(HeaderPlace::Panel),
            heading_size: Some(26),
            animation: Some(Animation::Slide),
            ..base
        },
        "editorial" => Layout {
            width: Some(900),
            panel: Some(Panel::Right),
            header: Some(HeaderPlace::Content),
            panel_size: Some(380),
            align: Some(Align::Left),
            padding: Some([52, 40, 32, 40]),
            heading_size: Some(34),
            gap: Some(26),
            icons: Some(false),
            buttons: Some(Buttons::Left),
            primary: Some(super::Primary::First),
            footer: Some(Footer::Above),
            animation: Some(Animation::Fade),
            ..base
        },
        "floating" => Layout {
            width: Some(760),
            surface: Some(Surface::Card),
            content_width: Some(480),
            padding: Some([36, 36, 36, 36]),
            heading_size: Some(26),
            ..base
        },
        "native" => Layout {
            align: Some(Align::Left),
            logo: Some(LogoPlace::Beside),
            padding: Some([32, 36, 28, 36]),
            gap: Some(22),
            heading_size: Some(24),
            icons: Some(false),
            buttons: Some(Buttons::Left),
            primary: Some(super::Primary::First),
            footer: Some(Footer::Above),
            animation: Some(Animation::Fade),
            ..base
        },
        "compact" => Layout {
            width: Some(420),
            height: Some(700),
            padding: Some([48, 28, 28, 28]),
            text_size: Some(15),
            rows: Some(Rows::Separate),
            row_padding: Some(14),
            buttons: Some(Buttons::Stretch),
            button_height: Some(40),
            footer: Some(Footer::Hidden),
            animation: Some(Animation::Drop),
            ..base
        },
        "tiles" => Layout {
            width: Some(780),
            align: Some(Align::Left),
            logo: Some(LogoPlace::Beside),
            padding: Some([40, 36, 28, 36]),
            heading_size: Some(26),
            rows: Some(Rows::Separate),
            columns: Some(2),
            ..base
        },
        "minimal" => Layout {
            align: Some(Align::Left),
            padding: Some([52, 44, 32, 44]),
            gap: Some(32),
            heading_size: Some(28),
            icons: Some(false),
            dividers: Some(false),
            footer: Some(Footer::Hidden),
            animation: Some(Animation::Fade),
            ..base
        },
        _ => base,
    };
    Some((found, layout))
}

/// The built-in layout whose name is closest to a misspelt one.
pub fn closest(name: &str) -> Option<&'static str> {
    let name = name.trim().to_ascii_lowercase();
    LAYOUT_PRESETS
        .iter()
        .map(|preset| (preset.name, crate::style::edit_distance(&name, preset.name)))
        .filter(|(_, distance)| *distance <= 2)
        .min_by_key(|(_, distance)| *distance)
        .map(|(name, _)| name)
}
