//! `[layout]`: how the setup window is arranged, sized and animated, on top
//! of its style. Every field is optional; [`Layout::resolve`] fills in the
//! window Bach has always drawn, so a config without `[layout]` looks
//! exactly as before.
//!
//! The manifest carries the developer's choices as written (`Layout`);
//! the runtime works from the resolved values (`Arrangement`).

use serde::{Deserialize, Serialize};

/// Where the logo, heading and subtitle sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Header {
    /// Above the content, the classic column.
    #[default]
    Top,
    /// In a side panel on the left, the content on the right.
    Left,
    /// In a side panel on the right.
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

/// Where the logo sits relative to the heading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogoPlace {
    #[default]
    Above,
    /// In one line with the heading, before it.
    Beside,
}

/// How the rows of choices are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rows {
    /// One card, rows divided by hairlines.
    #[default]
    Joined,
    /// Each row its own card, with a gap between.
    Separate,
}

/// Where the buttons sit in the footer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Buttons {
    #[default]
    Right,
    Left,
    Center,
    /// Sharing the whole width.
    Stretch,
}

/// Whether the main button comes after the others or before them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Primary {
    #[default]
    Last,
    First,
}

/// Where the footer line (publisher and version) goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Footer {
    /// On the same line as the buttons.
    #[default]
    Beside,
    /// On its own line above them.
    Above,
    Hidden,
}

/// How a page comes in as it replaces the last one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Animation {
    /// Fades in while rising a few points.
    #[default]
    Rise,
    /// Fades in where it is.
    Fade,
    /// Fades in while sliding in from the right.
    Slide,
    /// Fades in while dropping from above.
    Drop,
    /// Appears at once.
    None,
}

/// The developer's `[layout]`, as written. None is "the default".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Header>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_width: Option<u16>,
    /// Whether a side header gets a panel of its own colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<LogoPlace>,
    /// Top, right, bottom, left, in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<[u8; 4]>,
    /// Between the heading and the content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_size: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_size: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_padding: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<Rows>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icons: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dividers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button_height: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buttons: Option<Buttons>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<Primary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<Footer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<Animation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation_speed: Option<f32>,
}

/// The window's arrangement with every default filled in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrangement {
    pub width: f32,
    /// None: as tall as the choices need, within the screen.
    pub height: Option<f32>,
    pub header: Header,
    pub header_width: f32,
    pub panel: bool,
    pub align: Align,
    pub logo: LogoPlace,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    pub gap: f32,
    pub heading_size: f32,
    pub text_size: f32,
    pub row_padding: f32,
    pub rows: Rows,
    pub icons: bool,
    pub dividers: bool,
    pub button_height: f32,
    pub buttons: Buttons,
    pub primary: Primary,
    pub footer: Footer,
    pub animation: Animation,
    pub animation_speed: f32,
}

/// The window Bach draws without `[layout]`.
pub const DEFAULT_WIDTH: u16 = 540;
/// A window with the header at the side starts wider and lower.
pub const SIDE_WIDTH: u16 = 780;
pub const SIDE_HEIGHT: u16 = 470;
/// Top, right, bottom, left.
pub const DEFAULT_PADDING: [u8; 4] = [44, 32, 24, 32];

/// The bounds `bach check` holds each number to.
pub mod bounds {
    pub const WIDTH: (u16, u16) = (380, 1400);
    pub const HEIGHT: (u16, u16) = (340, 1000);
    pub const HEADER_WIDTH: (u16, u16) = (160, 700);
    pub const PADDING: (u8, u8) = (0, 96);
    pub const GAP: (u8, u8) = (0, 96);
    pub const HEADING_SIZE: (u8, u8) = (14, 48);
    pub const TEXT_SIZE: (u8, u8) = (11, 20);
    pub const ROW_PADDING: (u8, u8) = (6, 32);
    pub const BUTTON_HEIGHT: (u8, u8) = (24, 56);
    pub const ANIMATION_SPEED: (f32, f32) = (0.25, 4.0);
}

impl Layout {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn resolve(&self) -> Arrangement {
        let header = self.header.unwrap_or_default();
        let side = header != Header::Top;
        let width = f32::from(self.width.unwrap_or(if side { SIDE_WIDTH } else { DEFAULT_WIDTH }));
        let height = self.height.map(f32::from).or(side.then_some(f32::from(SIDE_HEIGHT)));
        let header_width = self.header_width.map(f32::from).unwrap_or((width * 0.4).round()).min(width - 220.0).max(120.0);
        Arrangement {
            width,
            height,
            header,
            header_width,
            panel: self.panel.unwrap_or(side),
            align: self.align.unwrap_or(if side { Align::Left } else { Align::Center }),
            logo: self.logo.unwrap_or_default(),
            padding: self.padding.unwrap_or(DEFAULT_PADDING).map(f32::from),
            gap: f32::from(self.gap.unwrap_or(if side { 0 } else { 28 })),
            heading_size: f32::from(self.heading_size.unwrap_or(22)),
            text_size: f32::from(self.text_size.unwrap_or(14)),
            row_padding: f32::from(self.row_padding.unwrap_or(16)),
            rows: self.rows.unwrap_or_default(),
            icons: self.icons.unwrap_or(true),
            dividers: self.dividers.unwrap_or(true),
            button_height: f32::from(self.button_height.unwrap_or(32)),
            buttons: self.buttons.unwrap_or_default(),
            primary: self.primary.unwrap_or_default(),
            footer: self.footer.unwrap_or_default(),
            animation: self.animation.unwrap_or_default(),
            animation_speed: self.animation_speed.unwrap_or(1.0).clamp(bounds::ANIMATION_SPEED.0, bounds::ANIMATION_SPEED.1),
        }
    }
}

impl Arrangement {
    /// Whether the header sits in a side panel.
    pub fn side(&self) -> bool {
        self.header != Header::Top
    }

    /// The height of one row of choices, for sizing the window to them.
    pub fn row_height(&self) -> f32 {
        let title = (self.text_size * 20.0 / 14.0).round();
        let description = ((self.text_size - 1.0) * 18.0 / 13.0).round();
        2.0 * self.row_padding + title + 2.0 + description + 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_layout_is_the_window_bach_always_drew() {
        let plain = Layout::default().resolve();
        assert_eq!((plain.width, plain.height, plain.header), (540.0, None, Header::Top));
        assert_eq!(plain.padding, [44.0, 32.0, 24.0, 32.0]);
        assert_eq!(plain.align, Align::Center);
        assert_eq!(plain.row_height(), 73.0, "the row height the window always grew by");
        assert_eq!((plain.animation, plain.animation_speed), (Animation::Rise, 1.0));
    }

    #[test]
    fn a_side_header_changes_its_defaults() {
        let side = Layout { header: Some(Header::Left), ..Layout::default() }.resolve();
        assert_eq!((side.width, side.height), (780.0, Some(470.0)));
        assert_eq!(side.header_width, 312.0);
        assert!(side.panel);
        assert_eq!(side.align, Align::Left);
        let narrow = Layout { header: Some(Header::Right), width: Some(400), header_width: Some(600), ..Layout::default() }.resolve();
        assert_eq!(narrow.header_width, 180.0, "the content keeps room");
    }

    #[test]
    fn the_manifest_keeps_only_what_was_set() {
        let layout = Layout { buttons: Some(Buttons::Stretch), padding: Some([8, 8, 8, 8]), ..Layout::default() };
        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(json, r#"{"padding":[8,8,8,8],"buttons":"stretch"}"#);
        assert_eq!(serde_json::from_str::<Layout>(&json).unwrap(), layout);
    }
}
