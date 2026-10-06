use crate::viewer::{CommonMarkCache, CommonMarkViewer};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{AppConfig, ThemeMode};
use crate::editor::{
    calculate_line_count, calculate_word_count, content_fingerprint, PendingDocumentAction,
    ViewMode,
};
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
    pub mode: ViewMode,
    pub dirty: bool,
    pub last_saved_fingerprint: Option<u64>,
    pub pending_action: Option<PendingDocumentAction>,
    pub external_change_pending: bool,
    editor_state_generation: u64,
    last_applied_theme: Option<(ThemeMode, egui::SystemTheme)>,
    last_window_title: Option<String>,
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
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    let wide_url: Vec<u16> = OsStr::new(url)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let wide_op: Vec<u16> = OsStr::new("open")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
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
            mode: ViewMode::Reading,
            dirty: false,
            last_saved_fingerprint: None,
            pending_action: None,
            external_change_pending: false,
            editor_state_generation: 0,
            last_applied_theme: None,
            last_window_title: None,
        };

        // Document zoom is applied by the Markdown viewer, never to the app chrome.
        // Disable egui's built-in shortcuts so they cannot also zoom the whole window.
        ctx.options_mut(|options| {
            options.zoom_with_keyboard = false;
            // Theme changes are applied explicitly below. Keeping egui's
            // automatic native-window sync enabled would enqueue SetTheme on
            // every pass when the app also owns the theme preference.
            options.sync_window_theme = false;
        });
        ctx.set_zoom_factor(1.0);

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
            self.config.save();
            self.last_zoom_change = Some(Instant::now());
            ctx.request_repaint();
        }
    }

    pub fn open_file(&mut self, path: &Path) -> bool {
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
            self.show_toast(format!(
                "已打开: {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            true
        } else {
            self.show_toast("打开失败: 文件不存在或非 UTF-8 编码".to_string());
            false
        }
    }

    pub fn reload_current_file(&mut self) -> bool {
        if let Some(path) = self.file_path.clone() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                self.file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.set_content(content);
                self.show_toast("文档已自动刷新".to_string());
                return true;
            }
        }
        self.show_toast("重新载入失败: 文件不存在或无法读取".to_string());
        false
    }

    pub fn replace_document_content(&mut self, content: String) {
        self.editor_state_generation = self.editor_state_generation.wrapping_add(1);
        self.last_saved_fingerprint = Some(content_fingerprint(&content));
        self.dirty = false;
        self.file_content = content;
        self.rebuild_derived_state();
    }

    pub fn rebuild_derived_state(&mut self) {
        self.line_count = calculate_line_count(&self.file_content);
        self.word_count = calculate_word_count(&self.file_content);

        let processed = process_markdown_with_toc(&self.file_content);
        self.rendered_markdown = processed.rendered_markdown;
        self.toc = processed.toc;

        if self.search.is_open {
            self.search.update(&self.rendered_markdown);
        }
    }

    pub fn set_content(&mut self, content: String) {
        self.replace_document_content(content);
    }

    pub fn save_current_file(&mut self) -> Result<(), String> {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => {
                let err = "当前没有可保存的文件".to_string();
                self.show_toast(err.clone());
                return Err(err);
            }
        };

        match std::fs::write(&path, self.file_content.as_bytes()) {
            Ok(()) => {
                let fp = content_fingerprint(&self.file_content);
                self.last_saved_fingerprint = Some(fp);
                self.dirty = false;
                self.file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.show_toast("文件已保存".to_string());
                Ok(())
            }
            Err(e) => {
                let err = format!("保存失败: {}", e);
                self.show_toast(err.clone());
                Err(err)
            }
        }
    }

    pub fn toggle_mode(&mut self) {
        if self.file_path.is_none() {
            self.show_toast("请先打开 Markdown 文件".to_string());
            return;
        }
        match self.mode {
            ViewMode::Reading => {
                self.mode = ViewMode::Editing;
                if self.search.is_open {
                    self.search.close();
                }
            }
            ViewMode::Editing => {
                self.mode = ViewMode::Reading;
                self.rebuild_derived_state();
            }
        }
    }

    pub fn request_toggle_mode(&mut self) {
        if self.file_path.is_none() {
            self.show_toast("请先打开 Markdown 文件".to_string());
            return;
        }

        if self.mode == ViewMode::Editing && self.dirty {
            self.pending_action = Some(PendingDocumentAction::SwitchToReading);
        } else {
            self.toggle_mode();
        }
    }

    pub fn request_open_file(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.request_open_file_with_fragment(path, None, ctx);
    }

    pub fn request_open_file_with_fragment(
        &mut self,
        path: PathBuf,
        fragment: Option<String>,
        ctx: &egui::Context,
    ) {
        if self.dirty {
            self.pending_action = Some(PendingDocumentAction::OpenFile { path, fragment });
        } else {
            let opened = self.open_file(&path);
            if opened {
                if let Some(fragment) = fragment {
                    self.scroll_to_heading_or_slug(&fragment, ctx);
                }
            }
        }
    }

    pub fn request_reload(&mut self, _ctx: &egui::Context) {
        if self.dirty {
            self.pending_action = Some(PendingDocumentAction::ReloadCurrentFile);
        } else {
            self.reload_current_file();
        }
    }

    pub fn request_close(&mut self, ctx: &egui::Context) {
        if self.dirty {
            self.pending_action = Some(PendingDocumentAction::CloseApplication);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub fn execute_pending_action(
        &mut self,
        action: PendingDocumentAction,
        ctx: &egui::Context,
        discard_changes: bool,
    ) -> bool {
        match action {
            PendingDocumentAction::SwitchToReading => {
                if discard_changes && !self.reload_current_file() {
                    return false;
                }
                self.toggle_mode();
                true
            }
            PendingDocumentAction::OpenFile { path, fragment } => {
                if self.open_file(&path) {
                    if let Some(fragment) = fragment {
                        self.scroll_to_heading_or_slug(&fragment, ctx);
                    }
                    true
                } else {
                    false
                }
            }
            PendingDocumentAction::ReloadCurrentFile => self.reload_current_file(),
            PendingDocumentAction::CloseApplication => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                true
            }
            PendingDocumentAction::HandleExternalChange => true,
        }
    }

    pub fn handle_watcher_event(&mut self) {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => return,
        };

        let disk_content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let disk_fp = content_fingerprint(&disk_content);

        // 1. If disk content matches last saved fingerprint, it is our own save event.
        if self.last_saved_fingerprint == Some(disk_fp) {
            return;
        }

        // 2. If disk content is identical to current in-memory content:
        if content_fingerprint(&self.file_content) == disk_fp {
            self.last_saved_fingerprint = Some(disk_fp);
            self.dirty = false;
            return;
        }

        // 3. If in-memory is modified, conflict detected.
        if self.dirty {
            self.external_change_pending = true;
        } else {
            // Clean document: safe auto reload
            self.file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            self.replace_document_content(disk_content);
            self.show_toast("文档已自动刷新".to_string());
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
                if item.anchor_id == frag || item.title.eq_ignore_ascii_case(frag) {
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

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let (sys_theme, visuals) = match self.config.theme_mode {
            ThemeMode::Dark => (egui::SystemTheme::Dark, egui::Visuals::dark()),
            ThemeMode::Light => (egui::SystemTheme::Light, egui::Visuals::light()),
            ThemeMode::System => {
                if ctx.system_theme() == Some(egui::Theme::Light) {
                    (egui::SystemTheme::Light, egui::Visuals::light())
                } else {
                    (egui::SystemTheme::Dark, egui::Visuals::dark())
                }
            }
        };

        let theme_state = (self.config.theme_mode, sys_theme);
        if self.last_applied_theme == Some(theme_state) {
            return;
        }

        let (theme_preference, window_theme) = match self.config.theme_mode {
            ThemeMode::Dark => (egui::ThemePreference::Dark, egui::SystemTheme::Dark),
            ThemeMode::Light => (egui::ThemePreference::Light, egui::SystemTheme::Light),
            ThemeMode::System => (egui::ThemePreference::System, egui::SystemTheme::SystemDefault),
        };
        ctx.set_theme(theme_preference);
        ctx.set_visuals(visuals);
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(window_theme));
        self.last_applied_theme = Some(theme_state);
    }

    fn handle_shortcuts(&mut self, ui: &mut egui::Ui) {
        let input = ui.input(|i| i.clone());

        // Confirmation modals own the current interaction. Do not let shortcuts,
        // file drops, or reload commands mutate the document behind them.
        if self.pending_action.is_some() || self.external_change_pending {
            if input.key_pressed(egui::Key::Escape) {
                self.pending_action = None;
                self.external_change_pending = false;
            }
            return;
        }

        // Ctrl + S: Save file
        if input.modifiers.command && input.key_pressed(egui::Key::S) {
            let _ = self.save_current_file();
        }

        // Ctrl + E: Toggle Edit/Reading mode
        if input.modifiers.command && input.key_pressed(egui::Key::E) {
            self.request_toggle_mode();
        }

        // Ctrl + O: Open file
        if input.modifiers.command && input.key_pressed(egui::Key::O) {
            self.trigger_open_dialog(ui.ctx());
        }

        // F5 or Ctrl + R: Reload
        if input.key_pressed(egui::Key::F5)
            || (input.modifiers.command && input.key_pressed(egui::Key::R))
        {
            self.request_reload(ui.ctx());
        }

        // Ctrl + F: Toggle search
        if input.modifiers.command && input.key_pressed(egui::Key::F) {
            if self.mode == ViewMode::Editing {
                self.show_toast("全文搜索暂仅在阅读模式可用 (Ctrl+E 切换)".to_string());
            } else if self.search.is_open {
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
            if self.pending_action.is_some() {
                self.pending_action = None;
            }
            if self.external_change_pending {
                self.external_change_pending = false;
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
                self.request_open_file(path.to_path_buf(), ui.ctx());
                break;
            }
        }
    }

    fn trigger_open_dialog(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "Markdown 文档 (*.md, *.markdown)",
                &["md", "markdown", "mdown", "mkd", "txt"],
            )
            .add_filter("所有文件 (*.*)", &["*"])
            .pick_file()
        {
            self.request_open_file(path, ctx);
        }
    }

    fn render_modals(&mut self, ctx: &egui::Context) {
        // About Dialog
        if self.about_open {
            egui::Window::new("关于 MdReader")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("MdReader v0.1.1");
                        ui.add_space(8.0);
                        ui.label("极速启动 · 极低内存 · 纯绿色便携 Markdown 查看/编辑器");
                        ui.add_space(12.0);
                        ui.label("技术栈: Rust + eframe/egui + pulldown-cmark + syntect");
                        ui.label("无需 WebView2 / Chromium 运行时，纯原生即点即看即改。");
                        ui.add_space(16.0);
                        if ui.button("确定").clicked() {
                            self.about_open = false;
                        }
                    });
                });
        }

        // Pending Document Action (Unsaved protection)
        if let Some(action) = self.pending_action.clone() {
            let mut cancel_pending = false;
            let mut execute_action = None;

            egui::Modal::new(egui::Id::new("unsaved_document_modal")).show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("⚠️ 当前文档有未保存的修改")
                            .size(15.0)
                            .strong()
                            .color(ui.visuals().warn_fg_color),
                    );
                    ui.add_space(8.0);
                    let action_desc = match &action {
                        PendingDocumentAction::SwitchToReading => "切回阅读模式",
                        PendingDocumentAction::OpenFile { .. } => "打开新文件",
                        PendingDocumentAction::ReloadCurrentFile => "重新载入文件",
                        PendingDocumentAction::CloseApplication => "退出程序",
                        PendingDocumentAction::HandleExternalChange => "继续操作",
                    };
                    ui.label(format!("是否在{}之前保存当前文档的修改？", action_desc));
                    ui.add_space(16.0);

                    ui.horizontal(|ui| {
                        if ui.button("保存并继续").clicked() && self.save_current_file().is_ok()
                        {
                            execute_action = Some((action.clone(), false));
                        }

                        if ui.button("放弃修改").clicked() {
                            execute_action = Some((action.clone(), true));
                        }

                        if ui.button("取消").clicked() {
                            cancel_pending = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            });

            if cancel_pending {
                self.pending_action = None;
            }
            if let Some((act, discard_changes)) = execute_action {
                if self.execute_pending_action(act, ctx, discard_changes) || !discard_changes {
                    self.pending_action = None;
                }
            }
        }

        // External Change Conflict Dialog
        if self.external_change_pending {
            let mut resolve_external_change = None;
            let mut cancel_external_change = false;
            egui::Modal::new(egui::Id::new("external_change_modal")).show(ctx, |ui| {
                    ui.set_min_width(420.0);
                    ui.vertical_centered(|ui| {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("⚠️ 外部文件已发生变更")
                                .size(15.0)
                                .strong()
                                .color(ui.visuals().warn_fg_color),
                        );
                        ui.add_space(8.0);
                        ui.label("磁盘上的文件已被其他程序修改，且本地编辑器有未保存的修改。\n请选择如何处理此冲突：");
                        ui.add_space(16.0);

                        ui.horizontal(|ui| {
                            if ui.button("保留本地修改").clicked() {
                                resolve_external_change = Some(false);
                            }

                            if ui.button("载入磁盘版本").clicked() {
                                resolve_external_change = Some(true);
                            }

                            if ui.button("取消").clicked() {
                                cancel_external_change = true;
                            }
                        });
                        ui.add_space(4.0);
                    });
                });

            if cancel_external_change {
                resolve_external_change = None;
            }
            if let Some(load_disk_version) = resolve_external_change {
                if load_disk_version {
                    if self.reload_current_file() {
                        self.external_change_pending = false;
                        self.show_toast("已载入磁盘最新版本，本地修改已丢弃".to_string());
                    }
                } else {
                    self.external_change_pending = false;
                    self.show_toast("已保留本地修改，存盘时将覆盖磁盘文件".to_string());
                }
            }
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
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(700));
            }
        }

        // Post-zoom safe memory trimming after user finishes zooming (350ms idle)
        #[cfg(windows)]
        if let Some(t) = self.last_zoom_change {
            if t.elapsed() >= std::time::Duration::from_millis(350) {
                self.last_zoom_change = None;
                trim_working_set();
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(350));
            }
        }

        self.apply_theme(ui.ctx());
        self.handle_shortcuts(ui);

        // Check file watcher for external changes
        if self.config.auto_reload && self.watcher.check_reload() {
            self.handle_watcher_event();
        }

        // Intercept viewport close requests (Alt+F4 or system close)
        if ui.ctx().input(|i| i.viewport().close_requested()) && self.dirty {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_action = Some(PendingDocumentAction::CloseApplication);
        }

        // Top Panel: Integrated Title Bar + Menus + Controls + (optional) Search
        egui::Panel::top("top_panel")
            .frame(
                egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin {
                    left: 0,
                    right: 0,
                    top: 0,
                    bottom: if self.search.is_open { 4 } else { 0 },
                }),
            )
            .show(ui, |ui| {
                let is_maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                let title_bar_height = 30.0;

                egui::MenuBar::new().ui(ui, |ui| {
                    let available_width = ui.available_width();
                    // Reserve controls and a usable drag region before laying out menus.
                    let (menu_width, control_width, compact_menu) =
                        title_bar_layout(available_width);
                    let controls_total_width = control_width * 3.0;
                    let mut render_menus = |ui: &mut egui::Ui| {
                        ui.add_space(8.0);
                        let icon_image =
                            egui::Image::new(egui::include_image!("../assets/app_icon.png"))
                                .fit_to_exact_size(egui::vec2(16.0, 16.0));
                        if ui
                            .add(egui::Button::image(icon_image).frame(false))
                            .on_hover_text("MdReader - 关于")
                            .clicked()
                        {
                            self.about_open = true;
                        }
                        ui.add_space(4.0);

                        ui.menu_button("文件 (F)", |ui| {
                            if ui.button("打开... (Ctrl+O)").clicked() {
                                self.trigger_open_dialog(ui.ctx());
                                ui.close();
                            }
                            let save_enabled = self.file_path.is_some();
                            if ui
                                .add_enabled(save_enabled, egui::Button::new("保存 (Ctrl+S)"))
                                .clicked()
                            {
                                let _ = self.save_current_file();
                                ui.close();
                            }
                            if ui.button("重新载入 (F5)").clicked() {
                                self.request_reload(ui.ctx());
                                ui.close();
                            }
                            ui.separator();
                            ui.menu_button("最近打开", |ui| {
                                if self.config.recent_files.is_empty() {
                                    ui.label("暂无记录");
                                } else {
                                    let recent = self.config.recent_files.clone();
                                    for path in recent {
                                        let name =
                                            path.file_name().unwrap_or_default().to_string_lossy();
                                        if ui.button(format!("{}", name)).clicked() {
                                            self.request_open_file(path, ui.ctx());
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
                                        self.show_toast(
                                            "已成功将 MdReader 设为系统默认 Markdown 阅读器！"
                                                .to_string(),
                                        );
                                    }
                                    Err(e) => {
                                        self.show_toast(format!("设置失败: {}", e));
                                    }
                                }
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("退出").clicked() {
                                self.request_close(ui.ctx());
                                ui.close();
                            }
                        });

                        ui.menu_button("视图 (V)", |ui| {
                            let mode_label = match self.mode {
                                ViewMode::Reading => "进入编辑模式 (Ctrl+E)",
                                ViewMode::Editing => "返回阅读模式 (Ctrl+E)",
                            };
                            if ui
                                .add_enabled(
                                    self.file_path.is_some(),
                                    egui::Button::new(mode_label),
                                )
                                .clicked()
                            {
                                self.request_toggle_mode();
                                ui.close();
                            }
                            ui.separator();
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
                            if ui
                                .radio_value(
                                    &mut self.config.theme_mode,
                                    ThemeMode::Dark,
                                    "深色模式 (Dark)",
                                )
                                .clicked()
                            {
                                self.config.save();
                                self.apply_theme(ui.ctx());
                            }
                            if ui
                                .radio_value(
                                    &mut self.config.theme_mode,
                                    ThemeMode::Light,
                                    "浅色模式 (Light)",
                                )
                                .clicked()
                            {
                                self.config.save();
                                self.apply_theme(ui.ctx());
                            }
                            if ui
                                .radio_value(
                                    &mut self.config.theme_mode,
                                    ThemeMode::System,
                                    "跟随系统 (System)",
                                )
                                .clicked()
                            {
                                self.config.save();
                                self.apply_theme(ui.ctx());
                            }
                        });

                        ui.menu_button("查找 (S)", |ui| {
                            if ui.button("全文搜索 (Ctrl+F)").clicked() {
                                if self.mode == ViewMode::Editing {
                                    self.show_toast(
                                        "全文搜索暂仅在阅读模式可用 (Ctrl+E 切换)".to_string(),
                                    );
                                } else {
                                    self.search.open();
                                    self.search.update(&self.rendered_markdown);
                                }
                                ui.close();
                            }
                        });

                        ui.menu_button("帮助 (H)", |ui| {
                            if ui.button("关于 MdReader").clicked() {
                                self.about_open = true;
                                ui.close();
                            }
                        });

                        ui.add_space(6.0);
                        let has_file = self.file_path.is_some();
                        let mode_btn_text = match self.mode {
                            ViewMode::Reading => egui::RichText::new("📝 编辑").size(12.0),
                            ViewMode::Editing => egui::RichText::new("📖 阅读").size(12.0).strong(),
                        };
                        if ui
                            .add_enabled(has_file, egui::Button::new(mode_btn_text))
                            .on_hover_text("切换阅读/编辑模式 (Ctrl+E)")
                            .clicked()
                        {
                            self.request_toggle_mode();
                        }

                        if self.dirty {
                            ui.add_space(4.0);
                            let save_btn = egui::Button::new(
                                egui::RichText::new("💾 保存")
                                    .size(12.0)
                                    .color(ui.visuals().warn_fg_color),
                            );
                            if ui
                                .add(save_btn)
                                .on_hover_text("保存当前修改 (Ctrl+S)")
                                .clicked()
                            {
                                let _ = self.save_current_file();
                            }
                        }
                    };

                    ui.allocate_ui_with_layout(
                        egui::vec2(menu_width, title_bar_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if compact_menu {
                                ui.menu_button("☰", |ui| render_menus(ui));
                            } else {
                                render_menus(ui);
                            }
                        },
                    );

                    // Zero item spacing for draggable title and window controls
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let avail = ui.available_width();
                    let drag_width = (avail - controls_total_width).max(0.0);

                    let (title_rect, title_resp) = ui.allocate_exact_size(
                        egui::vec2(drag_width, title_bar_height),
                        egui::Sense::click_and_drag(),
                    );

                    if title_resp.drag_started() || title_resp.dragged() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if title_resp.double_clicked() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                    }

                    let title_text = if let Some(path) = &self.file_path {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        if self.dirty {
                            format!("{}* - MdReader", name)
                        } else {
                            format!("{} - MdReader", name)
                        }
                    } else {
                        "MdReader".to_string()
                    };
                    if self.last_window_title.as_deref() != Some(title_text.as_str()) {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Title(title_text.clone()));
                        self.last_window_title = Some(title_text.clone());
                    }

                    let font_id = egui::FontId::proportional(12.0);
                    let title_color = ui.visuals().weak_text_color();
                    let painter = ui.painter().with_clip_rect(title_rect);
                    let galley = painter.layout_no_wrap(title_text, font_id, title_color);
                    let text_width = galley.size().x;

                    let screen_center_x = ui
                        .ctx()
                        .input(|i| i.raw.screen_rect)
                        .map_or_else(|| ui.max_rect().center().x, |r| r.center().x);
                    let draw_x = if screen_center_x - text_width / 2.0 >= title_rect.left() + 6.0
                        && screen_center_x + text_width / 2.0 <= title_rect.right() - 6.0
                    {
                        screen_center_x - text_width / 2.0
                    } else {
                        (title_rect.center().x - text_width / 2.0).max(title_rect.left() + 6.0)
                    };
                    let draw_y = title_rect.center().y - galley.size().y / 2.0;
                    painter.galley(egui::pos2(draw_x, draw_y), galley, title_color);

                    // Window control buttons: Minimize, Maximize/Restore, Close
                    let mut close_clicked = false;
                    render_window_controls_sized_with_action(
                        ui,
                        is_maximized,
                        control_width,
                        Some(&mut || close_clicked = true),
                    );
                    if close_clicked {
                        self.request_close(ui.ctx());
                    }
                });

                // Inline Search Bar (Ctrl+F)
                if self.search.is_open {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
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
                    ui.add_space(2.0);
                }
            });

        // Bottom Status Bar
        egui::Panel::bottom("bottom_panel").show(ui, |ui| {
            if self
                .status_toast
                .as_ref()
                .is_some_and(|(_, time)| time.elapsed().as_secs() >= 3)
            {
                self.status_toast = None;
            }
            let message = if let Some((msg, _)) = &self.status_toast {
                egui::RichText::new(format!("ℹ {}", msg)).color(ui.visuals().warn_fg_color)
            } else if let Some(path) = &self.file_path {
                egui::RichText::new(display_path(path)).weak()
            } else {
                egui::RichText::new("就绪").weak()
            };

            let mode_str = match self.mode {
                ViewMode::Reading => "阅读",
                ViewMode::Editing => "编辑",
            };
            let dirty_str = if self.dirty { "未保存" } else { "已保存" };
            let reload_str = if self.config.auto_reload {
                "热重载: 开启"
            } else {
                "热重载: 关闭"
            };
            let summary = format!(
                "{}/{} | 缩放: {:.0}%",
                mode_str,
                dirty_str,
                self.config.zoom_factor * 100.0
            );
            let details = if self.file_path.is_some() {
                let size_kb = self.file_size_bytes as f64 / 1024.0;
                format!(
                    "UTF-8 | {} 模式 | {} | {} 词 | {} 行 | {:.1} KB | {} | 缩放: {:.0}%",
                    mode_str,
                    dirty_str,
                    self.word_count,
                    self.line_count,
                    size_kb,
                    reload_str,
                    self.config.zoom_factor * 100.0
                )
            } else {
                summary.clone()
            };
            render_status_row(ui, message, &details, &summary);
        });

        // Left TOC Sidebar
        if self.config.show_toc && !self.toc.is_empty() {
            egui::Panel::left("toc_sidebar")
                .resizable(true)
                .default_size(240.0)
                .size_range(180.0..=450.0)
                .frame(
                    egui::Frame::side_top_panel(ui.style())
                        .inner_margin(egui::Margin::symmetric(12, 10)),
                )
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

                    let mut clicked_anchor = None;
                    let mut show_editing_toast = false;

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
                                    let btn =
                                        egui::Button::new(
                                            egui::RichText::new(&item.title)
                                                .size(if item.level <= 2 { 14.0 } else { 13.0 }),
                                        )
                                        .wrap()
                                        .frame(false);

                                    if ui.add(btn).clicked() {
                                        if self.mode == ViewMode::Editing {
                                            show_editing_toast = true;
                                        } else {
                                            clicked_anchor = Some(item.anchor_id.clone());
                                        }
                                    }
                                });
                                ui.add_space(2.0);
                            }
                        });

                    if show_editing_toast {
                        self.show_toast("请在阅读模式中点击大纲跳转 (Ctrl+E 切换)".to_string());
                    }
                    if let Some(anchor) = clicked_anchor {
                        *self.cache.scroll_to_id_target_mut() = Some(anchor);
                        self.scroll_to_offset = None;
                        ui.ctx().request_repaint();
                    }
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
                    ui.label(
                        egui::RichText::new("极速轻量 · 秒启动 · 原生 Markdown 阅读器")
                            .size(16.0)
                            .weak(),
                    );
                    ui.add_space(24.0);

                    if ui
                        .button(egui::RichText::new(" 📂 打开 Markdown 文件 (Ctrl+O) ").size(16.0))
                        .clicked()
                    {
                        self.trigger_open_dialog(ui.ctx());
                    }

                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new(
                            "支持直接将 .md 文件拖入窗口，或在资源管理器右键选择本程序打开",
                        )
                        .weak(),
                    );

                    if !self.config.recent_files.is_empty() {
                        ui.add_space(32.0);
                        ui.label(egui::RichText::new("最近打开的文档:").strong());
                        ui.add_space(8.0);
                        let recent = self.config.recent_files.clone();
                        for path in recent.iter().take(5) {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if ui.button(format!("📄 {}", name)).clicked() {
                                self.request_open_file(path.clone(), ui.ctx());
                            }
                        }
                    }
                });
            } else {
                match self.mode {
                    ViewMode::Reading => {
                        let mut scroll_area = egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .id_salt("markdown_scroll_area");

                        if let Some(target_y) = self.scroll_to_offset.take() {
                            scroll_area = scroll_area.vertical_scroll_offset(target_y);
                        }

                        let scroll_output = scroll_area.show(ui, |ui| {
                            ui.style_mut().url_in_tooltip = true;
                            let (search_query, active_match) =
                                if self.search.is_open && !self.search.query.trim().is_empty() {
                                    (
                                        Some(self.search.query.clone()),
                                        Some(self.search.current_match),
                                    )
                                } else {
                                    (None, None)
                                };

                            let base_dir = self
                                .file_path
                                .as_ref()
                                .and_then(|p| p.parent().map(|d| d.to_path_buf()));

                            let viewer = CommonMarkViewer::new()
                                .zoom_factor(self.config.zoom_factor)
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
                                    let content_width =
                                        (available_width - horizontal_margin * 2.0).max(200.0);
                                    // Give the Markdown renderer a real wrapping width. A max-only
                                    // constraint lets long paragraphs grow past the central panel.
                                    ui.set_width(content_width);
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
                    ViewMode::Editing => {
                        let available_width = ui.available_width();
                        let max_content_width = 880.0;
                        let horizontal_margin = if available_width > max_content_width + 80.0 {
                            (available_width - max_content_width) / 2.0
                        } else {
                            42.0
                        };

                        let scroll_output = egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .id_salt("editor_scroll_area")
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_space(horizontal_margin);
                                    ui.vertical(|ui| {
                                        let content_width =
                                            (available_width - horizontal_margin * 2.0).max(200.0);
                                        ui.set_width(content_width);
                                        ui.add_space(18.0);

                                        let text_edit =
                                            egui::TextEdit::multiline(&mut self.file_content)
                                                .id_salt((
                                                    "markdown_editor",
                                                    self.editor_state_generation,
                                                ))
                                                .desired_width(content_width)
                                                .desired_rows(30)
                                                .font(egui::TextStyle::Monospace)
                                                .lock_focus(true);

                                        let resp = ui.add(text_edit);
                                        if resp.changed() {
                                            let current_fp =
                                                content_fingerprint(&self.file_content);
                                            self.dirty =
                                                self.last_saved_fingerprint != Some(current_fp);
                                            self.line_count =
                                                calculate_line_count(&self.file_content);
                                            self.word_count =
                                                calculate_word_count(&self.file_content);
                                        }
                                        ui.add_space(60.0);
                                    });
                                    ui.add_space(horizontal_margin);
                                });
                            });

                        self.last_content_height = scroll_output.content_size.y;
                        self.last_viewport_height = scroll_output.inner_rect.height();
                    }
                }
            }

            // Hover overlay for drag-and-drop
            if ui.ctx().input(|i| !i.raw.hovered_files.is_empty()) {
                let rect = ui.max_rect();
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(140));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "松开鼠标以打开 Markdown 文件",
                    egui::FontId::proportional(22.0),
                    egui::Color32::WHITE,
                );
            }
        });

        // Modals (About, Unsaved Protection, External Conflict)
        self.render_modals(ui.ctx());

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
            if !url.starts_with("http://")
                && !url.starts_with("https://")
                && !url.starts_with("mailto:")
            {
                let (raw_file_part, fragment) = match url.split_once('#') {
                    Some((f, frag)) => (f, Some(frag)),
                    None => (url.as_str(), None),
                };

                let file_part = percent_decode(raw_file_part);
                let frag_part = fragment.map(percent_decode);

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
                    let is_same_file = self.file_path.as_ref().is_some_and(|curr| {
                        curr == &resolved_path
                            || curr.canonicalize().ok() == resolved_path.canonicalize().ok()
                    });

                    if !is_same_file {
                        self.request_open_file_with_fragment(
                            resolved_path.clone(),
                            frag_part,
                            ui.ctx(),
                        );
                    } else if let Some(frag) = frag_part {
                        self.scroll_to_heading_or_slug(&frag, ui.ctx());
                    }
                    continue;
                }
            }

            #[cfg(windows)]
            open_browser(&url);
        }

        // Handle window resizing and draw 1px border stroke when unmaximized
        let is_maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        if !is_maximized {
            handle_window_resize(ui);

            let screen_rect = ui
                .ctx()
                .input(|i| i.raw.screen_rect)
                .unwrap_or_else(|| ui.max_rect());
            let border_color = if ui.visuals().dark_mode {
                egui::Color32::from_gray(65)
            } else {
                egui::Color32::from_gray(195)
            };
            ui.painter().rect_stroke(
                screen_rect,
                0.0,
                egui::Stroke::new(1.0, border_color),
                egui::StrokeKind::Inside,
            );
        }
    }
}

