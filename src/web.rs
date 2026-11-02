use anyhow::{Context, Result};
use eframe::egui;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageEncoder};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Format {
    #[default]
    Automatic,
    Original,
    Png,
    Jpeg,
    Webp,
}
impl Format {
    fn label(self) -> &'static str {
        match self {
            Self::Automatic => crate::i18n::text("Automatic PNG / JPEG / SVG"),
            Self::Original => crate::i18n::text("Keep PNG / JPEG / SVG"),
            Self::Png => crate::i18n::text("PNG — lossless"),
            Self::Jpeg => crate::i18n::text("JPEG — smaller photos"),
            Self::Webp => crate::i18n::text("WebP — lossless"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub format: Format,
    pub max_edge: u32,
    pub quality: u8,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Automatic,
            max_edge: 1600,
            quality: 85,
        }
    }
}
impl Options {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_id_salt("web-format")
            .selected_text(self.format.label())
            .show_ui(ui, |ui| {
                for format in [
                    Format::Automatic,
                    Format::Original,
                    Format::Png,
                    Format::Jpeg,
                    Format::Webp,
                ] {
                    ui.selectable_value(&mut self.format, format, format.label());
                }
            });
        ui.horizontal(|ui| {
            ui.label(crate::i18n::text("Longest edge"));
            ui.add(egui::DragValue::new(&mut self.max_edge).suffix(" px"));
        });
        ui.weak(crate::i18n::text(
            "0 keeps dimensions. Smaller images are never enlarged.",
        ));
        if matches!(
            self.format,
            Format::Automatic | Format::Original | Format::Jpeg
        ) {
            ui.add(
                egui::Slider::new(&mut self.quality, 40..=100)
                    .text(crate::i18n::text("JPEG quality")),
            );
        }
        if self.format == Format::Automatic {
            ui.weak(crate::i18n::text("PNG for transparency and compact artwork; JPEG when much smaller. SVG stays vector."));
        }
        if self.format == Format::Jpeg {
            ui.weak(crate::i18n::text("JPEG flattens transparency onto white."));
        }
        if self.format == Format::Webp {
            ui.weak(crate::i18n::text(
                "Check that your website accepts WebP uploads.",
            ));
        }
        ui.weak(crate::i18n::text(
            "Normal image files in a folder. No ZIP archive.",
        ));
    }
}
#[derive(Clone)]
pub struct Source {
    pub path: PathBuf,
    pub edited_svg: Option<Vec<u8>>,
}
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub extension: String,
    pub input_bytes: u64,
}
pub struct Saved {
    pub path: PathBuf,
    pub before: u64,
    pub after: u64,
}
pub fn summary(before: u64, after: u64) -> String {
    if before == 0 {
        return format!("{:.1} KB", after as f64 / 1000.0);
    }
    let delta = (1.0 - after as f64 / before as f64) * 100.0;
    crate::tr!(
        "{:.1} KB → {:.1} KB · {:.1}% {}",
        before as f64 / 1000.0,
        after as f64 / 1000.0,
        delta.abs(),
        if delta >= 0.0 {
            crate::i18n::text("smaller")
        } else {
            crate::i18n::text("larger")
        }
    )
}
fn original_format(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut head = [0u8; 8192];
    let count = fs::File::open(path).ok()?.read(&mut head).ok()?;
    crate::download::detected_extension(&head[..count], "")
}
fn profile(path: &Path) -> Option<Vec<u8>> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?
        .icc_profile()
        .ok()
        .flatten()
}
fn shrink(image: DynamicImage, max_edge: u32) -> DynamicImage {
    if max_edge > 0 && image.width().max(image.height()) > max_edge {
        image.resize(max_edge, max_edge, image::imageops::FilterType::Lanczos3)
    } else {
        image
    }
}
fn png(image: &DynamicImage, icc: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let opaque = match image {
        DynamicImage::ImageRgba8(p) => p.pixels().all(|p| p[3] == 255),
        DynamicImage::ImageLumaA8(p) => p.pixels().all(|p| p[1] == 255),
        _ => false,
    };
    let stripped = if opaque {
        Some(match image {
            DynamicImage::ImageLumaA8(_) => DynamicImage::ImageLuma8(image.to_luma8()),
            _ => DynamicImage::ImageRgb8(image.to_rgb8()),
        })
    } else {
        None
    };
    let image = stripped.as_ref().unwrap_or(image);
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut bytes,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    );
    if let Some(icc) = icc {
        encoder.set_icc_profile(icc)?;
    }
    encoder.write_image(
        image.as_bytes(),
        image.width(),
        image.height(),
        image.color().into(),
    )?;
    Ok(bytes)
}
fn jpeg(image: &DynamicImage, quality: u8, icc: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
    if let Some(icc) = icc {
        encoder.set_icc_profile(icc)?;
    }
    if let Some(rgb) = image.as_rgb8() {
        encoder.encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    } else {
        let rgb = crate::convert::flatten_white(image);
        encoder.encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    }
    Ok(bytes)
}
fn webp(image: &DynamicImage, icc: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut bytes);
    if let Some(icc) = icc {
        encoder.set_icc_profile(icc)?;
    }
    match image {
        DynamicImage::ImageRgb8(rgb) => encoder.write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )?,
        DynamicImage::ImageRgba8(rgba) => encoder.write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )?,
        _ => {
            let rgba = image.to_rgba8();
            encoder.write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        }
    }
    Ok(bytes)
}
fn extension(format: Format, source: Option<&str>) -> &'static str {
    match format {
        Format::Automatic | Format::Png => "png",
        Format::Jpeg => "jpg",
        Format::Webp => "webp",
        Format::Original => match source {
            Some("jpg" | "jpeg") => "jpg",
            Some("webp") => "webp",
            _ => "png",
        },
    }
}
pub fn encode_raster(
    image: DynamicImage,
    source: &Path,
    options: &Options,
    unchanged: bool,
) -> Result<Encoded> {
    let original = original_format(source);
    let old_size = image.dimensions();
    let image = shrink(image, options.max_edge);
    let icc = profile(source);
    let input_bytes = source.metadata().map(|m| m.len()).unwrap_or(0);
    let can_keep_original = unchanged && old_size == image.dimensions() && input_bytes > 0;
    let (ext, mut bytes) = if options.format == Format::Automatic {
        let mut png_bytes = png(&image, icc.clone())?;
        if can_keep_original
            && original.as_deref() == Some("png")
            && input_bytes < (png_bytes.len() as u64)
        {
            png_bytes = fs::read(source)?;
        }
        let opaque = match &image {
            DynamicImage::ImageRgba8(p) => p.pixels().all(|p| p[3] == 255),
            DynamicImage::ImageLumaA8(p) => p.pixels().all(|p| p[1] == 255),
            DynamicImage::ImageRgba16(p) => p.pixels().all(|p| p[3] == 65535),
            DynamicImage::ImageLumaA16(p) => p.pixels().all(|p| p[1] == 65535),
            _ => !image.color().has_alpha(),
        };
        if opaque {
            let mut jpeg_bytes = jpeg(&image, options.quality, icc)?;
            if can_keep_original
                && matches!(original.as_deref(), Some("jpg" | "jpeg"))
                && input_bytes < (jpeg_bytes.len() as u64)
            {
                jpeg_bytes = fs::read(source)?;
            }
            if (jpeg_bytes.len() as f64) < (png_bytes.len() as f64 * 0.8) {
                ("jpg", jpeg_bytes)
            } else {
                ("png", png_bytes)
            }
        } else {
            ("png", png_bytes)
        }
    } else {
        let ext = extension(options.format, original.as_deref());
        let bytes = match ext {
            "jpg" => jpeg(&image, options.quality, icc)?,
            "webp" => webp(&image, icc)?,
            _ => png(&image, icc)?,
        };
        (ext, bytes)
    };
    let same_format = original
        .as_deref()
        .is_some_and(|f| f == ext || f == "jpeg" && ext == "jpg");
    if unchanged
        && old_size == image.dimensions()
        && same_format
        && input_bytes > 0
        && input_bytes < bytes.len() as u64
    {
        bytes = fs::read(source)?;
    }
    Ok(Encoded {
        bytes,
        extension: ext.into(),
        input_bytes,
    })
}
fn compact_svg(source: &[u8]) -> Result<Vec<u8>> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(source);
    let mut writer = quick_xml::Writer::new(Vec::new());
    let mut preserve = Vec::new();
    loop {
        let event = reader.read_event()?;
        match &event {
            Event::Start(element) => {
                let name = element.local_name();
                let protected = preserve.last().copied().unwrap_or(false)
                    || [
                        b"text".as_slice(),
                        b"tspan",
                        b"textPath",
                        b"style",
                        b"script",
                        b"foreignObject",
                        b"metadata",
                        b"title",
                        b"desc",
                    ]
                    .contains(&name.as_ref())
                    || element
                        .attributes()
                        .flatten()
                        .any(|a| a.key.as_ref() == b"xml:space" && a.value.as_ref() == b"preserve");
                preserve.push(protected);
            }
            Event::End(_) => {
                preserve.pop();
            }
            Event::Text(text)
                if !preserve.last().copied().unwrap_or(false)
                    && text.iter().all(|c| c.is_ascii_whitespace()) =>
            {
                continue;
            }
            Event::Eof => break,
            _ => {}
        }
        writer.write_event(event)?;
    }
    let output = writer.into_inner();
    resvg::usvg::Tree::from_data(&output, &Default::default())
        .context(crate::i18n::text("Invalid SVG after optimization"))?;
    Ok(output)
}
pub(crate) fn render_svg(bytes: &[u8], path: &Path, max_edge: u32) -> Result<DynamicImage> {
    let options = resvg::usvg::Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        fontdb: crate::convert::system_fonts(),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(bytes, &options)?;
    let size = tree.size();
    let scale = if max_edge == 0 {
        1.0
    } else {
        (max_edge as f32 / size.width().max(size.height())).min(1.0)
    };
    let mut pixmap = resvg::tiny_skia::Pixmap::new(
        (size.width() * scale).round().max(1.0) as u32,
        (size.height() * scale).round().max(1.0) as u32,
    )
    .context(crate::i18n::text(
        "Image dimensions exceed available memory",
    ))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(image::load_from_memory(&pixmap.encode_png()?)?)
}
pub fn preview(encoded: &Encoded, source: &Path) -> Result<DynamicImage> {
    if encoded.extension == "svg" {
        render_svg(&encoded.bytes, source, 1800)
    } else {
        Ok(image::load_from_memory(&encoded.bytes)?)
    }
}

