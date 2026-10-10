use std::iter::Peekable;
use std::ops::Range;
use std::path::{Path, PathBuf};

use egui::{self, Color32, Id, TextStyle, Ui};
use egui_commonmark_backend::elements::*;
pub use egui_commonmark_backend::misc::CommonMarkCache;
use egui_commonmark_backend::misc::*;
use egui_commonmark_backend::pulldown::*;
use pulldown_cmark::{CowStr, HeadingLevel};

/// Resolves an image destination URL.
/// If `url` is a relative path (e.g. "images/sample.png" or "./sample.png"),
/// it joins it with `base_dir` and converts it into a standard `file:///` URI that egui's file loader understands.
pub fn resolve_image_url(url: &str, base_dir: Option<&Path>) -> String {
    if url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("file://")
        || url.starts_with("data:")
        || url.starts_with("bytes://")
    {
        return url.to_string();
    }

    let path = Path::new(url);
    let full_path = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(base) = base_dir {
        base.join(path)
    } else {
        path.to_path_buf()
    };

    if cfg!(windows) {
        let clean = full_path.to_string_lossy().replace('\\', "/");
        // canonicalize() returns extended Windows paths. The file loader expects
        // a drive path or a UNC authority, not a URI containing the device prefix.
        if let Some(unc) = clean.strip_prefix("//?/UNC/") {
            return format!("file://{unc}");
        }
        let clean = clean.strip_prefix("//?/").unwrap_or(&clean);
        if clean.starts_with("//") {
            return format!("file:{clean}");
        }
        let clean = clean.trim_start_matches('/');
        format!("file:///{clean}")
    } else {
        let clean = full_path.to_string_lossy();
        format!("file://{clean}")
    }
}

pub(crate) struct ListLevel {
    current_number: Option<u64>,
}

#[derive(Default)]
pub(crate) struct List {
    items: Vec<ListLevel>,
    has_list_begun: bool,
}

impl List {
    pub fn start_level_with_number(&mut self, start_number: u64) {
        self.items.push(ListLevel {
            current_number: Some(start_number),
        });
    }

    pub fn start_level_without_number(&mut self) {
        self.items.push(ListLevel {
            current_number: None,
        });
    }

    pub fn is_inside_a_list(&self) -> bool {
        !self.items.is_empty()
    }

    pub fn is_last_level(&self) -> bool {
        self.items.len() == 1
    }

    pub fn start_item(&mut self, ui: &mut egui::Ui, options: &CommonMarkOptions) {
        if self.has_list_begun {
            newline(ui);
        } else {
            self.has_list_begun = true;
        }

        let len = self.items.len();
        if let Some(item) = self.items.last_mut() {
            ui.label(" ".repeat((len - 1) * options.indentation_spaces));

            if let Some(number) = &mut item.current_number {
                number_point(ui, &number.to_string());
                *number += 1;
            } else if len > 1 {
                bullet_point_hollow(ui);
            } else {
                bullet_point(ui);
            }
        } else {
            unreachable!();
        }

        ui.add_space(4.0);
    }

    pub fn end_level(&mut self, ui: &mut egui::Ui, insert_newline: bool) {
        self.items.pop();

        if self.items.is_empty() && insert_newline {
            newline(ui);
        }
    }
}

struct Newline {
    should_not_start_newline_forced: bool,
    should_start_newline: bool,
    should_end_newline: bool,
    should_end_newline_forced: bool,
}

impl Default for Newline {
    fn default() -> Self {
        Self {
            should_not_start_newline_forced: true,
            should_start_newline: true,
            should_end_newline: true,
            should_end_newline_forced: true,
        }
    }
}

impl Newline {
    pub fn can_insert_end(&self) -> bool {
        self.should_end_newline && self.should_end_newline_forced
    }

    pub fn can_insert_start(&self) -> bool {
        self.should_start_newline && !self.should_not_start_newline_forced
    }

    pub fn try_insert_start(&self, ui: &mut Ui) {
        if self.can_insert_start() {
            newline(ui);
        }
    }

    pub fn try_insert_end(&self, ui: &mut Ui) {
        if self.can_insert_end() {
            newline(ui);
        }
    }
}

#[derive(Default)]
struct DefinitionList {
    is_first_item: bool,
    is_def_list_def: bool,
}

#[derive(Debug, Default)]
pub struct CommonMarkViewer<'f> {
    options: CommonMarkOptions<'f>,
    zoom_factor: Option<f32>,
    search_query: Option<String>,
    active_match_index: Option<usize>,
    base_dir: Option<PathBuf>,
}

impl<'f> CommonMarkViewer<'f> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn zoom_factor(mut self, zoom_factor: f32) -> Self {
        self.zoom_factor = Some(zoom_factor.clamp(0.5, 3.0));
        self
    }

    pub fn base_dir<P: Into<PathBuf>>(mut self, dir: Option<P>) -> Self {
        self.base_dir = dir.map(Into::into);
        self
    }

    pub fn enable_scroll_to_heading(mut self, enable: bool) -> Self {
        self.options.enable_scroll_to_heading = enable;
        self
    }

    pub fn syntax_theme_dark<S: Into<String>>(mut self, theme: S) -> Self {
        self.options.theme_dark = theme.into();
        self
    }

    pub fn syntax_theme_light<S: Into<String>>(mut self, theme: S) -> Self {
        self.options.theme_light = theme.into();
        self
    }

    pub fn search(mut self, query: Option<String>, active_match_index: Option<usize>) -> Self {
        self.search_query = query;
        self.active_match_index = active_match_index;
        self
    }

    pub fn show(
        self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        text: &str,
    ) -> egui::InnerResponse<()> {
        egui_commonmark_backend::prepare_show(cache, ui.ctx());

        let mut internal = CommonMarkViewerInternal::new();
        internal.zoom_factor = self.zoom_factor.unwrap_or(1.0);
        internal.search_query = self.search_query.filter(|q| !q.trim().is_empty());
        internal.active_match_index = self.active_match_index;
        internal.base_dir = self.base_dir;

        let (response, _) = internal.show(ui, cache, &self.options, text, None);
        response
    }
}

