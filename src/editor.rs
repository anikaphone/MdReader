use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Reading,
    Editing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PendingDocumentAction {
    SwitchToReading,
    OpenFile {
        path: PathBuf,
        fragment: Option<String>,
    },
    ReloadCurrentFile,
    CloseApplication,
    HandleExternalChange,
}

/// Calculate a 64-bit fingerprint for document text content using standard DefaultHasher.
pub fn content_fingerprint(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Calculate line count for text.
pub fn calculate_line_count(content: &str) -> usize {
    content.lines().count()
}

/// Calculate word/character count for text (matching existing logic).
pub fn calculate_word_count(content: &str) -> usize {
    content.split_whitespace().map(|s| s.chars().count()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_fingerprint_consistency() {
        let content1 = "# Hello World\nThis is a test.";
        let content2 = "# Hello World\nThis is a test.";
        assert_eq!(content_fingerprint(content1), content_fingerprint(content2));
    }

    #[test]
    fn test_content_fingerprint_difference() {
        let content1 = "# Hello World\nThis is a test.";
        let content2 = "# Hello World\nThis is a test!";
        assert_ne!(content_fingerprint(content1), content_fingerprint(content2));
    }

    #[test]
    fn test_content_fingerprint_unicode() {
        let zh1 = "## 欢迎使用 MdReader\n支持中文与 Emoji 🎉";
        let zh2 = "## 欢迎使用 MdReader\n支持中文与 Emoji 🎉";
        let zh3 = "## 欢迎使用 MdReader\n支持中文与 Emoji ✨";
        assert_eq!(content_fingerprint(zh1), content_fingerprint(zh2));
        assert_ne!(content_fingerprint(zh1), content_fingerprint(zh3));
    }

    #[test]
    fn test_calculate_line_and_word_count() {
        let doc = "Line 1\nLine 2\nLine 3";
        assert_eq!(calculate_line_count(doc), 3);
        assert_eq!(calculate_word_count(doc), 15); // "Line" 4 + "1" 1 + "Line" 4 + "2" 1 + "Line" 4 + "3" 1 = 15
    }

    #[test]
    fn test_dirty_detection_on_add_delete_replace() {
        let original = "# Title\nInitial content";
        let initial_fp = content_fingerprint(original);

        // Addition
        let added = "# Title\nInitial content\nExtra line";
        assert_ne!(initial_fp, content_fingerprint(added));

        // Deletion
        let deleted = "# Title";
        assert_ne!(initial_fp, content_fingerprint(deleted));

        // Replacement
        let replaced = "# Title\nReplaced content";
        assert_ne!(initial_fp, content_fingerprint(replaced));

        // Reverting back to original should restore clean state
        let restored = "# Title\nInitial content";
        assert_eq!(initial_fp, content_fingerprint(restored));
    }

    #[test]
    fn test_unicode_and_crlf_line_counts() {
        let crlf_doc = "第一行\r\n第二行包含 Emoji 🚀\r\n第三行结束\r\n";
        assert_eq!(calculate_line_count(crlf_doc), 3);
        // Ensure fingerprint works seamlessly with CRLF vs LF
        let lf_doc = "第一行\n第二行包含 Emoji 🚀\n第三行结束\n";
        assert_ne!(content_fingerprint(crlf_doc), content_fingerprint(lf_doc));
    }
}
