#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod bootstrap;
mod convert;
mod crawl;
mod download;
mod i18n;
mod model;
#[cfg(test)]
mod profiling;
mod progress;
mod recovery;
mod update;
mod vector;
mod web;

fn main() -> eframe::Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if let Some(index) = args.iter().position(|value| value == "--apply-update") {
        if let Some(path) = args.get(index + 1)
            && let Err(error) = update::apply(std::path::Path::new(path))
        {
            record_error(&error.to_string());
            rfd::MessageDialog::new()
                .set_title("Image Atlas update")
                .set_description(error.to_string())
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
        return Ok(());
    }
    let standalone_update = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
        })
        .is_some_and(|name| name.starts_with("imageatlas-update"));
    if !standalone_update && !std::env::args_os().any(|arg| arg == "--ui-session") {
        recovery::supervise();
        return Ok(());
    }
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(10));
        update::cleanup_completed();
    });
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
            ..eframe::wgpu::InstanceDescriptor::new_without_display_handle()
        },
        power_preference: eframe::wgpu::PowerPreference::LowPower,
        ..eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle()
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
            surface: eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY,
            ..Default::default()
        },
        viewport: eframe::egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_app_id(if standalone_update {
                "com.nis.imageatlas.update"
            } else {
                "com.nis.imageatlas"
            })
            .with_inner_size(if standalone_update {
                [560.0, 380.0]
            } else {
                [1180.0, 800.0]
            })
            .with_min_inner_size(if standalone_update {
                [480.0, 340.0]
            } else {
                [900.0, 650.0]
            }),
        ..Default::default()
    };
    eframe::run_native(
        "Image Atlas",
        options,
        Box::new(move |cc| {
            let app: Box<dyn eframe::App> = if standalone_update {
                Box::new(bootstrap::Updater::new(cc))
            } else {
                Box::new(app::Atlas::new(cc))
            };
            Ok(app)
        }),
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
