#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod convert;
mod crawl;
mod download;
mod i18n;
mod model;
mod recovery;
mod update;
mod vector;
mod web;

fn main() -> eframe::Result<()> {
    if !std::env::args_os().any(|arg| arg == "--ui-session") {
        recovery::supervise();
        return Ok(());
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record_error(&info.to_string());
        default_hook(info);
    }));
    let icon = image::load_from_memory(include_bytes!("../assets/app.ico"))
        .expect("Bundled icon is valid")
        .into_rgba8();
    let icon = eframe::egui::IconData {
        width: icon.width(),
        height: icon.height(),
        rgba: icon.into_raw(),
    };
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew {
        instance_descriptor: eframe::wgpu::InstanceDescriptor {
            backends: if cfg!(windows) {
                eframe::wgpu::Backends::DX12
            } else {
                eframe::wgpu::Backends::PRIMARY
            },
            ..Default::default()
        },
        power_preference: eframe::wgpu::PowerPreference::LowPower,
        ..Default::default()
    };
    let default_descriptor = setup.device_descriptor.clone();
    setup.device_descriptor = std::sync::Arc::new(move |adapter| {
        let mut descriptor = default_descriptor(adapter);
        descriptor.memory_hints = eframe::wgpu::MemoryHints::MemoryUsage;
        descriptor
    });
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: setup.into(),
            desired_maximum_frame_latency: Some(1),
            ..Default::default()
        },
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
    .inspect_err(|error| record_error(&error.to_string()))
}

fn record_error(message: &str) {
    let folder = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
        .join("ImageAtlas");
    if std::fs::create_dir_all(&folder).is_ok() {
        let _ = std::fs::write(folder.join("last-error.txt"), message);
    }
}
