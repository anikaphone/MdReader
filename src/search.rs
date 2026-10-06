use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub byte_offset: usize,
    pub line_number: usize,
    pub line_preview: String,
}

#[derive(Default, Debug)]
pub struct SearchState {
    pub is_open: bool,
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub current_match: usize,
    pub request_focus: bool,
}

impl SearchState {
    pub fn open(&mut self) {
        self.is_open = true;
        self.request_focus = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.query.clear();
        self.matches.clear();
        self.current_match = 0;
    }

    pub fn update(&mut self, text: &str) {
        self.matches.clear();
        let query_trimmed = self.query.trim();
        if query_trimmed.is_empty() {
            self.current_match = 0;
            return;
        }

        // Fast search with line detection
        let mut line_starts: Vec<(usize, usize)> = Vec::new(); // (line_number, byte_offset)
        line_starts.push((1, 0));
        for (idx, ch) in text.char_indices() {
            if ch == '\n' {
                line_starts.push((line_starts.len() + 1, idx + 1));
            }
        }

        let find_line_info = |offset: usize| -> (usize, String) {
            let line_idx = match line_starts.binary_search_by_key(&offset, |&(_, start)| start) {
                Ok(idx) => idx,
                Err(idx) => idx.saturating_sub(1),
            };
            let (line_num, line_start) = line_starts[line_idx];
            let line_end = line_starts
                .get(line_idx + 1)
                .map(|&(_, s)| s.saturating_sub(1))
                .unwrap_or(text.len());
            let line_text =
                if line_start <= text.len() && line_end <= text.len() && line_start <= line_end {
                    text[line_start..line_end].trim().to_string()
                } else {
                    String::new()
                };
            (line_num, line_text)
        };

        let mut options = Options::all();
        options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
        let parser = Parser::new_ext(text, options).into_offset_iter();

        let mut in_image = false;
        let mut in_code_block = false;

        for (event, range) in parser {
            match event {
                Event::Start(Tag::Image { .. }) => {
                    in_image = true;
                }
                Event::End(TagEnd::Image) => {
                    in_image = false;
                }
                Event::Start(Tag::CodeBlock(_)) => {
                    in_code_block = true;
                }
                Event::End(TagEnd::CodeBlock) => {
                    in_code_block = false;
                }
                Event::Text(t) | Event::Code(t) | Event::InlineHtml(t) | Event::Html(t)
                    if !in_image && !in_code_block =>
                {
                    for (match_start, _) in find_matches_in_text(&t, query_trimmed) {
                        let doc_offset = (range.start + match_start).min(text.len());
                        let (line_num, line_text) = find_line_info(doc_offset);
                        self.matches.push(SearchMatch {
                            byte_offset: doc_offset,
                            line_number: line_num,
                            line_preview: line_text,
                        });
                    }
                }
                _ => {}
            }
        }

        if !self.matches.is_empty() {
            if self.current_match >= self.matches.len() {
                self.current_match = 0;
            }
        } else {
            self.current_match = 0;
        }
    }

    pub fn next(&mut self) -> bool {
        if !self.matches.is_empty() {
            self.current_match = (self.current_match + 1) % self.matches.len();
            true
        } else {
            false
        }
    }

    pub fn prev(&mut self) -> bool {
        if !self.matches.is_empty() {
            if self.current_match == 0 {
                self.current_match = self.matches.len() - 1;
            } else {
                self.current_match -= 1;
            }
            true
        } else {
            false
        }
    }

    pub fn current_match_info(&self) -> Option<&SearchMatch> {
        self.matches.get(self.current_match)
    }

    pub fn current_match_ratio(&self, content: &str) -> f32 {
        if let Some(m) = self.current_match_info() {
            calculate_line_y_ratio(content, m.byte_offset)
        } else {
            0.0
        }
    }

    pub fn match_status(&self) -> String {
        if self.query.trim().is_empty() {
            String::new()
        } else if self.matches.is_empty() {
            "未找到匹配项".to_string()
        } else {
            let current = self.current_match + 1;
            let total = self.matches.len();
            if let Some(m) = self.current_match_info() {
                format!("{}/{} (第 {} 行)", current, total, m.line_number)
            } else {
                format!("{}/{}", current, total)
            }
        }
    }
}