pub fn encode(source: &Source, options: &Options) -> Result<Encoded> {
    let original = original_format(&source.path);
    let native_svg =
        original.as_deref() == Some("svg") || original.is_none() && source.edited_svg.is_some();
    if matches!(options.format, Format::Automatic | Format::Original) && native_svg {
        let data = if let Some(bytes) = &source.edited_svg {
            bytes.clone()
        } else {
            fs::read(&source.path)?
        };
        let input_bytes = source
            .path
            .metadata()
            .map(|m| m.len())
            .unwrap_or(data.len() as u64);
        let optimized = compact_svg(&data)?;
        return Ok(Encoded {
            bytes: if optimized.len() < data.len() {
                optimized
            } else {
                data
            },
            extension: "svg".into(),
            input_bytes,
        });
    }
    let image = if let Some(svg) = &source.edited_svg {
        render_svg(svg, &source.path, options.max_edge)?
    } else if native_svg {
        render_svg(&fs::read(&source.path)?, &source.path, options.max_edge)?
    } else {
        crate::convert::load(&source.path)?
    };
    encode_raster(image, &source.path, options, source.edited_svg.is_none())
}
pub fn save(encoded: Encoded, source: &Path, folder: &Path) -> Result<Saved> {
    fs::create_dir_all(folder)?;
    let temp = tempfile::NamedTempFile::new_in(folder)?;
    fs::write(temp.path(), &encoded.bytes)?;
    let path = crate::exports::save(temp.path(), source, folder, &encoded.extension)?;
    Ok(Saved {
        path,
        before: encoded.input_bytes,
        after: encoded.bytes.len() as u64,
    })
}
pub fn export(source: &Source, folder: &Path, options: &Options) -> Result<Saved> {
    save(encode(source, options)?, &source.path, folder)
}