fn render_status_row(
    ui: &mut egui::Ui,
    message: egui::RichText,
    details: &str,
    summary: &str,
) -> (egui::Response, egui::Response) {
    let available_width = ui.available_width();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().weak_text_color();
    let measure = |text: &str| {
        ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(text.to_owned(), font.clone(), color)
                .size()
                .x
        })
    };
    // Keep some room for the path. On narrow windows the complete statistics
    // remain available in a tooltip while reload and zoom stay on the bar.
    let full_width = measure(details);
    let info = if full_width <= available_width * 0.75 {
        details
    } else {
        summary
    };
    let info_width = measure(info).ceil().min(available_width);
    let gap = ui
        .spacing()
        .item_spacing
        .x
        .min((available_width - info_width).max(0.0));
    let message_width = (available_width - info_width - gap).max(0.0);
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let (bar_rect, _) =
        ui.allocate_exact_size(egui::vec2(available_width, height), egui::Sense::hover());

    // Use dedicated left/right child layouts. `add_sized` centers its child
    // widget, which made a long path look centered even with a wide label.
    let message_rect =
        egui::Rect::from_min_size(bar_rect.left_top(), egui::vec2(message_width, height));
    let info_rect = egui::Rect::from_min_size(
        egui::pos2(bar_rect.right() - info_width, bar_rect.top()),
        egui::vec2(info_width, height),
    );
    let mut message_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("status_message")
            .max_rect(message_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let message_response = message_ui.add(
        egui::Label::new(message)
            .truncate()
            .halign(egui::Align::LEFT),
    );
    let mut info_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("status_info")
            .max_rect(info_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let info_response = info_ui
        .add(
            egui::Label::new(egui::RichText::new(info).weak())
                .truncate()
                .halign(egui::Align::RIGHT),
        )
        .on_hover_text(details);
    (message_response, info_response)
}

fn display_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = path.strip_prefix("\\\\?\\") {
        rest.to_owned()
    } else {
        path.into_owned()
    }
}

