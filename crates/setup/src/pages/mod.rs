//! The setup window's pages. One calm column: the mark, a heading and a
//! line of copy, then the choices in a bordered card, and the buttons along
//! the bottom. Each page rises in as it replaces the last, so nothing jumps.
//!
//! Access key and license (when there are any) → Ready → Working (the mark conducts while
//! a bar fills) → Done, or Failed with a readable sentence and "Try again".
//!
//! - `license.rs`: the license (smooth scrolling, a scrollbar, selectable text).
//! - `ready.rs`: the choices.
//! - `finish.rs`: working, done and failed.

mod access;
mod finish;
mod license;
mod ready;
mod requirement;

use bach_pack::i18n::Key;
use gpui::{
    div, img, linear_color_stop, linear_gradient, px, solid_background, AnyElement, Context, ElementId, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ObjectFit, ParentElement, Render, SharedString, Styled, StyledImage, Window,
};

use crate::assets::LOGO_PATH;
use crate::package::Logo;
use bach_pack::style::layout::{Align, Buttons, Footer, Header, LogoPlace, Primary};

use crate::theme::{ActiveTheme, Theme};
use crate::ui::motion::page_enter;
use crate::ui::prelude::*;
use crate::view::{SetupView, Stage};

pub use ready::row_count;

/// The mark and the page icons share one box, so pages swap in place.
const MARK_SLOT: f32 = 56.;

impl Render for SetupView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // The scrollbar needs the card's size, known after its first paint.
        if matches!(self.stage, Stage::Ready) && self.choices_scroll.bounds().size.height < px(1.) {
            window.request_animation_frame();
        }
        if matches!(self.stage, Stage::License) {
            self.license_tick(window);
        }
        let (kind, body) = match self.stage.clone() {
            Stage::Requirement(index) => ("requirement", self.requirement_page(index, cx)),
            Stage::Access => ("access", self.access_page(cx)),
            Stage::License => ("license", self.license_page(cx)),
            Stage::Ready => ("ready", self.ready_page(cx)),
            Stage::Working { progress, label } => ("working", self.working_page(progress, label, &theme, cx)),
            Stage::Done => ("done", self.done_page(cx)),
            Stage::Failed(message) => ("failed", self.failed_page(message, cx)),
        };
        let page_id = ElementId::Name(format!("setup-page-{kind}-{}", self.attempt).into());
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.on_key(event, window, cx)))
            .child(frame(Appear::new(page_id, page_enter(&theme.layout), body).flex_1().min_h_0().flex().flex_col(), &theme))
    }
}

impl SetupView {
    /// The footer's line: publisher and version (or the developer's own
    /// line), and a preview's tag.
    fn meta(&self) -> SharedString {
        let line = self.texts.get(Key::footer);
        if self.preview {
            format!("{line} · {}", self.texts.get(Key::preview)).into()
        } else {
            line.into()
        }
    }

    /// The app's logo, or the Bach mark (conducting while setup works), or
    /// nothing when the app asked for no logo.
    fn mark(&self, working: bool, theme: &Theme) -> Option<AnyElement> {
        if self.package.manifest.look.hide_logo {
            return None;
        }
        let look = &self.package.manifest.look;
        let height = look.logo_height.map_or(MARK_SLOT, f32::from);
        // A wide logo keeps its shape, up to the width of the column.
        let width = (height * look.logo_aspect.unwrap_or(1.0)).clamp(height * 0.25, 420.);
        Some(match &self.package.logo {
            Some(Logo::Svg(_)) => gpui::svg().path(LOGO_PATH).size(px(height)).text_color(theme.mark).into_any_element(),
            Some(Logo::Png(image)) => img(image.clone()).w(px(width)).h(px(height)).object_fit(ObjectFit::Contain).into_any_element(),
            None if working => BachMark::new("setup-mark").size(68.).slot(MARK_SLOT).into_any_element(),
            None => page_icon("bach", theme.mark),
        })
    }
}

