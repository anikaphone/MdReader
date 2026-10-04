use eframe::egui;
use crate::viewer::{CommonMarkCache, CommonMarkViewer};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{AppConfig, ThemeMode};
use crate::search::SearchState;
use crate::toc::{process_markdown_with_toc, TocItem};
use crate::watcher::FileWatcher;

pub struct MdReaderApp {
    pub config: AppConfig,
    pub file_path: Option<PathBuf>,
    pub file_content: String,
    pub rendered_markdown: String,
    pub toc: Vec<TocItem>,
    pub watcher: FileWatcher,
    pub search: SearchState,
    pub cache: CommonMarkCache,
    pub word_count: usize,
    pub line_count: usize,
    pub file_size_bytes: u64,
    pub status_toast: Option<(String, Instant)>,
    pub about_open: bool,
    pub scroll_to_offset: Option<f32>,
    pub last_content_height: f32,
    pub last_viewport_height: f32,
    pub startup_time: Instant,
    pub trimmed_working_set: bool,
    pub last_zoom_change: Option<Instant>,
}

#[cfg(windows)]
pub fn trim_working_set() {
    unsafe {
        use std::ffi::c_void;
        #[link(name = "psapi")]
        extern "system" {
            fn GetCurrentProcess() -> *mut c_void;
            fn EmptyWorkingSet(h_process: *mut c_void) -> i32;
        }
        let proc = GetCurrentProcess();
        EmptyWorkingSet(proc);
    }
}

#[cfg(windows)]
pub fn open_browser(url: &str) {
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;
    let wide_url: Vec<u16> = OsStr::new(url).encode_wide().chain(std::iter::once(0)).collect();
    let wide_op: Vec<u16> = OsStr::new("open").encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                lp_operation: *const u16,
                lp_file: *const u16,
                lp_parameters: *const u16,
                lp_directory: *const u16,
                n_show_cmd: i32,
            ) -> isize;
        }
        ShellExecuteW(
            std::ptr::null_mut(),
            wide_op.as_ptr(),
            wide_url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        );
    }
}

impl MdReaderApp {
    pub fn new(initial_file: Option<PathBuf>, ctx: &egui::Context) -> Self {
        let config = AppConfig::load();
        let (watcher, _) = FileWatcher::new();
        let mut app = Self {
            config,
            file_path: None,
            file_content: String::new(),
            rendered_markdown: String::new(),
            toc: Vec::new(),
            watcher,
            search: SearchState::default(),
            cache: CommonMarkCache::default(),
            word_count: 0,
            line_count: 0,
            file_size_bytes: 0,
            status_toast: None,
            about_open: false,
            scroll_to_offset: None,
            last_content_height: 1000.0,
            last_viewport_height: 600.0,
            startup_time: Instant::now(),
            trimmed_working_set: false,
            last_zoom_change: None,
        };

        // Apply saved zoom factor immediately on launch
        if (app.config.zoom_factor - 1.0).abs() > 0.001 {
            ctx.set_zoom_factor(app.config.zoom_factor);
        }

        // Apply saved theme immediately on launch
        app.apply_theme(ctx);

        if let Some(path) = initial_file {
            app.open_file(&path);
        } else if let Some(recent) = app.config.recent_files.first().cloned() {
            if recent.exists() {
                app.open_file(&recent);
            }
        }

        app
    }

    pub fn set_zoom(&mut self, ctx: &egui::Context, new_zoom: f32) {
        let clean_zoom = ((new_zoom * 10.0).round() / 10.0).clamp(0.5, 3.0);
        if (clean_zoom - self.config.zoom_factor).abs() > 0.001 {
            self.config.zoom_factor = clean_zoom;
            ctx.set_zoom_factor(clean_zoom);
            // Drop dead font atlas from previous zoom level so it does not accumulate in RAM
            ctx.set_fonts(ctx.fonts(|f| f.definitions().clone()));
            self.config.save();
            self.last_zoom_change = Some(Instant::now());
        }
    }

    pub fn open_file(&mut self, path: &Path) {
        if let Ok(content) = std::fs::read_to_string(path) {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            self.file_size_bytes = std::fs::metadata(&canonical).map(|m| m.len()).unwrap_or(0);
            self.file_path = Some(canonical.clone());
            self.scroll_to_offset = Some(0.0);
            self.set_content(content);

            if self.config.auto_reload {
                self.watcher.watch(&canonical);
            }

            self.config.add_recent(canonical);
            self.show_toast(format!("已打开: {}", path.file_name().unwrap_or_default().to_string_lossy()));
        } else {
            self.show_toast("打开失败: 文件不存在或非 UTF-8 编码".to_string());
        }
    }

