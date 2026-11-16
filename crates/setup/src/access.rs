//! The access key page's logic: typing and pasting the key, and checking it.
//!
//! GPUI's text input machinery (IME, selection, cursor) is a lot for a field
//! that holds one ASCII key, so the field is drawn by `pages/access.rs` and
//! fed from here: printable characters from the keyboard, Backspace, and
//! paste. Checking a key derives an encryption key with Argon2id, which is
//! slow on purpose, so it runs on the background executor, never the UI
//! thread.

use bach_pack::i18n::Key;
use bach_pack::protect::{KeyError, MAX_KEY};
use gpui::{Context, KeyDownEvent};

use crate::system::apps;
use crate::view::{SetupView, Stage};

impl SetupView {
    /// Handles a key press on the key page. True when it was typing or
    /// pasting, so the caller leaves it alone; Tab, Enter and Escape are the
    /// page's own and come back false.
    pub fn type_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.key_checking {
            return false;
        }
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        let paste = ((modifiers.control || modifiers.platform) && keystroke.key == "v") || (modifiers.shift && keystroke.key == "insert");
        if paste {
            self.paste_key(cx);
            return true;
        }
        match keystroke.key.as_str() {
            "backspace" => {
                if modifiers.control {
                    self.key.clear();
                } else {
                    self.key.pop();
                }
                self.key_error = None;
                cx.notify();
                return true;
            }
            _ if modifiers.control || modifiers.alt || modifiers.platform => return false,
            _ => {}
        }
        let Some(typed) = keystroke.key_char.as_deref() else {
            return false;
        };
        let typed: String = typed.chars().filter(|c| is_key_char(*c)).collect();
        if typed.is_empty() {
            return false;
        }
        self.add_to_key(&typed);
        cx.notify();
        true
    }

    /// Puts the clipboard's text in the field (whitespace around it, and
    /// anything a key can't hold, dropped).
    pub fn paste_key(&mut self, cx: &mut Context<Self>) {
        if self.key_checking {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let pasted: String = text.trim().chars().filter(|c| is_key_char(*c)).collect();
            if !pasted.is_empty() {
                self.key.clear();
                self.add_to_key(&pasted);
            }
        }
        cx.notify();
    }

    fn add_to_key(&mut self, text: &str) {
        for c in text.chars() {
            if self.key.len() < MAX_KEY {
                self.key.push(c);
            }
        }
        self.key_error = None;
    }

    /// Opens the page where people get a key.
    pub fn open_get_key(&mut self) {
        if let Some(url) = self.package.manifest.access.as_ref().and_then(|access| access.get_key_url.as_deref()) {
            apps::open_url(url);
        }
    }

    /// Checks the key and moves on when it fits. A setup file that locks its
    /// files can say so; one that only passes the key to a server has to
    /// take it on trust here, and the server says. A preview takes any key.
    pub fn submit_key(&mut self, cx: &mut Context<Self>) {
        if self.key_checking || self.key.is_empty() {
            return;
        }
        let encryption = self.package.manifest.access.as_ref().and_then(|access| access.encryption.clone());
        let Some(encryption) = encryption.filter(|_| !self.preview) else {
            self.accept_key(cx);
            return;
        };
        self.key_checking = true;
        self.key_error = None;
        cx.notify();
        let key = self.key.clone();
        let checking = cx.background_executor().spawn(async move { encryption.unlock(&key).map(|_| ()) });
        self.checker = Some(cx.spawn(async move |this, cx| {
            let result = checking.await;
            this.update(cx, |this, cx| {
                this.key_checking = false;
                this.checker = None;
                match result {
                    Ok(()) => this.accept_key(cx),
                    Err(KeyError::Wrong) => {
                        crate::log::line("The access key didn't fit.");
                        this.key_error = Some(this.texts.get(Key::access_wrong).into());
                    }
                    Err(KeyError::Damaged(why)) => {
                        crate::log::line(&format!("The protection data is unusable: {why}"));
                        this.stage = Stage::Failed(this.texts.get(Key::error_damaged).into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The key is in: on to the license, or the choices.
    fn accept_key(&mut self, cx: &mut Context<Self>) {
        self.access_pending = false;
        self.stage = self.first_page();
        self.focused = None;
        cx.notify();
    }
}

/// What a key may hold (the same as the build allows).
fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_.~+/=:".contains(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_key_characters_get_in() {
        for c in "aZ09-_.~+/=:".chars() {
            assert!(is_key_char(c), "{c}");
        }
        for c in [' ', '"', '\\', '\n', '\t', 'é', '%', '@', '!'] {
            assert!(!is_key_char(c), "{c:?}");
        }
    }
}
