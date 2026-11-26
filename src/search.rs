use crate::buffer::TextBuffer;

/// A single non-overlapping match within a buffer line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    /// 0-indexed line index
    pub line: usize,
    /// 0-indexed start character column (matching `TextBuffer::cursor_col`)
    pub start_col: usize,
    /// 0-indexed exclusive end character column
    pub end_col: usize,
    /// Start byte offset within `lines[line]`
    pub start_byte: usize,
    /// Exclusive end byte offset within `lines[line]`
    pub end_byte: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFocus {
    Find,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindOutcome {
    EmptyQuery,
    NotFound,
    Found {
        index: usize,
        total: usize,
        wrapped: bool,
        matched: SearchMatch,
    },
}

/// Find all non-overlapping occurrences of `query` across `lines`.
/// Guaranteed to produce valid UTF-8 byte offsets and exact character column indices,
/// including for Japanese / full-width / mixed-case Unicode strings.
pub fn find_all_matches(lines: &[String], query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();

    if case_sensitive {
        let query_char_len = query.chars().count();
        for (line_idx, line) in lines.iter().enumerate() {
            if !line.contains(query) {
                continue;
            }
            let mut byte_cursor = 0usize;
            let mut char_cursor = 0usize;
            for (start_byte, matched) in line.match_indices(query) {
                char_cursor += line[byte_cursor..start_byte].chars().count();
                let start_col = char_cursor;
                let end_byte = start_byte + matched.len();
                let end_col = start_col + query_char_len;
                char_cursor = end_col;
                byte_cursor = end_byte;
                out.push(SearchMatch {
                    line: line_idx,
                    start_col,
                    end_col,
                    start_byte,
                    end_byte,
                });
            }
        }
    } else {
        let query_lower: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
        if query_lower.is_empty() {
            return out;
        }
        for (line_idx, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let chars: Vec<(usize, char)> = line.char_indices().collect();
            let mut i = 0usize;
            while i < chars.len() {
                let mut j = i;
                let mut qk = 0usize;
                let mut ok = true;
                while qk < query_lower.len() {
                    if j >= chars.len() {
                        ok = false;
                        break;
                    }
                    let mut char_matched = true;
                    for lc in chars[j].1.to_lowercase() {
                        if qk >= query_lower.len() || query_lower[qk] != lc {
                            char_matched = false;
                            break;
                        }
                        qk += 1;
                    }
                    if !char_matched {
                        ok = false;
                        break;
                    }
                    j += 1;
                }
                if ok && j > i {
                    let start_byte = chars[i].0;
                    let end_byte = if j < chars.len() {
                        chars[j].0
                    } else {
                        line.len()
                    };
                    out.push(SearchMatch {
                        line: line_idx,
                        start_col: i,
                        end_col: j,
                        start_byte,
                        end_byte,
                    });
                    i = j;
                } else {
                    i += 1;
                }
            }
        }
    }

    out
}

/// Persistent search & replace state with revision-based match caching.
#[derive(Debug, Clone)]
pub struct SearchState {
    /// Whether the bottom search/replace bar is currently visible
    pub open: bool,
    /// Whether the replace row is also shown (Ctrl+Q+A / Ctrl+H)
    pub replace_mode: bool,
    /// Whether matches should be highlighted on the editor canvas
    pub highlight_active: bool,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    /// Pending focus request for the next UI render frame
    pub focus_target: Option<SearchFocus>,
    /// If true, select all text inside the newly focused TextEdit
    pub select_all_on_focus: bool,
    /// Cursor position recorded when incremental search started
    pub search_origin: (usize, usize),

    matches: Vec<SearchMatch>,
    pub current_match: Option<usize>,
    cache_key: Option<(usize, u64, String, bool)>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            open: false,
            replace_mode: false,
            highlight_active: false,
            query: String::new(),
            replacement: String::new(),
            case_sensitive: false,
            focus_target: None,
            select_all_on_focus: false,
            search_origin: (0, 0),
            matches: Vec::new(),
            current_match: None,
            cache_key: None,
        }
    }

    pub fn invalidate_cache(&mut self) {
        self.cache_key = None;
    }

    /// Recompute cached matches only when the active buffer, revision, query, or case sensitivity changes.
    pub fn ensure_matches(&mut self, buf_idx: usize, buf: &TextBuffer) {
        let valid = match &self.cache_key {
            Some((b, rev, q, cs)) => {
                *b == buf_idx
                    && *rev == buf.revision
                    && *q == self.query
                    && *cs == self.case_sensitive
            }
            None => false,
        };
        if valid {
            return;
        }
        self.matches = find_all_matches(&buf.lines, &self.query, self.case_sensitive);
        self.cache_key = Some((buf_idx, buf.revision, self.query.clone(), self.case_sensitive));
        self.sync_current_match_with_cursor(buf.cursor_line, buf.cursor_col);
    }

    pub fn matches(&self) -> &[SearchMatch] {
        &self.matches
    }

    /// Return `(first_global_idx, matches_slice)` for all matches on `line_idx` in O(log M).
    pub fn matches_on_line(&self, line_idx: usize) -> (usize, &[SearchMatch]) {
        let start = self.matches.partition_point(|m| m.line < line_idx);
        let len = self.matches[start..].partition_point(|m| m.line == line_idx);
        (start, &self.matches[start..start + len])
    }

    /// Find the index of the match containing `(line, col)`, if any.
    pub fn match_at_cursor(&self, line: usize, col: usize) -> Option<usize> {
        let (base, slice) = self.matches_on_line(line);
        slice
            .iter()
            .position(|m| col >= m.start_col && col < m.end_col)
            .map(|offset| base + offset)
    }

    pub fn sync_current_match_with_cursor(&mut self, line: usize, col: usize) {
        if self.matches.is_empty() {
            self.current_match = None;
            return;
        }
        if let Some(idx) = self.match_at_cursor(line, col) {
            self.current_match = Some(idx);
        } else if self.current_match.is_some_and(|i| i >= self.matches.len()) {
            self.current_match = None;
        }
    }

    /// Find the first match at or after `(line, col)`, wrapping around to index 0 if needed.
    pub fn find_at_or_after(&mut self, line: usize, col: usize) -> FindOutcome {
        if self.query.is_empty() {
            self.current_match = None;
            return FindOutcome::EmptyQuery;
        }
        if self.matches.is_empty() {
            self.current_match = None;
            return FindOutcome::NotFound;
        }
        let pos = self
            .matches
            .partition_point(|m| (m.line, m.start_col) < (line, col));
        let (index, wrapped) = if pos < self.matches.len() {
            (pos, false)
        } else {
            (0, true)
        };
        self.current_match = Some(index);
        FindOutcome::Found {
            index,
            total: self.matches.len(),
            wrapped,
            matched: self.matches[index],
        }
    }

    /// Find the first match strictly after `(line, col)`, wrapping around to index 0 if needed.
    pub fn find_next_after(&mut self, line: usize, col: usize) -> FindOutcome {
        if self.query.is_empty() {
            self.current_match = None;
            return FindOutcome::EmptyQuery;
        }
        if self.matches.is_empty() {
            self.current_match = None;
            return FindOutcome::NotFound;
        }
        let pos = self
            .matches
            .partition_point(|m| (m.line, m.start_col) <= (line, col));
        let (index, wrapped) = if pos < self.matches.len() {
            (pos, false)
        } else {
            (0, true)
        };
        self.current_match = Some(index);
        FindOutcome::Found {
            index,
            total: self.matches.len(),
            wrapped,
            matched: self.matches[index],
        }
    }

    /// Find the last match strictly before `(line, col)`, wrapping around to the last match if needed.
    pub fn find_prev_before(&mut self, line: usize, col: usize) -> FindOutcome {
        if self.query.is_empty() {
            self.current_match = None;
            return FindOutcome::EmptyQuery;
        }
        if self.matches.is_empty() {
            self.current_match = None;
            return FindOutcome::NotFound;
        }
        let pos = self
            .matches
            .partition_point(|m| (m.line, m.start_col) < (line, col));
        let (index, wrapped) = if pos > 0 {
            (pos - 1, false)
        } else {
            (self.matches.len() - 1, true)
        };
        self.current_match = Some(index);
        FindOutcome::Found {
            index,
            total: self.matches.len(),
            wrapped,
            matched: self.matches[index],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_all_matches_case_sensitive_and_insensitive_japanese() {
        let lines = vec![
            "VZ Editor と vz editor".to_string(),
            "検索テスト: 日本語検索と検索".to_string(),
        ];

        // Case-insensitive ASCII
        let m_ci = find_all_matches(&lines, "vz", false);
        assert_eq!(m_ci.len(), 2);
        assert_eq!((m_ci[0].line, m_ci[0].start_col, m_ci[0].end_col), (0, 0, 2));
        assert_eq!(
            (m_ci[1].line, m_ci[1].start_col, m_ci[1].end_col),
            (0, 12, 14)
        );
        assert_eq!(&lines[0][m_ci[0].start_byte..m_ci[0].end_byte], "VZ");
        assert_eq!(&lines[0][m_ci[1].start_byte..m_ci[1].end_byte], "vz");

        // Case-sensitive ASCII
        let m_cs = find_all_matches(&lines, "VZ", true);
        assert_eq!(m_cs.len(), 1);
        assert_eq!((m_cs[0].line, m_cs[0].start_col, m_cs[0].end_col), (0, 0, 2));

        // Japanese multibyte (both case-sensitive and case-insensitive)
        for &cs in &[false, true] {
            let m_ja = find_all_matches(&lines, "検索", cs);
            assert_eq!(m_ja.len(), 3);
            assert_eq!((m_ja[0].line, m_ja[0].start_col, m_ja[0].end_col), (1, 0, 2));
            assert_eq!(
                (m_ja[1].line, m_ja[1].start_col, m_ja[1].end_col),
                (1, 10, 12)
            );
            assert_eq!(
                (m_ja[2].line, m_ja[2].start_col, m_ja[2].end_col),
                (1, 13, 15)
            );
            for m in &m_ja {
                assert_eq!(&lines[m.line][m.start_byte..m.end_byte], "検索");
            }
        }
    }

    #[test]
    fn test_non_overlapping_and_navigation_wrap_around() {
        let mut buf = TextBuffer::new();
        buf.insert_text("aaaa\nfoo bar foo\nfoo");

        let mut st = SearchState::new();
        st.query = "aa".to_string();
        st.ensure_matches(0, &buf);
        // "aaaa" has 2 non-overlapping matches: 0..2 and 2..4
        assert_eq!(st.matches().len(), 2);
        assert_eq!(st.matches()[0].start_col, 0);
        assert_eq!(st.matches()[1].start_col, 2);

        st.query = "foo".to_string();
        st.ensure_matches(0, &buf);
        assert_eq!(st.matches().len(), 3);

        // Next from (0, 0) -> (1, 0)
        assert!(matches!(
            st.find_next_after(0, 0),
            FindOutcome::Found {
                index: 0,
                wrapped: false,
                ..
            }
        ));
        // Next from (1, 0) -> (1, 8)
        assert!(matches!(
            st.find_next_after(1, 0),
            FindOutcome::Found {
                index: 1,
                wrapped: false,
                ..
            }
        ));
        // Next from (2, 0) -> wraps to (1, 0)
        assert!(matches!(
            st.find_next_after(2, 0),
            FindOutcome::Found {
                index: 0,
                wrapped: true,
                ..
            }
        ));
        // Prev from (1, 0) -> wraps to (2, 0)
        assert!(matches!(
            st.find_prev_before(1, 0),
            FindOutcome::Found {
                index: 2,
                wrapped: true,
                ..
            }
        ));

        // matches_on_line
        let (base, line1) = st.matches_on_line(1);
        assert_eq!(base, 0);
        assert_eq!(line1.len(), 2);
        let (base2, line2) = st.matches_on_line(2);
        assert_eq!(base2, 2);
        assert_eq!(line2.len(), 1);
    }
}
