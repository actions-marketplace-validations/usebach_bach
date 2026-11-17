//! The license page: a card that glides when you scroll, a scrollbar you can
//! grab, and text you can select and copy.
//!
//! Scrolling is GPUI's own scroll container, so the card, its size and its
//! limits come for free; this file only eases the motion and adds the bar
//! and the selection. Nothing runs while the page is still: a frame is
//! requested only while the glide is moving or a drag sits beyond the edge.

use std::time::Instant;

use bach_pack::i18n::Key;
use gpui::{
    canvas, div, point, px, AnyElement, ClipboardItem, Context, DispatchPhase, HighlightStyle, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, ScrollWheelEvent,
    StatefulInteractiveElement, Styled, StyledText, Window,
};

use super::{footer, subtitle_text, PageLayout};
use crate::keyboard::Target;
use crate::license::Drag;
use crate::theme::{ActiveTheme, Theme, RADIUS};
use crate::ui::prelude::*;
use crate::ui::scroll::Thumb;
use crate::view::SetupView;

/// How wide the strip along the card's edge is that the bar can be grabbed
/// in; the visible thumb inside it is thinner.
const GRIP: f32 = 14.;
/// A click on the empty track moves this share of a page.
const PAGE_STEP: f32 = 0.9;
/// How fast text selected past the card's edge scrolls, per pixel beyond it
/// per second, and the most it may reach.
const EDGE_SPEED: f32 = 12.;
const EDGE_MAX: f32 = 900.;

impl SetupView {
    pub(super) fn license_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let selected = self.license.selected();
        let highlight = HighlightStyle { background_color: Some(theme.primary.opacity(0.28)), ..Default::default() };
        let mut layouts = Vec::with_capacity(self.license.text.paragraphs().len());
        let paragraphs: Vec<AnyElement> = self
            .license
            .text
            .paragraphs()
            .iter()
            .enumerate()
            .map(|(index, paragraph)| {
                let mut text = StyledText::new(paragraph.clone());
                if let Some(range) = selected.as_ref().and_then(|selected| self.license.text.within(index, selected)) {
                    text = text.with_highlights([(range, highlight)]);
                }
                layouts.push(text.layout().clone());
                div().text_size(px(13.)).line_height(px(19.)).child(text).into_any_element()
            })
            .collect();
        self.license.layouts = layouts;

        let viewport = div()
            .id("setup-license")
            .track_scroll(&self.license.scroll)
            .size_full()
            .overflow_y_scroll()
            .cursor_text()
            .on_scroll_wheel(cx.listener(|this, event, window, cx| this.license_wheel(event, window, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event, _, cx| this.license_press(event, cx)))
            .child(div().p(px(16.)).flex().flex_col().gap(px(10.)).children(paragraphs));
        // The card fills the room left, so a short license and a long one
        // make the same calm page; a long one scrolls inside it.
        let mut card = div().relative().mt(px(24.)).flex_1().min_h_0().rounded(RADIUS).border_1().border_color(theme.border).child(viewport);
        card = card.children(self.license_scrollbar(&theme, cx));
        if self.license.drag != Drag::None {
            card = card.child(self.license_drag_listeners(cx));
        }

        let texts = &self.texts;
        let buttons = vec![
            FormButton::new("setup-cancel", texts.get(Key::cancel))
                .outline()
                .focused(self.is_focused(Target::Cancel))
                .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                .into_any_element(),
            FormButton::new("setup-accept", texts.get(Key::license_accept))
                .focused(self.is_focused(Target::Primary))
                .on_click(cx.listener(|this, _, _, cx| this.accept_license(cx)))
                .into_any_element(),
        ];
        PageLayout {
            mark: self.mark(false, &theme),
            subtitle: Some(subtitle_text(texts.get(Key::license_subtitle), &theme)),
            content: Some(card.into_any_element()),
            grow: true,
            ..PageLayout::new(texts.get(Key::license_title), footer(Some(self.meta()), buttons, &theme))
        }
        .render()
    }