fn title_bar_layout(available_width: f32) -> (f32, f32, bool) {
    let control_width = ((available_width - 64.0) / 3.0).clamp(0.0, 46.0);
    let compact = available_width < 420.0 + control_width * 3.0 + 32.0;
    (if compact { 32.0 } else { 420.0 }, control_width, compact)
}

#[cfg(test)]
fn render_window_controls(ui: &mut egui::Ui, is_maximized: bool) {
    render_window_controls_sized(ui, is_maximized, 46.0);
}

#[cfg(test)]
fn render_window_controls_sized(ui: &mut egui::Ui, is_maximized: bool, width: f32) -> egui::Rect {
    render_window_controls_sized_with_action(ui, is_maximized, width, None)
}

fn render_window_controls_sized_with_action(
    ui: &mut egui::Ui,
    is_maximized: bool,
    width: f32,
    mut on_close: Option<&mut dyn FnMut()>,
) -> egui::Rect {
    let hover_bg = if ui.visuals().dark_mode {
        egui::Color32::from_white_alpha(26)
    } else {
        egui::Color32::from_black_alpha(18)
    };
    let active_bg = if ui.visuals().dark_mode {
        egui::Color32::from_white_alpha(45)
    } else {
        egui::Color32::from_black_alpha(35)
    };
    let icon_color = ui.visuals().text_color();

    // 1. Minimize button
    let (min_rect, min_resp) =
        ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
    let min_resp = min_resp.on_hover_text("最小化");
    if min_resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(min_rect, 0.0, active_bg);
    } else if min_resp.hovered() {
        ui.painter().rect_filled(min_rect, 0.0, hover_bg);
    }
    let center = min_rect.center();
    let line_y = center.y + 1.0;
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 5.0, line_y),
            egui::pos2(center.x + 5.0, line_y),
        ],
        egui::Stroke::new(1.0, icon_color),
    );
    if min_resp.clicked() {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }

    // 2. Maximize / Restore button
    let (max_rect, max_resp) =
        ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
    let max_resp = max_resp.on_hover_text(if is_maximized {
        "向下还原"
    } else {
        "最大化"
    });
    if max_resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(max_rect, 0.0, active_bg);
    } else if max_resp.hovered() {
        ui.painter().rect_filled(max_rect, 0.0, hover_bg);
    }
    let center = max_rect.center();
    if is_maximized {
        // Restore icon: two overlapping boxes
        let front_rect = egui::Rect::from_min_size(
            egui::pos2(center.x - 5.0, center.y - 3.0),
            egui::vec2(8.0, 8.0),
        );
        ui.painter().rect_stroke(
            front_rect,
            0.0,
            egui::Stroke::new(1.0, icon_color),
            egui::StrokeKind::Inside,
        );

        let back_min = egui::pos2(center.x - 3.0, center.y - 5.0);
        let back_max = egui::pos2(center.x + 5.0, center.y + 3.0);
        let stroke = egui::Stroke::new(1.0, icon_color);
        ui.painter()
            .line_segment([back_min, egui::pos2(back_max.x, back_min.y)], stroke);
        ui.painter()
            .line_segment([egui::pos2(back_max.x, back_min.y), back_max], stroke);
        ui.painter()
            .line_segment([egui::pos2(front_rect.max.x, back_max.y), back_max], stroke);
        ui.painter()
            .line_segment([back_min, egui::pos2(back_min.x, front_rect.min.y)], stroke);
    } else {
        // Maximize icon: single square
        let box_rect = egui::Rect::from_center_size(center, egui::vec2(10.0, 10.0));
        ui.painter().rect_stroke(
            box_rect,
            0.0,
            egui::Stroke::new(1.0, icon_color),
            egui::StrokeKind::Inside,
        );
    }
    if max_resp.clicked() {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
    }

    // 3. Close button
    let (close_rect, close_resp) =
        ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
    let close_resp = close_resp.on_hover_text("关闭");
    let close_hover_bg = egui::Color32::from_rgb(232, 17, 35); // Windows red #E81123
    let close_active_bg = egui::Color32::from_rgb(196, 43, 28); // #C42B1C

    let (bg_color, close_icon_color) = if close_resp.is_pointer_button_down_on() {
        (Some(close_active_bg), egui::Color32::WHITE)
    } else if close_resp.hovered() {
        (Some(close_hover_bg), egui::Color32::WHITE)
    } else {
        (None, icon_color)
    };

    if let Some(bg) = bg_color {
        ui.painter().rect_filled(close_rect, 0.0, bg);
    }
    let center = close_rect.center();
    let half = 4.5;
    let stroke = egui::Stroke::new(1.0, close_icon_color);
    ui.painter().line_segment(
        [
            egui::pos2(center.x - half, center.y - half),
            egui::pos2(center.x + half, center.y + half),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x + half, center.y - half),
            egui::pos2(center.x - half, center.y + half),
        ],
        stroke,
    );

    if close_resp.clicked() {
        if let Some(ref mut on_close_fn) = on_close {
            on_close_fn();
        } else {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    min_rect.union(max_rect).union(close_rect)
}

fn handle_window_resize(ui: &mut egui::Ui) {
    let screen_rect = ui
        .ctx()
        .input(|i| i.raw.screen_rect)
        .unwrap_or_else(|| ui.max_rect());
    let pointer_pos = ui.ctx().input(|i| i.pointer.latest_pos());

    if let Some(pos) = pointer_pos {
        // If mouse is already dragging inside the window, don't interrupt with resize cursor
        if ui.input(|i| {
            i.pointer.button_down(egui::PointerButton::Primary)
                && !i.pointer.button_pressed(egui::PointerButton::Primary)
        }) {
            return;
        }

        let border = 6.0;
        let corner = 14.0;

        let left = pos.x <= screen_rect.left() + border;
        let right = pos.x >= screen_rect.right() - border;
        let top = pos.y <= screen_rect.top() + border;
        let bottom = pos.y >= screen_rect.bottom() - border;

        // Reserve top-right window controls (138x30px) so clicks to Minimize/Maximize/Close are never intercepted
        let in_controls_x = pos.x > screen_rect.right() - 138.0;
        let in_controls_y = pos.y < screen_rect.top() + 30.0;
        if in_controls_x && in_controls_y {
            return;
        }

        let near_left = pos.x <= screen_rect.left() + corner;
        let near_right = pos.x >= screen_rect.right() - corner;
        let near_top = pos.y <= screen_rect.top() + corner;
        let near_bottom = pos.y >= screen_rect.bottom() - corner;

        let dir = if (top && near_left) || (left && near_top) {
            Some(egui::viewport::ResizeDirection::NorthWest)
        } else if (top && near_right) || (right && near_top) {
            Some(egui::viewport::ResizeDirection::NorthEast)
        } else if (bottom && near_left) || (left && near_bottom) {
            Some(egui::viewport::ResizeDirection::SouthWest)
        } else if (bottom && near_right) || (right && near_bottom) {
            Some(egui::viewport::ResizeDirection::SouthEast)
        } else if top {
            Some(egui::viewport::ResizeDirection::North)
        } else if bottom {
            Some(egui::viewport::ResizeDirection::South)
        } else if left {
            Some(egui::viewport::ResizeDirection::West)
        } else if right {
            Some(egui::viewport::ResizeDirection::East)
        } else {
            None
        };

        if let Some(dir) = dir {
            let cursor = match dir {
                egui::viewport::ResizeDirection::North => egui::CursorIcon::ResizeNorth,
                egui::viewport::ResizeDirection::South => egui::CursorIcon::ResizeSouth,
                egui::viewport::ResizeDirection::East => egui::CursorIcon::ResizeEast,
                egui::viewport::ResizeDirection::West => egui::CursorIcon::ResizeWest,
                egui::viewport::ResizeDirection::NorthEast => egui::CursorIcon::ResizeNorthEast,
                egui::viewport::ResizeDirection::NorthWest => egui::CursorIcon::ResizeNorthWest,
                egui::viewport::ResizeDirection::SouthEast => egui::CursorIcon::ResizeSouthEast,
                egui::viewport::ResizeDirection::SouthWest => egui::CursorIcon::ResizeSouthWest,
            };
            ui.ctx().set_cursor_icon(cursor);

            if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
            }
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
        let (file_part, fragment) = url
            .split_once('#')
            .map(|(f, frag)| (f, Some(frag)))
            .unwrap_or((url, None));
        assert_eq!(file_part, "docs/guide.md");
        assert_eq!(fragment, Some("installation"));

        let in_page = "#top";
        let (file_part2, fragment2) = in_page
            .split_once('#')
            .map(|(f, frag)| (f, Some(frag)))
            .unwrap_or((in_page, None));
        assert_eq!(file_part2, "");
        assert_eq!(fragment2, Some("top"));
    }

    #[test]
    fn test_status_row_keeps_long_paths_separate_from_statistics() {
        let ctx = egui::Context::default();
        crate::setup_custom_fonts(&ctx);
        for width in [500.0, 1040.0] {
            for _ in 0..2 {
                let mut left_edge = 0.0;
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 750.0),
                    )),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    left_edge = ui.max_rect().left();
                    egui::Panel::bottom("status_panel").show(ui, |ui| {
                        let right_edge = ui.max_rect().right();
                        let (path, info) = render_status_row(
                            ui,
                            egui::RichText::new(r"D:\Project\Applications\MdReader\很长的目录名称\编辑功能开发计划.md").weak(),
                            "UTF-8 | 7698 词 | 414 行 | 17.2 KB | 热重载: 开启 | 缩放: 210%",
                            "热重载: 开启 | 缩放: 210%",
                        );
                        assert!(path.rect.right() <= info.rect.left());
                        assert!(info.rect.right() <= right_edge + 0.01);
                        assert!(path.rect.height() < 30.0);
                    });
                });
                let path_shape = output
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        if let egui::Shape::Text(text_shape) = &shape.shape {
                            if text_shape
                                .galley
                                .job
                                .text
                                .contains("D:\\Project\\Applications")
                            {
                                return Some(text_shape);
                            }
                        }
                        None
                    })
                    .expect("status path should be rendered");
                assert!(
                    path_shape.pos.x <= left_edge + 16.0,
                    "path is not left aligned: x={}",
                    path_shape.pos.x
                );
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn test_display_path_removes_windows_extended_prefix() {
        assert_eq!(
            display_path(Path::new(r"\\?\D:\Project\demo.md")),
            r"D:\Project\demo.md"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\demo.md")),
            r"\\server\share\demo.md"
        );
        assert_eq!(
            display_path(Path::new(r"D:\Project\demo.md")),
            r"D:\Project\demo.md"
        );
    }

    #[test]
    fn test_window_controls_layout() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::Panel::top("title_panel").show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("文件", |_ui| {});
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let avail = ui.available_width();
                    let controls_total_width = 46.0 * 3.0;
                    let drag_width = (avail - controls_total_width).max(0.0);
                    let (_rect, _resp) = ui.allocate_exact_size(
                        egui::vec2(drag_width, 30.0),
                        egui::Sense::click_and_drag(),
                    );
                    render_window_controls(ui, false);
                    assert_eq!(ui.available_width(), 0.0);
                });
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn test_window_controls_render_maximized() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::Panel::top("title_panel").show(ui, |ui| {
                render_window_controls(ui, true);
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn test_title_bar_controls_fit_narrow_and_zoomed_windows() {
        for window_width in [500.0, 1040.0] {
            for zoom in [1.0, 2.0, 3.0] {
                let ctx = egui::Context::default();
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(window_width / zoom, 750.0 / zoom),
                    )),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    egui::Panel::top("title_panel").show(ui, |ui| {
                        egui::MenuBar::new().ui(ui, |ui| {
                            let right_edge = ui.max_rect().right();
                            let (menu_width, control_width, compact) =
                                title_bar_layout(ui.available_width());
                            ui.allocate_ui_with_layout(
                                egui::vec2(menu_width, 30.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    if compact {
                                        ui.menu_button("☰", |_| {});
                                    } else {
                                        ui.add_space(40.0);
                                        for label in [
                                            "文件 (F)",
                                            "视图 (V)",
                                            "主题 (T)",
                                            "查找 (S)",
                                            "帮助 (H)",
                                        ] {
                                            ui.menu_button(label, |_| {});
                                        }
                                    }
                                },
                            );
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let drag_width = ui.available_width() - control_width * 3.0;
                            assert!(drag_width >= 16.0, "width={window_width}, zoom={zoom}");
                            ui.allocate_exact_size(
                                egui::vec2(drag_width, 30.0),
                                egui::Sense::click_and_drag(),
                            );
                            let controls = render_window_controls_sized(ui, false, control_width);
                            assert!(controls.right() <= right_edge + 0.01);
                            assert!(control_width >= 24.0);
                        });
                    });
                });
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn test_app_edit_flow_dirty_and_rebuild_derived_state() {
        let ctx = egui::Context::default();
        let mut app = MdReaderApp::new(None, &ctx);
        app.replace_document_content("# Initial Title\n\nSome body text.".to_string());

        assert!(!app.dirty);
        assert_eq!(app.toc.len(), 1);
        assert_eq!(app.toc[0].title, "Initial Title");

        // Simulate typing in edit mode
        app.file_content = "# Updated Heading\n\n## Sub heading\n\nNew body.".to_string();
        let current_fp = content_fingerprint(&app.file_content);
        app.dirty = app.last_saved_fingerprint != Some(current_fp);
        assert!(app.dirty);

        // Rebuild derived state (e.g. when toggling back to reading mode)
        app.rebuild_derived_state();
        assert_eq!(app.toc.len(), 2);
        assert_eq!(app.toc[0].title, "Updated Heading");
        assert_eq!(app.toc[1].title, "Sub heading");
        assert_eq!(app.line_count, 5);
    }

    #[test]
    fn test_app_save_success_and_failure() {
        let ctx = egui::Context::default();
        let mut app = MdReaderApp::new(None, &ctx);
        app.file_path = None; // Isolate from user config recent_files

        // 1. Without file path, saving returns error
        let err = app.save_current_file();
        assert!(err.is_err());

        // 2. With valid temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!(
            "mdreader_test_{}.md",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&temp_file, "# Temp File").unwrap();

        app.open_file(&temp_file);
        assert!(!app.dirty);

        // Modify in-memory
        app.file_content = "# Temp File Modified\nNew content.".to_string();
        let fp = content_fingerprint(&app.file_content);
        app.dirty = app.last_saved_fingerprint != Some(fp);
        assert!(app.dirty);

        // Save
        let save_res = app.save_current_file();
        assert!(save_res.is_ok());
        assert!(!app.dirty);
        assert_eq!(app.last_saved_fingerprint, Some(fp));

        // Verify disk content
        let on_disk = std::fs::read_to_string(&temp_file).unwrap();
        assert_eq!(on_disk, app.file_content);

        // Clean up
        let _ = std::fs::remove_file(temp_file);
    }

    #[test]
    fn test_app_mode_toggle() {
        let ctx = egui::Context::default();
        let mut app = MdReaderApp::new(None, &ctx);
        app.file_path = None; // Isolate from user config recent_files
        app.mode = ViewMode::Reading;

        // Cannot toggle without file
        app.toggle_mode();
        assert_eq!(app.mode, ViewMode::Reading);

        // With file
        app.file_path = Some(PathBuf::from("dummy.md"));
        app.file_content = "# Title".to_string();
        app.toggle_mode();
        assert_eq!(app.mode, ViewMode::Editing);

        app.dirty = true;
        app.request_toggle_mode();
        assert_eq!(app.mode, ViewMode::Editing);
        assert_eq!(
            app.pending_action,
            Some(PendingDocumentAction::SwitchToReading)
        );

        app.pending_action = None;
        app.dirty = false;
        app.toggle_mode();
        assert_eq!(app.mode, ViewMode::Reading);
    }
}
