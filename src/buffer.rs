use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    AsciiWord,
    AsciiPunct,
    Hiragana,
    Katakana,
    Cjk,
    Other,
}

fn classify_char(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_ascii_alphanumeric() || c == '_' {
        CharClass::AsciiWord
    } else if c.is_ascii_punctuation() {
        CharClass::AsciiPunct
    } else if ('\u{3040}'..='\u{309F}').contains(&c) {
        CharClass::Hiragana
    } else if ('\u{30A0}'..='\u{30FF}').contains(&c) || ('\u{FF65}'..='\u{FF9F}').contains(&c) {
        CharClass::Katakana
    } else if ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c) {
        CharClass::Cjk
    } else {
        CharClass::Other
    }
}

/// Visual column width of a single character in a monospace terminal/editor grid.
pub fn char_display_width(c: char) -> usize {
    if c == '\t' {
        4
    } else {
        c.width().unwrap_or(1).max(1)
    }
}

/// Convert a character index in a line string to its byte index.
pub fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(s.len())
}

/// Expand tabs to spaces for consistent visual width display.
pub fn expand_tabs_for_display(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\t' {
            out.push_str("    ");
        } else {
            out.push(c);
        }
    }
    out
}

pub struct TextBuffer {
    pub lines: Vec<String>,
    /// 0-indexed current line
    pub cursor_line: usize,
    /// 0-indexed character column within `lines[cursor_line]`
    pub cursor_col: usize,
    /// Remembered visual column for vertical diamond movement (Ctrl+E / Ctrl+X)
    preferred_visual_col: Option<usize>,
    pub file_path: Option<PathBuf>,
    pub dirty: bool,
    /// Monotonically increasing revision incremented on every text mutation (for view caches)
    pub revision: u64,
    pub encoding: &'static str,
    pub line_ending: &'static str,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBuffer {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
            preferred_visual_col: None,
            file_path: None,
            dirty: false,
            revision: 0,
            encoding: "UTF-8",
            line_ending: "\n",
        }
    }

    pub fn from_file(path: &Path) -> io::Result<Self> {
        let raw = fs::read_to_string(path)?;
        let line_ending = if raw.contains("\r\n") { "\r\n" } else { "\n" };
        let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
        let mut lines: Vec<String> = normalized.split('\n').map(String::from).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Ok(Self {
            lines,
            cursor_line: 0,
            cursor_col: 0,
            preferred_visual_col: None,
            file_path: Some(path.to_path_buf()),
            dirty: false,
            revision: 1,
            encoding: "UTF-8",
            line_ending,
        })
    }

    pub fn save_to_path(&mut self, path: &Path) -> io::Result<()> {
        let content = self.lines.join(self.line_ending);
        fs::write(path, content)?;
        self.file_path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    pub fn display_name(&self) -> String {
        self.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "[無題]".to_string())
    }

    /// Returns true if the buffer's file path has a Markdown extension (`.md` or `.markdown`).
    pub fn is_markdown(&self) -> bool {
        self.file_path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
            })
    }

    pub fn current_line_str(&self) -> &str {
        &self.lines[self.cursor_line]
    }

    pub fn line_char_len(&self, line_idx: usize) -> usize {
        self.lines[line_idx].chars().count()
    }

    /// 0-indexed visual column (accounting for full-width Japanese characters and tabs).
    pub fn visual_col_at(&self, line_idx: usize, char_col: usize) -> usize {
        self.lines[line_idx]
            .chars()
            .take(char_col)
            .map(char_display_width)
            .sum()
    }

    /// 1-indexed line number for status bar display.
    pub fn display_line_num(&self) -> usize {
        self.cursor_line + 1
    }

    /// 1-indexed visual column number for status bar display (VZ style).
    pub fn display_col_num(&self) -> usize {
        self.visual_col_at(self.cursor_line, self.cursor_col) + 1
    }

    /// Find the closest char index on `line_idx` to a target visual column.
    fn char_col_for_visual_col(&self, line_idx: usize, target_visual: usize) -> usize {
        let mut width = 0usize;
        for (i, c) in self.lines[line_idx].chars().enumerate() {
            let cw = char_display_width(c);
            if width + cw > target_visual {
                // Pick the closer boundary
                if target_visual.saturating_sub(width) >= (width + cw).saturating_sub(target_visual) {
                    return i + 1;
                }
                return i;
            }
            width += cw;
        }
        self.line_char_len(line_idx)
    }

    pub fn set_cursor(&mut self, line: usize, col: usize) {
        let max_line = self.lines.len().saturating_sub(1);
        self.cursor_line = line.min(max_line);
        self.cursor_col = col.min(self.line_char_len(self.cursor_line));
        self.preferred_visual_col = None;
    }

    // --- VZ Diamond Cursor Movement (Ctrl+E / S / D / X) ---

    pub fn move_up(&mut self) {
        if self.cursor_line == 0 {
            return;
        }
        let target_vcol = self
            .preferred_visual_col
            .unwrap_or_else(|| self.visual_col_at(self.cursor_line, self.cursor_col));
        self.preferred_visual_col = Some(target_vcol);
        self.cursor_line -= 1;
        self.cursor_col = self.char_col_for_visual_col(self.cursor_line, target_vcol);
    }

    pub fn move_down(&mut self) {
        if self.cursor_line + 1 >= self.lines.len() {
            return;
        }
        let target_vcol = self
            .preferred_visual_col
            .unwrap_or_else(|| self.visual_col_at(self.cursor_line, self.cursor_col));
        self.preferred_visual_col = Some(target_vcol);
        self.cursor_line += 1;
        self.cursor_col = self.char_col_for_visual_col(self.cursor_line, target_vcol);
    }

    pub fn move_left(&mut self) {
        self.preferred_visual_col = None;
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.line_char_len(self.cursor_line);
        }
    }

    pub fn move_right(&mut self) {
        self.preferred_visual_col = None;
        let len = self.line_char_len(self.cursor_line);
        if self.cursor_col < len {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    // --- Word Movement (Ctrl+A / Ctrl+F) ---

    pub fn move_word_left(&mut self) {
        self.preferred_visual_col = None;
        if self.cursor_col == 0 {
            if self.cursor_line > 0 {
                self.cursor_line -= 1;
                self.cursor_col = self.line_char_len(self.cursor_line);
            }
            return;
        }

        let chars: Vec<char> = self.lines[self.cursor_line].chars().collect();
        let mut idx = self.cursor_col;

        // Skip trailing whitespace to the left
        while idx > 0 && classify_char(chars[idx - 1]) == CharClass::Whitespace {
            idx -= 1;
        }
        if idx > 0 {
            let class = classify_char(chars[idx - 1]);
            while idx > 0 && classify_char(chars[idx - 1]) == class {
                idx -= 1;
            }
        }
        self.cursor_col = idx;
    }

    pub fn move_word_right(&mut self) {
        self.preferred_visual_col = None;
        let chars: Vec<char> = self.lines[self.cursor_line].chars().collect();
        let len = chars.len();
        if self.cursor_col >= len {
            if self.cursor_line + 1 < self.lines.len() {
                self.cursor_line += 1;
                self.cursor_col = 0;
            }
            return;
        }

        let mut idx = self.cursor_col;
        let start_class = classify_char(chars[idx]);
        if start_class != CharClass::Whitespace {
            while idx < len && classify_char(chars[idx]) == start_class {
                idx += 1;
            }
        }
        // Skip whitespace after the word so cursor lands on the start of the next word (VZ style)
        while idx < len && classify_char(chars[idx]) == CharClass::Whitespace {
            idx += 1;
        }
        self.cursor_col = idx;
    }

    // --- Line & Document Navigation (Ctrl+Q+S / Ctrl+Q+D / Home / End / PageUp / PageDown) ---

    pub fn move_line_start(&mut self) {
        self.preferred_visual_col = None;
        self.cursor_col = 0;
    }

    pub fn move_line_end(&mut self) {
        self.preferred_visual_col = None;
        self.cursor_col = self.line_char_len(self.cursor_line);
    }

    pub fn move_doc_start(&mut self) {
        self.preferred_visual_col = None;
        self.cursor_line = 0;
        self.cursor_col = 0;
    }

    pub fn move_doc_end(&mut self) {
        self.preferred_visual_col = None;
        self.cursor_line = self.lines.len().saturating_sub(1);
        self.cursor_col = self.line_char_len(self.cursor_line);
    }

    pub fn move_page_up(&mut self, page_lines: usize) {
        let step = page_lines.max(1);
        let target_vcol = self
            .preferred_visual_col
            .unwrap_or_else(|| self.visual_col_at(self.cursor_line, self.cursor_col));
        self.preferred_visual_col = Some(target_vcol);
        self.cursor_line = self.cursor_line.saturating_sub(step);
        self.cursor_col = self.char_col_for_visual_col(self.cursor_line, target_vcol);
    }

    pub fn move_page_down(&mut self, page_lines: usize) {
        let step = page_lines.max(1);
        let target_vcol = self
            .preferred_visual_col
            .unwrap_or_else(|| self.visual_col_at(self.cursor_line, self.cursor_col));
        self.preferred_visual_col = Some(target_vcol);
        let max_line = self.lines.len().saturating_sub(1);
        self.cursor_line = (self.cursor_line + step).min(max_line);
        self.cursor_col = self.char_col_for_visual_col(self.cursor_line, target_vcol);
    }

    // --- Editing Operations ---

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.preferred_visual_col = None;
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut parts = normalized.split('\n');

        if let Some(first) = parts.next() {
            let byte_idx = char_to_byte_idx(&self.lines[self.cursor_line], self.cursor_col);
            self.lines[self.cursor_line].insert_str(byte_idx, first);
            self.cursor_col += first.chars().count();
        }

        for part in parts {
            self.insert_newline_internal();
            if !part.is_empty() {
                let byte_idx = char_to_byte_idx(&self.lines[self.cursor_line], self.cursor_col);
                self.lines[self.cursor_line].insert_str(byte_idx, part);
                self.cursor_col += part.chars().count();
            }
        }
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn insert_newline(&mut self) {
        self.preferred_visual_col = None;
        self.insert_newline_internal();
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    fn insert_newline_internal(&mut self) {
        let byte_idx = char_to_byte_idx(&self.lines[self.cursor_line], self.cursor_col);
        let tail = self.lines[self.cursor_line].split_off(byte_idx);
        self.cursor_line += 1;
        self.lines.insert(self.cursor_line, tail);
        self.cursor_col = 0;
    }

    /// Delete the character before the cursor (BackSpace / Ctrl+H).
    pub fn backspace(&mut self) {
        self.preferred_visual_col = None;
        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_line];
            let start = char_to_byte_idx(line, self.cursor_col - 1);
            let end = char_to_byte_idx(line, self.cursor_col);
            line.replace_range(start..end, "");
            self.cursor_col -= 1;
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        } else if self.cursor_line > 0 {
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            let prev_len = self.line_char_len(self.cursor_line);
            self.lines[self.cursor_line].push_str(&current);
            self.cursor_col = prev_len;
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Delete the character at the cursor (Ctrl+G / Delete).
    pub fn delete_char_at_cursor(&mut self) {
        self.preferred_visual_col = None;
        let len = self.line_char_len(self.cursor_line);
        if self.cursor_col < len {
            let line = &mut self.lines[self.cursor_line];
            let start = char_to_byte_idx(line, self.cursor_col);
            let end = char_to_byte_idx(line, self.cursor_col + 1);
            line.replace_range(start..end, "");
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        } else if self.cursor_line + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Delete the entire current line (Ctrl+Y).
    pub fn delete_current_line(&mut self) {
        self.preferred_visual_col = None;
        if self.lines.len() == 1 {
            if !self.lines[0].is_empty() {
                self.lines[0].clear();
                self.cursor_col = 0;
                self.dirty = true;
                self.revision = self.revision.wrapping_add(1);
            }
            return;
        }

        self.lines.remove(self.cursor_line);
        if self.cursor_line >= self.lines.len() {
            self.cursor_line = self.lines.len() - 1;
        }
        let max_col = self.line_char_len(self.cursor_line);
        self.cursor_col = self.cursor_col.min(max_col);
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diamond_movement_and_editing() {
        let mut buf = TextBuffer::new();
        buf.insert_text("こんにちは世界\nabc def");
        assert_eq!(buf.lines, vec!["こんにちは世界", "abc def"]);
        assert_eq!((buf.cursor_line, buf.cursor_col), (1, 7));

        buf.move_line_start();
        assert_eq!(buf.cursor_col, 0);

        buf.move_word_right();
        assert_eq!(buf.cursor_col, 4); // lands on 'd'

        buf.move_word_left();
        assert_eq!(buf.cursor_col, 0);

        buf.move_up();
        assert_eq!(buf.cursor_line, 0);
        assert_eq!(buf.cursor_col, 0);

        buf.move_line_end();
        assert_eq!(buf.cursor_col, 7);

        buf.backspace();
        assert_eq!(buf.lines[0], "こんにちは世");

        buf.move_line_start();
        buf.delete_char_at_cursor();
        assert_eq!(buf.lines[0], "んにちは世");

        buf.delete_current_line();
        assert_eq!(buf.lines, vec!["abc def"]);
        assert_eq!(buf.cursor_line, 0);
    }

    #[test]
    fn test_japanese_word_movement() {
        let mut buf = TextBuffer::new();
        buf.insert_text("日本語テストです hello");
        buf.move_line_start();
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 3); // after 日本語
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 6); // after テスト
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 9); // after です + space -> hello
        buf.move_word_left();
        assert_eq!(buf.cursor_col, 6); // back to です
    }

    #[test]
    fn test_is_markdown_and_revision() {
        let mut buf = TextBuffer::new();
        assert!(!buf.is_markdown());
        assert_eq!(buf.revision, 0);

        buf.file_path = Some(PathBuf::from("README.md"));
        assert!(buf.is_markdown());

        buf.file_path = Some(PathBuf::from("/tmp/notes.MARKDOWN"));
        assert!(buf.is_markdown());

        buf.file_path = Some(PathBuf::from("main.rs"));
        assert!(!buf.is_markdown());

        buf.insert_text("# Title");
        let rev1 = buf.revision;
        assert!(rev1 > 0);
        buf.insert_newline();
        assert!(buf.revision > rev1);
    }
}