/// Finds non-overlapping matches of `query` in `text` case-insensitively,
/// returning byte ranges `(start, end)` directly indexed into the original `text`.
///
/// Guaranteed:
/// - `start` and `end` are always valid UTF-8 character boundaries in `text`.
/// - `start <= end <= text.len()`.
/// - Never panics on Unicode case expansions (e.g. Turkish İ, German ß, etc.).
pub fn find_matches_in_text(text: &str, query: &str) -> Vec<(usize, usize)> {
    let query_trimmed = query.trim();
    if query_trimmed.is_empty() || text.is_empty() {
        return Vec::new();
    }

    let query_lower = query_trimmed.to_lowercase();
    let query_lower_chars: Vec<char> = query_lower.chars().collect();
    let mut matches = Vec::new();

    let char_indices: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < char_indices.len() {
        let start_byte = char_indices[i].0;
        let mut matched = true;
        let mut q_idx = 0;
        let mut text_char_idx = i;

        while q_idx < query_lower_chars.len() && text_char_idx < char_indices.len() {
            let tc = char_indices[text_char_idx].1;
            for tc_lower in tc.to_lowercase() {
                if q_idx < query_lower_chars.len() && tc_lower == query_lower_chars[q_idx] {
                    q_idx += 1;
                } else {
                    matched = false;
                    break;
                }
            }
            if !matched {
                break;
            }
            text_char_idx += 1;
        }

        if matched && q_idx == query_lower_chars.len() {
            let end_byte = if text_char_idx < char_indices.len() {
                char_indices[text_char_idx].0
            } else {
                text.len()
            };
            matches.push((start_byte, end_byte));
            // Advance past this match (non-overlapping)
            i = text_char_idx;
        } else {
            i += 1;
        }
    }

    matches
}

pub fn calculate_line_y_ratio(content: &str, target_byte_offset: usize) -> f32 {
    let mut total_weight = 0.0f32;
    let mut target_weight = 0.0f32;
    let mut reached_target = false;
    let mut current_offset = 0;

    let mut in_code_block = false;

    for line in content.lines() {
        let line_len = line.len() + 1;
        let trimmed = line.trim();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
        }

        let weight = if in_code_block {
            20.0
        } else if trimmed.starts_with("# ") {
            48.0
        } else if trimmed.starts_with("## ") {
            38.0
        } else if trimmed.starts_with("### ") {
            30.0
        } else if trimmed.starts_with("#### ") {
            26.0
        } else if trimmed.starts_with("|") {
            28.0
        } else if trimmed.is_empty() {
            12.0
        } else {
            let char_count = line.chars().count();
            let estimated_lines = (char_count as f32 / 42.0).ceil().max(1.0);
            estimated_lines * 22.0
        };

        if !reached_target && current_offset + line_len >= target_byte_offset {
            target_weight = total_weight;
            reached_target = true;
        }

        total_weight += weight;
        current_offset += line_len;
    }

    if total_weight <= 0.0 {
        0.0
    } else {
        (target_weight / total_weight).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_and_ratio() {
        let doc = "# Title\n\nSome introductory text.\n\n## Section 1\nRust is awesome.\n\n## Section 2\nRust runs fast.\n";
        let mut search = SearchState {
            query: "Rust".to_string(),
            ..Default::default()
        };
        search.update(doc);

        assert_eq!(search.matches.len(), 2);
        assert_eq!(search.matches[0].line_number, 6);
        assert_eq!(search.matches[1].line_number, 9);

        let ratio0 = search.current_match_ratio(doc);
        assert!(search.next());
        let ratio1 = search.current_match_ratio(doc);

        assert!(
            ratio0 < ratio1,
            "Expected ratio0 ({}) < ratio1 ({})",
            ratio0,
            ratio1
        );
    }

    #[test]
    fn test_unicode_case_search() {
        // Turkish İ (U+0130) lowercase expands from 2 bytes to 3 bytes (i + combining dot)
        let text = "Hello İSTANBUL world";
        let matches = find_matches_in_text(text, "İ");
        assert_eq!(matches.len(), 1);
        let (start, end) = matches[0];
        assert_eq!(&text[start..end], "İ");

        let matches_word = find_matches_in_text(text, "İstan");
        assert_eq!(matches_word.len(), 1);
        let (start, end) = matches_word[0];
        assert_eq!(&text[start..end], "İSTAN");

        // Chinese text search
        let zh_text = "这是一个Markdown阅读器测试";
        let zh_matches = find_matches_in_text(zh_text, "markdown");
        assert_eq!(zh_matches.len(), 1);
        assert_eq!(&zh_text[zh_matches[0].0..zh_matches[0].1], "Markdown");

        // Slicing with returned ranges must never panic
        for (s, e) in zh_matches {
            let _ = &zh_text[s..e];
        }
    }

    #[test]
    fn test_search_visible_text_only() {
        let doc = "Check out [Rust](https://rust-lang.org) and ![Rust Logo](images/rust.png).\n\n```rust\nfn rust_test() {}\n```\n";
        let mut search = SearchState {
            query: "rust".to_string(),
            ..Default::default()
        };
        search.update(doc);

        // Only the visible link text "[Rust]" should match!
        // Invisible URL "https://rust-lang.org", image alt/path, and code block should NOT be counted.
        assert_eq!(search.matches.len(), 1);
        assert_eq!(search.matches[0].line_number, 1);
    }

    #[test]
    fn test_search_html_block_text() {
        let doc = "<div>Rust content</div>\n";
        let mut search = SearchState {
            query: "rust".to_string(),
            ..Default::default()
        };
        search.update(doc);

        assert_eq!(search.matches.len(), 1);
        assert_eq!(search.matches[0].line_number, 1);
    }
}
