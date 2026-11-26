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

const MAX_UNDO_HISTORY: usize = 200;

#[derive(Debug, Clone)]
struct UndoSnapshot {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    dirty: bool,
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
    undo_stack: Vec<UndoSnapshot>,
    redo_stack: Vec<UndoSnapshot>,
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
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
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
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    pub fn save_to_path(&mut self, path: &Path) -> io::Result<()> {
        let content = self.lines.join(self.line_ending);
        fs::write(path, content)?;
        self.file_path = Some(path.to_path_buf());
        self.dirty = false;
        // Mark previous undo snapshots as dirty since they differ from the newly saved file
        for snap in &mut self.undo_stack {
            snap.dirty = true;
        }
        for snap in &mut self.redo_stack {
            snap.dirty = true;
        }
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

    // --- Undo / Redo ---

    fn current_snapshot(&self) -> UndoSnapshot {
        UndoSnapshot {
            lines: self.lines.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
            dirty: self.dirty,
        }
    }

    fn push_undo_snapshot(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(self.current_snapshot());
        self.redo_stack.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(self.current_snapshot());
        self.lines = prev.lines;
        self.cursor_line = prev.cursor_line;
        self.cursor_col = prev.cursor_col;
        self.dirty = prev.dirty;
        self.preferred_visual_col = None;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(self.current_snapshot());
        self.lines = next.lines;
        self.cursor_line = next.cursor_line;
        self.cursor_col = next.cursor_col;
        self.dirty = next.dirty;
        self.preferred_visual_col = None;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    // --- Editing Operations ---

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push_undo_snapshot();
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
        self.push_undo_snapshot();
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
            self.push_undo_snapshot();
            let line = &mut self.lines[self.cursor_line];
            let start = char_to_byte_idx(line, self.cursor_col - 1);
            let end = char_to_byte_idx(line, self.cursor_col);
            line.replace_range(start..end, "");
            self.cursor_col -= 1;
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        } else if self.cursor_line > 0 {
            self.push_undo_snapshot();
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
            self.push_undo_snapshot();
            let line = &mut self.lines[self.cursor_line];
            let start = char_to_byte_idx(line, self.cursor_col);
            let end = char_to_byte_idx(line, self.cursor_col + 1);
            line.replace_range(start..end, "");
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        } else if self.cursor_line + 1 < self.lines.len() {
            self.push_undo_snapshot();
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
                self.push_undo_snapshot();
                self.lines[0].clear();
                self.cursor_col = 0;
                self.dirty = true;
                self.revision = self.revision.wrapping_add(1);
            }
            return;
        }

        self.push_undo_snapshot();
        self.lines.remove(self.cursor_line);
        if self.cursor_line >= self.lines.len() {
            self.cursor_line = self.lines.len() - 1;
        }
        let max_col = self.line_char_len(self.cursor_line);
        self.cursor_col = self.cursor_col.min(max_col);
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Replace a byte range within a single line (supports Undo).
    /// Moves the cursor to the end of the inserted replacement and returns `(end_line, end_col)`.
    pub fn replace_in_line(
        &mut self,
        line: usize,
        start_byte: usize,
        end_byte: usize,
        replacement: &str,
    ) -> (usize, usize) {
        if line >= self.lines.len()
            || start_byte > end_byte
            || end_byte > self.lines[line].len()
            || !self.lines[line].is_char_boundary(start_byte)
            || !self.lines[line].is_char_boundary(end_byte)
        {
            return (self.cursor_line, self.cursor_col);
        }

        self.push_undo_snapshot();
        self.preferred_visual_col = None;

        let normalized = replacement.replace("\r\n", "\n").replace('\r', "\n");
        let start_col = self.lines[line][..start_byte].chars().count();

        if !normalized.contains('\n') {
            self.lines[line].replace_range(start_byte..end_byte, &normalized);
            self.cursor_line = line;
            self.cursor_col = start_col + normalized.chars().count();
        } else {
            let prefix = self.lines[line][..start_byte].to_string();
            let suffix = self.lines[line][end_byte..].to_string();
            let mut parts: Vec<String> = normalized.split('\n').map(String::from).collect();
            let last_part_chars = parts.last().map(|s| s.chars().count()).unwrap_or(0);
            if let Some(first) = parts.first_mut() {
                first.insert_str(0, &prefix);
            }
            if let Some(last) = parts.last_mut() {
                last.push_str(&suffix);
            }
            let added_lines = parts.len().saturating_sub(1);
            self.lines.splice(line..=line, parts);
            self.cursor_line = line + added_lines;
            self.cursor_col = last_part_chars;
        }

        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        (self.cursor_line, self.cursor_col)
    }