pub struct CommonMarkViewerInternal {
    curr_table: usize,
    curr_code_block: usize,
    zoom_factor: f32,
    text_style: Style,
    list: List,
    link: Option<Link>,
    image: Option<Image>,
    line: Newline,
    code_block: Option<CodeBlock>,
    html_block: String,
    is_list_item: bool,
    def_list: DefinitionList,
    is_table: bool,
    is_blockquote: bool,
    checkbox_events: Vec<CheckboxClickEvent>,
    deferred_scroll_to_heading: Option<String>,
    search_query: Option<String>,
    active_match_index: Option<usize>,
    match_counter: usize,
    base_dir: Option<PathBuf>,
}

pub(crate) struct CheckboxClickEvent {
    #[allow(dead_code)]
    pub(crate) checked: bool,
    #[allow(dead_code)]
    pub(crate) span: Range<usize>,
}

impl CommonMarkViewerInternal {
    pub fn new() -> Self {
        Self {
            curr_table: 0,
            curr_code_block: 0,
            zoom_factor: 1.0,
            text_style: Style::default(),
            list: List::default(),
            link: None,
            image: None,
            line: Newline::default(),
            is_list_item: false,
            def_list: Default::default(),
            code_block: None,
            html_block: String::new(),
            is_table: false,
            is_blockquote: false,
            checkbox_events: Vec::new(),
            deferred_scroll_to_heading: None,
            search_query: None,
            active_match_index: None,
            match_counter: 0,
            base_dir: None,
        }
    }
}

fn parser_options_extras(
    is_math_enabled: bool,
    is_scroll_to_heading_enabled: bool,
) -> pulldown_cmark::Options {
    let mut result = parser_options();
    if is_math_enabled {
        result |= pulldown_cmark::Options::ENABLE_MATH;
    }
    if is_scroll_to_heading_enabled {
        result |= pulldown_cmark::Options::ENABLE_HEADING_ATTRIBUTES;
    }
    result
}

