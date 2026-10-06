use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone)]
pub struct TocItem {
    pub level: u8,
    pub title: String,
    pub anchor_id: String,
}

pub struct ProcessedMarkdown {
    pub rendered_markdown: String,
    pub toc: Vec<TocItem>,
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

pub fn process_markdown_with_toc(markdown: &str) -> ProcessedMarkdown {
    let mut options = Options::all();
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(markdown, options).into_offset_iter();

    struct HeadingMeta {
        level: u8,
        existing_id: Option<String>,
        title: String,
        replace_start: usize,
        replace_end: usize,
    }

    let mut current_meta: Option<HeadingMeta> = None;
    let mut headings = Vec::new();

    for (event, range) in parser {
        match event {
            Event::Start(Tag::Heading { level, id, .. }) => {
                let slice = &markdown[range.start..range.end.min(markdown.len())];
                let line_end_rel = slice.find(['\r', '\n']).unwrap_or(slice.len());
                let line_slice = &slice[..line_end_rel];
                // In CommonMark / GFM, an ATX heading closing sequence of '#' must be preceded by whitespace:
                // e.g. "## Title ###" -> closing "###" is preceded by space, so insert before closing "###".
                // but "## C#" -> '#' is part of title, NOT an ATX closing sequence!
                let trimmed_right = line_slice.trim_end_matches([' ', '\t']);
                let without_hashes = trimmed_right.trim_end_matches('#');
                let content_end = if without_hashes.ends_with([' ', '\t']) {
                    without_hashes.trim_end_matches([' ', '\t']).len()
                } else {
                    trimmed_right.len()
                };
                let replace_start = range.start + content_end;
                let replace_end = range.start + line_end_rel;

                current_meta = Some(HeadingMeta {
                    level: heading_level_to_u8(level),
                    existing_id: id.map(|s| s.to_string()),
                    title: String::new(),
                    replace_start,
                    replace_end,
                });
            }
            Event::Text(text) => {
                if let Some(h) = current_meta.as_mut() {
                    h.title.push_str(&text);
                }
            }
            Event::Code(code) => {
                if let Some(h) = current_meta.as_mut() {
                    h.title.push('`');
                    h.title.push_str(&code);
                    h.title.push('`');
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(h) = current_meta.take() {
                    headings.push(h);
                }
            }
            _ => {}
        }
    }

    // Now construct the rendered_markdown and the TOC list
    let mut toc = Vec::new();
    let mut rendered_markdown = String::with_capacity(markdown.len() + headings.len() * 20);
    let mut last_idx = 0;

    for (i, h) in headings.into_iter().enumerate() {
        let (anchor_id, need_insert) = if let Some(existing) = h.existing_id {
            (existing, false)
        } else {
            (format!("toc-h-{}", i), true)
        };

        if need_insert {
            let start = h.replace_start.min(markdown.len());
            let end = h.replace_end.min(markdown.len());
            if start >= last_idx {
                rendered_markdown.push_str(&markdown[last_idx..start]);
                rendered_markdown.push_str(&format!(" {{#{}}}", anchor_id));
                last_idx = end;
            }
        }

        toc.push(TocItem {
            level: h.level,
            title: h.title.trim().to_string(),
            anchor_id,
        });
    }

    if last_idx < markdown.len() {
        rendered_markdown.push_str(&markdown[last_idx..]);
    }

    ProcessedMarkdown {
        rendered_markdown,
        toc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heading_attributes_and_toc() {
        let md = "# MdReader 技术文档与排版演示\r\n\r\n## 1. 核心设计特色\r\n正文内容\r\n## 2. 任务清单 {#my-custom-task}\r\n列表内容";
        let res = process_markdown_with_toc(md);

        assert_eq!(res.toc.len(), 3);
        assert_eq!(res.toc[0].title, "MdReader 技术文档与排版演示");
        assert_eq!(res.toc[0].anchor_id, "toc-h-0");

        assert_eq!(res.toc[1].title, "1. 核心设计特色");
        assert_eq!(res.toc[1].anchor_id, "toc-h-1");

        assert_eq!(res.toc[2].title, "2. 任务清单");
        assert_eq!(res.toc[2].anchor_id, "my-custom-task");

        // Verify that re-parsing with pulldown-cmark recognizes every heading's id
        let mut opts = Options::all();
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
        let parser = Parser::new_ext(&res.rendered_markdown, opts);

        let mut parsed_heading_ids = Vec::new();
        for event in parser {
            match event {
                Event::Start(Tag::Heading { id, .. }) => {
                    parsed_heading_ids.push(id.map(|s| s.to_string()));
                }
                Event::Text(t) => {
                    assert!(
                        !t.contains("{#"),
                        "Visible text contains anchor syntax: {}",
                        t
                    );
                }
                _ => {}
            }
        }

        assert_eq!(
            parsed_heading_ids,
            vec![
                Some("toc-h-0".to_string()),
                Some("toc-h-1".to_string()),
                Some("my-custom-task".to_string()),
            ]
        );
    }

    #[test]
    fn test_demo_md_rendering() {
        let content = std::fs::read_to_string("demo.md").unwrap();
        let res = process_markdown_with_toc(&content);

        let mut opts = Options::all();
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
        let parser = Parser::new_ext(&res.rendered_markdown, opts);

        let mut count = 0;
        for event in parser {
            match event {
                Event::Start(Tag::Heading { id, .. }) => {
                    assert!(
                        id.is_some(),
                        "Heading missing id in demo.md: count = {}",
                        count
                    );
                    count += 1;
                }
                Event::Text(t) => {
                    assert!(
                        !t.contains("{#"),
                        "Visible text in demo.md contains anchor syntax: {}",
                        t
                    );
                }
                _ => {}
            }
        }
        assert_eq!(count, res.toc.len());
    }

    #[test]
    fn test_headings_with_literal_hashes() {
        let md = "# C# Programming\n\n## Deep Dive into C#\n\n### Title with ATX Closing ###\n";
        let res = process_markdown_with_toc(md);

        assert_eq!(res.toc.len(), 3);
        assert_eq!(res.toc[0].title, "C# Programming");
        assert_eq!(res.toc[1].title, "Deep Dive into C#");
        assert_eq!(res.toc[2].title, "Title with ATX Closing");

        let mut opts = Options::all();
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
        let parser = Parser::new_ext(&res.rendered_markdown, opts);

        for event in parser {
            if let Event::Text(t) = event {
                assert!(
                    !t.contains("{#"),
                    "Anchor syntax leaked into heading text: {}",
                    t
                );
            }
        }
    }
}
