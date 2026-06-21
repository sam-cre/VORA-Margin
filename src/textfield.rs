//! A minimal, dependency-free single-line text input.
//!
//! Tracks the value plus a cursor position (as a char index, so it's
//! UTF-8 safe). Supports insert, delete, and horizontal movement —
//! enough for friendly form editing without pulling in a whole crate.

#[derive(Default, Clone)]
pub struct TextField {
    text: String,
    /// Cursor position as a character index in `0..=char_count`.
    cursor: usize,
}

impl TextField {
    pub fn new(s: &str) -> Self {
        Self {
            text: s.to_string(),
            cursor: s.chars().count(),
        }
    }

    pub fn value(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Byte offset of a given char index (clamped to the end).
    fn byte_idx(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    pub fn insert(&mut self, c: char) {
        let b = self.byte_idx(self.cursor);
        self.text.insert(b, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let b = self.byte_idx(self.cursor - 1);
            self.text.remove(b);
            self.cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.char_count() {
            let b = self.byte_idx(self.cursor);
            self.text.remove(b);
        }
    }

    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.char_count();
    }
}