impl CommonMarkViewerInternal {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        text: &str,
        _split_points_id: Option<Id>,
    ) -> (egui::InnerResponse<()>, Vec<CheckboxClickEvent>) {
        let max_width = options.max_width(ui);
        let layout = egui::Layout::left_to_right(egui::Align::BOTTOM).with_main_wrap(true);

        let re = ui.allocate_ui_with_layout(egui::vec2(max_width, 0.0), layout, |ui| {
            // Scale the document style inside this child only. The app's title bar,
            // status bar, TOC and search controls retain their original style.
            let style = ui.style_mut();
            for font in style.text_styles.values_mut() {
                font.size *= self.zoom_factor;
            }
            if let Some(font) = &mut style.override_font_id {
                font.size *= self.zoom_factor;
            }
            style.spacing.item_spacing *= self.zoom_factor;
            style.spacing.extra_text_line_spacing *= self.zoom_factor;
            style.spacing.indent *= self.zoom_factor;
            style.spacing.icon_width *= self.zoom_factor;
            style.spacing.icon_width_inner *= self.zoom_factor;
            style.spacing.icon_spacing *= self.zoom_factor;
            ui.spacing_mut().item_spacing.x = 0.0;
            let height = ui.text_style_height(&TextStyle::Body);
            ui.set_row_height(height);

            let mut events = pulldown_cmark::Parser::new_ext(
                text,
                parser_options_extras(options.math_fn.is_some(), options.enable_scroll_to_heading),
            )
            .into_offset_iter()
            .enumerate()
            .peekable();

            while let Some((index, (e, src_span))) = events.next() {
                if events.peek().is_none() {
                    self.line.should_end_newline_forced = false;
                }

                self.process_event(ui, &mut events, e, src_span, cache, options, max_width);

                if index == 0 {
                    self.line.should_not_start_newline_forced = false;
                }
            }

            *cache.scroll_to_id_target_mut() = self.deferred_scroll_to_heading.take();
        });

        (re, std::mem::take(&mut self.checkbox_events))
    }

    #[allow(clippy::too_many_arguments)]
    fn process_event<'e>(
        &mut self,
        ui: &mut Ui,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        event: pulldown_cmark::Event,
        src_span: Range<usize>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        self.event(ui, event, src_span, cache, options, max_width);

        self.def_list_def_wrapping(events, max_width, cache, options, ui);
        self.item_list_wrapping(events, max_width, cache, options, ui);
        self.table(events, cache, options, ui, max_width);
        self.blockquote(events, max_width, cache, options, ui);
    }

    fn def_list_def_wrapping<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.def_list.is_def_list_def {
            self.def_list.is_def_list_def = false;

            let item_events = delayed_events(events, |tag| {
                matches!(tag, pulldown_cmark::TagEnd::DefinitionListDefinition)
            });

            let mut events_iter = item_events.into_iter().enumerate().peekable();

            self.line.try_insert_start(ui);
            self.line.should_start_newline = false;
            if let Some((_, (e, src_span))) = events_iter.next() {
                self.process_event(ui, &mut events_iter, e, src_span, cache, options, max_width);
            }

            ui.label(" ".repeat(options.indentation_spaces));
            self.line.should_start_newline = true;
            self.line.should_end_newline = false;

            ui.horizontal_wrapped(|ui| {
                while let Some((_, (e, src_span))) = events_iter.next() {
                    self.process_event(
                        ui,
                        &mut events_iter,
                        e,
                        src_span,
                        cache,
                        options,
                        max_width,
                    );
                }
            });
            self.line.should_end_newline = true;

            if !matches!(
                events.peek(),
                Some((
                    _,
                    (
                        pulldown_cmark::Event::End(pulldown_cmark::TagEnd::DefinitionList),
                        _
                    )
                ))
            ) {
                self.line.try_insert_end(ui);
            }
        }
    }

    fn item_list_wrapping<'e>(
        &mut self,
        events: &mut impl Iterator<Item = EventIteratorItem<'e>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.is_list_item {
            self.is_list_item = false;

            let item_events = delayed_events_list_item(events);
            let mut events_iter = item_events.into_iter().enumerate().peekable();

            ui.horizontal_wrapped(|ui| {
                while let Some((_, (e, src_span))) = events_iter.next() {
                    self.process_event(
                        ui,
                        &mut events_iter,
                        e,
                        src_span,
                        cache,
                        options,
                        max_width,
                    );
                }
            });
        }
    }

    fn blockquote<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.is_blockquote {
            let mut collected_events = delayed_events(events, |tag| {
                matches!(tag, pulldown_cmark::TagEnd::BlockQuote(_))
            });
            self.line.try_insert_start(ui);
            self.line.should_not_start_newline_forced = false;

            if let Some(alert) = parse_alerts(&options.alerts, &mut collected_events) {
                egui_commonmark_backend::alert_ui(alert, ui, |ui| {
                    for (event, src_span) in collected_events {
                        self.event(ui, event, src_span, cache, options, max_width);
                    }
                })
            } else {
                blockquote(ui, ui.visuals().weak_text_color(), |ui| {
                    self.text_style.quote = true;
                    for (event, src_span) in collected_events {
                        self.event(ui, event, src_span, cache, options, max_width);
                    }
                    self.text_style.quote = false;
                });
            }

            if events.peek().is_none() {
                self.line.should_end_newline_forced = false;
            }

            self.line.try_insert_end(ui);
            self.is_blockquote = false;
        }
    }

    fn table<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
        max_width: f32,
    ) {
        if self.is_table {
            self.line.try_insert_start(ui);

            let id = ui.id().with("_table").with(self.curr_table);
            self.curr_table += 1;

            egui::Frame::group(ui.style()).show(ui, |ui| {
                let Table { header, rows } = parse_table(events);
                // Tables keep their natural cell widths and get a local horizontal
                // scrollbar, matching Markdown readers that preserve table layout.
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, false])
                    .id_salt(id.with("_horizontal_scroll"))
                    .show(ui, |ui| {
                        egui::Grid::new(id).striped(true).show(ui, |ui| {
                            for col in header {
                                ui.horizontal(|ui| {
                                    for (e, src_span) in col {
                                        let tmp_start = std::mem::replace(
                                            &mut self.line.should_start_newline,
                                            false,
                                        );
                                        let tmp_end = std::mem::replace(
                                            &mut self.line.should_end_newline,
                                            false,
                                        );
                                        self.event(ui, e, src_span, cache, options, max_width);
                                        self.line.should_start_newline = tmp_start;
                                        self.line.should_end_newline = tmp_end;
                                    }
                                });
                            }

                            ui.end_row();

                            for row in rows {
                                for col in row {
                                    ui.horizontal(|ui| {
                                        for (e, src_span) in col {
                                            let tmp_start = std::mem::replace(
                                                &mut self.line.should_start_newline,
                                                false,
                                            );
                                            let tmp_end = std::mem::replace(
                                                &mut self.line.should_end_newline,
                                                false,
                                            );
                                            self.event(ui, e, src_span, cache, options, max_width);
                                            self.line.should_start_newline = tmp_start;
                                            self.line.should_end_newline = tmp_end;
                                        }
                                    });
                                }

                                ui.end_row();
                            }
                        });
                    });
            });

            self.is_table = false;
            if events.peek().is_none() {
                self.line.should_end_newline_forced = false;
            }

            self.line.try_insert_end(ui);
        }
    }

    fn event(
        &mut self,
        ui: &mut Ui,
        event: pulldown_cmark::Event,
        src_span: Range<usize>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        match event {
            pulldown_cmark::Event::Start(tag) => self.start_tag(ui, tag, cache, options),
            pulldown_cmark::Event::End(tag) => self.end_tag(ui, tag, cache, options, max_width),
            pulldown_cmark::Event::Text(text) => {
                self.event_text(text, ui);
            }
            pulldown_cmark::Event::Code(text) => {
                self.text_style.code = true;
                self.event_text(text, ui);
                self.text_style.code = false;
            }
            pulldown_cmark::Event::InlineHtml(text) => {
                self.event_text(text, ui);
            }
            pulldown_cmark::Event::Html(text) => {
                if options.html_fn.is_some() {
                    self.html_block.push_str(&text);
                } else {
                    self.event_text(text, ui);
                }
            }
            pulldown_cmark::Event::FootnoteReference(footnote) => {
                footnote_start(ui, &footnote);
            }
            pulldown_cmark::Event::SoftBreak => {
                soft_break(ui);
            }
            pulldown_cmark::Event::HardBreak => newline(ui),
            pulldown_cmark::Event::Rule => {
                self.line.try_insert_start(ui);
                rule(ui, self.line.can_insert_end());
            }
            pulldown_cmark::Event::TaskListMarker(mut checkbox) => {
                if options.mutable {
                    if ui
                        .add(egui::Checkbox::without_text(&mut checkbox))
                        .clicked()
                    {
                        self.checkbox_events.push(CheckboxClickEvent {
                            checked: checkbox,
                            span: src_span,
                        });
                    }
                } else {
                    ui.add(ImmutableCheckbox::without_text(&mut checkbox));
                }
            }
            pulldown_cmark::Event::InlineMath(tex) => {
                if let Some(math_fn) = options.math_fn {
                    math_fn(ui, &tex, true);
                }
            }
            pulldown_cmark::Event::DisplayMath(tex) => {
                if let Some(math_fn) = options.math_fn {
                    math_fn(ui, &tex, false);
                }
            }
        }
    }

    fn push_or_show_label(
        &mut self,
        rich_text: egui::RichText,
        ui: &mut Ui,
        is_active_search: bool,
    ) {
        if let Some(image) = &mut self.image {
            image.alt_text.push(rich_text);
        } else if let Some(block) = &mut self.code_block {
            block.content.push_str(rich_text.text());
        } else if let Some(link) = &mut self.link {
            link.text.push(rich_text);
        } else {
            // Inline code and highlighted fragments are emitted as separate labels.
            // Force normal prose to honor the paragraph width. Table cells keep
            // their natural width and are handled by the table's local scroller.
            let label = egui::Label::new(rich_text);
            let resp = if self.is_table {
                ui.add(label.extend())
            } else {
                ui.add(label.wrap())
            };
            if is_active_search {
                resp.scroll_to_me(Some(egui::Align::Center));
            }
        }
    }

    fn event_text(&mut self, text: CowStr, ui: &mut Ui) {
        if self.image.is_some() || self.code_block.is_some() {
            let r = self.text_style.to_richtext(ui, &text);
            self.push_or_show_label(r, ui, false);
            return;
        }

        if let Some(query) = &self.search_query {
            let matches = crate::search::find_matches_in_text(&text, query);
            if !matches.is_empty() {
                let mut last_end = 0;
                for (match_start, match_end) in matches {
                    // Preceding unhighlighted segment
                    if match_start > last_end {
                        let seg = &text[last_end..match_start];
                        let r = self.text_style.to_richtext(ui, seg);
                        self.push_or_show_label(r, ui, false);
                    }

                    // Matching highlighted segment!
                    let matched_segment = &text[match_start..match_end];
                    let is_active = self.active_match_index == Some(self.match_counter);
                    self.match_counter += 1;

                    // Active match: Vibrant Orange with black text
                    // Other matches: Bright Gold Yellow with black text
                    let bg_color = if is_active {
                        Color32::from_rgb(255, 130, 0)
                    } else {
                        Color32::from_rgb(255, 215, 0)
                    };

                    let r = self
                        .text_style
                        .to_richtext(ui, matched_segment)
                        .background_color(bg_color)
                        .color(Color32::BLACK)
                        .strong();

                    self.push_or_show_label(r, ui, is_active);
                    last_end = match_end;
                }

                // Trailing unhighlighted segment
                if last_end < text.len() {
                    let seg = &text[last_end..];
                    let r = self.text_style.to_richtext(ui, seg);
                    self.push_or_show_label(r, ui, false);
                }
                return;
            }
        }

        let rich_text = self.text_style.to_richtext(ui, &text);
        self.push_or_show_label(rich_text, ui, false);
    }

    fn start_tag(
        &mut self,
        ui: &mut Ui,
        tag: pulldown_cmark::Tag,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
    ) {
        match tag {
            pulldown_cmark::Tag::Paragraph => {
                self.line.try_insert_start(ui);
            }
            pulldown_cmark::Tag::Heading { level, id, .. } => {
                if let (Some(scroll_target), Some(id)) = (cache.scroll_to_id_target(), id) {
                    if id.into_string() == scroll_target {
                        ui.scroll_to_cursor(Some(egui::Align::TOP));
                        cache.scroll_to_id_target_mut().take();
                    }
                }

                newline(ui);
                self.text_style.heading = Some(match level {
                    HeadingLevel::H1 => 0,
                    HeadingLevel::H2 => 1,
                    HeadingLevel::H3 => 2,
                    HeadingLevel::H4 => 3,
                    HeadingLevel::H5 => 4,
                    HeadingLevel::H6 => 5,
                });
            }
            pulldown_cmark::Tag::BlockQuote(_) => {
                self.is_blockquote = true;
            }
            pulldown_cmark::Tag::CodeBlock(c) => {
                match c {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                        self.code_block = Some(CodeBlock {
                            lang: Some(lang.to_string()),
                            content: "".to_string(),
                        });
                    }
                    pulldown_cmark::CodeBlockKind::Indented => {
                        self.code_block = Some(CodeBlock {
                            lang: None,
                            content: "".to_string(),
                        });
                    }
                }
                self.line.try_insert_start(ui);
            }
            pulldown_cmark::Tag::List(point) => {
                if !self.list.is_inside_a_list() && self.line.can_insert_start() {
                    newline(ui);
                }

                if let Some(number) = point {
                    self.list.start_level_with_number(number);
                } else {
                    self.list.start_level_without_number();
                }
                self.line.should_start_newline = false;
                self.line.should_end_newline = false;
            }
            pulldown_cmark::Tag::Item => {
                self.is_list_item = true;
                self.list.start_item(ui, options);
            }
            pulldown_cmark::Tag::FootnoteDefinition(note) => {
                self.line.try_insert_start(ui);
                self.line.should_start_newline = false;
                self.line.should_end_newline = false;
                footnote(ui, &note);
            }
            pulldown_cmark::Tag::Table(_) => {
                self.is_table = true;
            }
            pulldown_cmark::Tag::TableHead => {}
            pulldown_cmark::Tag::TableRow => {}
            pulldown_cmark::Tag::TableCell => {}
            pulldown_cmark::Tag::Emphasis => {
                self.text_style.emphasis = true;
            }
            pulldown_cmark::Tag::Strong => {
                self.text_style.strong = true;
            }
            pulldown_cmark::Tag::Strikethrough => {
                self.text_style.strikethrough = true;
            }
            pulldown_cmark::Tag::Link { dest_url, .. } => {
                self.link = Some(Link {
                    destination: dest_url.to_string(),
                    text: Vec::new(),
                });
            }
            pulldown_cmark::Tag::Image { dest_url, .. } => {
                let resolved = resolve_image_url(&dest_url, self.base_dir.as_deref());
                self.image = Some(Image::new(&resolved, options));
            }
            pulldown_cmark::Tag::HtmlBlock => {
                self.line.try_insert_start(ui);
            }
            pulldown_cmark::Tag::MetadataBlock(_) => {}
            pulldown_cmark::Tag::DefinitionList => {
                self.line.try_insert_start(ui);
                self.def_list.is_first_item = true;
            }
            pulldown_cmark::Tag::DefinitionListTitle => {
                if !self.def_list.is_first_item {
                    self.line.try_insert_start(ui)
                } else {
                    self.def_list.is_first_item = false;
                }
            }
            pulldown_cmark::Tag::DefinitionListDefinition => {
                self.def_list.is_def_list_def = true;
            }
            pulldown_cmark::Tag::Superscript | pulldown_cmark::Tag::Subscript => {}
        }
    }

    fn end_tag(
        &mut self,
        ui: &mut Ui,
        tag: pulldown_cmark::TagEnd,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        match tag {
            pulldown_cmark::TagEnd::Paragraph => {
                self.line.try_insert_end(ui);
            }
            pulldown_cmark::TagEnd::Heading { .. } => {
                self.line.try_insert_end(ui);
                self.text_style.heading = None;
            }
            pulldown_cmark::TagEnd::BlockQuote(_) => {}
            pulldown_cmark::TagEnd::CodeBlock => {
                self.end_code_block(ui, cache, options, max_width);
            }
            pulldown_cmark::TagEnd::List(_) => {
                if self.list.is_last_level() {
                    self.line.should_start_newline = true;
                    self.line.should_end_newline = true;
                }

                self.list.end_level(ui, self.line.can_insert_end());

                if !self.list.is_inside_a_list() {
                    self.list = List::default();
                }
            }
            pulldown_cmark::TagEnd::Item => {}
            pulldown_cmark::TagEnd::FootnoteDefinition => {
                self.line.should_start_newline = true;
                self.line.should_end_newline = true;
                self.line.try_insert_end(ui);
            }
            pulldown_cmark::TagEnd::Table => {}
            pulldown_cmark::TagEnd::TableHead => {}
            pulldown_cmark::TagEnd::TableRow => {}
            pulldown_cmark::TagEnd::TableCell => {
                ui.label("  ");
            }
            pulldown_cmark::TagEnd::Emphasis => {
                self.text_style.emphasis = false;
            }
            pulldown_cmark::TagEnd::Strong => {
                self.text_style.strong = false;
            }
            pulldown_cmark::TagEnd::Strikethrough => {
                self.text_style.strikethrough = false;
            }
            pulldown_cmark::TagEnd::Link => {
                if let Some(link) = self.link.take() {
                    link.end(ui, cache, options, &mut self.deferred_scroll_to_heading);
                }
            }
            pulldown_cmark::TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    let response = ui.add(
                        egui::Image::from_uri(&image.uri)
                            .fit_to_original_size(self.zoom_factor)
                            .max_width(options.max_width(ui)),
                    );
                    if !image.alt_text.is_empty() && options.show_alt_text_on_hover {
                        response.on_hover_ui_at_pointer(|ui| {
                            for alt in image.alt_text {
                                ui.label(alt);
                            }
                        });
                    }
                }
            }
            pulldown_cmark::TagEnd::HtmlBlock => {
                if let Some(html_fn) = options.html_fn {
                    html_fn(ui, &self.html_block);
                    self.html_block.clear();
                }
            }
            pulldown_cmark::TagEnd::MetadataBlock(_) => {}
            pulldown_cmark::TagEnd::DefinitionList => self.line.try_insert_end(ui),
            pulldown_cmark::TagEnd::DefinitionListTitle
            | pulldown_cmark::TagEnd::DefinitionListDefinition => {}
            pulldown_cmark::TagEnd::Superscript | pulldown_cmark::TagEnd::Subscript => {}
        }
    }

    fn end_code_block(
        &mut self,
        ui: &mut Ui,
        _cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        if let Some(block) = self.code_block.take() {
            let id = ui.id().with("_code_block").with(self.curr_code_block);
            self.curr_code_block += 1;

            if is_mermaid_language(block.lang.as_deref()) {
                ui.vertical(|ui| {
                    ui.set_width(max_width);
                    render_mermaid_block(ui, max_width, &block.content, self.zoom_factor);
                });
                self.line.try_insert_end(ui);
                return;
            }

            ui.scope(|ui| {
                // Code lines keep their natural width. The local horizontal scroll area
                // prevents a long line from changing the document's wrapping width.
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, false])
                    .id_salt(id.with("_horizontal_scroll"))
                    .show(ui, |ui| {
                        render_custom_code_block(
                            ui,
                            max_width,
                            block.lang.as_deref(),
                            &block.content,
                            options,
                        );
                    });
            });
            self.line.try_insert_end(ui);
        }
    }
}

