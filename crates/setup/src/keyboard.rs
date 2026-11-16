//! Using setup without a mouse.
//!
//! Tab and Shift+Tab (or the arrow keys) walk through the page's controls:
//! the rows of choices, then the buttons. Space or Enter does what a click
//! would; Enter with nothing focused presses the page's main button, and
//! Escape cancels. A focus ring shows only once the keyboard is used, like
//! the web's `:focus-visible`, so mouse users never see one.
//!
//! GPUI's own focus handling would need a focus handle per row; one list
//! of targets per page keeps the order obvious and the code small.

use gpui::{Context, KeyDownEvent, Window};

use crate::view::{SetupView, Stage, Toggle};

/// Something the keyboard can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The Location row, when the folder may change.
    Location,
    Toggle(Toggle),
    /// A link button: the website, or the uninstaller's feedback page.
    Link,
    /// "Get a key", on the access key page.
    GetKey,
    /// "Paste", beside the key field.
    Paste,
    /// Cancel or Close: the quiet button.
    Cancel,
    /// The second button: Try again on the requirement page.
    Secondary,
    /// "Save support info" on the failed page.
    Support,
    /// "Continue anyway" on the requirement page.
    Skip,
    /// The page's main button: I agree, Install, Open, Try again.
    Primary,
}

/// What on a page decides which controls it has.
pub struct PageFacts {
    /// The key page links to where people get a key.
    pub get_key_link: bool,
    /// The last page has a website or feedback link.
    pub done_link: bool,
    /// The last page offers to open the app.
    pub open_button: bool,
    /// The ready page's rows (Location, toggles), in order; None for a row
    /// that isn't shown.
    pub rows: Vec<Option<Target>>,
    /// The working page offers "Cancel".
    pub cancellable: bool,
}

/// The page's controls in Tab order. Pure, so the order can be tested
/// without a window.
pub fn page_targets(stage: &Stage, facts: &PageFacts) -> Vec<Target> {
    let buttons = |primary: bool| {
        let mut buttons = vec![Target::Cancel];
        if primary {
            buttons.push(Target::Primary);
        }
        buttons
    };
    match stage {
        Stage::Requirement(_) => vec![Target::Cancel, Target::Secondary, Target::Skip, Target::Primary],
        Stage::Access => {
            let get_key = facts.get_key_link.then_some(Target::GetKey);
            get_key.into_iter().chain([Target::Paste, Target::Cancel, Target::Primary]).collect()
        }
        Stage::Failed(_) => vec![Target::Cancel, Target::Support, Target::Primary],
        Stage::License => buttons(true),
        Stage::Ready => facts.rows.iter().flatten().copied().chain(buttons(true)).collect(),
        Stage::Working { .. } if facts.cancellable => vec![Target::Cancel],
        Stage::Working { .. } => Vec::new(),
        Stage::Done => {
            let link = facts.done_link.then_some(Target::Link);
            link.into_iter().chain(buttons(facts.open_button)).collect()
        }
    }
}

impl SetupView {
    /// The page's controls in Tab order.
    pub fn targets(&self) -> Vec<Target> {
        page_targets(
            &self.stage,
            &PageFacts {
                get_key_link: self.has_get_key_link(),
                done_link: self.done_link().is_some(),
                open_button: self.has_open_button(),
                rows: self.row_targets(),
                cancellable: self.can_cancel(),
            },
        )
    }

    /// The link the last page offers: the website after installing, the
    /// feedback page after removing.
    pub fn done_link(&self) -> Option<&str> {
        let manifest = &self.package.manifest;
        if self.is_removing() {
            manifest.uninstall.feedback_url.as_deref()
        } else if manifest.install.website_button {
            manifest.app.website.as_deref()
        } else {
            None
        }
    }

    /// Whether the key page links to where people get a key.
    pub fn has_get_key_link(&self) -> bool {
        self.package.manifest.access.as_ref().is_some_and(|access| access.get_key_url.is_some())
    }

    /// Whether the last page offers to open the app.
    pub fn has_open_button(&self) -> bool {
        !self.is_removing() && self.package.manifest.install.open_button
    }

    fn focused_target(&self) -> Option<Target> {
        self.focused.and_then(|index| self.targets().get(index).copied())
    }

    pub fn is_focused(&self, target: Target) -> bool {
        self.focused_target() == Some(target)
    }

