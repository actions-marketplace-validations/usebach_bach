//! The page that asks for an access key: a field for it, a way to paste it,
//! and a link to where people get one.

use bach_pack::i18n::Key;
use gpui::{div, px, AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled};

use super::{footer, page_icon, subtitle_text, PageLayout};
use crate::keyboard::Target;
use crate::theme::ActiveTheme;
use crate::ui::prelude::*;
use crate::view::SetupView;

/// How much of a long key the field shows (the end, where typing happens).
const SHOWN: usize = 34;

impl SetupView {
    pub(super) fn access_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let texts = self.texts.clone();
        let has_link = self.package.manifest.access.as_ref().is_some_and(|access| access.get_key_url.is_some());

        let empty = self.key.is_empty();
        let shown: SharedString = if empty { texts.get(Key::access_hint).into() } else { visible_end(&self.key).into() };
        let border = if self.key_error.is_some() { theme.destructive } else { theme.input };
        let field = div()
            .id("setup-key-field")
            .mt(px((theme.layout.gap - 4.).max(0.)))
            .h(px(44.))
            .w_full()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .pl(px(14.))
            .pr(px(6.))
            .rounded(theme.radius)
            .bg(theme.card)
            .border_1()
            .border_color(border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .flex()
                    .items_center()
                    .text_size(theme.text().0)
                    .text_color(if empty { theme.muted_foreground } else { theme.foreground })
                    .child(shown)
                    // The caret sits after what was typed, not the hint.
                    .children(
                        (!empty && !self.key_checking).then(|| div().ml(px(1.)).w(px(1.5)).h(px(18.)).flex_none().bg(theme.foreground)),
                    ),
            )
            .child(
                FormButton::new("setup-paste", texts.get(Key::access_paste))
                    .outline()
                    .disabled(self.key_checking)
                    .focused(self.is_focused(Target::Paste))
                    .on_click(cx.listener(|this, _, _, cx| this.paste_key(cx))),
            );
        let problem = div()
            .mt(px(8.))
            .min_h(px(20.))
            .text_size(theme.small_text().0)
            .line_height(px(20.))
            .text_color(theme.destructive)
            .children(self.key_error.clone());

        let mut buttons = vec![FormButton::new("setup-cancel", texts.get(Key::cancel))
            .outline()
            .focused(self.is_focused(Target::Cancel))
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
            .into_any_element()];
        if has_link {
            buttons.insert(
                0,
                FormButton::new("setup-get-key", texts.get(Key::access_get))
                    .outline()
                    .focused(self.is_focused(Target::GetKey))
                    .on_click(cx.listener(|this, _, _, _| this.open_get_key()))
                    .into_any_element(),
            );
        }
        buttons.push(
            FormButton::new("setup-key-continue", texts.get(Key::access_button))
                .disabled(empty)
                .pending(self.key_checking)
                .focused(self.is_focused(Target::Primary))
                .on_click(cx.listener(|this, _, _, cx| this.submit_key(cx)))
                .into_any_element(),
        );

        PageLayout {
            mark: Some(page_icon("lock-filled", theme.header_text, &theme)),
            subtitle: Some(subtitle_text(texts.get(Key::access_subtitle), &theme)),
            content: Some(div().flex().flex_col().child(field).child(problem).into_any_element()),
            ..PageLayout::new(texts.get(Key::access_title), footer(Some(self.meta()), buttons, &theme))
        }
        .render(&theme)
    }
}

/// A long key shows its end, after an ellipsis, so the field never overflows.
fn visible_end(key: &str) -> String {
    let count = key.chars().count();
    if count <= SHOWN {
        return key.to_string();
    }
    let tail: String = key.chars().skip(count - (SHOWN - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_keys_show_whole_and_long_ones_show_their_end() {
        assert_eq!(visible_end("K7QF-3MWD"), "K7QF-3MWD");
        let key = "A".repeat(SHOWN);
        assert_eq!(visible_end(&key), key);
        let long = format!("{}END", "x".repeat(100));
        let shown = visible_end(&long);
        assert_eq!(shown.chars().count(), SHOWN);
        assert!(shown.starts_with('…') && shown.ends_with("END"), "{shown}");
    }
}
