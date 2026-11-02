use crate::{model::Event, update};
use eframe::egui;

pub struct Updater {
    target: Option<std::path::PathBuf>,
    rx: crossbeam_channel::Receiver<Event>,
    tx: crossbeam_channel::Sender<Event>,
    cancel: crate::model::Cancel,
    running: bool,
    bytes: u64,
    total: Option<u64>,
    ready: Option<update::Prepared>,
    message: String,
    target_running: bool,
    last_process_check: std::time::Instant,
}

impl Updater {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let preferences = update::Preferences::load();
        crate::i18n::set_language(&preferences.language);
        crate::i18n::configure_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            target: update::installed_executable(),
            rx,
            tx,
            cancel: Default::default(),
            running: false,
            bytes: 0,
            total: None,
            ready: None,
            message: String::new(),
            target_running: true,
            last_process_check: std::time::Instant::now(),
        }
    }
    fn download(&mut self, ctx: &egui::Context) {
        let Some(target) = self.target.clone() else {
            return;
        };
        self.running = true;
        self.message.clear();
        self.cancel = Default::default();
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let repository = update::Preferences::load().repository;
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                let release = update::latest(&repository)?
                    .ok_or_else(|| anyhow::anyhow!("No published update is available"))?;
                let mut processes = update::running_instances(&target);
                processes.push(std::process::id());
                update::prepare_at(&release, cancel, tx.clone(), &target, processes)
            })();
            let event = match result {
                Ok(prepared) => Event::UpdateReady(prepared),
                Err(error) => Event::UpdateFailed(error.to_string()),
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }
}

impl eframe::App for Updater {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::UpdateProgress { bytes, total } => {
                    self.bytes = bytes;
                    self.total = total;
                }
                Event::UpdateReady(prepared) => {
                    self.running = false;
                    self.ready = Some(prepared);
                }
                Event::UpdateFailed(error) => {
                    self.running = false;
                    self.message = error;
                }
                _ => {}
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(23, 28, 34))
                    .inner_margin(28),
            )
            .show(ui, |ui| {
                ui.heading(crate::i18n::text("Updating Image Atlas"));
                ui.add_space(18.0);
                ui.label(crate::i18n::text(
                    "Update the existing copy in this folder.",
                ));
                if let Some(target) = &self.target {
                    ui.add(egui::Label::new(target.display().to_string()).wrap());
                }
                if !self.running
                    && self.ready.is_none()
                    && ui.button(crate::i18n::text("Choose folder")).clicked()
                    && let Some(folder) = rfd::FileDialog::new().pick_folder()
                {
                    let target = folder.join("ImageAtlas.exe");
                    if target.is_file() {
                        self.target = Some(target);
                        self.message.clear();
                    } else {
                        self.message = "ImageAtlas.exe was not found in that folder".into();
                    }
                }
                ui.add_space(18.0);
                if self.running {
                    if let Some(total) = self.total.filter(|total| *total > 0) {
                        ui.add(
                            egui::ProgressBar::new(self.bytes as f32 / total as f32)
                                .show_percentage(),
                        );
                    } else {
                        ui.spinner();
                    }
                    ui.label(format!("{:.1} MB", self.bytes as f64 / 1e6));
                    if ui.button(crate::i18n::text("Stop")).clicked() {
                        self.cancel.stop();
                    }
                } else if let Some(prepared) = &self.ready {
                    if self.last_process_check.elapsed() >= std::time::Duration::from_millis(500) {
                        self.target_running = self
                            .target
                            .as_ref()
                            .is_some_and(|target| !update::running_instances(target).is_empty());
                        self.last_process_check = std::time::Instant::now();
                    }
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(500));
                    ui.strong(crate::tr!("Image Atlas {}", prepared.version));
                    ui.label(crate::i18n::text(
                        "Save your work and close Atlas before updating.",
                    ));
                    if ui
                        .add_enabled(
                            !self.target_running,
                            egui::Button::new(crate::i18n::text("Update now")),
                        )
                        .clicked()
                    {
                        match update::start(prepared) {
                            Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                            Err(error) => self.message = error.to_string(),
                        }
                    }
                } else if ui
                    .add_enabled(
                        self.target.is_some(),
                        egui::Button::new(crate::i18n::text("Update now")),
                    )
                    .clicked()
                {
                    self.download(ui.ctx());
                }
                if !self.message.is_empty() {
                    ui.add_space(12.0);
                    ui.label(&self.message);
                }
            });
        if self.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
