//! The first pages: the license, and the card of choices.

use bach_pack::i18n::Key;
use bach_pack::manifest::{choices_auto_update, Manifest};
use gpui::{
    div, px, relative, AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled,
};

use super::{footer, subtitle_text, PageLayout};
use crate::keyboard::Target;
use bach_pack::style::layout::Rows;

use crate::theme::{ActiveTheme, Theme};
use crate::ui::prelude::*;
use crate::ui::scroll::Thumb;
use crate::view::{SetupView, Toggle};

/// One row of the card before it's drawn.
struct Choice {
    title: String,
    description: Option<String>,
    icon: &'static str,
    action: Action,
}

enum Action {
    /// Shows where the app goes; clicking picks another folder when allowed.
    Location,
    Toggle(Toggle),
}

/// How many rows the install page shows, so the window can make room.
pub fn row_count(manifest: &Manifest, removing: bool) -> usize {
    if removing {
        return 1 + usize::from(!manifest.uninstall.data.is_empty());
    }
    let install = &manifest.install;
    let offered = [
        install.desktop_shortcut,
        install.start_menu_shortcut,
        install.start_with_windows,
        install.add_to_path,
        choices_auto_update(manifest),
    ]
    .iter()
    .filter(|choice| choice.offered())
    .count();
    1 + manifest.components.len() + offered
}

impl SetupView {
    pub(super) fn ready_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let choices = self.choices();
        let count = choices.len();
        let rows: Vec<AnyElement> =
            choices.into_iter().enumerate().map(|(index, choice)| self.row(choice, index, index + 1 == count, cx)).collect();