    pub fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // On the key page, typing goes into the field before anything else.
        if matches!(self.stage, Stage::Access) && self.type_key(event, cx) {
            return;
        }
        let key = event.keystroke.key.as_str();
        let count = self.targets().len();
        match key {
            "tab" | "down" | "up" if count > 0 => {
                let back = key == "up" || (key == "tab" && event.keystroke.modifiers.shift);
                self.focused = Some(match (self.focused, back) {
                    (None, false) => 0,
                    (None, true) => count - 1,
                    (Some(index), false) => (index + 1) % count,
                    (Some(index), true) => (index + count - 1) % count,
                });
                self.scroll_focused_into_view();
                cx.notify();
            }
            // A key never has a space, and the field isn't a control to press.
            "space" if matches!(self.stage, Stage::Access) && self.focused_target().is_none() => {}
            "space" => {
                if let Some(target) = self.focused_target() {
                    self.activate(target, window, cx);
                }
            }
            "enter" => {
                let target = self.focused_target().unwrap_or(Target::Primary);
                self.activate(target, window, cx);
            }
            "escape" => self.close(cx),
            _ => {}
        }
    }

    fn activate(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Location => self.choose_folder(window, cx),
            Target::Toggle(toggle) => self.toggle(toggle, cx),
            Target::Cancel => self.close(cx),
            Target::Secondary => self.recheck_requirements(cx),
            Target::Support => self.save_support(cx),
            Target::Skip => self.skip_requirements(cx),
            Target::GetKey => self.open_get_key(),
            Target::Paste => self.paste_key(cx),
            Target::Link if self.is_removing() => self.open_feedback(),
            Target::Link => self.open_website(),
            Target::Primary => match self.stage {
                Stage::Requirement(_) => self.get_requirement(),
                Stage::Access => self.submit_key(cx),
                Stage::License => self.accept_license(cx),
                Stage::Failed(_) if self.app_is_open() => self.close_app_and_retry(cx),
                Stage::Ready | Stage::Failed(_) => self.start(cx),
                Stage::Done if self.has_open_button() => self.open_app(cx),
                Stage::Done => self.close(cx),
                Stage::Working { .. } => {}
            },
        }
    }

    /// Keeps a focused row visible in a card that scrolls.
    fn scroll_focused_into_view(&self) {
        let Some(target) = self.focused_target() else {
            return;
        };
        if let Some(row) = self.row_targets().iter().position(|row| *row == Some(target)) {
            self.choices_scroll.scroll_to_item(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Toggle;

    fn facts() -> PageFacts {
        PageFacts { get_key_link: false, done_link: false, open_button: true, rows: Vec::new(), cancellable: false }
    }

    #[test]
    fn every_page_has_its_tab_order() {
        let nothing = facts();
        assert_eq!(page_targets(&Stage::Requirement(0), &nothing), [Target::Cancel, Target::Secondary, Target::Skip, Target::Primary]);
        assert_eq!(page_targets(&Stage::Access, &nothing), [Target::Paste, Target::Cancel, Target::Primary]);
        assert_eq!(
            page_targets(&Stage::Access, &PageFacts { get_key_link: true, ..facts() }),
            [Target::GetKey, Target::Paste, Target::Cancel, Target::Primary]
        );
        assert_eq!(page_targets(&Stage::License, &nothing), [Target::Cancel, Target::Primary]);
        // The failed page: Close, Save support info, Try again, in that order.
        assert_eq!(page_targets(&Stage::Failed("x".into()), &nothing), [Target::Cancel, Target::Support, Target::Primary]);
        // Nothing to tab to once files move into place; while they are only unpacked there is Cancel.
        let working = Stage::Working { progress: 0.5, label: "x".into() };
        assert!(page_targets(&working, &nothing).is_empty());
        assert_eq!(page_targets(&working, &PageFacts { cancellable: true, ..facts() }), [Target::Cancel]);
    }

    #[test]
    fn the_ready_page_lists_its_rows_before_its_buttons() {
        let rows = vec![
            Some(Target::Location),
            None,
            Some(Target::Toggle(Toggle::DesktopShortcut)),
            Some(Target::Toggle(Toggle::StartWithWindows)),
        ];
        let order = page_targets(&Stage::Ready, &PageFacts { rows, ..facts() });
        assert_eq!(
            order,
            [
                Target::Location,
                Target::Toggle(Toggle::DesktopShortcut),
                Target::Toggle(Toggle::StartWithWindows),
                Target::Cancel,
                Target::Primary
            ],
            "rows that aren't shown are skipped"
        );
    }

    #[test]
    fn the_last_page_offers_what_the_app_has() {
        let both = PageFacts { done_link: true, open_button: true, ..facts() };
        assert_eq!(page_targets(&Stage::Done, &both), [Target::Link, Target::Cancel, Target::Primary]);
        let only_close = PageFacts { done_link: false, open_button: false, ..facts() };
        assert_eq!(page_targets(&Stage::Done, &only_close), [Target::Cancel], "no Open button: Close is the only one");
    }
}