/// The window's background and type, shared by every view: the style's
/// colour or gradient and its picture over that. The page lays out its own
/// padding, so a side panel can reach the window's edges.
pub fn frame(body: impl IntoElement, theme: &Theme) -> impl IntoElement {
    let backdrop = match theme.surface_end {
        Some(end) => linear_gradient(180., linear_color_stop(theme.surface, 0.), linear_color_stop(end, 1.)),
        None => solid_background(theme.surface),
    };
    let picture = theme
        .picture
        .clone()
        .map(|(picture, opacity)| img(picture).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Cover).opacity(opacity));
    div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .bg(backdrop)
        .text_color(theme.foreground)
        .font_family(theme.font.clone())
        .children(picture)
        .child(div().relative().size_full().flex().flex_col().child(body))
}

/// One page: mark, heading, copy line, the content and the footer.
pub struct PageLayout {
    /// Held in the middle, for the pages with nothing to choose.
    pub centered: bool,
    pub mark: Option<AnyElement>,
    pub title: SharedString,
    pub subtitle: Option<AnyElement>,
    pub content: Option<AnyElement>,
    /// The content takes all the room left (a long license).
    pub grow: bool,
    pub footer: AnyElement,
}

impl PageLayout {
    pub fn new(title: impl Into<SharedString>, footer: AnyElement) -> Self {
        Self { centered: false, mark: None, title: title.into(), subtitle: None, content: None, grow: false, footer }
    }

    /// The page in the window's arrangement: one column with the header on
    /// top, or the header in a side panel and the rest beside it.
    pub fn render(self, theme: &Theme) -> AnyElement {
        let layout = &theme.layout;
        let [top, right, bottom, left] = layout.padding;
        let header = header(self.mark, self.title, self.subtitle, theme);
        let content = self.content.map(|content| {
            let holder = div().flex().flex_col().min_h_0().child(content);
            if self.grow {
                holder.flex_1()
            } else {
                holder.flex_shrink()
            }
        });
        let fill = |on: bool| on.then(|| div().flex_1());
        if !layout.side() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .pt(px(top))
                .pr(px(right))
                .pb(px(bottom))
                .pl(px(left))
                .children(fill(self.centered))
                .child(header)
                .children(content)
                .children(fill(!self.grow))
                .child(self.footer)
                .into_any_element();
        }
        // The header column: its panel reaches the window's edges.
        let panel = layout.panel.then(|| {
            div().absolute().top_0().left_0().size_full().bg(theme.panel).children(
                theme
                    .panel_picture
                    .clone()
                    .map(|(picture, opacity)| img(picture).size_full().object_fit(ObjectFit::Cover).opacity(opacity)),
            )
        });
        let side = div()
            .relative()
            .flex_none()
            .w(px(layout.header_width))
            .h_full()
            .flex()
            .flex_col()
            .justify_center()
            .children(panel)
            .child(div().relative().pt(px(top)).pb(px(bottom)).px(px(left.max(right))).child(header));
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .pt(px(top))
            .pr(px(right))
            .pb(px(bottom))
            .pl(px(left))
            .children(fill(self.centered))
            .children(content)
            .children(fill(!self.grow))
            .child(self.footer);
        let row = div().size_full().flex();
        match layout.header {
            Header::Right => row.child(main).child(side),
            _ => row.child(side).child(main),
        }
        .into_any_element()
    }
}