    /// Called every frame the license page is drawn: advances the glide, and
    /// asks for the next frame only while something is still moving.
    pub(super) fn license_tick(&mut self, window: &mut Window) {
        let size = self.license.scroll.bounds().size;
        if size.height < px(1.) {
            // The card's size is known after its first paint.
            window.request_animation_frame();
            return;
        }
        let now = Instant::now();
        let seconds = self.license.last_frame.map_or(0., |before| now.duration_since(before).as_secs_f32()).min(0.05);
        let hidden = self.license_hidden();
        let actual = f32::from(self.license.scroll.offset().y);
        self.license.glide.follow(actual, hidden);

        let mut busy = self.license.glide.step(seconds);
        if self.license.drag == Drag::Text {
            let over = self.license_edge_overshoot();
            if over != 0. {
                let speed = (over * EDGE_SPEED).clamp(-EDGE_MAX, EDGE_MAX);
                let offset = self.license.glide.current - speed * seconds;
                self.license.glide.jump(offset, hidden);
                self.license_extend(self.license.pointer);
                busy = true;
            }
        }
        self.license.scroll.set_offset(point(px(0.), px(self.license.glide.current)));
        if busy {
            self.license.last_frame = Some(now);
            window.request_animation_frame();
        } else {
            self.license.last_frame = None;
        }
    }

    /// How far the card can still scroll.
    fn license_hidden(&self) -> f32 {
        f32::from(self.license.scroll.max_offset().height)
    }

    /// The scrollbar's thumb for where the card is now.
    fn license_thumb(&self) -> Option<Thumb> {
        let scroll = &self.license.scroll;
        Thumb::new(f32::from(scroll.bounds().size.height), self.license_hidden(), f32::from(-scroll.offset().y))
    }

