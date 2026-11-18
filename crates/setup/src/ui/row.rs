//! The choices card: a bordered card that stacks rows on hairline dividers,
//! and a row with an icon in a well, a title over a muted description, and
//! a control pinned right.
//!
//! A row with [`Row::on_click`] is one big target: clicking anywhere on it
//! does what its control does (ticks its box, opens the folder picker),
//! with a soft hover wash, so nobody has to aim for a 16px box.
//!
//! ```ignore
//! card([Row::new("Desktop shortcut").icon("device-desktop-filled").control(checkbox).on_click(id, toggle).into_any_element()], cx)
//! ```

use std::rc::Rc;

use gpui::{
    div, px, AnyElement, App, ClickEvent, CursorStyle, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window,
};

use super::color::{clear, mix};
use super::icon::Icon;
use super::interaction::{use_interaction, Interaction};
use super::surface::Surfaces;
use bach_pack::style::layout::Rows;

use crate::theme::{ActiveTheme, Theme};

type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A bordered card; rows divide themselves with hairlines. With
/// `[layout] rows = "separate"` each row is its own card instead, and this
/// only stacks them.
pub fn card(rows: impl IntoIterator<Item = AnyElement>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    if theme.layout.rows == Rows::Separate {
        return div().flex().flex_col().gap(px(8.)).children(rows);
    }
    div().rounded(theme.radius).bg(theme.card).border_1().border_color(theme.border).children(rows)
}

#[derive(IntoElement)]
pub struct Row {
    title: SharedString,
    description: Option<SharedString>,
    icon: Option<&'static str>,
    control: Option<AnyElement>,
    divided: bool,
    last: bool,
    focused: bool,
    click: Option<(ElementId, Handler)>,
}

impl Row {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), description: None, icon: None, control: None, divided: false, last: false, focused: false, click: None }
    }

    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// An 18px icon in a 36px well.
    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    /// Pinned right, beside the copy.
    pub fn control(mut self, element: impl IntoElement) -> Self {
        self.control = Some(element.into_any_element());
        self
    }

    /// Draws the hairline above: every row but a card's first.
    pub fn divided(mut self, divided: bool) -> Self {
        self.divided = divided;
        self
    }

    /// The card's last row, whose hover wash follows the bottom corners.
    pub fn last(mut self, last: bool) -> Self {
        self.last = last;
        self
    }

    /// Draws the keyboard focus ring (only after Tab, like `:focus-visible`).
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Makes the whole row clickable. The control's own click stops there,
    /// so a click on the box doesn't count twice.
    pub fn on_click(mut self, id: impl Into<ElementId>, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.click = Some((id.into(), Rc::new(handler)));
        self
    }
}

impl RenderOnce for Row {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hover = self.click.as_ref().map(|(id, _)| use_interaction(id.clone(), window, cx));
        let theme = cx.theme();
        let layout = &theme.layout;
        let separate = layout.rows == Rows::Separate;
        let wash = hover.as_ref().map(|state| state.read(cx).hover()).unwrap_or(0.0);
        let icon = self.icon.filter(|_| layout.icons).map(|name| {
            div()
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(36.))
                .well(theme, theme.control_radius)
                .text_color(theme.foreground)
                .child(div().relative().flex().child(Icon::new(name).size(18.)))
        });
        let ((title_size, title_line), (small_size, small_line)) = (theme.text(), theme.small_text());
        let copy =
            div()
                .flex_1()
                .min_w_0()
                .child(div().text_size(title_size).line_height(title_line).font_weight(FontWeight::MEDIUM).child(self.title))
                .children(self.description.map(|text| {
                    div().mt(px(2.)).text_size(small_size).line_height(small_line).text_color(theme.muted_foreground).child(text)
                }));
        let ring = self.focused.then(|| {
            div()
                .absolute()
                .top(px(2.))
                .left(px(2.))
                .right(px(2.))
                .bottom(px(2.))
                .rounded(Theme::inner_radius(theme.radius, 3.))
                .border_2()
                .border_color(theme.ring)
        });
        let mut row = div()
            .relative()
            .p(px(layout.row_padding))
            .flex()
            .items_center()
            .gap(px(12.))
            .bg(mix(clear(theme.hover), theme.hover, wash))
            .children(icon)
            .child(copy)
            .children(self.control.map(|control| div().flex_none().child(control)))
            .children(ring);
        if separate {
            // Its own card: every corner, its own edge and fill.
            row = row.rounded(theme.radius).border_1().border_color(theme.border).bg(mix(theme.card, theme.hover, wash));
        } else {
            // The card's border is 1px, so the wash sits inside a 1px smaller radius.
            let inner = Theme::inner_radius(theme.radius, 1.);
            let corners = &mut row.style().corner_radii;
            if !self.divided {
                corners.top_left = Some(inner.into());
                corners.top_right = Some(inner.into());
            }
            if self.last {
                corners.bottom_left = Some(inner.into());
                corners.bottom_right = Some(inner.into());
            }
            if self.divided && layout.dividers {
                row = row.border_t_1().border_color(theme.border);
            }
        }
        match (self.click, hover) {
            (Some((id, handler)), Some(state)) => {
                let row = row.id(id).cursor(CursorStyle::PointingHand).on_click(move |event, window, cx| handler(event, window, cx));
                Interaction::track(&state, row).into_any_element()
            }
            _ => row.into_any_element(),
        }
    }
}