/// The mark, heading and subtitle, aligned as the layout says, with the
/// mark above the heading or beside it.
fn header(mark: Option<AnyElement>, title: SharedString, subtitle: Option<AnyElement>, theme: &Theme) -> gpui::Div {
    let layout = &theme.layout;
    let slot =
        |mark: AnyElement| div().flex_none().min_w(px(MARK_SLOT)).min_h(px(MARK_SLOT)).flex().items_center().justify_center().child(mark);
    let title = heading(title, theme);
    let top = match (mark, layout.logo) {
        (Some(mark), LogoPlace::Beside) => {
            let line = div().flex().items_center().gap(px(14.)).child(slot(mark)).child(title);
            if layout.align == Align::Right {
                line.flex_row_reverse()
            } else {
                line
            }
        }
        (Some(mark), LogoPlace::Above) => div().flex().flex_col().child(slot(mark)).child(title.mt(px(18.))),
        (None, _) => div().flex().flex_col().child(title),
    };
    let block = div().flex().flex_none().flex_col().w_full().text_color(theme.header_text);
    match layout.align {
        Align::Left => block.items_start().text_left().child(top.items_start()),
        Align::Center => block.items_center().text_center().child(top.items_center()),
        Align::Right => block.items_end().text_right().child(top.items_end()),
    }
    .children(subtitle.map(|subtitle| div().text_color(theme.header_muted).child(subtitle)))
}

/// The page heading in the style's face and the layout's size; a long one
/// (a long app name, a wordy language) steps down a size so it wraps onto
/// two calm lines instead of three.
fn heading(title: SharedString, theme: &Theme) -> gpui::Div {
    let size = theme.layout.heading_size;
    let size = if title.chars().count() > 30 { size * 18. / 22. } else { size };
    div()
        .max_w(px(460.))
        .text_heading()
        .font_family(theme.heading_font.clone())
        .font_weight(theme.heading_weight)
        .text_size(px(size))
        .line_height(px((size * 32. / 22.).round()))
        .child(title)
}

/// An icon drawn in the mark's slot.
pub fn page_icon(name: &'static str, color: gpui::Hsla) -> AnyElement {
    Icon::new(name).size(MARK_SLOT - 4.).color(color).into_any_element()
}

/// The line under the heading; it takes the header's alignment.
pub fn subtitle_text(text: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    let (size, line) = theme.text();
    div().mt(px(6.)).max_w(px(420.)).text_size(size).line_height(line).child(text.into()).into_any_element()
}

/// The footer line and the buttons, arranged as the layout says: the line
/// beside the buttons, above them or not at all; the buttons right, left,
/// centred or sharing the width; the main one last or first. Buttons that
/// don't fit in one row (four with long translations) wrap onto a second.
pub fn footer(meta: Option<SharedString>, mut buttons: Vec<AnyElement>, theme: &Theme) -> AnyElement {
    let layout = &theme.layout;
    if layout.primary == Primary::First {
        buttons.reverse();
    }
    let (size, line_height) = theme.meta_text();
    let shown = meta.filter(|_| layout.footer != Footer::Hidden);
    let line = |text: SharedString| {
        div()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(size)
            .line_height(line_height)
            .font_weight(FontWeight::NORMAL)
            .text_color(theme.muted_foreground)
            .child(text)
    };
    let row = div().min_h(px(layout.button_height)).flex().flex_wrap().items_center().gap(px(8.));
    let beside = layout.footer != Footer::Above && matches!(layout.buttons, Buttons::Right | Buttons::Left);
    let bar = if beside {
        // The line fills the room the buttons leave, even when it is empty.
        let filler = div().flex_1().min_w_0().flex().children(shown.map(line));
        match layout.buttons {
            Buttons::Left => row.children(buttons).child(filler.justify_end()),
            _ => row.child(filler).children(buttons),
        }
    } else {
        let row = match layout.buttons {
            Buttons::Right => row.justify_end().children(buttons),
            Buttons::Left => row.justify_start().children(buttons),
            Buttons::Center => row.justify_center().children(buttons),
            // Each button in a column that stretches it across its share.
            Buttons::Stretch => row.children(buttons.into_iter().map(|button| div().flex_1().min_w_0().flex().flex_col().child(button))),
        };
        let above = shown.map(|text| {
            let holder = div().flex();
            match layout.align {
                Align::Left => holder.justify_start(),
                Align::Center => holder.justify_center(),
                Align::Right => holder.justify_end(),
            }
            .child(line(text))
        });
        div().flex().flex_col().gap(px(10.)).children(above).child(row)
    };
    div().mt(px(16.)).flex_none().child(bar).into_any_element()
}
