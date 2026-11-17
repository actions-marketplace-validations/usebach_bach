//! The license page's state: the text split into paragraphs, what is
//! selected, and the scroll position.
//!
//! The selection is two byte offsets into the whole license (paragraphs
//! joined by a blank line), so it can span paragraphs and copying it gives
//! back the text as the developer wrote it. This module has no GPUI drawing;
//! `pages/license.rs` draws it and feeds in the mouse.

use std::ops::Range;
use std::time::Instant;

use gpui::{Pixels, Point, ScrollHandle, SharedString, TextLayout};

use crate::ui::scroll::Glide;

/// What separates two paragraphs, in the text and when copied.
const BREAK: &str = "\n\n";

/// The license cut into paragraphs, with where each starts in the whole.
pub struct LicenseText {
    paragraphs: Vec<SharedString>,
    starts: Vec<usize>,
    whole: String,
}

impl LicenseText {
    pub fn new(license: &str) -> Self {
        let paragraphs: Vec<&str> = license.split(BREAK).map(str::trim).filter(|paragraph| !paragraph.is_empty()).collect();
        let mut starts = Vec::with_capacity(paragraphs.len());
        let mut whole = String::with_capacity(license.len());
        for (index, paragraph) in paragraphs.iter().enumerate() {
            if index > 0 {
                whole.push_str(BREAK);
            }
            starts.push(whole.len());
            whole.push_str(paragraph);
        }
        Self { paragraphs: paragraphs.into_iter().map(|paragraph| SharedString::from(paragraph.to_string())).collect(), starts, whole }
    }

    pub fn paragraphs(&self) -> &[SharedString] {
        &self.paragraphs
    }

    pub fn len(&self) -> usize {
        self.whole.len()
    }

    /// Where `local` (a byte offset inside paragraph `index`) is in the whole.
    pub fn global(&self, index: usize, local: usize) -> usize {
        let start = self.starts.get(index).copied().unwrap_or(self.whole.len());
        let length = self.paragraphs.get(index).map_or(0, |paragraph| paragraph.len());
        start + local.min(length)
    }

    /// The part of `selection` that falls inside paragraph `index`, as
    /// offsets inside that paragraph.
    pub fn within(&self, index: usize, selection: &Range<usize>) -> Option<Range<usize>> {
        let start = *self.starts.get(index)?;
        let end = start + self.paragraphs[index].len();
        let from = selection.start.max(start);
        let to = selection.end.min(end);
        (from < to).then(|| from - start..to - start)
    }

    /// The selected text, for the clipboard.
    pub fn slice(&self, selection: &Range<usize>) -> &str {
        let end = selection.end.min(self.whole.len());
        let start = selection.start.min(end);
        self.whole.get(start..end).unwrap_or_default()
    }

    /// The word around `at` (a double click); on a space, the spaces.
    pub fn word_at(&self, at: usize) -> Range<usize> {
        let at = at.min(self.whole.len());
        let kind = |c: char| (c.is_alphanumeric() || c == '_', c.is_whitespace());
        let before = self.whole[..at].chars().next_back();
        let after = self.whole[at..].chars().next();
        let Some(anchor) = after.or(before) else {
            return at..at;
        };
        let wanted = kind(anchor);
        let start = self.whole[..at].char_indices().rev().take_while(|(_, c)| kind(*c) == wanted).last().map_or(at, |(index, _)| index);
        let end = self.whole[at..].char_indices().find(|(_, c)| kind(*c) != wanted).map_or(self.whole.len(), |(index, _)| at + index);
        start..end
    }

    /// The paragraph that holds `at` (a triple click).
    pub fn paragraph_at(&self, at: usize) -> Range<usize> {
        let index = self.starts.partition_point(|start| *start <= at).saturating_sub(1);
        match (self.starts.get(index), self.paragraphs.get(index)) {
            (Some(start), Some(paragraph)) => *start..*start + paragraph.len(),
            _ => 0..0,
        }
    }
}

/// What the mouse is doing on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    None,
    /// Selecting text.
    Text,
    /// Holding the scrollbar's thumb, `grab` pixels below its top.
    Thumb { grab: f32 },
}

pub struct LicenseView {
    pub text: LicenseText,
    /// The scrolling card; also the source of its size and the scrollbar.
    pub scroll: ScrollHandle,
    pub glide: Glide,
    pub last_frame: Option<Instant>,
    /// The layout of each paragraph from the last frame, to turn the mouse
    /// into a place in the text.
    pub layouts: Vec<TextLayout>,
    /// Where the selection started and where it is now (not ordered).
    pub selection: Option<(usize, usize)>,
    pub drag: Drag,
    /// The last mouse position while dragging, to keep scrolling when it sits
    /// beyond the card's edge.
    pub pointer: Point<Pixels>,
}

impl LicenseView {
    pub fn new(license: &str) -> Self {
        Self {
            text: LicenseText::new(license),
            scroll: ScrollHandle::new(),
            glide: Glide::default(),
            last_frame: None,
            layouts: Vec::new(),
            selection: None,
            drag: Drag::None,
            pointer: Point::default(),
        }
    }

    /// The selection in order, when something is selected.
    pub fn selected(&self) -> Option<Range<usize>> {
        let (anchor, head) = self.selection?;
        (anchor != head).then(|| anchor.min(head)..anchor.max(head))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text() -> LicenseText {
        LicenseText::new("  First paragraph here.\n\nSecond one.\n\n\n\nThird, last.  ")
    }

    #[test]
    fn paragraphs_are_trimmed_and_blank_ones_dropped() {
        let text = text();
        let all: Vec<&str> = text.paragraphs().iter().map(|p| p.as_ref()).collect();
        assert_eq!(all, ["First paragraph here.", "Second one.", "Third, last."]);
    }

    #[test]
    fn a_selection_across_paragraphs_copies_with_a_blank_line() {
        let text = text();
        let from = text.global(0, 6);
        let to = text.global(1, 6);
        assert_eq!(text.slice(&(from..to)), "paragraph here.\n\nSecond");
    }

    #[test]
    fn each_paragraph_gets_only_its_part_of_the_selection() {
        let text = text();
        let selection = text.global(0, 6)..text.global(2, 5);
        assert_eq!(text.within(0, &selection), Some(6..21));
        assert_eq!(text.within(1, &selection), Some(0..11));
        assert_eq!(text.within(2, &selection), Some(0..5));
        assert_eq!(text.within(2, &(0..3)), None);
    }

    #[test]
    fn a_double_click_picks_the_word_and_a_triple_click_the_paragraph() {
        let text = text();
        let at = text.global(1, 2);
        assert_eq!(text.slice(&text.word_at(at)), "Second");
        assert_eq!(text.slice(&text.paragraph_at(at)), "Second one.");
        // Punctuation is its own run, not part of the word.
        assert_eq!(text.slice(&text.word_at(text.global(1, 10))), ".");
        assert_eq!(text.slice(&text.word_at(text.global(1, 6))), " ");
    }

    #[test]
    fn multibyte_text_is_never_cut_inside_a_character() {
        let text = LicenseText::new("Größe über alles\n\nÄrger");
        let word = text.word_at(text.global(0, 2));
        assert_eq!(text.slice(&word), "Größe");
        assert_eq!(text.slice(&text.paragraph_at(text.global(1, 1))), "Ärger");
    }

    #[test]
    fn an_empty_license_has_nothing_to_select() {
        let text = LicenseText::new("");
        assert_eq!(text.len(), 0);
        assert_eq!(text.word_at(0), 0..0);
        assert_eq!(text.paragraph_at(0), 0..0);
    }
}