fn render_custom_code_block(
    ui: &mut Ui,
    max_width: f32,
    lang: Option<&str>,
    content: &str,
    options: &CommonMarkOptions,
) {
    use egui::TextBuffer as _;
    let mut text = content.strip_suffix('\n').unwrap_or(content);

    let is_dark = ui.style().visuals.dark_mode;
    let theme_name = if is_dark {
        if !options.theme_dark.is_empty() {
            &options.theme_dark
        } else {
            "base16-ocean.dark"
        }
    } else {
        if !options.theme_light.is_empty() {
            &options.theme_light
        } else {
            "base16-ocean.light"
        }
    };

    static SYNTAX_SET: std::sync::LazyLock<syntect::parsing::SyntaxSet> =
        std::sync::LazyLock::new(syntect::parsing::SyntaxSet::load_defaults_newlines);
    static THEME_SET: std::sync::LazyLock<syntect::highlighting::ThemeSet> =
        std::sync::LazyLock::new(syntect::highlighting::ThemeSet::load_defaults);

    let theme = THEME_SET.themes.get(theme_name).or_else(|| {
        THEME_SET.themes.get(if is_dark {
            "base16-ocean.dark"
        } else {
            "base16-ocean.light"
        })
    });

    let bg_color = theme
        .and_then(|t| t.settings.background)
        .map(|c| Color32::from_rgb(c.r, c.g, c.b))
        .unwrap_or_else(|| ui.visuals().extreme_bg_color);

    let syntax = lang.and_then(|l| {
        SYNTAX_SET
            .find_syntax_by_token(l)
            .or_else(|| SYNTAX_SET.find_syntax_by_extension(l))
    });

    let mut layouter = |ui: &Ui, string: &dyn egui::TextBuffer, _wrap_width: f32| {
        let mut job = egui::text::LayoutJob::default();
        if let (Some(syntax), Some(theme)) = (syntax, theme) {
            let mut h = syntect::easy::HighlightLines::new(syntax, theme);
            for line in syntect::util::LinesWithEndings::from(string.as_str()) {
                if let Ok(ranges) = h.highlight_line(line, &SYNTAX_SET) {
                    for (style, text_segment) in ranges {
                        job.append(
                            text_segment,
                            0.0,
                            egui::TextFormat::simple(
                                TextStyle::Monospace.resolve(ui.style()),
                                Color32::from_rgb(
                                    style.foreground.r,
                                    style.foreground.g,
                                    style.foreground.b,
                                ),
                            ),
                        );
                    }
                } else {
                    job.append(
                        line,
                        0.0,
                        egui::TextFormat::simple(
                            TextStyle::Monospace.resolve(ui.style()),
                            ui.style().visuals.text_color(),
                        ),
                    );
                }
            }
        } else {
            job.append(
                string.as_str(),
                0.0,
                egui::TextFormat::simple(
                    TextStyle::Monospace.resolve(ui.style()),
                    ui.style().visuals.text_color(),
                ),
            );
        }
        // Never wrap code lines. The enclosing horizontal ScrollArea handles overflow.
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };

    // Pre-allocate background placeholder
    let where_to_put_background = ui.painter().add(egui::Shape::Noop);

    // Comfortable margin/padding: 14px left, 38px right, 10px top, 10px bottom
    let inner_margin = egui::Margin {
        left: 14_i8,
        right: 38_i8,
        top: 10_i8,
        bottom: 10_i8,
    };

    // Expand the code block to the longest source line so the enclosing
    // horizontal ScrollArea can expose the full line instead of clipping it.
    let code_font = TextStyle::Monospace.resolve(ui.style());
    let text_color = ui.visuals().text_color();
    let longest_line_width = ui.fonts_mut(|fonts| {
        text.split('\n')
            .map(|line| {
                fonts
                    .layout_no_wrap(line.to_owned(), code_font.clone(), text_color)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max)
    });
    let desired_width = max_width.max(longest_line_width + inner_margin.sum().x);
    // TextEdit otherwise clamps its allocation to the viewport width. Expand
    // the scroll area's child first so the natural code width is preserved.
    ui.set_min_width(desired_width);

    let output = egui::TextEdit::multiline(&mut text)
        .layouter(&mut layouter)
        .desired_width(desired_width)
        .desired_rows(1)
        .margin(inner_margin)
        .show(ui);

    let frame_rect = output.response.rect;

    // Background color + rounded border (output.response.rect already includes inner_margin)
    let corner_radius = ui.style().noninteractive().corner_radius;
    let border_stroke = ui.visuals().widgets.noninteractive.bg_stroke;

    ui.painter().set(
        where_to_put_background,
        egui::epaint::RectShape::new(
            frame_rect,
            corner_radius,
            bg_color,
            border_stroke,
            egui::StrokeKind::Outside,
        ),
    );

    // Keep the copy icon pinned to the visible right edge. For a long code line,
    // frame_rect.right() is outside the viewport until the user scrolls all the
    // way to the end, which would make the button appear to be missing.
    let button_size = egui::vec2(22.0, 22.0);
    let button_right = frame_rect.right().min(ui.clip_rect().right() - 6.0);
    let button_rect = egui::Rect::from_min_size(
        egui::pos2(button_right - button_size.x, frame_rect.top() + 6.0),
        button_size,
    );

    let persistent_id = ui.make_persistent_id(output.response.id);
    let copied_icon = ui.memory_mut(|m| *m.data.get_temp_mut_or_default::<bool>(persistent_id));

    let mut button_ui = ui.new_child(egui::UiBuilder::new().max_rect(button_rect).layout(
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
    ));
    let copy_button = button_ui
        .add(
            egui::Button::new(egui::RichText::new(if copied_icon { "✔" } else { "🗐" }).size(14.0))
                .small()
                .frame(false)
                .fill(Color32::TRANSPARENT),
        )
        .on_hover_cursor(
            ui.visuals()
                .interact_cursor
                .unwrap_or(egui::CursorIcon::Default),
        );

    if copied_icon && !copy_button.hovered() {
        ui.memory_mut(|m| *m.data.get_temp_mut_or_default(persistent_id) = false);
    }
    if !copied_icon && copy_button.clicked() {
        ui.memory_mut(|m| *m.data.get_temp_mut_or_default(persistent_id) = true);
    }

    if copy_button.clicked() {
        let copy_text = if let Some(cursor) = output.cursor_range {
            let selected_chars = cursor.as_sorted_char_range();
            let selected_text = text.char_range(selected_chars);
            if selected_text.is_empty() {
                text.to_owned()
            } else {
                selected_text.to_owned()
            }
        } else {
            text.to_owned()
        };
        ui.copy_text(copy_text);
    }
}

fn is_mermaid_language(lang: Option<&str>) -> bool {
    lang.is_some_and(|value| {
        value.split_ascii_whitespace().next().is_some_and(|name| {
            name.eq_ignore_ascii_case("mermaid") || name.eq_ignore_ascii_case("mmd")
        })
    })
}

#[derive(Clone)]
struct MermaidEntry {
    source: String,
    dark: bool,
    uri: String,
    svg: Result<std::sync::Arc<[u8]>, String>,
}

fn mermaid_svg(ctx: &egui::Context, content: &str, dark: bool) -> MermaidEntry {
    use std::hash::{Hash, Hasher};
    type Cache = std::collections::VecDeque<MermaidEntry>;
    let id = egui::Id::new("mermaid_svg_cache");
    let mut cache = ctx
        .data_mut(|data| data.get_temp::<Cache>(id))
        .unwrap_or_default();
    if let Some(index) = cache
        .iter()
        .position(|entry| entry.source == content && entry.dark == dark)
    {
        let entry = cache.remove(index).unwrap();
        cache.push_back(entry.clone());
        ctx.data_mut(|data| data.insert_temp(id, cache));
        return entry;
    }
    let mut theme = if dark {
        mermaid_rs_renderer::Theme::dark()
    } else {
        mermaid_rs_renderer::Theme::modern()
    };
    theme.font_family = "Microsoft YaHei, Segoe UI, sans-serif".to_owned();
    let options = mermaid_rs_renderer::RenderOptions {
        theme,
        layout: mermaid_rs_renderer::LayoutConfig::default(),
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    dark.hash(&mut hasher);
    let entry = MermaidEntry {
        source: content.to_owned(),
        dark,
        uri: format!("bytes://mdreader-mermaid-{:x}.svg", hasher.finish()),
        svg: mermaid_rs_renderer::render_with_options(content, options)
            .map(|svg| std::sync::Arc::from(svg.into_bytes()))
            .map_err(|error| error.to_string()),
    };
    if cache.len() >= 32 {
        if let Some(old) = cache.pop_front() {
            ctx.forget_image(&old.uri);
        }
    }
    cache.push_back(entry.clone());
    ctx.data_mut(|data| data.insert_temp(id, cache));
    entry
}

fn render_mermaid_block(ui: &mut Ui, max_width: f32, content: &str, zoom: f32) {
    let entry = mermaid_svg(ui.ctx(), content, ui.visuals().dark_mode);
    match entry.svg {
        Ok(svg) => {
            ui.add(
                egui::Image::from_bytes(entry.uri, svg)
                    .fit_to_original_size(zoom)
                    .max_width(max_width),
            );
            ui.collapsing("Mermaid 源码", |ui| {
                if ui.button("复制源码").clicked() {
                    ui.copy_text(content.to_owned());
                }
                ui.add(egui::Label::new(egui::RichText::new(content).monospace()).wrap());
            });
        }
        Err(error) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Mermaid 渲染失败：{error}"),
            );
            ui.add(egui::Label::new(egui::RichText::new(content).monospace()).wrap());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mermaid_cache_theme_source_and_errors() {
        let ctx = egui::Context::default();
        let source = "flowchart LR\n A[开始] --> B{判断}\n B -->|是| C[完成]";
        let first = mermaid_svg(&ctx, source, false);
        let second = mermaid_svg(&ctx, source, false);
        assert!(std::sync::Arc::ptr_eq(
            first.svg.as_ref().unwrap(),
            second.svg.as_ref().unwrap()
        ));
        let dark = mermaid_svg(&ctx, source, true);
        assert_ne!(first.uri, dark.uri);
        assert_ne!(first.svg, dark.svg);
        assert_ne!(first.uri, mermaid_svg(&ctx, "graph TD; A-->C", false).uri);
        assert!(mermaid_svg(&ctx, "not_a_diagram", false).svg.is_err());
        assert!(is_mermaid_language(Some("Mermaid")));
        assert!(is_mermaid_language(Some("mmd")));
        assert!(!is_mermaid_language(Some("rust")));
    }

    #[test]
    fn test_mermaid_fence_renders_image_at_zoom_and_narrow_width() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut cache = CommonMarkCache::default();
        let mut widths = Vec::new();
        for (zoom, width) in [(1.0, 800.0), (2.0, 800.0), (2.0, 180.0)] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(width);
                CommonMarkViewer::new().zoom_factor(zoom).show(
                    ui,
                    &mut cache,
                    "```mermaid\nflowchart LR\n A[开始] --> B[完成]\n```",
                );
            });
            let rect = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.brush.is_some() => Some(rect.rect),
                    _ => None,
                })
                .expect("Mermaid fence should paint an SVG image");
            assert!(rect.width() <= width + 0.01);
            widths.push(rect.width());
            output.textures_delta.clear();
        }
        assert!(widths[1] > widths[0] * 1.5);
        assert!(widths[2] < widths[1]);
    }

    #[test]
    fn test_document_zoom_scales_text_without_changing_chrome() {
        let ctx = egui::Context::default();
        crate::setup_custom_fonts(&ctx);
        for zoom in [0.5, 1.0, 2.1, 3.0] {
            for _ in 0..2 {
                let mut expected_fonts = Vec::new();
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.set_width(600.0);
                    let body = TextStyle::Body.resolve(ui.style()).size;
                    let heading = TextStyle::Heading.resolve(ui.style()).size;
                    let code = TextStyle::Monospace.resolve(ui.style()).size;
                    ui.label("chrome before");
                    let mut cache = CommonMarkCache::default();
                    let response = CommonMarkViewer::new().zoom_factor(zoom).show(
                        ui,
                        &mut cache,
                        "# Document heading\n\nDocument body\n\n```rust\nlet value = 42;\n```",
                    );
                    assert!(response.response.rect.width() <= 600.0 + 0.01);
                    assert_eq!(TextStyle::Body.resolve(ui.style()).size, body);
                    ui.label("chrome after");
                    expected_fonts = vec![
                        ("chrome before", body),
                        ("chrome after", body),
                        ("Document heading", heading * zoom),
                        ("Document body", body * zoom),
                        ("let value = 42;", code * zoom),
                    ];
                });
                for (text, expected_size) in expected_fonts {
                    let galley = output
                        .shapes
                        .iter()
                        .find_map(|shape| {
                            if let egui::Shape::Text(text_shape) = &shape.shape {
                                if text_shape.galley.job.text == text {
                                    return Some(&text_shape.galley);
                                }
                            }
                            None
                        })
                        .unwrap_or_else(|| panic!("Missing rendered text: {text}"));
                    for section in &galley.job.sections {
                        assert!(
                            (section.format.font_id.size - expected_size).abs() < 0.01,
                            "{text}: zoom={zoom}, expected={expected_size}, actual={}",
                            section.format.font_id.size
                        );
                    }
                }
                assert_eq!(ctx.zoom_factor(), 1.0);
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn test_resolve_image_url() {
        // Absolute http/https URLs should be kept as-is
        let http_url = "https://example.com/image.png";
        assert_eq!(
            resolve_image_url(http_url, Some(Path::new("C:/docs"))),
            http_url
        );

        // Relative path with base_dir
        let base = Path::new(r"C:\docs\sub");
        let rel_url = "images/pic.png";
        let resolved = resolve_image_url(rel_url, Some(base));
        if cfg!(windows) {
            assert!(
                resolved.starts_with("file:///C:/docs/sub/images/pic.png")
                    || resolved.starts_with("file:///")
            );
            assert!(resolved.ends_with("images/pic.png"));
        } else {
            assert!(resolved.starts_with("file://"));
            assert!(resolved.ends_with("images/pic.png"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn test_resolve_image_url_handles_windows_extended_and_unc_paths() {
        for (base, expected) in [
            (r"C:\docs", "file:///C:/docs/images/pic.png"),
            (r"\\?\C:\docs", "file:///C:/docs/images/pic.png"),
            (
                r"\\server\share\docs",
                "file://server/share/docs/images/pic.png",
            ),
            (
                r"\\?\UNC\server\share\docs",
                "file://server/share/docs/images/pic.png",
            ),
        ] {
            assert_eq!(
                resolve_image_url("images/pic.png", Some(Path::new(base))),
                expected
            );
        }
    }

    #[test]
    fn test_local_image_loader_can_read_project_asset() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let base_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let uri = resolve_image_url("assets/preview-light.png", Some(&base_dir));

        let mut loaded = false;
        for _ in 0..200 {
            match ctx.try_load_image(&uri, egui::load::SizeHint::default()) {
                Ok(egui::load::ImagePoll::Ready { image }) => {
                    assert!(image.size[0] > 0 && image.size[1] > 0);
                    loaded = true;
                    break;
                }
                Ok(egui::load::ImagePoll::Pending { .. }) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("failed to load {uri}: {error:?}"),
            }
        }
        assert!(loaded, "image loader did not finish loading {uri}");
    }

    #[test]
    fn test_http_image_loader_fetches_svg() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        // Serve a badge-like SVG locally so this test needs no external service.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let uri = format!("http://{}/badge.svg", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "HTTP image request was not sent");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("HTTP test server failed: {error}"),
                }
            };
            // Accepted sockets can inherit nonblocking mode on Windows.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><rect width="16" height="8" fill="blue"/></svg>"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{svg}", svg.len()).unwrap();
        });

        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut loaded = false;
        for _ in 0..200 {
            match ctx.try_load_image(&uri, egui::load::SizeHint::default()) {
                Ok(egui::load::ImagePoll::Ready { image }) => {
                    assert_eq!(image.size, [16, 8]);
                    loaded = true;
                    break;
                }
                Ok(egui::load::ImagePoll::Pending { .. }) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("failed to load HTTP image: {error:?}"),
            }
        }
        server.join().unwrap();
        assert!(loaded, "HTTP image did not finish loading");
    }

    #[test]
    fn test_local_markdown_image_renders_in_viewer() {
        let ctx = egui::Context::default();
        crate::setup_custom_fonts(&ctx);
        let mut cache = CommonMarkCache::default();
        let base_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let mut rendered_image = false;
        for _ in 0..200 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(600.0);
                CommonMarkViewer::new().base_dir(Some(&base_dir)).show(
                    ui,
                    &mut cache,
                    "![preview](assets/preview-light.png)",
                );
            });
            rendered_image |= output.shapes.iter().any(
                |shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.brush.is_some()),
            );
            output.textures_delta.clear();
            if rendered_image {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(rendered_image, "local Markdown image was not rendered");
    }

    #[test]
    fn test_code_blocks_layout() {
        let md = r#"
# 3. 代码块语法高亮

```rust
// Rust 示例代码
use std::time::Instant;

fn measure_startup_speed() {
    let start = Instant::now();
    println!("MdReader 极速加载完成！");
    let elapsed = start.elapsed();
    println!("耗时: {:?}", elapsed);
}
```

```python
# Python 示例代码
def calculate_memory_footprint():
    """验证原生 Rust 与 Chromium 内存占用对比"""
    native_rust_ram_mb = 18.5
    webview2_ram_mb = 120.0
    saved_ratio = (webview2_ram_mb - native_rust_ram_mb) / webview2_ram_mb
    print(f"原生模式节省了 {saved_ratio * 100:.1f}% 的内存！")
```

```json
{
  "application": "MdReader",
  "version": "0.1.0",
  "platform": "Windows x64",
  "features": [
    "instant_startup",
    "low_memory",
    "hot_reload",
    "toc_outline"
  ]
}
```

## 4. 表格排版 (GFM Tables)
"#;
        egui::__run_test_ctx(|ctx| {
            crate::setup_custom_fonts(ctx);
            // Frame 1 applies font definitions
            let mut out1 = ctx.run_ui(egui::RawInput::default(), |_| {});
            out1.textures_delta.clear();
            // Frame 2 has fonts loaded
            let mut out2 = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut cache = CommonMarkCache::default();
                let viewer = CommonMarkViewer::new();
                let resp = viewer.show(ui, &mut cache, md);
                assert!(
                    resp.response.rect.height() > 300.0,
                    "Response height should expand for all code blocks, got {}",
                    resp.response.rect.height()
                );
            });
            out2.textures_delta.clear();
        });
    }
}
