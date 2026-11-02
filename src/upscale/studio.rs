use super::*;
use eframe::egui;

#[derive(Default)]
pub struct Studio {
    open: bool,
    sources: Vec<Source>,
    settings: Option<crate::convert::Settings>,
    index: usize,
    options: Options,
    job: Option<crossbeam_channel::Receiver<Event>>,
    cancel: Cancel,
    status: String,
    progress: Option<(usize, usize, f64, Option<f64>, Instant)>,
    original: Option<egui::TextureHandle>,
    enhanced: Option<egui::TextureHandle>,
    split: f32,
    zoom: f32,
    folder: Option<PathBuf>,
    failed: Vec<Source>,
}

impl Studio {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn open(
        &mut self,
        sources: Vec<Source>,
        index: usize,
        settings: Option<crate::convert::Settings>,
    ) {
        self.open = true;
        if self.busy() {
            return;
        }
        self.sources = sources;
        self.settings = settings.filter(|settings| {
            settings.template.is_some()
                || settings.width != 0
                || settings.height != 0
                || settings.rules.iter().any(|rule| rule.enabled)
        });
        self.index = index.min(self.sources.len().saturating_sub(1));
        self.original = None;
        self.enhanced = None;
        self.split = 0.5;
        self.zoom = 1.0;
        self.failed.clear();
        self.progress = None;
        self.status.clear();
    }

