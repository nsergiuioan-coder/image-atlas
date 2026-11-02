use anyhow::{Context, Result};
use base64::Engine;
use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub dpi: f32,
    pub rules: Vec<ColorRule>,
    pub web: Option<crate::web::Options>,
    #[serde(default)]
    pub template: Option<crate::template::Template>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            format: "png".into(),
            width: 0,
            height: 0,
            dpi: 300.0,
            rules: Vec::new(),
            web: None,
            template: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ColorRule {
    pub enabled: bool,
    pub from: [u8; 3],
    pub to: [u8; 3],
    pub tolerance: f32,
    pub feather: f32,
    pub edges_only: bool,
    pub seed: Option<(f32, f32)>,
    pub region: Option<(f32, f32, f32, f32)>,
}
impl Default for ColorRule {
    fn default() -> Self {
        Self {
            enabled: true,
            from: [128; 3],
            to: [255; 3],
            tolerance: 28.0,
            feather: 0.0,
            edges_only: true,
            seed: None,
            region: None,
        }
    }
}

pub(crate) fn system_fonts() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
    static FONTS: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            std::sync::Arc::new(db)
        })
        .clone()
}

fn is_svg(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut head = [0u8; 8192];
    let n = fs::File::open(path)?.read(&mut head)?;
    Ok(crate::download::detected_extension(&head[..n], "").as_deref() == Some("svg"))
}

pub fn load(path: &Path) -> Result<DynamicImage> {
    if is_svg(path)? {
        let data = fs::read(path)?;
        let options = resvg::usvg::Options {
            resources_dir: path.parent().map(Path::to_path_buf),
            fontdb: system_fonts(),
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_data(&data, &options)?;
        let size = tree.size().to_int_size();
        let mut pixels = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
            .context("SVG dimensions exceed available memory")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::default(),
            &mut pixels.as_mut(),
        );
        let mut raw = pixels.take();
        for p in raw.as_chunks_mut::<4>().0 {
            if p[3] > 0 {
                for i in 0..3 {
                    p[i] = ((p[i] as u32 * 255) / p[3] as u32).min(255) as u8;
                }
            }
        }
        return Ok(DynamicImage::ImageRgba8(
            RgbaImage::from_raw(size.width(), size.height(), raw).context("Invalid SVG pixels")?,
        ));
    }
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

pub fn dimensions(path: &Path) -> Result<(u32, u32)> {
    if is_svg(path)? {
        let tree = resvg::usvg::Tree::from_data(&fs::read(path)?, &Default::default())?;
        let size = tree.size().to_int_size();
        return Ok((size.width(), size.height()));
    }
    Ok(image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_dimensions()?)
}

fn similar(p: &[u8], target: [u8; 3], tolerance: f32) -> bool {
    p[3] > 0
        && (0..3)
            .map(|i| (p[i] as f32 - target[i] as f32).powi(2))
            .sum::<f32>()
            .sqrt()
            <= tolerance
}

pub fn replace_background(image: &mut RgbaImage, settings: &ColorRule) {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return;
    }
    let inside = |x: u32, y: u32| {
        settings.region.is_none_or(|(x0, y0, x1, y1)| {
            let x = x as f32 / w as f32;
            let y = y as f32 / h as f32;
            x >= x0 && x <= x1 && y >= y0 && y <= y1
        })
    };
    let mut mask = vec![false; w as usize * h as usize];
    if settings.edges_only || settings.seed.is_some() {
        let mut queue = VecDeque::new();
        if let Some((x, y)) = settings.seed {
            queue.push_back((
                (x * w as f32).floor().clamp(0.0, (w - 1) as f32) as u32,
                (y * h as f32).floor().clamp(0.0, (h - 1) as f32) as u32,
            ));
        } else {
            for x in 0..w {
                queue.push_back((x, 0));
                queue.push_back((x, h - 1));
            }
            for y in 0..h {
                queue.push_back((0, y));
                queue.push_back((w - 1, y));
            }
        }
        while let Some((x, y)) = queue.pop_front() {
            let index = (y * w + x) as usize;
            if mask[index]
                || !inside(x, y)
                || !similar(&image.get_pixel(x, y).0, settings.from, settings.tolerance)
            {
                continue;
            }
            mask[index] = true;
            if x > 0 {
                queue.push_back((x - 1, y));
            }
            if x + 1 < w {
                queue.push_back((x + 1, y));
            }
            if y > 0 {
                queue.push_back((x, y - 1));
            }
            if y + 1 < h {
                queue.push_back((x, y + 1));
            }
        }
    } else {
        for (i, p) in image.pixels().enumerate() {
            mask[i] = inside(i as u32 % w, i as u32 / w)
                && similar(&p.0, settings.from, settings.tolerance);
        }
    }
    for (i, p) in image.pixels_mut().enumerate() {
        if mask[i] {
            let distance = (0..3)
                .map(|c| (p[c] as f32 - settings.from[c] as f32).powi(2))
                .sum::<f32>()
                .sqrt();
            let weight = if settings.feather <= 0.0 {
                1.0
            } else {
                ((settings.tolerance - distance) / settings.feather).clamp(0.0, 1.0)
            };
            for c in 0..3 {
                p[c] =
                    (p[c] as f32 * (1.0 - weight) + settings.to[c] as f32 * weight).round() as u8;
            }
        }
    }
}