        let (title, subtitle, action) = self.ready_copy();
        let go = FormButton::new("setup-go", action)
            .focused(self.is_focused(Target::Primary))
            .on_click(cx.listener(|this, _, _, cx| this.start(cx)));
        let go = if self.is_removing() { go.destructive() } else { go };
        // The card itself scrolls, so a focused row can be scrolled to by
        // its index; the scrollbar sits over its right edge.
        let list = div()
            .id("setup-choices")
            .track_scroll(&self.choices_scroll)
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .size_full()
            .overflow_y_scroll();
        // Separate rows are cards of their own; joined ones share this one.
        let list = if theme.layout.two_columns() {
            // The folder (a long path and a button) gets a line of its own;
            // the options follow two to a line, as tall as each other. An
            // odd last one keeps a tile's width. Two on a line share it
            // exactly, so their outer edges meet the folder's.
            let alone = count > 1 && (count - 1) % 2 == 1;
            let tiles = rows.into_iter().enumerate().map(|(index, row)| {
                let tile = div().flex().flex_col().min_w_0().child(row);
                if index == 0 {
                    tile.w_full()
                } else if alone && index + 1 == count {
                    tile.flex_grow().flex_basis(relative(0.45)).max_w(relative(0.494))
                } else {
                    tile.flex_grow().flex_basis(relative(0.45))
                }
            });
            list.flex().flex_row().flex_wrap().content_start().gap(px(8.)).children(tiles)
        } else {
            match theme.layout.rows {
                Rows::Separate => list.flex().flex_col().gap(px(8.)).children(rows),
                Rows::Plain => list.children(rows),
                Rows::Joined => list.rounded(theme.radius).bg(theme.card).border_1().border_color(theme.border).children(rows),
            }
        };
        let holder = if theme.layout.rows == Rows::Plain { crate::ui::row::plain(&theme) } else { div() };
        let choices = holder.relative().mt(px(theme.layout.gap)).min_h_0().flex().flex_col().child(list).children(self.scrollbar(&theme));
        PageLayout {
            mark: self.mark(false, &theme),
            subtitle: subtitle.map(|text| subtitle_text(text, &theme)),
            content: Some(choices.into_any_element()),
            ..PageLayout::new(
                title,
                footer(
                    Some(self.meta()),
                    vec![
                        FormButton::new("setup-cancel", self.texts.get(Key::cancel))
                            .outline()
                            .focused(self.is_focused(Target::Cancel))
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                            .into_any_element(),
                        go.into_any_element(),
                    ],
                    &theme,
                ),
            )
        }
        .render(&theme)
    }

    /// A thin thumb showing where the card is scrolled, when it scrolls.
    fn scrollbar(&self, theme: &Theme) -> Option<AnyElement> {
        let handle = &self.choices_scroll;
        let thumb =
            Thumb::new(f32::from(handle.bounds().size.height), f32::from(handle.max_offset().height), f32::from(-handle.offset().y))?;
        Some(
            div()
                .absolute()
                .top(px(thumb.top))
                .right(px(3.))
                .w(px(4.))
                .h(px(thumb.height))
                .rounded(px(2.))
                .bg(theme.muted_foreground.opacity(0.45))
                .into_any_element(),
        )
    }

    /// What the keyboard can reach on each row, in order; None for a row it
    /// skips (the Location row when the folder is fixed).
    pub(crate) fn row_targets(&self) -> Vec<Option<Target>> {
        self.choices()
            .into_iter()
            .map(|choice| match choice.action {
                Action::Location if self.can_change_folder() => Some(Target::Location),
                Action::Location => None,
                Action::Toggle(toggle) => Some(Target::Toggle(toggle)),
            })
            .collect()
    }

    /// The rows of the card, in order.
    fn choices(&self) -> Vec<Choice> {
        let texts = &self.texts;
        let manifest = &self.package.manifest;
        let mut rows = vec![Choice {
            title: texts.get(Key::location),
            description: Some(self.dir.display().to_string()),
            icon: "folder-filled",
            action: Action::Location,
        }];
        if self.is_removing() {
            if !manifest.uninstall.data.is_empty() {
                // The developer's line says what is in the folders; which folders they are is
                // shown as they are on this computer, so nobody has to take the line on trust.
                let mut description = texts.get(Key::delete_data_description);
                for folder in manifest
                    .uninstall
                    .data
                    .iter()
                    .filter_map(|template| bach_pack::paths::expand_data_folder(template, |name| std::env::var(name).ok()))
                {
                    description.push('\n');
                    description.push_str(&folder.display().to_string());
                }
                rows.push(Choice {
                    title: texts.get(Key::delete_data),
                    description: Some(description),
                    icon: "trash-filled",
                    action: Action::Toggle(Toggle::DeleteData),
                });
            }
            return rows;
        }
        for (index, component) in manifest.components.iter().enumerate() {
            let (title, description) = texts.component(component, manifest);
            rows.push(Choice { title, description, icon: "puzzle-filled", action: Action::Toggle(Toggle::Component(index)) });
        }
        let install = &manifest.install;
        let builtin = [
            (
                install.desktop_shortcut,
                Key::desktop_shortcut,
                Key::desktop_shortcut_description,
                "device-desktop-filled",
                Toggle::DesktopShortcut,
            ),
            (install.start_menu_shortcut, Key::start_menu, Key::start_menu_description, "layout-grid-filled", Toggle::StartMenu),
            (
                install.start_with_windows,
                Key::start_with_windows,
                Key::start_with_windows_description,
                "bolt-filled",
                Toggle::StartWithWindows,
            ),
            (install.add_to_path, Key::add_to_path, Key::add_to_path_description, "terminal-2", Toggle::AddToPath),
            (choices_auto_update(manifest), Key::auto_update, Key::auto_update_description, "refresh", Toggle::AutoUpdate),
        ];
        for (choice, title, description, icon, toggle) in builtin {
            if choice.offered() {
                rows.push(Choice {
                    title: texts.get(title),
                    description: Some(texts.get(description)),
                    icon,
                    action: Action::Toggle(toggle),
                });
            }
        }
        rows
    }

    fn row(&self, choice: Choice, index: usize, last: bool, cx: &mut Context<Self>) -> AnyElement {
        let target = match choice.action {
            Action::Location => Target::Location,
            Action::Toggle(toggle) => Target::Toggle(toggle),
        };
        let row = Row::new(choice.title)
            .icon(choice.icon)
            .divided(index > 0)
            .last(last)
            .fill(cx.theme().layout.two_columns())
            .focused(self.is_focused(target));
        let row = match choice.description {
            Some(description) => row.description(description),
            None => row,
        };
        match choice.action {
            Action::Location if self.can_change_folder() => row
                .control(
                    FormButton::new("setup-change-folder", self.texts.get(Key::change))
                        .outline()
                        .on_click(cx.listener(|this, _, window, cx| this.choose_folder(window, cx))),
                )
                .on_click("setup-row-location", cx.listener(|this, _, window, cx| this.choose_folder(window, cx)))
                .into_any_element(),
            Action::Location => row.into_any_element(),
            Action::Toggle(toggle) => row
                .control(
                    Checkbox::new(("setup-check", index), self.is_on(toggle))
                        .on_toggle(cx.listener(move |this, _, _, cx| this.toggle(toggle, cx))),
                )
                .on_click(("setup-row", index), cx.listener(move |this, _, _, cx| this.toggle(toggle, cx)))
                .into_any_element(),
        }
    }

    /// Heading, copy line and button label for the first page.
    fn ready_copy(&self) -> (SharedString, Option<SharedString>, SharedString) {
        let texts = &self.texts;
        let version = self.package.version();
        if self.is_removing() {
            return (
                texts.get(Key::remove_title).into(),
                Some(texts.get(Key::remove_subtitle).into()),
                texts.get(Key::remove_button).into(),
            );
        }
        if !self.overwrites.is_empty() {
            // Say what would be replaced before it is: the folder isn't this app's.
            let files = crate::work::describe_files(&self.overwrites);
            return (
                texts.get(Key::overwrite_title).into(),
                Some(texts.with(Key::overwrite_subtitle, &[("files", files.as_str())]).into()),
                texts.get(Key::downgrade_button).into(),
            );
        }
        match &self.existing {
            None => {
                // No built-in line: the app's own description, if it has one.
                let subtitle = texts.get(Key::install_subtitle);
                let subtitle = if subtitle.trim().is_empty() {
                    self.package.manifest.app.description.clone().filter(|text| !text.trim().is_empty())
                } else {
                    Some(subtitle)
                };
                (texts.get(Key::install_title).into(), subtitle.map(Into::into), texts.get(Key::install_button).into())
            }
            // A newer version is installed: say so plainly before replacing it.
            Some(installed) if self.downgrade => (
                texts.get(Key::downgrade_title).into(),
                Some(texts.with(Key::downgrade_subtitle, &[("old_version", installed.version.as_str())]).into()),
                texts.get(Key::downgrade_button).into(),
            ),
            Some(installed) if installed.version == version => (
                texts.get(Key::reinstall_title).into(),
                Some(texts.get(Key::reinstall_subtitle).into()),
                texts.get(Key::reinstall_button).into(),
            ),
            Some(installed) => {
                let old = if installed.version.is_empty() { "?" } else { installed.version.as_str() };
                (
                    texts.get(Key::update_title).into(),
                    Some(texts.with(Key::update_subtitle, &[("old_version", old)]).into()),
                    texts.get(Key::update_button).into(),
                )
            }
        }
    }
}
