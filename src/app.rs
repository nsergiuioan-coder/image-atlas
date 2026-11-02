use crate::{
    convert::{self, ColorRule},
    crawl, download,
    model::{Cancel, Event, SiteMap, UpdateRelease},
    update,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{self, Color32, RichText};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const INK: Color32 = Color32::from_rgb(228, 234, 240);
const ACCENT: Color32 = Color32::from_rgb(31, 124, 110);
const PAPER: Color32 = Color32::from_rgb(23, 28, 34);

struct MappingStatus {
    scanned: usize,
    queued: usize,
    images: usize,
    eta_seconds: Option<f64>,
    received: Instant,
    current_url: String,
}
impl MappingStatus {
    fn label(&self) -> String {
        let eta = match self.eta_seconds {
            Some(seconds) if seconds > self.received.elapsed().as_secs_f64() => {
                let left = (seconds - self.received.elapsed().as_secs_f64()).ceil() as u64;
                crate::tr!("ETA ~{}m {:02}s", left / 60, left % 60)
            }
            Some(_) if self.queued == 0 => crate::i18n::text("Finishing mapping…").into(),
            Some(_) => crate::i18n::text("Waiting for the server; recalculating ETA…").into(),
            None => crate::i18n::text("Measuring scan speed…").into(),
        };
        crate::tr!(
            "{} pages scanned · {} queued · {} images · {eta}",
            self.scanned,
            self.queued,
            self.images
        )
    }
}
struct MappingNotice {
    pages: usize,
    images: usize,
    notices: usize,
    mode: update::MappingNotification,
    created: Instant,
}

impl MappingNotice {
    fn show(&self, ctx: &egui::Context) -> bool {
        let summary = crate::tr!(
            "{} pages scanned · {} images found",
            self.pages,
            self.images
        );
        let warnings = self.notices;
        match self.mode {
            update::MappingNotification::Quick => {
                if self.created.elapsed() >= Duration::from_secs(5) {
                    return true;
                }
                let remaining = Duration::from_secs(5).saturating_sub(self.created.elapsed());
                egui::Area::new(egui::Id::unique("mapping-toast"))
                    .anchor(egui::Align2::RIGHT_BOTTOM, [-24.0, -72.0])
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .show(ctx, |ui| {
                        Atlas::card().show(ui, |ui| {
                            ui.strong(crate::i18n::text("Mapping completed"));
                            ui.label(&summary);
                            if warnings > 0 {
                                ui.label(crate::tr!("{warnings} notices in Details"));
                            }
                        });
                    });
                ctx.request_repaint_after(remaining);
                false
            }
            update::MappingNotification::Modal => {
                let mut close = false;
                egui::Modal::new(egui::Id::unique("mapping-completed"))
                    .frame(Atlas::card())
                    .show(ctx, |ui| {
                        ui.set_min_width(350.0);
                        ui.heading(crate::i18n::text("Mapping completed"));
                        ui.add_space(10.0);
                        ui.label(&summary);
                        if warnings > 0 {
                            ui.label(crate::tr!("{warnings} notices are available in Details."));
                        }
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            if Atlas::primary(ui, crate::i18n::text("OK")).clicked() {
                                close = true;
                            }
                        });
                    });
                close
            }
        }
    }
}