pub fn prepare(mut image: DynamicImage, settings: &Settings) -> Result<DynamicImage> {
    if settings.rules.iter().any(|r| r.enabled) {
        let mut rgba = image.into_rgba8();
        for rule in settings.rules.iter().filter(|r| r.enabled) {
            replace_background(&mut rgba, rule);
        }
        image = DynamicImage::ImageRgba8(rgba);
    }
    let (w, h) = image.dimensions();
    let (width, height) = match (settings.width, settings.height) {
        (0, 0) => (w, h),
        (x, 0) => (x, ((h as f64 * x as f64 / w as f64).round() as u32).max(1)),
        (0, y) => (((w as f64 * y as f64 / h as f64).round() as u32).max(1), y),
        (x, y) => (x, y),
    };
    if (w, h) != (width, height) {
        image = image.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
    }
    match &settings.template {
        Some(template) => template.apply(image, settings.dpi),
        None => Ok(image),
    }
}

pub(crate) fn flatten_white(image: &DynamicImage) -> image::RgbImage {
    let rgba = image.as_rgba8().map_or_else(
        || std::borrow::Cow::Owned(image.to_rgba8()),
        std::borrow::Cow::Borrowed,
    );
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y);
        let a = p[3] as u32;
        image::Rgb(std::array::from_fn(|c| {
            ((p[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8
        }))
    })
}

fn pdf(image: &DynamicImage, path: &Path, dpi: f32) -> Result<()> {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};
    let mut doc = Document::with_version("1.5");
    let pages = doc.new_object_id();
    let rgb = flatten_white(image);
    let (w, h) = rgb.dimensions();
    let pw = w as f32 * 72.0 / dpi.max(1.0);
    let ph = h as f32 * 72.0 / dpi.max(1.0);
    let mut stream = Stream::new(
        dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>w,"Height"=>h,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8},
        rgb.into_raw(),
    );
    stream.compress()?;
    let image_id = doc.add_object(stream);
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![pw.into(), 0.into(), 0.into(), ph.into(), 0.into(), 0.into()],
            ),
            Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
            Operation::new("Q", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode()?));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),pw.into(),ph.into()],"Resources"=>dictionary!{"XObject"=>dictionary!{"Im0"=>image_id}},"Contents"=>content_id});
    doc.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}.into(),
    );
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", catalog);
    doc.save(path)?;
    Ok(())
}

