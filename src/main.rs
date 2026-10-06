#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod default_app;
mod editor;
mod search;
mod toc;
mod viewer;
mod watcher;

use app::MdReaderApp;
use eframe::egui;
use std::path::PathBuf;

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Check system Chinese fonts on Windows
    let candidate_fonts = [
        r"C:\Windows\Fonts\msyh.ttc", // 微软雅黑
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simsun.ttc", // 宋体
        r"C:\Windows\Fonts\simhei.ttf", // 黑体
    ];

    let mut loaded_font_name = None;
    for path in &candidate_fonts {
        if let Ok(file) = std::fs::File::open(path) {
            if let Ok(mmap) = unsafe { memmap2::Mmap::map(&file) } {
                // Intentionally leak the mmap allocation so it lives for the app's lifetime
                let mmap_ref: &'static memmap2::Mmap = Box::leak(Box::new(mmap));
                let static_slice: &'static [u8] = &mmap_ref[..];
                let font_key = "system_cjk_font".to_string();
                fonts.font_data.insert(
                    font_key.clone(),
                    std::sync::Arc::new(egui::FontData::from_static(static_slice)),
                );
                loaded_font_name = Some(font_key);
                break;
            }
        }
    }

    if let Some(font_key) = loaded_font_name {
        // Prepend to Proportional family (standard text)
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, font_key.clone());

        // Also append to Monospace family for code comments / strings
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push(font_key);
    }

    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result<()> {
    // Parse CLI argument if any: mdreader.exe <file_path>
    let cli_file = std::env::args().nth(1).map(PathBuf::from);

    let icon_bytes = include_bytes!("../assets/app_icon.png");
    let icon = eframe::icon_data::from_png_bytes(icon_bytes).ok();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MdReader")
        .with_decorations(false)
        .with_inner_size([1040.0, 750.0])
        .with_min_inner_size([500.0, 380.0])
        .with_drag_and_drop(true);

    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "MdReader",
        native_options,
        Box::new(move |cc| {
            // Setup CJK fallback fonts so Chinese characters render clearly
            setup_custom_fonts(&cc.egui_ctx);

            // Install egui image loaders for local/network images and SVGs
            egui_extras::install_image_loaders(&cc.egui_ctx);

            Ok(Box::new(MdReaderApp::new(cli_file, &cc.egui_ctx)))
        }),
    )
}