    pub fn reload_current_file(&mut self) {
        if let Some(path) = self.file_path.clone() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                self.file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.set_content(content);
                self.show_toast("文档已自动刷新".to_string());
            }
        }
    }

    fn set_content(&mut self, content: String) {
        self.file_content = content;
        self.line_count = self.file_content.lines().count();
        self.word_count = self
            .file_content
            .split_whitespace()
            .map(|s| s.chars().count())
            .sum();

        let processed = process_markdown_with_toc(&self.file_content);
        self.rendered_markdown = processed.rendered_markdown;
        self.toc = processed.toc;

        if self.search.is_open {
            self.search.update(&self.rendered_markdown);
        }
    }

    pub fn jump_to_current_search_match(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        let ratio = self.search.current_match_ratio(&self.rendered_markdown);
        let content_height = if self.last_content_height > 100.0 {
            self.last_content_height
        } else {
            1000.0
        };
        let viewport_height = if self.last_viewport_height > 100.0 {
            self.last_viewport_height
        } else {
            600.0
        };

        // Center the match around 35% from the top of the viewport
        let target_y = (ratio * content_height - viewport_height * 0.35).max(0.0);
        let max_scroll = (content_height - viewport_height).max(0.0);
        let final_y = target_y.min(max_scroll);

        self.scroll_to_offset = Some(final_y);
        self.cache.scroll_to_id_target_mut().take();

        if let Some(m) = self.search.current_match_info() {
            let preview = if m.line_preview.chars().count() > 32 {
                format!("{}...", m.line_preview.chars().take(30).collect::<String>())
            } else {
                m.line_preview.clone()
            };
            self.show_toast(format!("已定位至第 {} 行: {}", m.line_number, preview));
        }
    }

    pub fn search_next(&mut self) {
        if self.search.next() {
            self.jump_to_current_search_match();
        }
    }

    pub fn search_prev(&mut self) {
        if self.search.prev() {
            self.jump_to_current_search_match();
        }
    }

    pub fn scroll_to_heading_or_slug(&mut self, frag: &str, ctx: &egui::Context) {
        if frag.is_empty() {
            return;
        }

        let target_anchor = self
            .toc
            .iter()
            .find_map(|item| {
                if item.anchor_id == frag {
                    Some(item.anchor_id.clone())
                } else if item.title.eq_ignore_ascii_case(frag) {
                    Some(item.anchor_id.clone())
                } else {
                    let slug = item
                        .title
                        .to_lowercase()
                        .replace([' ', '\t', '.', '、', '，', ':', '：'], "-");
                    let slug = slug.trim_matches('-');
                    if slug == frag.to_lowercase() {
                        Some(item.anchor_id.clone())
                    } else {
                        None
                    }
                }
            })
            .unwrap_or_else(|| frag.to_string());

        *self.cache.scroll_to_id_target_mut() = Some(target_anchor);
        self.scroll_to_offset = None;
        ctx.request_repaint();
    }

    pub fn show_toast(&mut self, msg: String) {
        self.status_toast = Some((msg, Instant::now()));
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        match self.config.theme_mode {
            ThemeMode::Dark => {
                ctx.set_visuals(egui::Visuals::dark());
            }
            ThemeMode::Light => {
                ctx.set_visuals(egui::Visuals::light());
            }
            ThemeMode::System => {
                let is_light = ctx.system_theme() == Some(egui::Theme::Light);
                if is_light {
                    ctx.set_visuals(egui::Visuals::light());
                } else {
                    ctx.set_visuals(egui::Visuals::dark());
                }
            }
        }
    }

    fn handle_shortcuts(&mut self, ui: &mut egui::Ui) {
        let input = ui.input(|i| i.clone());

        // Ctrl + O: Open file
        if input.modifiers.command && input.key_pressed(egui::Key::O) {
            self.trigger_open_dialog();
        }

        // F5 or Ctrl + R: Reload
        if input.key_pressed(egui::Key::F5) || (input.modifiers.command && input.key_pressed(egui::Key::R)) {
            self.reload_current_file();
        }

        // Ctrl + F: Toggle search
        if input.modifiers.command && input.key_pressed(egui::Key::F) {
            if self.search.is_open {
                self.search.close();
            } else {
                self.search.open();
                self.search.update(&self.rendered_markdown);
                self.jump_to_current_search_match();
            }
        }

        // Enter / Shift + Enter: Next / Prev search match when search bar is open
        if self.search.is_open && input.key_pressed(egui::Key::Enter) {
            if input.modifiers.shift {
                self.search_prev();
            } else {
                self.search_next();
            }
            ui.ctx().request_repaint();
        }

        // Ctrl + T: Toggle TOC
        if input.modifiers.command && input.key_pressed(egui::Key::T) {
            self.config.show_toc = !self.config.show_toc;
            self.config.save();
        }

        // Escape: Close search or modals
        if input.key_pressed(egui::Key::Escape) {
            if self.search.is_open {
                self.search.close();
            }
            if self.about_open {
                self.about_open = false;
            }
        }

        // Zoom shortcuts: Ctrl + '=', Ctrl + '+', Ctrl + '-'
        if input.modifiers.command {
            if input.key_pressed(egui::Key::Equals) || input.key_pressed(egui::Key::Plus) {
                self.set_zoom(ui.ctx(), self.config.zoom_factor + 0.1);
            } else if input.key_pressed(egui::Key::Minus) {
                self.set_zoom(ui.ctx(), self.config.zoom_factor - 0.1);
            } else if input.key_pressed(egui::Key::Num0) {
                self.set_zoom(ui.ctx(), 1.0);
            }
        }

        // Stepped zoom delta (handles Ctrl + wheel and pinch gestures cleanly without flooding font atlas)
        let zoom_delta = input.zoom_delta();
        if (zoom_delta - 1.0).abs() > 0.015 {
            let step = if zoom_delta > 1.0 { 0.1 } else { -0.1 };
            self.set_zoom(ui.ctx(), self.config.zoom_factor + step);
        }

        // Handle dropped files
        for dropped in &input.raw.dropped_files {
            let path = dropped.path();
            if !path.as_os_str().is_empty() && path.exists() {
                self.open_file(path);
                break;
            }
        }
    }

    fn trigger_open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Markdown 文档 (*.md, *.markdown)", &["md", "markdown", "mdown", "mkd", "txt"])
            .add_filter("所有文件 (*.*)", &["*"])
            .pick_file()
        {
            self.open_file(&path);
        }
    }
}