pub fn encode_web(source: &Path, settings: &Settings) -> Result<crate::web::Encoded> {
    let options = settings
        .web
        .as_ref()
        .context("Enable web optimized export first")?;
    let untouched = settings.width == 0
        && settings.height == 0
        && settings.template.is_none()
        && !settings.rules.iter().any(|r| r.enabled);
    if untouched {
        crate::web::encode(
            &crate::web::Source {
                path: source.into(),
                edited_svg: None,
            },
            options,
        )
    } else {
        let image = prepare(load(source)?, settings)?;
        let width = image.width();
        let mut encoded = crate::web::encode_raster(image, source, options, false)?;
        let saved_width = image::ImageReader::new(std::io::Cursor::new(&encoded.bytes))
            .with_guessed_format()?
            .into_dimensions()?
            .0;
        crate::template::density(
            &mut encoded.bytes,
            &encoded.extension,
            settings.dpi * saved_width as f32 / width as f32,
        )?;
        Ok(encoded)
    }
}

pub fn export(source: &Path, folder: &Path, settings: &Settings) -> Result<PathBuf> {
    fs::create_dir_all(folder)?;
    if settings.web.is_some() {
        return Ok(crate::web::save(encode_web(source, settings)?, source, folder)?.path);
    }
    let image = prepare(load(source)?, settings)?;
    let extension = if settings.format.starts_with("svg") {
        "svg"
    } else {
        &settings.format
    };
    let stem = crate::model::safe_segment(
        source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image"),
    );
    let mut output = folder.join(format!("{stem}.{extension}"));
    let mut i = 2;
    while output.exists() {
        output = folder.join(format!("{stem}-{i}.{extension}"));
        i += 1;
    }
    match settings.format.as_str() {
        "pdf" => pdf(&image, &output, settings.dpi)?,
        "svg trace" => {
            let temp = tempfile::Builder::new().suffix(".png").tempfile()?;
            image.save_with_format(temp.path(), ImageFormat::Png)?;
            vtracer::convert_image_to_svg(temp.path(), &output, vtracer::Config::default())
                .map_err(anyhow::Error::msg)?;
        }
        "svg embedded" => {
            let mut data = std::io::Cursor::new(Vec::new());
            image.write_to(&mut data, ImageFormat::Png)?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(data.into_inner());
            fs::write(
                &output,
                format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><image width=\"100%\" height=\"100%\" xlink:href=\"data:image/png;base64,{}\"/></svg>",
                    image.width(),
                    image.height(),
                    image.width(),
                    image.height(),
                    encoded
                ),
            )?;
        }
        format => {
            let format =
                ImageFormat::from_extension(format).context("Unsupported output format")?;
            if format == ImageFormat::Jpeg {
                let rgb = flatten_white(&image);
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    fs::File::create(&output)?,
                    95,
                );
                encoder.set_pixel_density(image::codecs::jpeg::PixelDensity::dpi(
                    settings.dpi.round().clamp(1.0, 65535.0) as u16,
                ));
                encoder.encode_image(&DynamicImage::ImageRgb8(rgb))?;
            } else if format == ImageFormat::Pnm {
                DynamicImage::ImageRgb8(flatten_white(&image)).save_with_format(&output, format)?;
            } else {
                image.save_with_format(&output, format)?;
            }
        }
    }
    if ["png", "jpg", "jpeg", "webp"].contains(&extension) {
        let mut bytes = fs::read(&output)?;
        crate::template::density(&mut bytes, extension, settings.dpi)?;
        fs::write(&output, bytes)?;
    } else if extension == "svg" {
        let bytes = crate::template::svg_size(
            &fs::read(&output)?,
            image.width(),
            image.height(),
            settings.dpi,
        )?;
        fs::write(&output, bytes)?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dima_and_dpi_are_present_in_batch_exports() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("art.png");
        DynamicImage::new_rgb8(100, 50).save(&source).unwrap();
        let mut settings = Settings {
            dpi: 254.0,
            template: Some(crate::template::Template {
                layout: crate::template::Layout::Fixed,
                width: 30.0,
                height: 20.0,
                margin: 2.0,
                ..Default::default()
            }),
            ..Default::default()
        };
        for format in ["png", "jpg", "webp", "pdf", "svg trace", "svg embedded"] {
            settings.format = format.into();
            let output = export(&source, &folder.path().join("output"), &settings).unwrap();
            match format {
                "png" | "jpg" | "webp" => assert_eq!(dimensions(&output).unwrap(), (300, 200)),
                "pdf" => {
                    let pdf = lopdf::Document::load(&output).unwrap();
                    let page = *pdf.get_pages().values().next().unwrap();
                    let media = pdf
                        .get_object(page)
                        .unwrap()
                        .as_dict()
                        .unwrap()
                        .get(b"MediaBox")
                        .unwrap()
                        .as_array()
                        .unwrap();
                    assert!((media[2].as_float().unwrap() - 30.0 * 72.0 / 25.4).abs() < 0.01);
                }
                _ => {
                    let root = xmltree::Element::parse(fs::File::open(&output).unwrap()).unwrap();
                    assert_eq!(root.attributes["width"], "30mm");
                    assert_eq!(root.attributes["height"], "20mm");
                }
            }
        }
    }
    #[test]
    fn only_connected_background_changes() {
        let mut img = RgbaImage::from_pixel(5, 5, image::Rgba([128, 128, 128, 255]));
        for n in 1..4 {
            img.put_pixel(n, 1, image::Rgba([0, 0, 0, 255]));
            img.put_pixel(n, 3, image::Rgba([0, 0, 0, 255]));
            img.put_pixel(1, n, image::Rgba([0, 0, 0, 255]));
            img.put_pixel(3, n, image::Rgba([0, 0, 0, 255]));
        }
        replace_background(&mut img, &ColorRule::default());
        assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(2, 2).0, [128, 128, 128, 255]);
    }
    #[test]
    fn export_pdf_and_raster() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.png");
        DynamicImage::new_rgb8(100, 50).save(&input).unwrap();
        for format in ["png", "jpg", "webp", "pdf", "svg embedded"] {
            let settings = Settings {
                format: format.into(),
                ..Default::default()
            };
            let path = export(&input, &dir.path().join("out"), &settings).unwrap();
            assert!(path.metadata().unwrap().len() > 20);
            if format == "pdf" {
                let pdf = lopdf::Document::load(path).unwrap();
                assert_eq!(pdf.get_pages().len(), 1);
            }
        }
        let settings = Settings {
            width: 200,
            ..Default::default()
        };
        assert_eq!(
            prepare(load(&input).unwrap(), &settings)
                .unwrap()
                .dimensions(),
            (200, 100)
        );
    }
    #[test]
    fn object_and_background_can_change_independently() {
        let mut pixels = RgbaImage::from_pixel(5, 5, image::Rgba([200, 200, 200, 255]));
        pixels.put_pixel(2, 2, image::Rgba([100, 100, 100, 255]));
        let object = ColorRule {
            from: [100; 3],
            to: [255; 3],
            edges_only: false,
            seed: Some((0.5, 0.5)),
            ..Default::default()
        };
        let background = ColorRule {
            from: [200; 3],
            to: [20, 30, 40],
            ..Default::default()
        };
        let settings = Settings {
            rules: vec![object, background],
            ..Default::default()
        };
        let output = prepare(DynamicImage::ImageRgba8(pixels), &settings)
            .unwrap()
            .to_rgba8();
        assert_eq!(output.get_pixel(2, 2).0, [255, 255, 255, 255]);
        assert_eq!(output.get_pixel(0, 0).0, [20, 30, 40, 255]);
    }
    #[test]
    fn a_region_protects_matching_colors_elsewhere() {
        let mut pixels = RgbaImage::from_pixel(8, 4, image::Rgba([128, 128, 128, 255]));
        let rule = ColorRule {
            edges_only: false,
            region: Some((0.0, 0.0, 0.4, 1.0)),
            ..Default::default()
        };
        replace_background(&mut pixels, &rule);
        assert_eq!(pixels.get_pixel(1, 1).0, [255, 255, 255, 255]);
        assert_eq!(pixels.get_pixel(6, 1).0, [128, 128, 128, 255]);
    }
}
