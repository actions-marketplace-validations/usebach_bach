//! `[layout]`: how the setup window is arranged, sized and animated, on top
//! of its style. Every field is optional; without `[layout]` the window is
//! the classic column it always was.
//!
//! A layout can start from a preset (`preset = "showcase"`, see
//! `presets.rs`) and change any field of it. The manifest carries the
//! developer's choices as written (`Layout`); the runtime works from the
//! resolved values (`Arrangement`).

use serde::{Deserialize, Serialize};

mod presets;

pub use presets::{closest, preset, LayoutPreset, LAYOUT_PRESETS};

/// A panel beside or above the choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Panel {
    /// No panel: the classic column.
    #[default]
    None,
    Left,
    Right,
    /// A banner across the top.
    Top,
}

/// Where the logo, heading and subtitle sit when there is a panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeaderPlace {
    /// Above the choices; the panel shows the picture (or the logo, large).
    #[default]
    Content,
    /// On the panel, over its colour or picture.
    Panel,
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
    /// No card at all: the rows sit straight on the window, divided by
    /// hairlines.
    Plain,
}

/// What the choices and buttons sit on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    /// Straight on the window.
    #[default]
    Plain,
    /// A card floating over the window's colour or picture.
    Card,
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

/// The developer's `[layout]`, as written (a preset's fields filled in
/// under the developer's own). None is "the default".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// The preset it started from, for `bach inspect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<Panel>,
    /// The panel's width (left or right) or height (top), in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_size: Option<u16>,
    /// Space around the panel; with a radius it floats as a card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_inset: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_radius: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<HeaderPlace>,
    /// The widest the choices and buttons get, centred in their room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_width: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<Surface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<LogoPlace>,
    /// Top, right, bottom, left, in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<[u8; 4]>,
    /// Between the heading and the choices.
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
    /// One or two columns of separate rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u8>,
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
    pub panel: Panel,
    pub panel_size: f32,
    pub panel_inset: f32,
    pub panel_radius: f32,
    pub header: HeaderPlace,
    pub content_width: Option<f32>,
    pub surface: Surface,
    pub align: Align,
    pub logo: LogoPlace,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    pub gap: f32,
    pub heading_size: f32,
    pub text_size: f32,
    pub row_padding: f32,
    pub rows: Rows,
    pub columns: u8,
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
/// A window with a side panel starts wider, and at least this tall.
pub const SIDE_WIDTH: u16 = 880;
pub const SIDE_HEIGHT: u16 = 520;
/// A window with a banner starts a little wider than the classic one.
pub const BANNER_WIDTH: u16 = 600;
/// Top, right, bottom, left.
pub const DEFAULT_PADDING: [u8; 4] = [44, 32, 24, 32];

/// The bounds `bach check` holds each number to.
pub mod bounds {
    pub const WIDTH: (u16, u16) = (380, 1400);
    pub const HEIGHT: (u16, u16) = (340, 1000);
    pub const PANEL_SIZE: (u16, u16) = (120, 800);
    pub const PANEL_INSET: (u8, u8) = (0, 48);
    pub const PANEL_RADIUS: (u8, u8) = (0, 48);
    pub const CONTENT_WIDTH: (u16, u16) = (260, 1200);
    pub const PADDING: (u8, u8) = (0, 96);
    pub const GAP: (u8, u8) = (0, 96);
    pub const HEADING_SIZE: (u8, u8) = (14, 48);
    pub const TEXT_SIZE: (u8, u8) = (11, 20);
    pub const ROW_PADDING: (u8, u8) = (6, 32);
    pub const COLUMNS: (u8, u8) = (1, 2);
    pub const BUTTON_HEIGHT: (u8, u8) = (24, 56);
    pub const ANIMATION_SPEED: (f32, f32) = (0.25, 4.0);
}

impl Layout {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// `own`'s fields where it sets them, this layout's elsewhere.
    pub fn with(mut self, own: &Layout) -> Self {
        macro_rules! take {
            ($($field:ident),*) => { $( if own.$field.is_some() { self.$field = own.$field.clone(); } )* };
        }
        take!(preset, width, height, panel, panel_size, panel_inset, panel_radius, header, content_width, surface, align, logo, padding);
        take!(gap, heading_size, text_size, row_padding, rows, columns, icons, dividers, button_height, buttons, primary, footer);
        take!(animation, animation_speed);
        self
    }

    pub fn resolve(&self) -> Arrangement {
        let panel = self.panel.unwrap_or_default();
        let side = matches!(panel, Panel::Left | Panel::Right);
        let header = if panel == Panel::None { HeaderPlace::Content } else { self.header.unwrap_or_default() };
        let width = f32::from(self.width.unwrap_or(match panel {
            Panel::None => DEFAULT_WIDTH,
            Panel::Top => BANNER_WIDTH,
            _ => SIDE_WIDTH,
        }));
        let panel_size = match (panel, self.panel_size) {
            (Panel::None, _) => 0.0,
            (Panel::Top, size) => f32::from(size.unwrap_or(190)),
            (_, Some(size)) => f32::from(size).min(width - 300.0).max(120.0),
            (_, None) => (width * if header == HeaderPlace::Panel { 0.4 } else { 0.46 }).round(),
        };
        let header_in_panel = panel != Panel::None && header == HeaderPlace::Panel;
        Arrangement {
            width,
            height: self.height.map(f32::from),
            panel,
            panel_size,
            panel_inset: f32::from(self.panel_inset.unwrap_or(0)),
            panel_radius: f32::from(self.panel_radius.unwrap_or(0)),
            header,
            content_width: self.content_width.map(f32::from),
            surface: self.surface.unwrap_or_default(),
            align: self.align.unwrap_or(if panel == Panel::None { Align::Center } else { Align::Left }),
            logo: self.logo.unwrap_or_default(),
            padding: self.padding.unwrap_or(DEFAULT_PADDING).map(f32::from),
            gap: f32::from(self.gap.unwrap_or(if side && header_in_panel { 0 } else { 28 })),
            heading_size: f32::from(self.heading_size.unwrap_or(22)),
            text_size: f32::from(self.text_size.unwrap_or(14)),
            row_padding: f32::from(self.row_padding.unwrap_or(16)),
            rows: self.rows.unwrap_or_default(),
            columns: self.columns.unwrap_or(1).clamp(bounds::COLUMNS.0, bounds::COLUMNS.1),
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
    /// The window without `[layout]`.
    pub fn default_window() -> Self {
        Layout::default().resolve()
    }

    /// Whether a panel stands beside the choices.
    pub fn side(&self) -> bool {
        matches!(self.panel, Panel::Left | Panel::Right)
    }

    /// Whether the logo, heading and subtitle sit on the panel.
    pub fn header_on_panel(&self) -> bool {
        self.panel != Panel::None && self.header == HeaderPlace::Panel
    }

    /// Whether separate rows are laid out in two columns.
    pub fn two_columns(&self) -> bool {
        self.columns == 2 && self.rows == Rows::Separate
    }

    /// The height of one row of choices, for sizing the window to them.
    pub fn row_height(&self) -> f32 {
        let title = (self.text_size * 20.0 / 14.0).round();
        let description = ((self.text_size - 1.0) * 18.0 / 13.0).round();
        2.0 * self.row_padding + title + 2.0 + description + 1.0
    }
}

#[cfg(test)]
mod tests;