impl eframe::App for MdReaderApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // One-shot safe working set trimming after startup idle (700ms)
        #[cfg(windows)]
        if !self.trimmed_working_set {
            if self.startup_time.elapsed() >= std::time::Duration::from_millis(700) {
                self.trimmed_working_set = true;
                trim_working_set();
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(700));
            }
        }

        // Post-zoom safe memory trimming after user finishes zooming (350ms idle)
        #[cfg(windows)]
        if let Some(t) = self.last_zoom_change {
            if t.elapsed() >= std::time::Duration::from_millis(350) {
                self.last_zoom_change = None;
                trim_working_set();
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(350));
            }
        }

        self.apply_theme(ui.ctx());
        self.handle_shortcuts(ui);

        // Check file watcher for external changes
        if self.config.auto_reload && self.watcher.check_reload() {
            self.reload_current_file();
        }

        // Top Menu Bar
        egui::Panel::top("top_panel").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("文件 (F)", |ui| {
                    if ui.button("打开... (Ctrl+O)").clicked() {
                        self.trigger_open_dialog();
                        ui.close();
                    }
                    if ui.button("重新载入 (F5)").clicked() {
                        self.reload_current_file();
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("最近打开", |ui| {
                        if self.config.recent_files.is_empty() {
                            ui.label("暂无记录");
                        } else {
                            let recent = self.config.recent_files.clone();
                            for path in recent {
                                let name = path.file_name().unwrap_or_default().to_string_lossy();
                                if ui.button(format!("{}", name)).clicked() {
                                    self.open_file(&path);
                                    ui.close();
                                }
                            }
                        }
                    });
                    ui.separator();
                    let auto_reload_label = if self.config.auto_reload {
                        "自动热重载: [开]"
                    } else {
                        "自动热重载: [关]"
                    };
                    if ui.button(auto_reload_label).clicked() {
                        self.config.auto_reload = !self.config.auto_reload;
                        if self.config.auto_reload {
                            if let Some(path) = &self.file_path {
                                self.watcher.watch(path);
                            }
                        } else {
                            self.watcher.unwatch();
                        }
                        self.config.save();
                        ui.close();
                    }
                    ui.separator();
                    let default_btn_label = if crate::default_app::is_default_md_reader() {
                        "设为默认md阅读器 (已是默认)"
                    } else {
                        "设为默认md阅读器"
                    };
                    if ui.button(default_btn_label).clicked() {
                        match crate::default_app::set_as_default_md_reader() {
                            Ok(()) => {
                                self.show_toast("已成功将 MdReader 设为系统默认 Markdown 阅读器！".to_string());
                            }
                            Err(e) => {
                                self.show_toast(format!("设置失败: {}", e));
                            }
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("退出").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("视图 (V)", |ui| {
                    let toc_label = if self.config.show_toc {
                        "隐藏大纲侧边栏 (Ctrl+T)"
                    } else {
                        "显示大纲侧边栏 (Ctrl+T)"
                    };
                    if ui.button(toc_label).clicked() {
                        self.config.show_toc = !self.config.show_toc;
                        self.config.save();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("放大 (Ctrl++)").clicked() {
                        self.set_zoom(ui.ctx(), self.config.zoom_factor + 0.1);
                    }
                    if ui.button("缩小 (Ctrl+-)").clicked() {
                        self.set_zoom(ui.ctx(), self.config.zoom_factor - 0.1);
                    }
                    if ui.button("重置缩放 100% (Ctrl+0)").clicked() {
                        self.set_zoom(ui.ctx(), 1.0);
                    }
                });

                ui.menu_button("主题 (T)", |ui| {
                    if ui.radio_value(&mut self.config.theme_mode, ThemeMode::Dark, "深色模式 (Dark)").clicked() {
                        self.config.save();
                    }
                    if ui.radio_value(&mut self.config.theme_mode, ThemeMode::Light, "浅色模式 (Light)").clicked() {
                        self.config.save();
                    }
                    if ui.radio_value(&mut self.config.theme_mode, ThemeMode::System, "跟随系统 (System)").clicked() {
                        self.config.save();
                    }
                });

                ui.menu_button("查找 (S)", |ui| {
                    if ui.button("全文搜索 (Ctrl+F)").clicked() {
                        self.search.open();
                        self.search.update(&self.rendered_markdown);
                        ui.close();
                    }
                });

                ui.menu_button("帮助 (H)", |ui| {
                    if ui.button("关于 MdReader").clicked() {
                        self.about_open = true;
                        ui.close();
                    }
                });

                // Display file name on header right side
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(path) = &self.file_path {
                        let filename = path.file_name().unwrap_or_default().to_string_lossy();
                        ui.label(egui::RichText::new(filename).strong());
                    } else {
                        ui.label(egui::RichText::new("未打开文件").weak());
                    }
                });
            });

            // Inline Search Bar (Ctrl+F)
            if self.search.is_open {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("🔍 查找:");
                    let text_edit = egui::TextEdit::singleline(&mut self.search.query)
                        .hint_text("输入关键字搜索...")
                        .desired_width(220.0);
                    let resp = ui.add(text_edit);
                    if self.search.request_focus {
                        resp.request_focus();
                        self.search.request_focus = false;
                    }
                    if resp.changed() {
                        self.search.update(&self.rendered_markdown);
                        self.jump_to_current_search_match();
                    }

                    // Match status
                    ui.label(egui::RichText::new(self.search.match_status()).strong());

                    if ui.button(" 上一处 (↑) ").clicked() {
                        self.search_prev();
                    }
                    if ui.button(" 下一处 (↓) ").clicked() {
                        self.search_next();
                    }
                    if ui.button(" ✖ 关闭 ").clicked() {
                        self.search.close();
                    }
                });
            }
        });

        // Bottom Status Bar
        egui::Panel::bottom("bottom_panel").show(ui, |ui| {
            ui.horizontal(|ui| {
                // Toast notification or file path
                if let Some((msg, time)) = &self.status_toast {
                    if time.elapsed().as_secs() < 3 {
                        ui.label(egui::RichText::new(format!("ℹ {}", msg)).color(ui.visuals().warn_fg_color));
                    } else {
                        self.status_toast = None;
                    }
                } else if let Some(path) = &self.file_path {
                    ui.label(egui::RichText::new(path.to_string_lossy()).weak());
                } else {
                    ui.label(egui::RichText::new("就绪").weak());
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let reload_str = if self.config.auto_reload { "热重载: 开启" } else { "热重载: 关闭" };
                    ui.label(egui::RichText::new(format!("{} | 缩放: {:.0}%", reload_str, self.config.zoom_factor * 100.0)).weak());

                    if self.file_path.is_some() {
                        let size_kb = self.file_size_bytes as f64 / 1024.0;
                        ui.label(egui::RichText::new(format!("UTF-8 | {} 词 | {} 行 | {:.1} KB |", self.word_count, self.line_count, size_kb)).weak());
                    }
                });
            });
        });

        // Left TOC Sidebar
        if self.config.show_toc && !self.toc.is_empty() {
            egui::Panel::left("toc_sidebar")
                .resizable(true)
                .default_size(240.0)
                .size_range(180.0..=450.0)
                .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(12, 10)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading(egui::RichText::new("目录大纲").size(16.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("折叠 ⏴").clicked() {
                                self.config.show_toc = false;
                                self.config.save();
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .id_salt("toc_scroll_area")
                        .show(ui, |ui| {
                            for item in &self.toc {
                                let indent = ((item.level.saturating_sub(1)) as f32) * 14.0;
                                ui.horizontal(|ui| {
                                    if indent > 0.0 {
                                        ui.add_space(indent);
                                    }
                                    let btn = egui::Button::new(
                                        egui::RichText::new(&item.title)
                                            .size(if item.level <= 2 { 14.0 } else { 13.0 }),
                                    )
                                    .wrap()
                                    .frame(false);

                                    if ui.add(btn).clicked() {
                                        *self.cache.scroll_to_id_target_mut() = Some(item.anchor_id.clone());
                                        self.scroll_to_offset = None;
                                        ui.ctx().request_repaint();
                                    }
                                });
                                ui.add_space(2.0);
                            }
                        });
                });
        }

        // Central Markdown Content
        egui::CentralPanel::default().show(ui, |ui| {
            if self.file_path.is_none() {
                // Empty state / Welcome screen
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.heading(egui::RichText::new("📖 MdReader").size(32.0).strong());
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("极速轻量 · 秒启动 · 原生 Markdown 阅读器").size(16.0).weak());
                    ui.add_space(24.0);

                    if ui.button(egui::RichText::new(" 📂 打开 Markdown 文件 (Ctrl+O) ").size(16.0)).clicked() {
                        self.trigger_open_dialog();
                    }

                    ui.add_space(16.0);
                    ui.label(egui::RichText::new("支持直接将 .md 文件拖入窗口，或在资源管理器右键选择本程序打开").weak());

                    if !self.config.recent_files.is_empty() {
                        ui.add_space(32.0);
                        ui.label(egui::RichText::new("最近打开的文档:").strong());
                        ui.add_space(8.0);
                        let recent = self.config.recent_files.clone();
                        for path in recent.iter().take(5) {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if ui.button(format!("📄 {}", name)).clicked() {
                                self.open_file(path);
                            }
                        }
                    }
                });
            } else {
                let mut scroll_area = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt("markdown_scroll_area");

                if let Some(target_y) = self.scroll_to_offset.take() {
                    scroll_area = scroll_area.vertical_scroll_offset(target_y);
                }

                let scroll_output = scroll_area.show(ui, |ui| {
                    ui.style_mut().url_in_tooltip = true;
                    let (search_query, active_match) = if self.search.is_open && !self.search.query.trim().is_empty() {
                        (Some(self.search.query.clone()), Some(self.search.current_match))
                    } else {
                        (None, None)
                    };

                    let base_dir = self
                        .file_path
                        .as_ref()
                        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

                    let viewer = CommonMarkViewer::new()
                        .base_dir(base_dir)
                        .enable_scroll_to_heading(true)
                        .syntax_theme_dark("base16-ocean.dark")
                        .syntax_theme_light("base16-ocean.light")
                        .search(search_query, active_match);

                    let available_width = ui.available_width();
                    let max_content_width = 880.0;
                    let horizontal_margin = if available_width > max_content_width + 80.0 {
                        (available_width - max_content_width) / 2.0
                    } else {
                        42.0 // 左右保留至少 42px 的舒适阅读留白
                    };

                    ui.horizontal(|ui| {
                        ui.add_space(horizontal_margin);
                        ui.vertical(|ui| {
                            let content_width = (available_width - horizontal_margin * 2.0).max(200.0);
                            ui.set_max_width(content_width);
                            ui.add_space(18.0);
                            viewer.show(ui, &mut self.cache, &self.rendered_markdown);
                            ui.add_space(60.0);
                        });
                        ui.add_space(horizontal_margin);
                    });
                });

                self.last_content_height = scroll_output.content_size.y;
                self.last_viewport_height = scroll_output.inner_rect.height();
            }

            // Hover overlay for drag-and-drop
            if ui.ctx().input(|i| !i.raw.hovered_files.is_empty()) {
                let rect = ui.max_rect();
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_black_alpha(140),
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "松开鼠标以打开 Markdown 文件",
                    egui::FontId::proportional(22.0),
                    egui::Color32::WHITE,
                );
            }
        });

        // About Dialog Modal
        if self.about_open {
            egui::Window::new("关于 MdReader")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("MdReader v0.1.0");
                        ui.add_space(8.0);
                        ui.label("极速启动 · 极低内存 · 纯绿色便携 Markdown 查看器");
                        ui.add_space(12.0);
                        ui.label("技术栈: Rust + eframe/egui + pulldown-cmark + syntect");
                        ui.label("无需 WebView2 / Chromium 运行时，纯原生即点即看。");
                        ui.add_space(16.0);
                        if ui.button("确定").clicked() {
                            self.about_open = false;
                        }
                    });
                });
        }

        // Handle pending OpenUrl commands (hyperlinks clicked in Markdown document or UI)
        let open_urls: Vec<String> = ui.ctx().output_mut(|o| {
            let mut urls = Vec::new();
            o.commands.retain(|cmd| {
                if let egui::OutputCommand::OpenUrl(open_url) = cmd {
                    urls.push(open_url.url.clone());
                    false
                } else {
                    true
                }
            });
            urls
        });

        for url in open_urls {
            // Check if it is a relative link or file link to a local markdown file (possibly with #fragment)
            if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("mailto:") {
                let (raw_file_part, fragment) = match url.split_once('#') {
                    Some((f, frag)) => (f, Some(frag)),
                    None => (url.as_str(), None),
                };

                let file_part = percent_decode(raw_file_part);
                let frag_part = fragment.map(|f| percent_decode(f));

                // If file_part is empty, it refers to a heading/anchor in the CURRENT document
                if file_part.is_empty() {
                    if let Some(frag) = frag_part {
                        self.scroll_to_heading_or_slug(&frag, ui.ctx());
                    }
                    continue;
                }

                let resolved_path = if let Some(current_file) = &self.file_path {
                    if let Some(parent) = current_file.parent() {
                        parent.join(&file_part)
                    } else {
                        PathBuf::from(&file_part)
                    }
                } else {
                    PathBuf::from(&file_part)
                };

                if resolved_path.is_file() {
                    let is_same_file = self.file_path.as_ref().map_or(false, |curr| {
                        curr == &resolved_path
                            || curr.canonicalize().ok() == resolved_path.canonicalize().ok()
                    });

                    if !is_same_file {
                        self.open_file(&resolved_path);
                    }

                    if let Some(frag) = frag_part {
                        self.scroll_to_heading_or_slug(&frag, ui.ctx());
                    }
                    continue;
                }
            }

            #[cfg(windows)]
            open_browser(&url);
        }
    }
}

pub fn percent_decode(s: &str) -> String {
    let mut bytes = Vec::new();
    let mut iter = s.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let h1 = iter.next();
            let h2 = iter.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                if let Ok(hex_str) = std::str::from_utf8(&[h1, h2]) {
                    if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                        bytes.push(val);
                        continue;
                    }
                }
            }
        }
        bytes.push(b);
    }
    String::from_utf8(bytes).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_decode() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("%E6%A6%82%E8%BF%B0"), "概述");
        assert_eq!(percent_decode("normal_text"), "normal_text");
    }

    #[test]
    fn test_split_link_and_fragment() {
        let url = "docs/guide.md#installation";
        let (file_part, fragment) = url.split_once('#').map(|(f, frag)| (f, Some(frag))).unwrap_or((url, None));
        assert_eq!(file_part, "docs/guide.md");
        assert_eq!(fragment, Some("installation"));

        let in_page = "#top";
        let (file_part2, fragment2) = in_page.split_once('#').map(|(f, frag)| (f, Some(frag))).unwrap_or((in_page, None));
        assert_eq!(file_part2, "");
        assert_eq!(fragment2, Some("top"));
    }
}
