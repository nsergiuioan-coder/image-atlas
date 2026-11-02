#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod convert;
mod crawl;
mod download;
mod model;
mod update;
mod vector;
mod web;

fn main() -> eframe::Result<()> {
    let icon = image::load_from_memory(include_bytes!("../assets/app.ico"))
        .expect("Bundled icon is valid")
        .into_rgba8();
    let icon = eframe::egui::IconData {
        width: icon.width(),
        height: icon.height(),
        rgba: icon.into_raw(),
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_app_id("com.nis.imageatlas")
            .with_inner_size([1180.0, 800.0])
            .with_min_inner_size([900.0, 650.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Image Atlas",
        options,
        Box::new(|cc| Ok(Box::new(app::Atlas::new(cc)))),
    )
}