    /// The wheel moves the goal; the glide gets there. GPUI's own handler has
    /// already moved the card by the same step, so that is undone first.
    fn license_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
        let license = &mut self.license;
        license.scroll.set_offset(point(px(0.), px(license.glide.current)));
        if license.glide.idle() {
            license.last_frame = Some(Instant::now());
        }
        let hidden = f32::from(license.scroll.max_offset().height);
        license.glide.push(delta, hidden);
        cx.notify();
    }

    /// The thin bar along the card's right edge, when the text scrolls.
    fn license_scrollbar(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(thumb) = self.license_thumb() else {
            return Vec::new();
        };
        let grabbed = matches!(self.license.drag, Drag::Thumb { .. });
        let alpha = if grabbed { 0.7 } else { 0.45 };
        let track = div()
            .absolute()
            .top(px(0.))
            .bottom(px(0.))
            .right(px(0.))
            .w(px(GRIP))
            .block_mouse_except_scroll()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| this.license_track_press(event.position, cx)));
        let handle = div()
            .absolute()
            .top(px(thumb.top))
            .right(px(0.))
            .w(px(GRIP))
            .h(px(thumb.height))
            .flex()
            .justify_end()
            .block_mouse_except_scroll()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.license_grab(event.position, cx);
                    cx.stop_propagation();
                }),
            )
            .child(div().mr(px(3.)).w(px(4.)).h_full().rounded(px(2.)).bg(theme.muted_foreground.opacity(alpha)));
        vec![track.into_any_element(), handle.into_any_element()]
    }

    /// A click on the empty track pages up or down, with the glide.
    fn license_track_press(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(thumb) = self.license_thumb() else {
            return;
        };
        let scroll = &self.license.scroll;
        let top = f32::from(scroll.bounds().top()) + thumb.top;
        let page = f32::from(scroll.bounds().size.height) * PAGE_STEP;
        self.page(if f32::from(position.y) < top { page } else { -page }, cx);
    }

    /// Scrolls by `delta` pixels (positive moves up), with the glide.
    fn page(&mut self, delta: f32, cx: &mut Context<Self>) {
        let hidden = self.license_hidden();
        if self.license.glide.idle() {
            self.license.last_frame = Some(Instant::now());
        }
        self.license.glide.push(delta, hidden);
        cx.notify();
    }

    /// Takes hold of the thumb where the pointer is on it.
    fn license_grab(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(thumb) = self.license_thumb() else {
            return;
        };
        let top = f32::from(self.license.scroll.bounds().top()) + thumb.top;
        self.license.drag = Drag::Thumb { grab: f32::from(position.y) - top };
        self.license.pointer = position;
        cx.notify();
    }

    /// The pointer went down on the text: a caret, a word, or a paragraph.
    fn license_press(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(at) = self.license_index_at(event.position) else {
            return;
        };
        let text = &self.license.text;
        let before = self.license.selection;
        self.license.selection = Some(match event.click_count {
            0 | 1 => match before {
                // Shift keeps the old start and moves the end.
                Some((anchor, _)) if event.modifiers.shift => (anchor, at),
                _ => (at, at),
            },
            2 => {
                let word = text.word_at(at);
                (word.start, word.end)
            }
            _ => {
                let paragraph = text.paragraph_at(at);
                (paragraph.start, paragraph.end)
            }
        });
        self.license.drag = Drag::Text;
        self.license.pointer = event.position;
        cx.notify();
    }

    /// The pointer moved with the button down: the selection or the thumb follows.
    fn license_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            // The button was let go where this window couldn't see it.
            self.license_release(cx);
            return;
        }
        self.license.pointer = event.position;
        match self.license.drag {
            Drag::Text => self.license_extend(event.position),
            Drag::Thumb { grab } => {
                if let Some(thumb) = self.license_thumb() {
                    let top = f32::from(event.position.y - self.license.scroll.bounds().top()) - grab;
                    let hidden = self.license_hidden();
                    self.license.glide.jump(-thumb.scrolled_at(top), hidden);
                    self.license.scroll.set_offset(point(px(0.), px(self.license.glide.current)));
                }
            }
            Drag::None => {}
        }
        cx.notify();
    }

    fn license_release(&mut self, cx: &mut Context<Self>) {
        if self.license.drag == Drag::Text && self.license.selected().is_none() {
            // A plain click leaves nothing selected.
            self.license.selection = None;
        }
        self.license.drag = Drag::None;
        cx.notify();
    }

    /// Moves the end of the selection to the text under `position`.
    fn license_extend(&mut self, position: Point<Pixels>) {
        if let (Some((anchor, _)), Some(at)) = (self.license.selection, self.license_index_at(position)) {
            self.license.selection = Some((anchor, at));
        }
    }

    /// How many pixels the pointer is above (negative) or below (positive)
    /// the card; 0 when it is inside.
    fn license_edge_overshoot(&self) -> f32 {
        let bounds = self.license.scroll.bounds();
        let y = self.license.pointer.y;
        if y < bounds.top() {
            f32::from(y - bounds.top())
        } else if y > bounds.bottom() {
            f32::from(y - bounds.bottom())
        } else {
            0.
        }
    }

    /// The place in the whole license under `position`, nearest if it is
    /// between paragraphs or outside the text.
    fn license_index_at(&self, position: Point<Pixels>) -> Option<usize> {
        let layouts = &self.license.layouts;
        let last = layouts.len().checked_sub(1)?;
        // Half the gap between paragraphs belongs to each neighbour.
        let index = layouts.iter().position(|layout| position.y <= layout.bounds().bottom() + px(5.)).unwrap_or(last);
        let local = match layouts[index].index_for_position(position) {
            Ok(at) | Err(at) => at,
        };
        Some(self.license.text.global(index, local))
    }

    /// Mouse moves and the button coming up, wherever the pointer is: a
    /// selection or a thumb is held until the button is let go, even outside
    /// the card. Only in the tree while something is held.
    fn license_drag_listeners(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let moved = view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        moved.update(cx, |this, cx| this.license_drag(event, cx));
                    }
                });
                let released = view.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        released.update(cx, |this, cx| this.license_release(cx));
                    }
                });
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    }

    /// Copy, select all, and the keys that scroll. True when the key was used.
    pub(crate) fn license_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        let page = f32::from(self.license.scroll.bounds().size.height) * PAGE_STEP;
        if event.keystroke.modifiers.secondary() {
            match key {
                "c" => {
                    if let Some(range) = self.license.selected() {
                        cx.write_to_clipboard(ClipboardItem::new_string(self.license.text.slice(&range).to_string()));
                    }
                    return true;
                }
                "a" => {
                    self.license.selection = Some((0, self.license.text.len()));
                    cx.notify();
                    return true;
                }
                _ => return false,
            }
        }
        let hidden = self.license_hidden();
        match key {
            "pagedown" => self.page(-page, cx),
            "pageup" => self.page(page, cx),
            "home" => self.page(hidden, cx),
            "end" => self.page(-hidden, cx),
            _ => return false,
        }
        true
    }
}
