use anyhow::{Context, Result};
use eframe::egui::{self, Color32, RichText};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use xmltree::{Element, XMLNode};

#[derive(Clone)]
struct Object {
    id: String,
    name: String,
    bounds: egui::Rect,
    inverse: resvg::tiny_skia::Transform,
}
struct DocumentState {
    root: Element,
    undo: Vec<Element>,
    redo: Vec<Element>,
}

#[derive(Default)]
pub struct Editor {
    web_dialog: bool,
    web_options: crate::web::Options,
    web_status: String,
    web_progress: Option<(usize, usize, f64, Option<f64>, std::time::Instant)>,
    web_preview: Option<egui::TextureHandle>,
    web_folder: Option<PathBuf>,
    web_job: Option<crossbeam_channel::Receiver<crate::model::Event>>,
    web_cancel: crate::model::Cancel,
    documents: Vec<PathBuf>,
    document_index: usize,
    saved_documents: HashMap<PathBuf, DocumentState>,
    modified: bool,
    root: Option<Element>,
    path: Option<PathBuf>,
    objects: Vec<Object>,
    selected: Option<String>,
    texture: Option<egui::TextureHandle>,
    undo: Vec<Element>,
    redo: Vec<Element>,
    dirty: bool,
    zoom: f32,
    pan: egui::Vec2,
    fill: [u8; 3],
    stroke: [u8; 3],
    stroke_width: f32,
    scale: f32,
    message: String,
}
fn element_mut<'a>(root: &'a mut Element, id: &str) -> Option<&'a mut Element> {
    if root.attributes.get("id").is_some_and(|s| s == id) {
        return Some(root);
    }
    root.children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .find_map(|e| element_mut(e, id))
}
fn assign_ids(root: &mut Element, counter: &mut usize) {
    if [
        "g", "rect", "circle", "ellipse", "path", "polygon", "polyline", "line", "text", "image",
        "use",
    ]
    .contains(&root.name.as_str())
        && !root.attributes.contains_key("id")
    {
        *counter += 1;
        root.attributes
            .insert("id".into(), format!("image-atlas-{}", counter));
    }
    for child in root.children.iter_mut().filter_map(XMLNode::as_mut_element) {
        assign_ids(child, counter);
    }
}
fn remove(root: &mut Element, id: &str) -> bool {
    if let Some(index) = root.children.iter().position(|n| {
        n.as_element()
            .and_then(|e| e.attributes.get("id"))
            .is_some_and(|s| s == id)
    }) {
        root.children.remove(index);
        return true;
    }
    root.children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .any(|e| remove(e, id))
}
fn reorder(root: &mut Element, id: &str, front: bool) -> bool {
    if let Some(index) = root.children.iter().position(|n| {
        n.as_element()
            .and_then(|e| e.attributes.get("id"))
            .is_some_and(|s| s == id)
    }) {
        let node = root.children.remove(index);
        if front {
            root.children.push(node);
        } else {
            root.children.insert(0, node);
        }
        return true;
    }
    root.children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .any(|e| reorder(e, id, front))
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    documents: Vec<PathBuf>,
    index: usize,
    path: Option<PathBuf>,
    active: Option<SavedDocument>,
    remembered: Vec<(PathBuf, SavedDocument)>,
    modified: bool,
    selected: Option<String>,
    zoom: f32,
    pan: [f32; 2],
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedDocument {
    root: String,
    undo: Vec<String>,
    redo: Vec<String>,
}

impl SavedDocument {
    fn capture(root: &Element, undo: &[Element], redo: &[Element]) -> Result<Self> {
        let xml = |root: &Element| -> Result<String> {
            let mut bytes = Vec::new();
            root.write(&mut bytes)?;
            Ok(String::from_utf8(bytes)?)
        };
        Ok(Self {
            root: xml(root)?,
            undo: undo.iter().map(xml).collect::<Result<_>>()?,
            redo: redo.iter().map(xml).collect::<Result<_>>()?,
        })
    }
    fn restore(self) -> Result<DocumentState> {
        let xml = |value: String| -> Result<Element> { Ok(Element::parse(value.as_bytes())?) };
        Ok(DocumentState {
            root: xml(self.root)?,
            undo: self.undo.into_iter().map(xml).collect::<Result<_>>()?,
            redo: self.redo.into_iter().map(xml).collect::<Result<_>>()?,
        })
    }
}

impl Editor {
    pub fn can_restart(&self) -> bool {
        self.web_job.is_none()
    }
    pub fn workspace_snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            documents: self.documents.clone(),
            index: self.document_index,
            path: self.path.clone(),
            active: self
                .root
                .as_ref()
                .filter(|_| self.modified || self.path.is_none())
                .map(|root| SavedDocument::capture(root, &self.undo, &self.redo))
                .transpose()?,
            remembered: self
                .saved_documents
                .iter()
                .map(|(path, state)| {
                    Ok((
                        path.clone(),
                        SavedDocument::capture(&state.root, &state.undo, &state.redo)?,
                    ))
                })
                .collect::<Result<_>>()?,
            modified: self.modified,
            selected: self.selected.clone(),
            zoom: self.zoom,
            pan: [self.pan.x, self.pan.y],
        })
    }
    pub fn restore_snapshot(&mut self, saved: Snapshot) -> Result<()> {
        let mut editor = Self::default();
        if let Some(active) = saved.active {
            let state = active.restore()?;
            editor.root = Some(state.root);
            editor.undo = state.undo;
            editor.redo = state.redo;
        } else if let Some(path) = &saved.path {
            editor.load(path)?;
        }
        editor.saved_documents = saved
            .remembered
            .into_iter()
            .map(|(path, state)| Ok((path, state.restore()?)))
            .collect::<Result<_>>()?;
        editor.documents = saved.documents;
        editor.document_index = saved.index;
        editor.path = saved.path;
        editor.modified = saved.modified;
        editor.selected = saved.selected;
        editor.zoom = saved.zoom;
        editor.pan = egui::vec2(saved.pan[0], saved.pan[1]);
        editor.dirty = true;
        *self = editor;
        Ok(())
    }

    fn load(&mut self, path: &Path) -> Result<()> {
        let mut root = if crate::download::detected_extension(&std::fs::read(path)?, "").as_deref()
            == Some("svg")
        {
            Element::parse(std::fs::File::open(path)?)?
        } else {
            use base64::Engine;
            let image = crate::convert::load(path)?;
            let mut data = std::io::Cursor::new(Vec::new());
            image.write_to(&mut data, image::ImageFormat::Png)?;
            Element::parse(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{}\" height=\"{}\"><image width=\"{}\" height=\"{}\" xlink:href=\"data:image/png;base64,{}\"/></svg>",image.width(),image.height(),image.width(),image.height(),base64::engine::general_purpose::STANDARD.encode(data.into_inner())).as_bytes())?
        };
        anyhow::ensure!(root.name == "svg", "This file is not an SVG document");
        assign_ids(&mut root, &mut 0);
        self.root = Some(root);
        self.path = Some(path.into());
        self.selected = None;
        self.undo.clear();
        self.redo.clear();
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        self.dirty = true;
        self.message = crate::i18n::text("Select an object on the canvas or in Layers.").into();
        Ok(())
    }
    pub fn open(&mut self, path: &Path) -> Result<()> {
        self.add_files(vec![path.to_path_buf()])
    }
    pub fn add_files(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        if self.root.is_some() && self.path.is_none() {
            let mut number = 1;
            let mut key = PathBuf::from("Untitled.svg");
            while self.documents.contains(&key) {
                number += 1;
                key = PathBuf::from(format!("Untitled-{number}.svg"));
            }
            self.documents.push(key.clone());
            self.path = Some(key);
            self.web_preview = None;
            self.modified = true;
        }
        let first = paths.first().cloned();
        for path in paths {
            if path.is_file() && !self.documents.contains(&path) {
                self.documents.push(path);
            }
        }
        if let Some(index) = first.and_then(|path| self.documents.iter().position(|p| p == &path)) {
            self.switch_document(index)?;
        }
        Ok(())
    }
    fn switch_document(&mut self, index: usize) -> Result<()> {
        let path = self
            .documents
            .get(index)
            .context(crate::i18n::text("Image is not in this collection"))?
            .clone();
        if self.path.as_ref() == Some(&path) {
            self.document_index = index;
            return Ok(());
        }
        let remembered = self.saved_documents.remove(&path);
        let mut candidate = Self::default();
        let loaded = if remembered.is_none() {
            candidate.load(&path)
        } else {
            Ok(())
        };
        if self.modified
            && let (Some(old_path), Some(root)) = (self.path.clone(), self.root.take())
        {
            self.saved_documents.insert(
                old_path,
                DocumentState {
                    root,
                    undo: std::mem::take(&mut self.undo),
                    redo: std::mem::take(&mut self.redo),
                },
            );
        }
        self.modified = remembered.is_some();
        if let Some(state) = remembered {
            self.root = Some(state.root);
            self.undo = state.undo;
            self.redo = state.redo;
        } else {
            self.root = candidate.root;
            self.undo.clear();
            self.redo.clear();
        }
        self.web_preview = None;
        self.path = Some(path);
        self.document_index = index;
        self.selected = None;
        self.texture = None;
        self.objects.clear();
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        self.dirty = true;
        self.message = crate::i18n::text("Select an object on the canvas or in Layers.").into();
        loaded
    }
    fn snapshot(&mut self) {
        self.web_preview = None;
        self.modified = true;
        if let Some(root) = &self.root {
            self.undo.push(root.clone());
            self.redo.clear();
        }
    }
    fn bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.root
            .as_ref()
            .context(crate::i18n::text("Open an image first"))?
            .write(&mut bytes)?;
        Ok(bytes)
    }
    fn transform(&mut self, value: String) {
        if let (Some(root), Some(id)) = (&mut self.root, &self.selected)
            && let Some(e) = element_mut(root, id)
        {
            let old = e.attributes.get("transform").cloned().unwrap_or_default();
            e.attributes
                .insert("transform".into(), format!("{old} {value}"));
            self.web_preview = None;
            self.modified = true;
            self.dirty = true;
        }
    }
    fn scale_selected(&mut self, factor: f32) {
        if let Some(object) = self
            .objects
            .iter()
            .find(|o| Some(&o.id) == self.selected.as_ref())
        {
            let center = object.bounds.center();
            let mut point = resvg::tiny_skia::Point::from_xy(center.x, center.y);
            object.inverse.map_point(&mut point);
            self.snapshot();
            self.transform(format!(
                "translate({} {}) scale({factor}) translate({} {})",
                point.x, point.y, -point.x, -point.y
            ));
        }
    }
    fn paint_color(&mut self, property: &str, value: String) {
        if let (Some(root), Some(id)) = (&mut self.root, &self.selected)
            && let Some(e) = element_mut(root, id)
        {
            e.attributes.insert(property.into(), value.clone());
            let old = e.attributes.get("style").cloned().unwrap_or_default();
            let filtered = old
                .split(';')
                .filter(|s| s.split(':').next().is_none_or(|k| k.trim() != property))
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join(";");
            e.attributes
                .insert("style".into(), format!("{filtered};{property}:{value}"));
            self.web_preview = None;
            self.modified = true;
            self.dirty = true;
        }
    }
    fn rebuild(&mut self, ctx: &egui::Context) -> Result<()> {
        let bytes = self.bytes()?;
        let options = resvg::usvg::Options {
            resources_dir: self
                .path
                .as_ref()
                .and_then(|p| p.parent())
                .map(Path::to_path_buf),
            fontdb: crate::convert::system_fonts(),
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_data(&bytes, &options)?;
        let mut names = HashMap::new();
        fn names_of(e: &Element, names: &mut HashMap<String, String>) {
            if let Some(id) = e.attributes.get("id") {
                names.insert(id.clone(), format!("{} · {}", e.name, id));
            }
            for c in e.children.iter().filter_map(XMLNode::as_element) {
                names_of(c, names);
            }
        }
        names_of(self.root.as_ref().unwrap(), &mut names);
        fn collect(
            group: &resvg::usvg::Group,
            objects: &mut Vec<Object>,
            names: &HashMap<String, String>,
        ) {
            for node in group.children() {
                if !node.id().is_empty()
                    && let Some(name) = names.get(node.id())
                {
                    let b = node.abs_bounding_box();
                    objects.push(Object {
                        id: node.id().into(),
                        name: name.clone(),
                        bounds: egui::Rect::from_min_size(
                            egui::pos2(b.x(), b.y()),
                            egui::vec2(b.width(), b.height()),
                        ),
                        inverse: node.abs_transform().invert().unwrap_or_default(),
                    });
                }
                if let resvg::usvg::Node::Group(g) = node {
                    collect(g, objects, names);
                }
            }
        }
        self.objects.clear();
        collect(tree.root(), &mut self.objects, &names);
        let size = tree.size();
        self.scale = (1400.0 / size.width()).min(1100.0 / size.height()).min(1.0);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(
            (size.width() * self.scale).ceil().max(1.0) as u32,
            (size.height() * self.scale).ceil().max(1.0) as u32,
        )
        .context(crate::i18n::text("Canvas too large"))?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(self.scale, self.scale),
            &mut pixmap.as_mut(),
        );
        let rgba = image::load_from_memory(&pixmap.encode_png()?)?.to_rgba8();
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        );
        self.texture = Some(ctx.load_texture("svg-canvas", color, Default::default()));
        self.dirty = false;
        Ok(())
    }
    fn add_shape(&mut self, kind: &str) {
        if self.root.is_none() {
            self.root = Some(
                Element::parse(
                    b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"800\" height=\"600\"/>"
                        .as_slice(),
                )
                .unwrap(),
            );
        }
        self.snapshot();
        let mut element = Element::new(kind);
        let id = format!(
            "atlas-new-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        element.attributes.insert("id".into(), id.clone());
        element.attributes.insert("fill".into(), "#20776b".into());
        match kind {
            "rect" => {
                for (k, v) in [
                    ("x", "100"),
                    ("y", "100"),
                    ("width", "200"),
                    ("height", "120"),
                ] {
                    element.attributes.insert(k.into(), v.into());
                }
            }
            "ellipse" => {
                for (k, v) in [("cx", "200"), ("cy", "160"), ("rx", "100"), ("ry", "60")] {
                    element.attributes.insert(k.into(), v.into());
                }
            }
            _ => {
                element.attributes.insert("x".into(), "100".into());
                element.attributes.insert("y".into(), "150".into());
                element.attributes.insert("font-size".into(), "40".into());
                element
                    .children
                    .push(XMLNode::Text(crate::i18n::text("Your text").into()));
            }
        }
        self.root
            .as_mut()
            .unwrap()
            .children
            .push(XMLNode::Element(element));
        self.selected = Some(id);
        self.dirty = true;
        self.zoom = self.zoom.max(1.0);
    }
    fn web_source(&self) -> Result<crate::web::Source> {
        Ok(crate::web::Source {
            path: self
                .path
                .clone()
                .unwrap_or_else(|| PathBuf::from("Untitled.svg")),
            edited_svg: if self.modified || self.path.is_none() {
                Some(self.bytes()?)
            } else {
                None
            },
        })
    }
    fn web_sources(&self) -> Result<Vec<crate::web::Source>> {
        self.documents
            .iter()
            .map(|path| {
                if Some(path) == self.path.as_ref() {
                    return self.web_source();
                }
                let edited_svg = if let Some(saved) = self.saved_documents.get(path) {
                    let mut data = Vec::new();
                    saved.root.write(&mut data)?;
                    Some(data)
                } else {
                    None
                };
                Ok(crate::web::Source {
                    path: path.clone(),
                    edited_svg,
                })
            })
            .collect()
    }
    fn start_web_export(&mut self, sources: Vec<crate::web::Source>) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title(crate::i18n::text("Choose a folder for optimized images"))
            .pick_folder()
        else {
            return;
        };
        self.web_folder = Some(folder.clone());
        self.web_cancel = crate::model::Cancel::default();
        let cancel = self.web_cancel.clone();
        let options = self.web_options.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        self.web_progress = None;
        self.web_job = Some(rx);
        self.web_status = crate::i18n::text("Optimizing image files…").into();
        std::thread::spawn(move || {
            let event = match crate::web::export_collection(
                sources,
                folder.clone(),
                options,
                cancel,
                tx.clone(),
            ) {
                Ok(report) => crate::model::Event::Optimized {
                    folder,
                    files: report.files,
                    failures: report.failures,
                    before: report.before,
                    after: report.after,
                },
                Err(e) => crate::model::Event::Failed(e.to_string()),
            };
            let _ = tx.send(event);
        });
    }
    fn web_export_ui(&mut self, ctx: &egui::Context) {
        if self.web_job.is_some()
            && let Some((done, total, rate, eta, received)) = self.web_progress
        {
            self.web_status = crate::progress::label(done, total, rate, eta, received.elapsed());
        }
        if let Some(rx) = self.web_job.clone() {
            while let Ok(event) = rx.try_recv() {
                match event {
                    crate::model::Event::ExportProgress {
                        done,
                        total,
                        rate,
                        eta_seconds,
                    } => {
                        self.web_progress =
                            Some((done, total, rate, eta_seconds, std::time::Instant::now()));
                    }
                    crate::model::Event::Optimized {
                        folder,
                        files,
                        failures,
                        before,
                        after,
                    } => {
                        self.web_status = crate::tr!(
                            "Saved {files} files · {failures} errors · {}",
                            crate::web::summary(before, after)
                        );
                        self.web_folder = Some(folder);
                        self.web_job = None;
                    }
                    crate::model::Event::Failed(error) => {
                        self.web_status = error;
                        self.web_job = None;
                    }
                    crate::model::Event::Log(message) => self.web_status = message,
                    _ => {}
                }
            }
        }
        if self.web_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if !self.web_dialog {
            return;
        }
        let mut open = true;
        egui::Window::new(crate::i18n::text("Export for website"))
            .open(&mut open)
            .resizable(false)
            .default_width(390.0)
            .show(ctx, |ui| {
                ui.label(crate::i18n::text(
                    "Smaller, regular image files. Your originals stay untouched.",
                ));
                ui.add_space(8.0);
                let old_options = self.web_options.clone();
                self.web_options.ui(ui);
                if old_options != self.web_options {
                    self.web_preview = None;
                    if self.web_job.is_none() {
                        self.web_status.clear();
                    }
                }
                ui.add_space(12.0);
                if ui
                    .button(crate::i18n::text("Preview optimized file"))
                    .clicked()
                {
                    let result = self.web_source().and_then(|source| {
                        let encoded = crate::web::encode(&source, &self.web_options)?;
                        let preview = crate::web::preview(&encoded, &source.path)?
                            .thumbnail(700, 420)
                            .into_rgba8();
                        Ok((encoded, preview))
                    });
                    match result {
                        Ok((encoded, pixels)) => {
                            let color = egui::ColorImage::from_rgba_unmultiplied(
                                [pixels.width() as usize, pixels.height() as usize],
                                pixels.as_raw(),
                            );
                            self.web_preview = Some(ctx.load_texture(
                                "web-export-preview",
                                color,
                                Default::default(),
                            ));
                            self.web_status = crate::web::summary(
                                encoded.input_bytes,
                                encoded.bytes.len() as u64,
                            );
                        }
                        Err(e) => self.web_status = e.to_string(),
                    }
                }
                if let Some(texture) = &self.web_preview {
                    ui.add(egui::Image::new(texture).max_size(egui::vec2(360.0, 230.0)));
                }
                ui.add_enabled_ui(self.web_job.is_none(), |ui| {
                    if ui.button(crate::i18n::text("Export this image")).clicked() {
                        match self.web_source() {
                            Ok(source) => self.start_web_export(vec![source]),
                            Err(e) => self.web_status = e.to_string(),
                        }
                    }
                    if !self.documents.is_empty()
                        && ui
                            .button(crate::tr!(
                                "Export collection ({} images)",
                                self.documents.len()
                            ))
                            .clicked()
                    {
                        match self.web_sources() {
                            Ok(sources) => self.start_web_export(sources),
                            Err(e) => self.web_status = e.to_string(),
                        }
                    }
                });
                if self.web_job.is_some() && ui.button(crate::i18n::text("Stop export")).clicked() {
                    self.web_cancel.stop();
                }
                ui.label(&self.web_status);
                if let Some(folder) = &self.web_folder
                    && ui.button(crate::i18n::text("Open output folder")).clicked()
                {
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(folder)
                        .spawn();
                }
            });
        self.web_dialog = open;
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading(crate::i18n::text("Vector editor"));
            if ui.button(crate::i18n::text("Open image")).clicked()
                && let Some(path) = rfd::FileDialog::new().pick_file()
                && let Err(e) = self.open(&path)
            {
                self.message = e.to_string();
            }
            if ui
                .add_enabled(
                    self.root.is_some(),
                    egui::Button::new(crate::i18n::text("Save SVG")),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name("edited.svg")
                    .save_file()
            {
                match self
                    .bytes()
                    .and_then(|b| std::fs::write(&path, b).map_err(Into::into))
                {
                    Ok(()) => self.message = crate::tr!("Saved {}", path.display()),
                    Err(e) => self.message = e.to_string(),
                }
            }
            if ui
                .add_enabled(
                    self.root.is_some(),
                    egui::Button::new(crate::i18n::text("Export for website")),
                )
                .clicked()
            {
                self.web_dialog = true;
            }
            ui.with_layout(crate::i18n::trailing_layout(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.redo.is_empty(),
                        egui::Button::new(crate::i18n::text("Redo")),
                    )
                    .clicked()
                    && let Some(previous) = self.redo.pop()
                {
                    if let Some(root) = self.root.replace(previous) {
                        self.undo.push(root);
                    }
                    self.web_preview = None;
                    self.modified = true;
                    self.dirty = true;
                }
                if ui
                    .add_enabled(
                        !self.undo.is_empty(),
                        egui::Button::new(crate::i18n::text("Undo")),
                    )
                    .clicked()
                    && let Some(previous) = self.undo.pop()
                {
                    if let Some(root) = self.root.replace(previous) {
                        self.redo.push(root);
                    }
                    self.web_preview = None;
                    self.modified = true;
                    self.dirty = true;
                }
            });
        });
        if !self.documents.is_empty() {
            let mut next = self.document_index;
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(next > 0, egui::Button::new(crate::i18n::text("Previous")))
                    .clicked()
                {
                    next -= 1;
                }
                let last = self.documents.len() - 1;
                ui.add_enabled(
                    last > 0,
                    egui::Slider::new(&mut next, 0..=last).show_value(false),
                );
                if ui
                    .add_enabled(next < last, egui::Button::new(crate::i18n::text("Next")))
                    .clicked()
                {
                    next += 1;
                }
                ui.label(format!("{} / {}", next + 1, self.documents.len()));
                if let Some(path) = self.documents.get(next) {
                    ui.add(
                        egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy())
                            .truncate(),
                    )
                    .on_hover_text(path.display().to_string());
                }
            });
            if next != self.document_index
                && let Err(e) = self.switch_document(next)
            {
                self.message = e.to_string();
            }
        }
        ui.horizontal(|ui| {
            for (kind, label) in [
                ("rect", crate::i18n::text("Rectangle")),
                ("ellipse", crate::i18n::text("Ellipse")),
                ("text", crate::i18n::text("Text")),
            ] {
                if ui.button(label).clicked() {
                    self.add_shape(kind);
                }
            }
            if ui.button(crate::i18n::text("Fit")).clicked() {
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
            }
            ui.weak(crate::i18n::text(
                "Select and drag objects. Hold Space to pan. Scroll to zoom.",
            ));
        });
        if self.dirty
            && let Err(e) = self.rebuild(ctx)
        {
            self.message = e.to_string();
            self.dirty = false;
        }
        ui.add_space(10.0);
        let height = (ui.available_height() - 30.0).max(300.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(230.0, height),
                egui::Layout::top_down(crate::i18n::text_alignment()),
                |ui| {
                    ui.strong(crate::i18n::text("Layers"));
                    egui::ScrollArea::vertical()
                        .max_height(height * 0.36)
                        .show(ui, |ui| {
                            for object in self.objects.iter().rev() {
                                if ui
                                    .selectable_label(
                                        self.selected.as_ref() == Some(&object.id),
                                        &object.name,
                                    )
                                    .clicked()
                                {
                                    self.selected = Some(object.id.clone());
                                }
                            }
                        });
                    ui.separator();
                    ui.add_enabled_ui(self.selected.is_some(), |ui| {
                        ui.strong(crate::i18n::text("Object color"));
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Fill"));
                            let before = self.fill;
                            if ui.color_edit_button_srgb(&mut self.fill).changed() {
                                let new = self.fill;
                                self.fill = before;
                                self.snapshot();
                                self.fill = new;
                                self.paint_color(
                                    "fill",
                                    format!("#{:02x}{:02x}{:02x}", new[0], new[1], new[2]),
                                );
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Stroke"));
                            let before = self.stroke;
                            if ui.color_edit_button_srgb(&mut self.stroke).changed() {
                                let new = self.stroke;
                                self.stroke = before;
                                self.snapshot();
                                self.stroke = new;
                                self.paint_color(
                                    "stroke",
                                    format!("#{:02x}{:02x}{:02x}", new[0], new[1], new[2]),
                                );
                            }
                        });
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.stroke_width)
                                    .range(0.0..=100.0)
                                    .suffix(" stroke"),
                            )
                            .changed()
                        {
                            self.snapshot();
                            self.paint_color("stroke-width", self.stroke_width.to_string());
                        }
                        ui.horizontal(|ui| {
                            if ui.button(crate::i18n::text("Smaller")).clicked() {
                                self.scale_selected(0.9);
                            }
                            if ui.button(crate::i18n::text("Larger")).clicked() {
                                self.scale_selected(1.1);
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button(crate::i18n::text("Front")).clicked() {
                                self.snapshot();
                                if let (Some(root), Some(id)) = (&mut self.root, &self.selected) {
                                    reorder(root, id, true);
                                }
                                self.web_preview = None;
                                self.modified = true;
                                self.dirty = true;
                            }
                            if ui.button(crate::i18n::text("Back")).clicked() {
                                self.snapshot();
                                if let (Some(root), Some(id)) = (&mut self.root, &self.selected) {
                                    reorder(root, id, false);
                                }
                                self.web_preview = None;
                                self.modified = true;
                                self.dirty = true;
                            }
                        });
                        if let (Some(root), Some(id)) = (&mut self.root, &self.selected)
                            && let Some(element) = element_mut(root, id)
                            && element.name == "text"
                        {
                            let mut text = element.get_text().unwrap_or_default().into_owned();
                            if ui.text_edit_singleline(&mut text).changed() {
                                self.undo.push(root.clone());
                                if let Some(element) = element_mut(root, id) {
                                    element.children = vec![XMLNode::Text(text)];
                                    self.web_preview = None;
                                    self.modified = true;
                                    self.dirty = true;
                                }
                            }
                        }
                        if ui.button(crate::i18n::text("Delete object")).clicked() {
                            self.snapshot();
                            if let (Some(root), Some(id)) = (&mut self.root, &self.selected) {
                                remove(root, id);
                            }
                            self.selected = None;
                            self.web_preview = None;
                            self.modified = true;
                            self.dirty = true;
                        }
                    });
                },
            );
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), height),
                egui::Sense::click_and_drag(),
            );
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 12, Color32::from_rgb(14, 18, 23));
            if let Some(texture) = &self.texture {
                let texture = texture.clone();
                let factor = (rect.width() / texture.size_vec2().x)
                    .min(rect.height() / texture.size_vec2().y)
                    * 0.88
                    * self.zoom.max(0.1);
                let image_rect = egui::Rect::from_center_size(
                    rect.center() + self.pan,
                    texture.size_vec2() * factor,
                );
                painter.rect_filled(image_rect, 0, Color32::WHITE);
                painter.image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if response.hovered() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        self.zoom = (self.zoom.max(0.1) * (scroll * 0.002).exp()).clamp(0.1, 12.0);
                        ctx.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                    }
                }
                let global_scale = factor * self.scale;
                if (response.clicked() || response.drag_started())
                    && let Some(position) = response.interact_pointer_pos()
                {
                    let point = egui::Pos2::ZERO + (position - image_rect.min) / global_scale;
                    self.selected = self
                        .objects
                        .iter()
                        .rev()
                        .find(|o| o.bounds.contains(point))
                        .map(|o| o.id.clone());
                    if response.drag_started() {
                        self.snapshot();
                    }
                }
                if response.dragged() {
                    if ctx.input(|i| i.key_down(egui::Key::Space)) {
                        self.pan += response.drag_delta();
                    } else if let Some(object) = self
                        .objects
                        .iter()
                        .find(|o| Some(&o.id) == self.selected.as_ref())
                    {
                        let delta = response.drag_delta() / global_scale;
                        let mut points = [
                            resvg::tiny_skia::Point::from_xy(0.0, 0.0),
                            resvg::tiny_skia::Point::from_xy(delta.x, delta.y),
                        ];
                        object.inverse.map_points(&mut points);
                        self.transform(format!(
                            "translate({} {})",
                            points[1].x - points[0].x,
                            points[1].y - points[0].y
                        ));
                    }
                }
                if let Some(object) = self
                    .objects
                    .iter()
                    .find(|o| Some(&o.id) == self.selected.as_ref())
                {
                    let bounds = egui::Rect::from_min_size(
                        image_rect.min + object.bounds.min.to_vec2() * global_scale,
                        object.bounds.size() * global_scale,
                    );
                    painter.rect_stroke(
                        bounds,
                        0,
                        egui::Stroke::new(1.5_f32, Color32::from_rgb(15, 120, 105)),
                        egui::StrokeKind::Outside,
                    );
                }
            } else {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if self.documents.is_empty() {
                        crate::i18n::text("Open an SVG, or add a shape to start")
                    } else {
                        crate::i18n::text("Preview unavailable for this image")
                    },
                    egui::FontId::proportional(20.0),
                    Color32::from_gray(100),
                );
            }
        });
        ui.label(RichText::new(&self.message).small().weak());
        self.web_export_ui(ctx);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_snapshot_preserves_unsaved_objects_and_undo_history() {
        let mut editor = Editor::default();
        editor.add_shape("rect");
        editor.add_shape("ellipse");
        let bytes = editor.bytes().unwrap();
        let undo = editor.undo.len();
        let snapshot = editor.workspace_snapshot().unwrap();
        let encoded = serde_json::to_vec(&snapshot).unwrap();
        let mut restored = Editor::default();
        restored
            .restore_snapshot(serde_json::from_slice(&encoded).unwrap())
            .unwrap();
        assert_eq!(
            Element::parse(restored.bytes().unwrap().as_slice()).unwrap(),
            Element::parse(bytes.as_slice()).unwrap()
        );
        assert_eq!(restored.undo.len(), undo);
        assert!(restored.modified);
    }
    #[test]
    fn svg_edits_preserve_vectors_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shape.svg");
        std::fs::write(&path,b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\"><rect id=\"box\" width=\"20\" height=\"30\" fill=\"red\"/></svg>").unwrap();
        let mut editor = Editor::default();
        editor.open(&path).unwrap();
        editor.selected = Some("box".into());
        editor.snapshot();
        editor.paint_color("fill", "#ffffff".into());
        editor.transform("translate(10 20)".into());
        let bytes = editor.bytes().unwrap();
        let tree = resvg::usvg::Tree::from_data(&bytes, &Default::default()).unwrap();
        assert!(!tree.root().children().is_empty());
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("#ffffff"));
        assert!(text.contains("translate(10 20)"));
        assert!(!text.contains("data:image"));
        editor.root = editor.undo.pop();
        assert!(
            !String::from_utf8(editor.bytes().unwrap())
                .unwrap()
                .contains("translate")
        );
    }
    #[test]
    fn collection_navigation_keeps_edits_and_loads_only_current_document() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("one.svg");
        let second = dir.path().join("two.svg");
        let bad = dir.path().join("bad.svg");
        let svg=b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\"><rect id=\"box\" width=\"20\" height=\"30\"/></svg>";
        std::fs::write(&first, svg).unwrap();
        std::fs::write(&second, svg).unwrap();
        std::fs::write(&bad, b"not an image").unwrap();
        let mut editor = Editor::default();
        editor
            .add_files(vec![
                first.clone(),
                second.clone(),
                bad.clone(),
                first.clone(),
            ])
            .unwrap();
        assert_eq!(editor.documents.len(), 3);
        assert!(editor.saved_documents.is_empty());
        editor.selected = Some("box".into());
        editor.snapshot();
        editor.paint_color("fill", "#ffffff".into());
        editor.switch_document(1).unwrap();
        assert_eq!(editor.saved_documents.len(), 1);
        assert!(
            !String::from_utf8(editor.bytes().unwrap())
                .unwrap()
                .contains("#ffffff")
        );
        editor.switch_document(0).unwrap();
        assert!(
            String::from_utf8(editor.bytes().unwrap())
                .unwrap()
                .contains("#ffffff")
        );
        assert_eq!(editor.undo.len(), 1);
        assert!(editor.switch_document(2).is_err());
        assert_eq!(editor.document_index, 2);
        assert!(editor.root.is_none());
        editor.switch_document(0).unwrap();
        assert!(
            String::from_utf8(editor.bytes().unwrap())
                .unwrap()
                .contains("#ffffff")
        );
    }
}