    fn preview(&mut self, ctx: &egui::Context) {
        let Some(source) = self.sources.get(self.index).cloned() else {
            return;
        };
        self.cancel = Cancel::default();
        let cancel = self.cancel.clone();
        let options = self.options;
        let settings = self.settings.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        self.job = Some(rx);
        self.progress = None;
        self.status.clear();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<_> {
                let root = if options.mode == Mode::Photo
                    && (settings.is_some() || vector(&source)?.is_none())
                {
                    Some(engine::install(&cancel, |bytes, total| {
                        let _ = tx.send(Event::Download(bytes, total));
                    })?)
                } else {
                    None
                };
                let original = input(&source, settings.as_ref())?.thumbnail(1800, 1400);
                let encoded = enhance(
                    &source,
                    options,
                    root.as_deref(),
                    &cancel,
                    settings.as_ref(),
                )?;
                let result = crate::web::preview(&encoded, &source.path)?.thumbnail(1800, 1400);
                Ok(Event::Preview(original, result))
            })();
            let _ = tx.send(result.unwrap_or_else(|error| Event::Failed(error.to_string())));
            ctx.request_repaint();
        });
    }

    fn export(&mut self, sources: Vec<Source>, folder: PathBuf, ctx: &egui::Context) {
        self.cancel = Cancel::default();
        let cancel = self.cancel.clone();
        let options = self.options;
        let settings = self.settings.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        self.job = Some(rx);
        self.progress = None;
        self.status.clear();
        self.folder = Some(folder.clone());
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Err(error) =
                super::export(sources, folder, options, cancel, tx.clone(), settings)
            {
                let _ = tx.send(Event::Failed(error.to_string()));
            }
            ctx.request_repaint();
        });
    }

    fn texture(ctx: &egui::Context, label: &str, image: DynamicImage) -> egui::TextureHandle {
        let rgba = image.into_rgba8();
        ctx.load_texture(
            label,
            egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        )
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.job.clone() {
            while let Ok(event) = rx.try_recv() {
                match event {
                    Event::Download(bytes, total) => {
                        self.status = format!(
                            "{} · {:.1} / {:.1} MB",
                            crate::i18n::text("Download"),
                            bytes as f64 / 1e6,
                            total.unwrap_or(45_474_481) as f64 / 1e6
                        );
                    }
                    Event::Progress(done, total, rate, eta) => {
                        self.progress = Some((done, total, rate, eta, Instant::now()));
                    }
                    Event::Preview(original, enhanced) => {
                        self.original = Some(Self::texture(ctx, "upscale-original", original));
                        self.enhanced = Some(Self::texture(ctx, "upscale-enhanced", enhanced));
                        self.job = None;
                        self.status.clear();
                    }
                    Event::Completed {
                        folder,
                        files,
                        failed,
                    } => {
                        let failures = failed.len();
                        self.status = crate::tr!(
                            "Saved {files} files · {failures} errors · {}",
                            folder.display(),
                        );
                        self.failed = failed;
                        self.folder = Some(folder);
                        self.job = None;
                        self.progress = None;
                    }
                    Event::Failed(error) => {
                        self.status = error;
                        self.job = None;
                        self.progress = None;
                    }
                }
            }
        }
        if self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            if let Some((done, total, rate, eta, received)) = self.progress {
                self.status = crate::progress::label(done, total, rate, eta, received.elapsed());
            }
        }
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new(crate::i18n::text("Upscale images"))
            .id(egui::Id::unique(self as *const Self as usize))
            .open(&mut open)
            .default_size(egui::vec2(900.0, 660.0))
            .min_size(egui::vec2(620.0, 460.0))
            .show(ctx, |ui| {
                let old_options = self.options;
                ui.add_enabled_ui(!self.busy(), |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.options.mode,
                            Mode::Photo,
                            crate::i18n::text("Photos (AI)"),
                        );
                        ui.selectable_value(
                            &mut self.options.mode,
                            Mode::Artwork,
                            crate::i18n::text("Signs and logos (SVG)"),
                        );
                        if self.options.mode == Mode::Photo {
                            ui.separator();
                            ui.selectable_value(&mut self.options.scale, 2, "2×");
                            ui.selectable_value(&mut self.options.scale, 4, "4×");
                        }
                    });
                    if self.options.mode == Mode::Photo {
                        ui.weak("Real-ESRGAN · Vulkan GPU · Xintao Wang / nihui");
                        if !engine::ready() {
                            ui.weak(crate::i18n::text("First use downloads a 46 MB AI engine."));
                        }
                    } else {
                        ui.weak(crate::i18n::text(
                            "Vector tracing works best on flat artwork.",
                        ));
                    }
                    let old_index = self.index;
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.index > 0, egui::Button::new("‹"))
                            .clicked()
                        {
                            self.index -= 1;
                        }
                        let last = self.sources.len().saturating_sub(1);
                        ui.add(egui::Slider::new(&mut self.index, 0..=last).show_value(false));
                        if ui
                            .add_enabled(self.index < last, egui::Button::new("›"))
                            .clicked()
                        {
                            self.index += 1;
                        }
                        ui.label(format!("{} / {}", self.index + 1, self.sources.len()));
                        if ui.button(crate::i18n::text("Preview")).clicked() {
                            self.preview(ctx);
                        }
                    });
                    if self.index != old_index || self.options != old_options {
                        self.original = None;
                        self.enhanced = None;
                    }
                });
                if let Some(source) = self.sources.get(self.index) {
                    ui.label(source.path.display().to_string());
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text("Original"));
                    ui.add(egui::Slider::new(&mut self.split, 0.0..=1.0).show_value(false));
                    ui.label(crate::i18n::text("Edited"));
                    if ui.button(crate::i18n::text("Fit image")).clicked() {
                        self.zoom = 1.0;
                    }
                    ui.add(egui::Slider::new(&mut self.zoom, 1.0..=4.0).text("×"));
                });
                egui::ScrollArea::both()
                    .id_salt("upscale-preview")
                    .max_height(420.0)
                    .show(ui, |ui| {
                        if let (Some(original), Some(enhanced)) = (&self.original, &self.enhanced) {
                            let fit = (ui.available_width() / enhanced.size_vec2().x)
                                .min(360.0 / enhanced.size_vec2().y)
                                .min(1.0);
                            let size = enhanced.size_vec2() * fit * self.zoom;
                            let (rect, response) =
                                ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                            if response.dragged()
                                && let Some(pointer) = response.interact_pointer_pos()
                            {
                                self.split =
                                    ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                            }
                            let uv =
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                            ui.painter()
                                .image(enhanced.id(), rect, uv, egui::Color32::WHITE);
                            let divider = rect.left() + rect.width() * self.split;
                            let clip = egui::Rect::from_min_max(
                                rect.min,
                                egui::pos2(divider, rect.bottom()),
                            );
                            ui.painter()
                                .with_clip_rect(clip.intersect(ui.clip_rect()))
                                .image(original.id(), rect, uv, egui::Color32::WHITE);
                            ui.painter().line_segment(
                                [
                                    egui::pos2(divider, rect.top()),
                                    egui::pos2(divider, rect.bottom()),
                                ],
                                egui::Stroke::new(2.0, egui::Color32::WHITE),
                            );
                        } else {
                            ui.allocate_space(egui::vec2(500.0, 260.0));
                        }
                    });
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
                ui.horizontal(|ui| {
                    if self.busy() {
                        ui.spinner();
                        if ui.button(crate::i18n::text("Stop")).clicked() {
                            self.cancel.stop();
                        }
                    } else {
                        if ui
                            .add_enabled(
                                !self.sources.is_empty(),
                                egui::Button::new(crate::tr!(
                                    "Export collection ({} images)",
                                    self.sources.len()
                                )),
                            )
                            .clicked()
                            && let Some(folder) = rfd::FileDialog::new().pick_folder()
                        {
                            self.export(self.sources.clone(), folder, ctx);
                        }
                        if !self.failed.is_empty()
                            && ui.button(crate::i18n::text("Retry failed files")).clicked()
                            && let Some(folder) = self.folder.clone()
                        {
                            self.export(self.failed.clone(), folder, ctx);
                        }
                    }
                    if let Some(folder) = &self.folder
                        && ui.button(crate::i18n::text("Open output folder")).clicked()
                    {
                        let _ = std::process::Command::new("explorer.exe")
                            .arg(folder)
                            .spawn();
                    }
                });
            });
        self.open = open;
    }
}

impl Drop for Studio {
    fn drop(&mut self) {
        self.cancel.stop();
    }
}