    /// Replace all non-overlapping `(line, start_byte, end_byte)` ranges (sorted in ascending order)
    /// in a single atomic Undo step. Returns the number of replaced occurrences.
    pub fn replace_all_matches(
        &mut self,
        matches: &[(usize, usize, usize)],
        replacement: &str,
    ) -> usize {
        if matches.is_empty() {
            return 0;
        }

        self.push_undo_snapshot();
        self.preferred_visual_col = None;

        let normalized = replacement.replace("\r\n", "\n").replace('\r', "\n");
        let has_newline = normalized.contains('\n');
        let mut new_lines: Vec<String> = Vec::with_capacity(self.lines.len());
        let mut replaced_count = 0usize;
        let mut mi = 0usize;

        for (line_idx, orig_line) in self.lines.iter().enumerate() {
            if mi >= matches.len() || matches[mi].0 != line_idx {
                new_lines.push(orig_line.clone());
                continue;
            }

            let mut rebuilt = String::with_capacity(orig_line.len());
            let mut last_byte = 0usize;
            while mi < matches.len() && matches[mi].0 == line_idx {
                let (_, start_byte, end_byte) = matches[mi];
                if start_byte >= last_byte
                    && start_byte <= end_byte
                    && end_byte <= orig_line.len()
                    && orig_line.is_char_boundary(start_byte)
                    && orig_line.is_char_boundary(end_byte)
                {
                    rebuilt.push_str(&orig_line[last_byte..start_byte]);
                    rebuilt.push_str(&normalized);
                    last_byte = end_byte;
                    replaced_count += 1;
                }
                mi += 1;
            }
            rebuilt.push_str(&orig_line[last_byte..]);

            if has_newline {
                for seg in rebuilt.split('\n') {
                    new_lines.push(seg.to_string());
                }
            } else {
                new_lines.push(rebuilt);
            }
        }

        if replaced_count == 0 {
            self.undo_stack.pop();
            return 0;
        }

        if new_lines.is_empty() {
            new_lines.push(String::new());
        }
        self.lines = new_lines;
        let max_line = self.lines.len().saturating_sub(1);
        self.cursor_line = self.cursor_line.min(max_line);
        let max_col = self.line_char_len(self.cursor_line);
        self.cursor_col = self.cursor_col.min(max_col);
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        replaced_count
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

    #[test]
    fn test_undo_redo() {
        let mut buf = TextBuffer::new();
        assert!(!buf.can_undo());
        assert!(!buf.can_redo());

        buf.insert_text("hello");
        assert!(buf.dirty);
        assert!(buf.can_undo());
        buf.insert_text(" world");
        assert_eq!(buf.lines, vec!["hello world"]);

        assert!(buf.undo());
        assert_eq!(buf.lines, vec!["hello"]);
        assert!(buf.dirty);
        assert!(buf.can_redo());

        assert!(buf.undo());
        assert_eq!(buf.lines, vec![""]);
        assert!(!buf.dirty);
        assert!(!buf.can_undo());

        assert!(buf.redo());
        assert_eq!(buf.lines, vec!["hello"]);
        assert!(buf.dirty);

        // New edit clears redo history
        buf.insert_text("!");
        assert!(!buf.can_redo());
        assert_eq!(buf.lines, vec!["hello!"]);
    }

    #[test]
    fn test_replace_in_line_and_replace_all_with_undo() {
        let mut buf = TextBuffer::new();
        buf.insert_text("こんにちは世界\n世界は広い 世界");

        // Replace single Japanese occurrence on line 0
        let start = "こんにちは".len();
        let end = start + "世界".len();
        let (end_line, end_col) = buf.replace_in_line(0, start, end, "宇宙");
        assert_eq!((end_line, end_col), (0, 7));
        assert_eq!(buf.lines[0], "こんにちは宇宙");

        // Undo reverts single replacement
        assert!(buf.undo());
        assert_eq!(buf.lines[0], "こんにちは世界");

        // Replace all 3 occurrences in one atomic undo step
        let s1 = "こんにちは".len();
        let e1 = s1 + "世界".len();
        let s2 = 0;
        let e2 = "世界".len();
        let s3 = "世界は広い ".len();
        let e3 = s3 + "世界".len();
        let count = buf.replace_all_matches(&[(0, s1, e1), (1, s2, e2), (1, s3, e3)], "地球");
        assert_eq!(count, 3);
        assert_eq!(buf.lines, vec!["こんにちは地球", "地球は広い 地球"]);

        // Single undo reverts all 3 replacements atomically
        assert!(buf.undo());
        assert_eq!(buf.lines, vec!["こんにちは世界", "世界は広い 世界"]);
    }
}
