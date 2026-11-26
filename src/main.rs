mod app;
mod buffer;
mod dialog;
mod filer;
mod fonts;
mod icon;
mod markdown;
mod search;

use app::LzEditorApp;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

fn main() -> eframe::Result<()> {
    let initial_file = std::env::args_os().nth(1).map(PathBuf::from);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("lz-editor")
            .with_title("LZ Editor")
            .with_icon(Arc::new(icon::build_app_icon()))
            .with_inner_size([1024.0, 680.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "LZ Editor",
        native_options,
        Box::new(move |cc| Ok(Box::new(LzEditorApp::new(cc, initial_file)))),
    )
}