pub struct Report {
    pub files: usize,
    pub failures: usize,
    pub before: u64,
    pub after: u64,
}
pub fn common_root(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut common = paths.first()?.parent()?.to_path_buf();
    for path in paths.iter().skip(1) {
        while !path.starts_with(&common) {
            if !common.pop() {
                return None;
            }
        }
    }
    Some(common)
}
pub fn destination(path: &Path, common: Option<&Path>, folder: &Path) -> PathBuf {
    common
        .and_then(|root| path.strip_prefix(root).ok())
        .and_then(|relative| relative.parent())
        .map(|relative| folder.join(relative))
        .unwrap_or_else(|| folder.to_path_buf())
}

pub fn export_collection(
    sources: Vec<Source>,
    folder: PathBuf,
    options: Options,
    cancel: crate::model::Cancel,
    events: crossbeam_channel::Sender<crate::model::Event>,
) -> Result<Report> {
    use std::io::Write;
    fs::create_dir_all(&folder)?;
    let mut report = Report {
        files: 0,
        failures: 0,
        before: 0,
        after: 0,
    };
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let common = common_root(&sources.iter().map(|s| s.path.clone()).collect::<Vec<_>>());
    let mut estimate = crate::progress::Exports::new(
        sources
            .iter()
            .map(|source| crate::progress::Work::file(&source.path))
            .collect(),
    );
    let total = sources.len();
    for (index, source) in sources.iter().enumerate() {
        if cancel.stopped() {
            break;
        }
        let destination = destination(&source.path, common.as_deref(), &folder);
        let started = std::time::Instant::now();
        let mut success = false;
        match export(source, &destination, &options) {
            Ok(saved) => {
                success = true;
                report.files += 1;
                report.before += saved.before;
                report.after += saved.after;
                rows.push(format!(
                    "\"{}\",\"{}\",{},{}",
                    source.path.display().to_string().replace('"', "\"\""),
                    saved.path.display().to_string().replace('"', "\"\""),
                    saved.before,
                    saved.after
                ));
            }
            Err(e) => {
                report.failures += 1;
                errors.push(format!("{}: {e}", source.path.display()));
                let _ = events.send(crate::model::Event::Log(format!(
                    "{}: {e}",
                    source.path.display()
                )));
            }
        }
        let done = index + 1;
        let (rate, eta_seconds) = estimate.finish(started.elapsed(), success);
        let _ = events.send(crate::model::Event::ExportProgress {
            done,
            total,
            rate,
            eta_seconds,
        });
    }
    let mut file = fs::File::create(folder.join("optimization-report.csv"))?;
    writeln!(file, "source,file,before_bytes,after_bytes")?;
    for row in rows {
        writeln!(file, "{row}")?;
    }
    fs::write(folder.join("optimization-errors.txt"), errors.join("\n"))?;
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn source(path: &Path) -> Source {
        Source {
            path: path.into(),
            edited_svg: None,
        }
    }
    fn patterned() -> DynamicImage {
        DynamicImage::ImageRgba8(image::RgbaImage::from_fn(240, 160, |x, y| {
            image::Rgba([
                ((x * 11 + y * 7) % 256) as u8,
                ((x * 3 + y * 13) % 256) as u8,
                ((x + y) % 256) as u8,
                if x < 20 { 0 } else { 255 },
            ])
        }))
    }
    #[test]
    fn png_export_is_lossless_and_does_not_overwrite_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.png");
        let image = patterned();
        image.save(&path).unwrap();
        let original = fs::read(&path).unwrap();
        let encoded = encode(
            &source(&path),
            &Options {
                max_edge: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(encoded.extension, "png");
        assert!(encoded.bytes.len() <= original.len());
        assert_eq!(
            image::load_from_memory(&encoded.bytes).unwrap().to_rgba8(),
            image.to_rgba8()
        );
        let saved = save(encoded, &path, dir.path()).unwrap();
        assert_ne!(saved.path, path);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            image::open(saved.path).unwrap().to_rgba8(),
            image.to_rgba8()
        );
    }
    #[test]
    fn sixteen_bit_png_keeps_its_sample_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("16.png");
        let pixels = image::ImageBuffer::from_fn(16, 16, |x, y| {
            image::Rgba([x as u16 * 4093, y as u16 * 4093, 12031, 65535])
        });
        DynamicImage::ImageRgba16(pixels.clone())
            .save(&path)
            .unwrap();
        let encoded = encode(
            &source(&path),
            &Options {
                max_edge: 0,
                format: Format::Png,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            image::load_from_memory(&encoded.bytes).unwrap().to_rgba16(),
            pixels
        );
    }
    #[test]
    fn web_resize_preserves_aspect_ratio_and_never_enlarges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.png");
        patterned().save(&path).unwrap();
        let small = encode(
            &source(&path),
            &Options {
                max_edge: 120,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            image::load_from_memory(&small.bytes).unwrap().dimensions(),
            (120, 80)
        );
        let large = encode(
            &source(&path),
            &Options {
                max_edge: 1600,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            image::load_from_memory(&large.bytes).unwrap().dimensions(),
            (240, 160)
        );
    }
    #[test]
    fn jpeg_and_webp_are_real_decodable_image_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.png");
        patterned().save(&path).unwrap();
        for format in [Format::Jpeg, Format::Webp] {
            let encoded = encode(
                &source(&path),
                &Options {
                    format,
                    max_edge: 0,
                    quality: 85,
                },
            )
            .unwrap();
            let image = image::load_from_memory(&encoded.bytes).unwrap();
            assert_eq!(image.dimensions(), (240, 160));
            if format == Format::Webp {
                assert_eq!(image.to_rgba8(), patterned().to_rgba8());
            } else {
                let pixel = image.to_rgb8().get_pixel(0, 80).0;
                assert!(pixel.iter().all(|c| *c > 240));
                assert_eq!(
                    image::guess_format(&encoded.bytes).unwrap(),
                    image::ImageFormat::Jpeg
                );
            }
        }
    }
    #[test]
    fn svg_compaction_preserves_vectors_text_and_attribution() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
        <!-- Copyright NIS -->
        <metadata>Attribution remains here</metadata>
        <rect width="200" height="100" fill="#ffffff"/>
        <text x="5" y="30" font-size="18">one <tspan>two</tspan> three</text>
        <path d="M10 60 L100 60 L50 90 Z" fill="blue"/>
        </svg>"##;
        let output = compact_svg(svg).unwrap();
        assert!(output.len() < svg.len());
        let text = String::from_utf8(output.clone()).unwrap();
        assert!(text.contains("Copyright NIS"));
        assert!(text.contains("Attribution remains here"));
        assert!(text.contains("one <tspan>two</tspan> three"));
        assert!(text.contains("<path"));
        assert!(!text.contains("data:image"));
        assert_eq!(
            render_svg(svg, Path::new("image.svg"), 0)
                .unwrap()
                .to_rgba8(),
            render_svg(&output, Path::new("image.svg"), 0)
                .unwrap()
                .to_rgba8()
        );
    }
    #[test]
    fn edited_raster_document_exports_its_current_appearance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.png");
        DynamicImage::new_rgb8(30, 20).save(&path).unwrap();
        let original = fs::read(&path).unwrap();
        let edited=Source{path:path.clone(),edited_svg:Some(br##"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="20"><rect width="30" height="20" fill="#0080ff"/></svg>"##.to_vec())};
        let output = encode(
            &edited,
            &Options {
                max_edge: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.extension, "png");
        assert_eq!(
            image::load_from_memory(&output.bytes)
                .unwrap()
                .to_rgb8()
                .get_pixel(5, 5)
                .0,
            [0, 128, 255]
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn collection_uses_plain_files_and_retains_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("source/Signs/art.png");
        let second = dir.path().join("source/Photos/art.png");
        for path in [&first, &second] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            patterned().save(path).unwrap();
        }
        let output = dir.path().join("optimized");
        let (tx, rx) = crossbeam_channel::unbounded();
        let report = export_collection(
            vec![source(&first), source(&second)],
            output.clone(),
            Options::default(),
            Default::default(),
            tx,
        )
        .unwrap();
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], crate::model::Event::ExportProgress {
            done: 1, total: 2, rate, eta_seconds: None
        } if rate.is_finite() && rate > 0.0));
        assert!(matches!(
            events[1],
            crate::model::Event::ExportProgress {
                done: 2,
                total: 2,
                eta_seconds: Some(0.0),
                ..
            }
        ));
        assert_eq!(report.files, 2);
        assert_eq!(report.failures, 0);
        assert!(output.join("Signs/art.png").is_file());
        assert!(output.join("Photos/art.png").is_file());
        assert!(!output.with_extension("zip").exists());
        assert!(report.before >= report.after);
    }
    #[test]
    fn batch_web_export_keeps_the_color_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gray.png");
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            10,
            10,
            image::Rgba([128, 128, 128, 255]),
        ))
        .save(&path)
        .unwrap();
        let settings = crate::convert::Settings {
            web: Some(Options {
                max_edge: 0,
                ..Default::default()
            }),
            rules: vec![crate::convert::ColorRule::default()],
            ..Default::default()
        };
        let encoded = crate::convert::encode_web(&path, &settings).unwrap();
        assert_eq!(
            image::load_from_memory(&encoded.bytes)
                .unwrap()
                .to_rgb8()
                .get_pixel(5, 5)
                .0,
            [255, 255, 255]
        );
    }
    #[test]
    fn automatic_format_uses_jpeg_only_when_it_is_substantially_smaller() {
        let dir = tempfile::tempdir().unwrap();
        let artwork = dir.path().join("art.png");
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            120,
            80,
            image::Rgb([0, 110, 220]),
        ))
        .save(&artwork)
        .unwrap();
        let output = encode(
            &source(&artwork),
            &Options {
                max_edge: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.extension, "png");
        let photo = dir.path().join("photo.png");
        let mut state = 0x91ab23cdu32;
        let pixels = image::RgbImage::from_fn(120, 80, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            image::Rgb([
                (state & 255) as u8,
                ((state >> 8) & 255) as u8,
                ((state >> 16) & 255) as u8,
            ])
        });
        DynamicImage::ImageRgb8(pixels).save(&photo).unwrap();
        let output = encode(
            &source(&photo),
            &Options {
                max_edge: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.extension, "jpg");
        assert!((output.bytes.len() as u64) < output.input_bytes);
    }
    #[test]
    #[ignore = "Optional measurement against a local sample supplied with IMAGE_ATLAS_SAMPLE"]
    fn measure_local_sample() {
        let path = PathBuf::from(
            std::env::var_os("IMAGE_ATLAS_SAMPLE")
                .expect("Set IMAGE_ATLAS_SAMPLE to a local image"),
        );
        for max_edge in [0, 1600] {
            for format in [
                Format::Automatic,
                Format::Original,
                Format::Png,
                Format::Jpeg,
                Format::Webp,
            ] {
                let result = encode(
                    &source(&path),
                    &Options {
                        max_edge,
                        format,
                        quality: 85,
                    },
                )
                .unwrap();
                println!(
                    "{:?}, edge {}: {}",
                    format,
                    max_edge,
                    summary(result.input_bytes, result.bytes.len() as u64)
                );
                assert!(image::load_from_memory(&result.bytes).is_ok());
            }
        }
    }
}