pub struct Atlas {
    mapping_status: Option<MappingStatus>,
    mapping_notice: Option<MappingNotice>,
    vector: crate::vector::Editor,
    tab: usize,
    url: String,
    render: bool,
    sitemap: bool,
    map: SiteMap,
    filter: String,
    edit_folders: bool,
    folder: PathBuf,
    bandwidth: u32,
    zip: bool,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    cancel: Cancel,
    busy: bool,
    status: String,
    logs: Vec<String>,
    progress: f32,
    files: Vec<(PathBuf, String)>,
    settings: convert::Settings,
    output: PathBuf,
    preview: Option<egui::TextureHandle>,
    pixels: Option<image::RgbaImage>,
    image_index: usize,
    preview_key: String,
    web_preview: Option<(String, crate::web::Encoded)>,
    original: bool,
    zoom: f32,
    pan: egui::Vec2,
    pick_rule: Option<usize>,
    region_rule: Option<usize>,
    preferences: update::Preferences,
    release: Option<UpdateRelease>,
    physical: f32,
    unit: usize,
}
impl Atlas {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark));
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        let preferences = update::Preferences::load();
        crate::i18n::set_language(&preferences.language);
        crate::i18n::configure_fonts(&cc.egui_ctx);
        let mut style = (*cc.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = PAPER;
        style.visuals.window_fill = Color32::from_rgb(31, 37, 45);
        style.visuals.override_text_color = Some(INK);
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(44, 53, 62);
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(58, 72, 82);
        style.spacing.item_spacing = egui::vec2(12.0, 10.0);
        style.spacing.button_padding = egui::vec2(16.0, 10.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(27.0));
        style.spacing.interact_size = egui::vec2(40.0, 32.0);
        for visuals in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            visuals.corner_radius = egui::CornerRadius::same(7);
            visuals.bg_stroke = egui::Stroke::NONE;
        }
        cc.egui_ctx.set_style_of(egui::Theme::Light, style.clone());
        cc.egui_ctx.set_style_of(egui::Theme::Dark, style);
        let (tx, rx) = unbounded();
        if !preferences.disable_updates {
            let repository = preferences.repository.clone();
            let tx = tx.clone();
            let skip = preferences.skip_version.clone();
            let repaint = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                if let Ok(Some(r)) = update::check(&repository)
                    && r.version != skip
                {
                    let _ = tx.send(Event::Update(r));
                    repaint.request_repaint();
                }
            });
        }
        let home =
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Downloads");
        let mut atlas = Self {
            mapping_status: None,
            mapping_notice: None,
            vector: Default::default(),
            tab: 0,
            url: String::new(),
            render: false,
            sitemap: true,
            map: SiteMap::default(),
            filter: String::new(),
            edit_folders: false,
            folder: home.join("Image Atlas"),
            bandwidth: 0,
            zip: false,
            tx,
            rx,
            cancel: Cancel::default(),
            busy: false,
            status: crate::i18n::text("Ready when you are").into(),
            logs: Vec::new(),
            progress: 0.0,
            files: Vec::new(),
            settings: Default::default(),
            output: home.join("Image Atlas edited"),
            preview: None,
            pixels: None,
            image_index: 0,
            preview_key: String::new(),
            web_preview: None,
            original: false,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            pick_rule: None,
            region_rule: None,
            preferences,
            release: None,
            physical: 0.0,
            unit: 0,
        };
        if let Some((options, map)) = crawl::pending_mapping() {
            atlas.url = options.website;
            atlas.render = options.render;
            atlas.sitemap = options.sitemap;
            atlas.map = map;
            atlas.map_website();
        } else if let Some(job) = download::pending_job() {
            atlas.map = job.map;
            atlas.folder = job.options.folder;
            atlas.bandwidth = job.options.bandwidth_mbps;
            atlas.zip = job.options.make_zip;
            atlas.tab = 1;
            atlas.status = crate::i18n::text(
                "An unfinished download is ready to resume. Completed files will be reused.",
            )
            .into();
        }
        let paths = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            atlas.import_files(paths);
            atlas.tab = 2;
        }
        atlas
    }
    fn start(&mut self) {
        self.mapping_status = None;
        self.mapping_notice = None;
        self.cancel = Cancel::default();
        self.busy = true;
        self.progress = 0.0;
    }
    fn error(&mut self, error: impl ToString) {
        self.status = error.to_string();
        self.logs.push(self.status.clone());
    }
    fn output_format_label(format: &str) -> &str {
        match format {
            "svg embedded" => crate::i18n::text("SVG — embedded image"),
            "svg trace" => crate::i18n::text("SVG — traced vectors"),
            _ => format,
        }
    }
    fn primary(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
        ui.add(
            egui::Button::new(RichText::new(text.into()).color(Color32::WHITE).strong())
                .fill(ACCENT)
                .min_size(egui::vec2(150.0, 44.0))
                .corner_radius(10),
        )
    }
    fn card() -> egui::Frame {
        egui::Frame::new()
            .fill(Color32::from_rgb(31, 37, 45))
            .corner_radius(16)
            .inner_margin(24)
            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(48, 57, 67)))
    }
    fn import_files(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            if path.is_dir() {
                let paths = walkdir::WalkDir::new(path)
                    .into_iter()
                    .filter_map(Result::ok)
                    .filter(|e| {
                        e.file_type().is_file()
                            && !e
                                .path()
                                .components()
                                .any(|c| c.as_os_str() == ".atlas-cache")
                    })
                    .map(|e| e.into_path())
                    .collect();
                self.import_files(paths);
            } else if !self.files.iter().any(|(p, _)| p == &path) {
                match convert::dimensions(&path) {
                    Ok((w, h)) => self.files.push((path, format!("{w} × {h} px"))),
                    Err(e) => self
                        .logs
                        .push(crate::tr!("Skipped {}: {e}", path.display())),
                }
            }
        }
        self.preview_key.clear();
    }
    fn map_website(&mut self) {
        self.start();
        let options = crawl::CrawlOptions {
            website: self.url.clone(),
            render: self.render,
            sitemap: self.sitemap,
        };
        let tx = self.tx.clone();
        let cancel = self.cancel.clone();
        std::thread::spawn(move || {
            let event = match crawl::map_site(options, cancel, tx.clone()) {
                Ok(map) => Event::Mapped(map),
                Err(e) => Event::Failed(e.to_string()),
            };
            let _ = tx.send(event);
        });
    }
    fn website(&mut self, ui: &mut egui::Ui) {
        ui.add_space(45.0);
        ui.label(
            RichText::new(crate::i18n::text("Website image library"))
                .size(36.0)
                .strong(),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(crate::i18n::text(
                "Find the originals. Review their folders. Save them together.",
            ))
            .size(18.0)
            .weak(),
        );
        ui.add_space(30.0);
        Self::card().show(ui, |ui| {
            ui.set_min_width((ui.available_width() - 48.0).max(300.0));
            ui.strong(crate::i18n::text("Website address"));
            ui.horizontal(|ui| {
                ui.add_sized(
                    [ui.available_width() - 180.0, 46.0],
                    egui::TextEdit::singleline(&mut self.url)
                        .hint_text("https://your-website.com")
                        .margin(egui::vec2(14.0, 13.0)),
                );
                ui.add_enabled_ui(!self.busy && !self.url.trim().is_empty(), |ui| {
                    if Self::primary(ui, crate::i18n::text("Find images")).clicked() {
                        if !self.url.contains("://") {
                            self.url = format!("https://{}", self.url.trim());
                        }
                        self.map_website();
                    }
                });
            });
            ui.add_space(6.0);
            ui.weak(crate::i18n::text(
                "You’ll review everything before downloading.",
            ));
            egui::CollapsingHeader::new(crate::i18n::text("Website options")).show(ui, |ui| {
                ui.checkbox(
                    &mut self.sitemap,
                    crate::i18n::text("Look through the sitemap"),
                );
                ui.checkbox(
                    &mut self.render,
                    crate::i18n::text("Use browser rendering for a dynamic website"),
                );
                ui.weak(crate::i18n::text(
                    "Browser rendering is slower and requires Edge or Chrome.",
                ));
            });
        });
        ui.add_space(28.0);
        ui.columns(3, |cols| {
            for (col, (title, text)) in cols.iter_mut().zip([
                (
                    crate::i18n::text("Original quality"),
                    crate::i18n::text("Files keep their real format, including SVG."),
                ),
                (
                    crate::i18n::text("Your folder structure"),
                    crate::i18n::text("Review and edit categories before saving."),
                ),
                (
                    crate::i18n::text("A considered pace"),
                    crate::i18n::text("Downloads adapt to your connection and the server."),
                ),
            ]) {
                col.strong(title);
                col.add_space(5.0);
                col.label(RichText::new(text).weak());
            }
        });
        ui.add_space(24.0);
        if ui
            .link(crate::i18n::text("Open a saved website map"))
            .clicked()
        {
            self.open_map();
        }
    }
    fn open_map(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter(crate::i18n::text("Website map"), &["json"])
            .pick_file()
        {
            match std::fs::read(p)
                .map_err(anyhow::Error::from)
                .and_then(|b| serde_json::from_slice(&b).map_err(Into::into))
            {
                Ok(map) => {
                    self.map = map;
                    self.tab = 4;
                }
                Err(e) => self.error(e),
            }
        }
    }
    fn review(&mut self, ui: &mut egui::Ui) {
        ui.heading(crate::i18n::text("Review your images"));
        ui.label(crate::i18n::text(
            "Choose the images to keep and where they belong.",
        ));
        ui.add_space(18.0);
        let count = self.map.assets.iter().filter(|a| a.selected).count();
        ui.horizontal(|ui| {
            ui.strong(crate::tr!("{count} selected"));
            ui.weak(crate::tr!(
                "from {} images · {} pages",
                self.map.assets.len(),
                self.map.pages
            ));
            ui.with_layout(crate::i18n::trailing_layout(egui::Align::Center), |ui| {
                if Self::primary(ui, crate::i18n::text("Continue to download")).clicked()
                    && count > 0
                {
                    self.tab = 1;
                }
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text(crate::i18n::text("Find an image or category"))
                    .desired_width(330.0)
                    .margin(egui::vec2(12.0, 9.0)),
            );
            ui.checkbox(&mut self.edit_folders, crate::i18n::text("Edit folders"));
            ui.menu_button(crate::i18n::text("Selection"), |ui| {
                if ui.button(crate::i18n::text("Select all")).clicked() {
                    for a in &mut self.map.assets {
                        a.selected = true;
                    }
                    ui.close();
                }
                if ui.button(crate::i18n::text("Deselect all")).clicked() {
                    for a in &mut self.map.assets {
                        a.selected = false;
                    }
                    ui.close();
                }
            });
            ui.menu_button(crate::i18n::text("Map"), |ui| {
                if ui.button(crate::i18n::text("Save map")).clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_file_name("website-map.json")
                        .save_file()
                    {
                        match serde_json::to_vec_pretty(&self.map)
                            .map_err(anyhow::Error::from)
                            .and_then(|b| std::fs::write(p, b).map_err(Into::into))
                        {
                            Ok(()) => self.status = crate::i18n::text("Map saved").into(),
                            Err(e) => self.error(e),
                        }
                    }
                    ui.close();
                }
                if ui.button(crate::i18n::text("Open map")).clicked() {
                    self.open_map();
                    ui.close();
                }
            });
        });
        ui.add_space(10.0);
        let filter = self.filter.to_lowercase();
        let visible: Vec<_> = self
            .map
            .assets
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                filter.is_empty()
                    || a.label.to_lowercase().contains(&filter)
                    || a.url.to_lowercase().contains(&filter)
                    || a.folders
                        .iter()
                        .any(|folder| folder.to_lowercase().contains(&filter))
            })
            .map(|(i, _)| i)
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("map")
            .max_height((ui.available_height() - 12.0).max(100.0))
            .show_rows(ui, 76.0, visible.len(), |ui, range| {
                for row in range {
                    let index = visible[row];
                    let a = &mut self.map.assets[index];
                    ui.push_id(index, |ui| {
                        egui::Frame::new()
                            .fill(Color32::from_rgb(31, 37, 45))
                            .corner_radius(10)
                            .inner_margin(12)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut a.selected, "");
                                    if self.edit_folders {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut a.label)
                                                .desired_width(240.0),
                                        );
                                        let mut path = a.folders.join(" / ");
                                        if ui
                                            .add(
                                                egui::TextEdit::singleline(&mut path)
                                                    .desired_width(360.0),
                                            )
                                            .changed()
                                        {
                                            a.folders = path
                                                .split('/')
                                                .map(str::trim)
                                                .filter(|s| !s.is_empty())
                                                .map(str::to_owned)
                                                .collect();
                                        }
                                    } else {
                                        ui.vertical(|ui| {
                                            ui.strong(&a.label);
                                            ui.label(
                                                RichText::new(a.folders.join(" / ")).small().weak(),
                                            );
                                        });
                                    }
                                    ui.with_layout(
                                        crate::i18n::trailing_layout(egui::Align::Center),
                                        |ui| {
                                            if !a.url.starts_with("data:") {
                                                ui.hyperlink_to(
                                                    crate::i18n::text("View source"),
                                                    &a.url,
                                                );
                                            } else {
                                                ui.weak(crate::i18n::text("Inline image"));
                                            }
                                        },
                                    );
                                });
                            });
                    });
                }
            });
        if self.map.assets.is_empty() {
            ui.add_space(50.0);
            ui.centered_and_justified(|ui| {
                ui.label(crate::i18n::text(
                    "No images found. Try browser rendering, or review the activity details.",
                ));
            });
        }
    }
    fn begin_download(&mut self) {
        self.start();
        let map = self.map.clone();
        let folder = self.folder.clone();
        let options = download::Options {
            folder: folder.clone(),
            bandwidth_mbps: self.bandwidth,
            make_zip: self.zip,
        };
        let tx = self.tx.clone();
        let cancel = self.cancel.clone();
        let stopped = cancel.clone();
        let collection_map = map.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|rt| {
                    rt.block_on(download::download_map(map, options, cancel, tx.clone()))
                });
            let _ = tx.send(match result {
                Ok(failures) if !stopped.stopped() => Event::DownloadFinished {
                    files: download::downloaded_files(&collection_map, &folder),
                    folder,
                    failures,
                },
                Ok(failures) => Event::Finished { folder, failures },
                Err(e) => Event::Failed(e.to_string()),
            });
        });
    }
    fn downloads(&mut self, ui: &mut egui::Ui) {
        ui.add_space(25.0);
        ui.heading(crate::i18n::text("Save your collection"));
        ui.label(crate::i18n::text(
            "Original files, organized exactly as you reviewed them.",
        ));
        ui.add_space(24.0);
        Self::card().show(ui, |ui| {
            ui.set_min_width((ui.available_width() - 48.0).max(300.0));
            ui.strong(crate::i18n::text("Save location"));
            ui.horizontal(|ui| {
                ui.label(self.folder.display().to_string());
                if ui.button(crate::i18n::text("Change")).clicked()
                    && let Some(p) = rfd::FileDialog::new().pick_folder()
                {
                    self.folder = p;
                }
            });
            ui.add_space(8.0);
            ui.checkbox(&mut self.zip, crate::i18n::text("Create a ZIP archive too"));
            egui::CollapsingHeader::new(crate::i18n::text("Connection preferences")).show(
                ui,
                |ui| {
                    ui.label(crate::i18n::text(
                        "Automatic works for most connections. Add a speed hint if you know it.",
                    ));
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::text("Connection speed"));
                        ui.add(egui::DragValue::new(&mut self.bandwidth).suffix(" Mbps"));
                        ui.weak(crate::i18n::text("0 = automatic"));
                    });
                },
            );
            ui.add_space(18.0);
            let count = self.map.assets.iter().filter(|a| a.selected).count();
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.busy && count > 0, |ui| {
                    if Self::primary(ui, crate::tr!("Download {count} images")).clicked() {
                        self.begin_download();
                    }
                });
                if ui.button(crate::i18n::text("Open folder")).clicked() {
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(&self.folder)
                        .spawn();
                }
                if !self.busy && ui.link(crate::i18n::text("Edit selection")).clicked() {
                    self.tab = 4;
                }
            });
        });
        if self.busy {
            ui.add_space(25.0);
            ui.add(
                egui::ProgressBar::new(self.progress)
                    .desired_height(12.0)
                    .fill(ACCENT),
            );
            ui.add_space(8.0);
            ui.label(&self.status);
        }
        ui.add_space(20.0);
        ui.weak(crate::i18n::text(
            "You can stop and resume later. Completed originals are reused automatically.",
        ));
    }
    fn edit_key(&self) -> String {
        format!(
            "{:?}:{}:{:?}:{:?}:{}:{}",
            self.files.get(self.image_index).map(|(p, _)| p),
            self.image_index,
            self.settings.rules,
            self.settings.web,
            self.settings.width,
            self.settings.height
        )
    }
    fn refresh_preview(&mut self, ctx: &egui::Context) {
        if self.files.is_empty() {
            self.preview = None;
            self.pixels = None;
            return;
        }
        self.image_index = self.image_index.min(self.files.len() - 1);
        let settings_key = self.edit_key();
        let key = format!("{}:{settings_key}", self.original);
        if self.preview_key == key {
            return;
        }
        let path = self.files[self.image_index].0.clone();
        let image = if !self.original
            && let Some((key, encoded)) = &self.web_preview
            && key == &settings_key
        {
            crate::web::preview(encoded, &path)
        } else {
            convert::load(&path).map(|image| {
                if self.original {
                    image
                } else {
                    convert::prepare(image, &self.settings)
                }
            })
        };
        match image {
            Ok(image) => {
                let rgba = image.thumbnail(1800, 1400).into_rgba8();
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                );
                self.preview =
                    Some(ctx.load_texture("batch-preview", color, egui::TextureOptions::LINEAR));
                self.pixels = Some(rgba);
                self.preview_key = key;
            }
            Err(e) => {
                self.error(e);
                self.preview_key = key;
                self.preview = None;
            }
        }
    }
    fn preview_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(
                    self.original,
                    RichText::new(crate::i18n::text("Original")).color(if self.original {
                        Color32::WHITE
                    } else {
                        INK
                    }),
                )
                .clicked()
            {
                self.original = true;
            }
            if ui
                .selectable_label(
                    !self.original,
                    RichText::new(crate::i18n::text("Edited")).color(if !self.original {
                        Color32::WHITE
                    } else {
                        INK
                    }),
                )
                .clicked()
            {
                self.original = false;
            }
            ui.with_layout(crate::i18n::trailing_layout(egui::Align::Center), |ui| {
                if ui.button(crate::i18n::text("Fit image")).clicked() {
                    self.zoom = 1.0;
                    self.pan = egui::Vec2::ZERO;
                }
                ui.weak(format!("{}%", (self.zoom * 100.0).round()));
            });
        });
        self.refresh_preview(ctx);
        let size = egui::vec2(
            ui.available_width(),
            (ui.available_height() - 100.0).max(280.0),
        );
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 14, Color32::from_rgb(14, 18, 23));
        if let Some(texture) = &self.preview {
            if response.dragged() && self.pick_rule.is_none() && self.region_rule.is_none() {
                self.pan += response.drag_delta();
            }
            if response.hovered() {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.1, 12.0);
                    ctx.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                }
            }
            let factor = (rect.width() / texture.size_vec2().x)
                .min(rect.height() / texture.size_vec2().y)
                * 0.92
                * self.zoom;
            let image_rect = egui::Rect::from_center_size(
                rect.center() + self.pan,
                texture.size_vec2() * factor,
            );
            painter.image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            if response.drag_stopped()
                && let Some(index) = self.region_rule
                && let Some(end) = response.interact_pointer_pos()
            {
                let start = end - response.drag_delta();
                let drag = ctx
                    .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("region-start")))
                    .unwrap_or(start);
                let a = (drag - image_rect.min) / image_rect.size();
                let b = (end - image_rect.min) / image_rect.size();
                if let Some(rule) = self.settings.rules.get_mut(index) {
                    rule.region = Some((
                        a.x.min(b.x).clamp(0.0, 1.0),
                        a.y.min(b.y).clamp(0.0, 1.0),
                        a.x.max(b.x).clamp(0.0, 1.0),
                        a.y.max(b.y).clamp(0.0, 1.0),
                    ));
                }
                self.region_rule = None;
            }
            if self.region_rule.is_some()
                && response.drag_started()
                && let Some(position) = response.interact_pointer_pos()
            {
                ctx.data_mut(|d| d.insert_temp(egui::Id::unique("region-start"), position));
            }
            if let Some(index) = self.region_rule
                && response.dragged()
                && let (Some(start), Some(end)) = (
                    ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("region-start"))),
                    response.interact_pointer_pos(),
                )
            {
                painter.rect_stroke(
                    egui::Rect::from_two_pos(start, end),
                    0,
                    egui::Stroke::new(1.5_f32, ACCENT),
                    egui::StrokeKind::Inside,
                );
                let _ = index;
            }
            for rule in &self.settings.rules {
                if let Some((x0, y0, x1, y1)) = rule.region {
                    let region = egui::Rect::from_min_max(
                        image_rect.min + image_rect.size() * egui::vec2(x0, y0),
                        image_rect.min + image_rect.size() * egui::vec2(x1, y1),
                    );
                    painter.rect_stroke(
                        region,
                        0,
                        egui::Stroke::new(1.0_f32, ACCENT),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            if self.pick_rule.is_some() {
                response
                    .clone()
                    .on_hover_cursor(egui::CursorIcon::Crosshair);
            }
            if response.clicked()
                && let (Some(position), Some(index), Some(pixels)) = (
                    response.interact_pointer_pos(),
                    self.pick_rule,
                    self.pixels.as_ref(),
                )
                && image_rect.contains(position)
            {
                let uv = (position - image_rect.min) / image_rect.size();
                let x = (uv.x * pixels.width() as f32)
                    .floor()
                    .clamp(0.0, (pixels.width() - 1) as f32) as u32;
                let y = (uv.y * pixels.height() as f32)
                    .floor()
                    .clamp(0.0, (pixels.height() - 1) as f32) as u32;
                if let Some(rule) = self.settings.rules.get_mut(index) {
                    let p = pixels.get_pixel(x, y);
                    rule.enabled = true;
                    rule.from = [p[0], p[1], p[2]];
                    if rule.seed.is_some() {
                        rule.seed = Some((uv.x, uv.y));
                    }
                }
                self.pick_rule = None;
                self.original = false;
            }
        } else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                crate::i18n::text("Drop your images here"),
                egui::FontId::proportional(22.0),
                Color32::from_gray(110),
            );
        }
        ui.add_space(8.0);
        if !self.files.is_empty() {
            let previous = self.image_index;
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.image_index > 0,
                        egui::Button::new(crate::i18n::text("Previous")),
                    )
                    .clicked()
                {
                    self.image_index -= 1;
                }
                let max = self.files.len() - 1;
                ui.add(egui::Slider::new(&mut self.image_index, 0..=max).show_value(false));
                if ui
                    .add_enabled(
                        self.image_index < max,
                        egui::Button::new(crate::i18n::text("Next")),
                    )
                    .clicked()
                {
                    self.image_index += 1;
                }
                ui.weak(format!("{} / {}", self.image_index + 1, self.files.len()));
            });
            if previous != self.image_index {
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
                ctx.request_repaint();
            }
            let (path, size) = &self.files[self.image_index];
            ui.label(
                RichText::new(format!(
                    "{}  ·  {size}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ))
                .small()
                .weak(),
            );
        }
        ui.label(
            RichText::new(if self.pick_rule.is_some() {
                crate::i18n::text("Click the color to replace in the image.")
            } else {
                crate::i18n::text("Scroll to zoom · Drag to pan · Originals are preserved")
            })
            .small()
            .weak(),
        );
    }
    fn export_batch(&mut self) {
        self.start();
        let paths = self.files.clone();
        let settings = self.settings.clone();
        let folder = self.output.clone();
        let tx = self.tx.clone();
        let cancel = self.cancel.clone();
        std::thread::spawn(move || {
            let mut failures = Vec::new();
            let total = paths.len();
            let common = crate::web::common_root(
                &paths
                    .iter()
                    .map(|(path, _)| path.clone())
                    .collect::<Vec<_>>(),
            );
            let mut optimized_files = 0;
            let mut before = 0;
            let mut after = 0;
            let started = std::time::Instant::now();
            for (index, (path, _)) in paths.iter().enumerate() {
                if cancel.stopped() {
                    break;
                }
                let destination = if settings.web.is_some() {
                    crate::web::destination(path, common.as_deref(), &folder)
                } else {
                    folder.clone()
                };
                match convert::export(path, &destination, &settings) {
                    Ok(output) => {
                        optimized_files += 1;
                        before += path.metadata().map(|m| m.len()).unwrap_or(0);
                        after += output.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                    Err(e) => {
                        failures.push(format!("{}: {e}", path.display()));
                        let _ = tx.send(Event::Log(failures.last().unwrap().clone()));
                    }
                }
                let done = index + 1;
                let _ = tx.send(Event::Progress {
                    done,
                    total,
                    bytes: 0,
                    active: 1,
                    speed: 0.0,
                    eta_seconds: Some(
                        started.elapsed().as_secs_f64() / done as f64 * (total - done) as f64,
                    ),
                });
            }
            let _ = std::fs::create_dir_all(&folder);
            let _ = std::fs::write(folder.join("conversion-errors.txt"), failures.join("\n"));
            let _ = tx.send(if settings.web.is_some() {
                Event::Optimized {
                    folder,
                    files: optimized_files,
                    failures: failures.len(),
                    before,
                    after,
                }
            } else {
                Event::Finished {
                    folder,
                    failures: failures.len(),
                }
            });
        });
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        let mut web_enabled = self.settings.web.is_some();
        if ui
            .checkbox(&mut web_enabled, crate::i18n::text("Web optimized export"))
            .changed()
        {
            self.settings.web = if web_enabled {
                Some(crate::web::Options::default())
            } else {
                None
            };
        }
        if let Some(options) = &mut self.settings.web {
            options.ui(ui);
            ui.separator();
        }
        if web_enabled {
            if ui
                .add_enabled(
                    !self.files.is_empty(),
                    egui::Button::new(crate::i18n::text("Preview optimized file")),
                )
                .clicked()
            {
                let path = self.files[self.image_index].0.clone();
                match convert::encode_web(&path, &self.settings) {
                    Ok(encoded) => {
                        self.web_preview = Some((self.edit_key(), encoded));
                        self.preview_key.clear();
                        self.original = false;
                    }
                    Err(e) => self.error(e),
                }
            }
            if let Some((key, encoded)) = &self.web_preview {
                if *key == self.edit_key() {
                    ui.label(crate::web::summary(
                        encoded.input_bytes,
                        encoded.bytes.len() as u64,
                    ));
                } else {
                    ui.weak(crate::i18n::text(
                        "Settings changed. Refresh the optimized preview.",
                    ));
                }
            }
        }
        if !web_enabled {
            ui.strong(crate::i18n::text("Save as"));
            egui::ComboBox::from_id_salt("format")
                .width(235.0)
                .selected_text(Self::output_format_label(&self.settings.format))
                .show_ui(ui, |ui| {
                    for format in [
                        "png",
                        "jpg",
                        "webp",
                        "tiff",
                        "bmp",
                        "gif",
                        "ico",
                        "qoi",
                        "tga",
                        "avif",
                        "pdf",
                        "svg embedded",
                        "svg trace",
                    ] {
                        ui.selectable_value(
                            &mut self.settings.format,
                            format.into(),
                            Self::output_format_label(format),
                        );
                    }
                });
            if self.settings.format == "svg embedded" {
                ui.weak(crate::i18n::text("A valid SVG containing pixels."));
            }
            if self.settings.format == "svg trace" {
                ui.weak(crate::i18n::text(
                    "Vector tracing works best on flat artwork.",
                ));
            }
        }
        ui.add_space(10.0);
        egui::CollapsingHeader::new(crate::i18n::text("Image size"))
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text("Width"));
                    ui.add(egui::DragValue::new(&mut self.settings.width).suffix(" px"));
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text("Height"));
                    ui.add(egui::DragValue::new(&mut self.settings.height).suffix(" px"));
                });
                ui.weak(crate::i18n::text(
                    "0 keeps the original. Set one to keep proportions.",
                ));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.physical).range(0.0..=100000.0));
                    egui::ComboBox::from_id_salt("units")
                        .selected_text(["mm", "cm", "in"][self.unit])
                        .show_ui(ui, |ui| {
                            for (i, s) in ["mm", "cm", "in"].iter().enumerate() {
                                ui.selectable_value(&mut self.unit, i, *s);
                            }
                        });
                });
                ui.add(
                    egui::DragValue::new(&mut self.settings.dpi)
                        .range(1.0..=2400.0)
                        .suffix(" DPI"),
                );
                if ui.button(crate::i18n::text("Use physical width")).clicked()
                    && self.physical > 0.0
                {
                    self.settings.width = (self.physical / [25.4, 2.54, 1.0][self.unit]
                        * self.settings.dpi)
                        .round()
                        .max(1.0) as u32;
                    self.settings.height = 0;
                }
            });
        ui.separator();
        ui.strong(crate::i18n::text("Color changes"));
        ui.weak(crate::i18n::text("Add one rule for each area or color."));
        let mut remove = None;
        for (i, rule) in self.settings.rules.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                egui::Frame::new()
                    .fill(PAPER)
                    .corner_radius(10)
                    .inner_margin(10)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut rule.enabled, crate::tr!("Change {}", i + 1));
                            if ui.small_button(crate::i18n::text("Remove")).clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("From"));
                            ui.color_edit_button_srgb(&mut rule.from);
                            ui.label(crate::i18n::text("To"));
                            ui.color_edit_button_srgb(&mut rule.to);
                        });
                        if ui
                            .button(if self.pick_rule == Some(i) {
                                crate::i18n::text("Click a color in the preview…")
                            } else {
                                crate::i18n::text("Pick from image")
                            })
                            .clicked()
                        {
                            self.pick_rule = Some(i);
                            self.original = true;
                        }
                        if ui
                            .button(crate::i18n::text("Restrict to a rectangle"))
                            .clicked()
                        {
                            self.region_rule = Some(i);
                            self.pick_rule = None;
                        }
                        if rule.region.is_some()
                            && ui
                                .small_button(crate::i18n::text("Clear area restriction"))
                                .clicked()
                        {
                            rule.region = None;
                        }
                        let mut scope = if rule.seed.is_some() {
                            2
                        } else if rule.edges_only {
                            0
                        } else {
                            1
                        };
                        egui::ComboBox::from_id_salt("scope")
                            .width(205.0)
                            .selected_text(
                                [
                                    crate::i18n::text("Background at edges"),
                                    crate::i18n::text("Matching colors everywhere"),
                                    crate::i18n::text("Connected area I click"),
                                ][scope],
                            )
                            .show_ui(ui, |ui| {
                                for (n, label) in [
                                    crate::i18n::text("Background at edges"),
                                    crate::i18n::text("Matching colors everywhere"),
                                    crate::i18n::text("Connected area I click"),
                                ]
                                .iter()
                                .enumerate()
                                {
                                    ui.selectable_value(&mut scope, n, *label);
                                }
                            });
                        rule.edges_only = scope == 0;
                        if scope == 2 {
                            if rule.seed.is_none() {
                                rule.seed = Some((0.5, 0.5));
                            }
                        } else {
                            rule.seed = None;
                        }
                        ui.add(
                            egui::Slider::new(&mut rule.tolerance, 0.0..=441.7)
                                .text(crate::i18n::text("Tolerance")),
                        );
                        egui::CollapsingHeader::new(crate::i18n::text("Soften color match")).show(
                            ui,
                            |ui| {
                                ui.add(
                                    egui::Slider::new(&mut rule.feather, 0.0..=80.0)
                                        .text(crate::i18n::text("Feather")),
                                );
                            },
                        );
                    });
            });
        }
        if let Some(i) = remove {
            self.settings.rules.remove(i);
            self.pick_rule = None;
        }
        if ui.button(crate::i18n::text("Add color change")).clicked() {
            self.settings.rules.push(ColorRule {
                enabled: false,
                edges_only: false,
                seed: Some((0.5, 0.5)),
                tolerance: 8.0,
                ..Default::default()
            });
        }
        ui.separator();
        ui.strong(crate::i18n::text("Destination"));
        ui.label(
            RichText::new(
                self.output
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            )
            .small()
            .weak(),
        )
        .on_hover_text(self.output.display().to_string());
        if ui.button(crate::i18n::text("Choose folder")).clicked()
            && let Some(p) = rfd::FileDialog::new().pick_folder()
        {
            self.output = p;
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(!self.busy && !self.files.is_empty(), |ui| {
            if Self::primary(
                ui,
                crate::tr!(
                    "Export {} {}",
                    self.files.len(),
                    if self.files.len() == 1 {
                        "image"
                    } else {
                        "images"
                    }
                ),
            )
            .clicked()
            {
                self.export_batch();
            }
        });
    }
    fn batch(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading(crate::i18n::text("  Batch editor"));
            ui.with_layout(crate::i18n::trailing_layout(egui::Align::Center), |ui| {
                if ui.button(crate::i18n::text("Add images")).clicked()
                    && let Some(paths) = rfd::FileDialog::new().pick_files()
                {
                    self.import_files(paths);
                }
                if ui.button(crate::i18n::text("Add folder")).clicked()
                    && let Some(p) = rfd::FileDialog::new().pick_folder()
                {
                    self.import_files(vec![p]);
                }
                ui.menu_button(crate::i18n::text("Batch"), |ui| {
                    ui.label(crate::tr!("{} images", self.files.len()));
                    if ui.button(crate::i18n::text("Clear batch")).clicked() {
                        self.files.clear();
                        self.preview = None;
                        self.preview_key.clear();
                        ui.close();
                    }
                });
            });
        });
        ui.label(crate::i18n::text(
            "Preview your changes, then apply them to the whole batch.",
        ));
        ui.add_space(18.0);
        let height = ui.available_height();
        ui.with_layout(crate::i18n::trailing_layout(egui::Align::Min), |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(278.0, height),
                egui::Layout::top_down(crate::i18n::text_alignment()),
                |ui| {
                    Self::card().inner_margin(18).show(ui, |ui| {
                        ui.set_width(242.0);
                        egui::ScrollArea::vertical()
                            .id_salt("inspector")
                            .max_height(height - 40.0)
                            .show(ui, |ui| self.inspector(ui));
                    });
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| self.preview_canvas(ui, ctx),
            );
        });
    }
    fn show_mapping_notice(&mut self, ctx: &egui::Context) {
        if self
            .mapping_notice
            .as_ref()
            .is_some_and(|notice| notice.show(ctx))
        {
            self.mapping_notice = None;
        }
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(crate::i18n::text("Preferences"));
        ui.add_space(20.0);
        Self::card().show(ui, |ui| {
            ui.strong(crate::i18n::text("Interface language"));
            let old = self.preferences.language.clone();
            let selected = crate::i18n::languages()
                .find(|(code, _)| *code == self.preferences.language)
                .map(|(_, name)| name)
                .unwrap_or(crate::i18n::text("Follow Windows language"));
            egui::ComboBox::from_id_salt("interface-language")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.preferences.language,
                        String::new(),
                        crate::i18n::text("Follow Windows language"),
                    );
                    for (code, name) in crate::i18n::languages() {
                        ui.selectable_value(&mut self.preferences.language, code.to_owned(), name);
                    }
                });
            if old != self.preferences.language {
                crate::i18n::set_language(&self.preferences.language);
                crate::i18n::configure_fonts(ui.ctx());
                if let Err(error) = self.preferences.save() {
                    self.error(error);
                }
            }
            ui.add_space(16.0);
            ui.separator();
            ui.strong(crate::i18n::text("Mapping completion"));
            let old = self.preferences.mapping_notification;
            ui.radio_value(
                &mut self.preferences.mapping_notification,
                update::MappingNotification::Quick,
                crate::i18n::text("Quick popup — disappears after a few seconds"),
            );
            ui.radio_value(
                &mut self.preferences.mapping_notification,
                update::MappingNotification::Modal,
                crate::i18n::text("Blocking popup — requires OK to continue"),
            );
            if old != self.preferences.mapping_notification
                && let Err(e) = self.preferences.save()
            {
                self.error(e);
            }
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.strong(crate::i18n::text("App updates"));
            ui.checkbox(
                &mut self.preferences.disable_updates,
                crate::i18n::text("Turn off automatic update checks"),
            );
            egui::CollapsingHeader::new(crate::i18n::text("Release source")).show(ui, |ui| {
                ui.text_edit_singleline(&mut self.preferences.repository);
            });
            ui.horizontal(|ui| {
                if Self::primary(ui, crate::i18n::text("Save preferences")).clicked() {
                    match self.preferences.save() {
                        Ok(()) => self.status = crate::i18n::text("Preferences saved").into(),
                        Err(e) => self.error(e),
                    }
                }
                if ui.button(crate::i18n::text("Check for updates")).clicked() {
                    let repo = self.preferences.repository.clone();
                    let tx = self.tx.clone();
                    let repaint = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let event = match update::check(&repo) {
                            Ok(Some(r)) => Event::Update(r),
                            Ok(None) => Event::Log(
                                crate::i18n::text(
                                    "You're up to date. No newer release is available.",
                                )
                                .into(),
                            ),
                            Err(e) => Event::Log(e.to_string()),
                        };
                        let _ = tx.send(event);
                        repaint.request_repaint();
                    });
                }
            });
        });
        ui.add_space(24.0);
        ui.weak(crate::tr!(
            "Image Atlas {} · NIS · MIT",
            env!("CARGO_PKG_VERSION")
        ));
    }
}
impl eframe::App for Atlas {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Log(s) => {
                    self.status = s.clone();
                    self.logs.push(s);
                    if self.logs.len() > 500 {
                        self.logs.remove(0);
                    }
                }
                Event::MappingProgress {
                    scanned,
                    queued,
                    images,
                    eta_seconds,
                    current_url,
                } => {
                    self.mapping_status = Some(MappingStatus {
                        scanned,
                        queued,
                        images,
                        eta_seconds,
                        received: Instant::now(),
                        current_url,
                    });
                    self.progress = scanned as f32 / (scanned + queued).max(1) as f32;
                }
                Event::Mapped(map) => {
                    self.busy = false;
                    self.mapping_status = None;
                    if !self.cancel.stopped() {
                        self.mapping_notice = Some(MappingNotice {
                            pages: map.pages,
                            images: map.assets.len(),
                            notices: map.notices.len(),
                            mode: self.preferences.mapping_notification,
                            created: Instant::now(),
                        });
                    }
                    self.status = crate::tr!(
                        "Found {} images across {} pages",
                        map.assets.len(),
                        map.pages
                    );
                    self.logs.extend(map.notices.clone());
                    self.map = map;
                    self.tab = 4;
                }
                Event::Progress {
                    done,
                    total,
                    bytes,
                    active,
                    speed,
                    eta_seconds,
                } => {
                    self.progress = done as f32 / total.max(1) as f32;
                    let eta = eta_seconds
                        .map(|s| crate::tr!("About {}m {:02}s left", s as u64 / 60, s as u64 % 60))
                        .unwrap_or(crate::i18n::text("Estimating time remaining…").into());
                    self.status = crate::tr!("{done} of {total} · {:.1} MB/s · {eta}", speed / 1e6);
                    self.logs
                        .push(format!("{done}/{total}, {bytes} bytes, {active} active"));
                    if self.logs.len() > 500 {
                        self.logs.remove(0);
                    }
                }
                Event::DownloadFinished {
                    folder,
                    files,
                    failures,
                } => {
                    self.busy = false;
                    let count = files.len();
                    self.status = crate::tr!(
                        "Downloaded {count} files · {failures} errors · {}",
                        folder.display()
                    );
                    if !files.is_empty() {
                        self.import_files(files.clone());
                        if let Err(e) = self.vector.add_files(files) {
                            self.error(crate::tr!("Files are downloaded; editor preview: {e}"));
                        }
                        self.tab = 5;
                    }
                }
                Event::Optimized {
                    folder,
                    files,
                    failures,
                    before,
                    after,
                } => {
                    self.busy = false;
                    self.status = crate::tr!(
                        "Saved {files} files · {failures} errors · {} · {}",
                        crate::web::summary(before, after),
                        folder.display()
                    );
                }
                Event::Finished { folder, failures } => {
                    self.busy = false;
                    self.status = crate::tr!(
                        "{} · {failures} errors · {}",
                        if self.cancel.stopped() {
                            crate::i18n::text("Stopped. Completed files are safe")
                        } else {
                            crate::i18n::text("All finished")
                        },
                        folder.display()
                    );
                }
                Event::Failed(s) => {
                    self.busy = false;
                    self.mapping_status = None;
                    self.error(s);
                }
                Event::Update(r) => self.release = Some(r),
            }
        }
        if let Some(mapping) = &self.mapping_status {
            self.status = mapping.label();
        }
        let mapping_blocked = self
            .mapping_notice
            .as_ref()
            .is_some_and(|n| n.mode == update::MappingNotification::Modal);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if !dropped.is_empty() && !mapping_blocked {
            self.import_files(dropped);
            self.tab = 2;
        }
        let direction = crate::i18n::text_alignment();
        root.with_layout(egui::Layout::top_down(direction), |root| {
        egui::Panel::top("app-bar")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(29, 34, 40))
                    .inner_margin(egui::Margin::symmetric(28, 18)),
            )
            .show(root, |ui| {
                if mapping_blocked {
                    ui.disable();
                }
                ui.visuals_mut().override_text_color = Some(Color32::from_rgb(238, 242, 245));
                ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::from_rgb(44, 50, 58);
                ui.visuals_mut().widgets.inactive.fg_stroke.color =
                    Color32::from_rgb(225, 230, 235);
                ui.visuals_mut().widgets.hovered.weak_bg_fill = Color32::from_rgb(57, 65, 73);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Image Atlas")
                            .size(24.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(35.0);
                    if ui
                        .selectable_label(
                            matches!(self.tab, 0 | 1 | 4),
                            crate::i18n::text("  Website images"),
                        )
                        .clicked()
                    {
                        self.tab = if self.map.assets.is_empty() { 0 } else { 4 };
                    }
                    if ui
                        .selectable_label(self.tab == 5, crate::i18n::text("  Vector editor"))
                        .clicked()
                    {
                        self.tab = 5;
                    }
                    if ui
                        .selectable_label(self.tab == 2, crate::i18n::text("  Batch editor"))
                        .clicked()
                    {
                        self.tab = 2;
                    }
                    ui.with_layout(crate::i18n::trailing_layout(egui::Align::Center), |ui| {
                        if ui.button(crate::i18n::text("  Preferences")).clicked() {
                            self.tab = 3;
                        }
                    });
                });
            });
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(31, 37, 45))
                    .inner_margin(egui::Margin::symmetric(28, 10)),
            )
            .show(root, |ui| {
                if mapping_blocked {
                    ui.disable();
                }
                ui.horizontal(|ui| {
                    if self.busy {
                        ui.spinner();
                        if ui.small_button(crate::i18n::text("Stop")).clicked() {
                            self.cancel.stop();
                            self.status = "Stopping…".into();
                        }
                    }
                    ui.add(egui::Label::new(RichText::new(&self.status).small()).truncate());
                    ui.with_layout(crate::i18n::trailing_layout(egui::Align::Center), |ui| {
                        ui.menu_button(crate::i18n::text("Details"), |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(220.0)
                                .max_width(700.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    for line in &self.logs {
                                        ui.label(RichText::new(line).small());
                                    }
                                });
                        });
                    });
                });
                if self.busy {
                    ui.add(
                        egui::ProgressBar::new(self.progress)
                            .desired_height(4.0)
                            .fill(ACCENT),
                    );
                }
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAPER)
                    .inner_margin(egui::Margin::symmetric(30, 22)),
            )
            .show(root, |ui| {
                if mapping_blocked{ui.disable();}
                if matches!(self.tab, 0 | 1 | 4) {
                    ui.horizontal(|ui| {
                        for (tab, label) in [(0, crate::i18n::text("Website")), (4, crate::i18n::text("Review images")), (1, crate::i18n::text("Download"))]
                        {
                            let enabled = tab == 0 || !self.map.assets.is_empty();
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(RichText::new(label).color(
                                        if self.tab == tab {
                                            ACCENT
                                        } else {
                                            Color32::from_gray(120)
                                        },
                                    ))
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.tab = tab;
                            }
                            if tab != 1 {
                                ui.label(RichText::new("/").weak());
                            }
                        }
                    });
                    ui.add_space(12.0);
                }
                if let Some(mapping)=&self.mapping_status{
                    Self::card().inner_margin(16).show(ui,|ui|{
                        ui.strong(crate::i18n::text("Mapping your website"));
                        ui.label(mapping.label());
                        ui.add(egui::ProgressBar::new(self.progress).desired_height(6.0).fill(ACCENT));
                        ui.add(egui::Label::new(RichText::new(&mapping.current_url).small().weak()).truncate());
                        ui.weak(crate::i18n::text("Based on measured page response times and the known queue. The estimate updates as more pages are discovered."));
                    });ui.add_space(14.0);
                }
                if self.mapping_status.is_some() && matches!(self.tab, 0 | 1 | 4) {
                    ui.weak(crate::i18n::text("Your image list will appear when mapping finishes."));
                } else {
                ui.add_enabled_ui((!self.busy || self.tab == 3) && !mapping_blocked, |ui| match self.tab {
                    0 => {
                        egui::ScrollArea::vertical().show(ui, |ui| self.website(ui));
                    }
                    1 => {
                        egui::ScrollArea::vertical().show(ui, |ui| self.downloads(ui));
                    }
                    2 => self.batch(ui, ctx),
                    5 => self.vector.ui(ui, ctx),
                    4 => self.review(ui),
                    _ => self.settings_ui(ui),
                });
                }
            });
        });
        if !mapping_blocked && let Some(release) = self.release.clone() {
            egui::Window::new(crate::i18n::text("A fresh version is ready"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading(crate::tr!("Image Atlas {}", release.version));
                    ui.label(&release.notes);
                    ui.horizontal(|ui| {
                        if Self::primary(ui, crate::i18n::text("Update now")).clicked() {
                            let tx = self.tx.clone();
                            let r = release.clone();
                            let repaint = ui.ctx().clone();
                            std::thread::spawn(move || {
                                let event = match update::install(&r) {
                                    Ok(()) => Event::Log(
                                        crate::i18n::text("Installer opened. Close Image Atlas to finish updating.")
                                            .into(),
                                    ),
                                    Err(e) => Event::Failed(e.to_string()),
                                };
                                let _ = tx.send(event);
                                repaint.request_repaint();
                            });
                            self.release = None;
                        }
                        if ui.button(crate::i18n::text("Later")).clicked() {
                            self.release = None;
                        }
                        if ui.button(crate::i18n::text("Skip this version")).clicked() {
                            self.preferences.skip_version = release.version.clone();
                            if let Err(e) = self.preferences.save() {
                                self.error(e);
                            }
                            self.release = None;
                        }
                    });
                });
        }
        self.show_mapping_notice(ctx);
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(if self.mapping_status.is_some() {
                250
            } else {
                100
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draw_notice(
        ctx: &egui::Context,
        notice: &MappingNotice,
        events: Vec<egui::Event>,
    ) -> (bool, egui::FullOutput) {
        let mut dismissed = false;
        let output = ctx.run_pass(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                dismissed = notice.show(ctx);
            },
        );
        (dismissed, output)
    }
    #[test]
    #[ignore = "Manual isolated full-window rendering"]
    fn render_translated_windows() {
        let folder = PathBuf::from(
            std::env::var_os("IMAGE_ATLAS_RENDER_FOLDER").expect("Set IMAGE_ATLAS_RENDER_FOLDER"),
        );
        assert!(std::env::var_os("IMAGE_ATLAS_ISOLATED_TEST").is_some());
        std::fs::create_dir_all(&folder).unwrap();
        for language in ["it", "ar", "hi", "th", "he"] {
            let mut harness = egui_kittest::Harness::builder()
                .with_size([1180.0, 800.0])
                .with_accessibility_check(false)
                .build_eframe(|cc| Atlas::new(cc));
            for tab in [0, 3] {
                let atlas = harness.state_mut();
                atlas.preferences.language = language.into();
                atlas.tab = tab;
                crate::i18n::set_language(language);
                crate::i18n::configure_fonts(&harness.ctx);
                harness.run_steps(2);
                harness
                    .render()
                    .unwrap()
                    .save(folder.join(format!("window-{language}-{tab}.png")))
                    .unwrap();
            }
        }
    }
    fn click(position: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]
    }
    #[test]
    fn blocking_mapping_notice_waits_for_its_ok_button() {
        let ctx = egui::Context::default();
        let notice = MappingNotice {
            pages: 12,
            images: 30,
            notices: 0,
            mode: update::MappingNotification::Modal,
            created: Instant::now(),
        };
        assert!(!draw_notice(&ctx, &notice, vec![]).0);
        let (_, output) = draw_notice(&ctx, &notice, vec![]);
        assert!(ctx.memory(|m| m.top_modal_layer().is_some()));
        assert!(!draw_notice(&ctx, &notice, click(egui::pos2(5.0, 5.0))).0);
        fn ok_position(shape: &egui::epaint::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == "OK" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(ok_position),
                _ => None,
            }
        }
        let position = output
            .shapes
            .iter()
            .find_map(|shape| ok_position(&shape.shape))
            .expect("The completion popup must have a visible OK button");
        assert!(draw_notice(&ctx, &notice, click(position)).0);
    }
    #[test]
    fn quick_mapping_notice_expires_without_blocking() {
        let ctx = egui::Context::default();
        let mut notice = MappingNotice {
            pages: 2,
            images: 3,
            notices: 0,
            mode: update::MappingNotification::Quick,
            created: Instant::now(),
        };
        assert!(!draw_notice(&ctx, &notice, vec![]).0);
        assert!(ctx.memory(|m| m.top_modal_layer().is_none()));
        notice.created = Instant::now() - Duration::from_secs(6);
        assert!(draw_notice(&ctx, &notice, vec![]).0);
    }
}
