mod engine;
mod studio;

use crate::{model::Cancel, web::Source};
use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
pub use studio::Studio;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    #[default]
    Photo,
    Artwork,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Options {
    mode: Mode,
    scale: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Photo,
            scale: 2,
        }
    }
}

enum Event {
    Download(u64, Option<u64>),
    Progress(usize, usize, f64, Option<f64>),
    Preview(DynamicImage, DynamicImage),
    Completed {
        folder: PathBuf,
        files: usize,
        failed: Vec<Source>,
    },
    Failed(String),
}

fn digest(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn input(source: &Source, settings: Option<&crate::convert::Settings>) -> Result<DynamicImage> {
    let image = match &source.edited_svg {
        Some(bytes) => crate::web::render_svg(bytes, &source.path, 0),
        None => crate::convert::load(&source.path),
    }?;
    match settings {
        Some(settings) => {
            let mut processing = settings.clone();
            processing.template = None;
            crate::convert::prepare(image, &processing)
        }
        None => Ok(image),
    }
}

fn vector(source: &Source) -> Result<Option<Vec<u8>>> {
    let mut file = fs::File::open(&source.path)?;
    let mut head = [0; 8192];
    let length = file.read(&mut head)?;
    if crate::download::detected_extension(&head[..length], "").as_deref() == Some("svg") {
        return Ok(Some(
            source.edited_svg.clone().unwrap_or(fs::read(&source.path)?),
        ));
    }
    Ok(None)
}

fn ensure_memory(width: u32, height: u32) -> Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut memory = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if unsafe { GlobalMemoryStatusEx(&mut memory) } != 0 {
            let required = width as u64 * height as u64 * 4 * 16 * 3;
            anyhow::ensure!(
                required < memory.ullAvailPhys / 2,
                "Not enough free memory for a 4× AI pass on this image. Close other applications or use a smaller source."
            );
        }
    }
    #[cfg(not(windows))]
    let _ = (width, height);
    Ok(())
}

fn enhance(
    source: &Source,
    options: Options,
    root: Option<&Path>,
    cancel: &Cancel,
    settings: Option<&crate::convert::Settings>,
) -> Result<crate::web::Encoded> {
    anyhow::ensure!([2, 4].contains(&options.scale), "Choose 2× or 4×");
    anyhow::ensure!(!cancel.stopped(), "Stopped");
    let input_bytes = source.path.metadata()?.len();
    if settings.is_none()
        && let Some(bytes) = vector(source)?
    {
        return Ok(crate::web::Encoded {
            bytes,
            extension: "svg".into(),
            input_bytes,
        });
    }
    if options.mode == Mode::Photo {
        let (width, height) = crate::convert::dimensions(&source.path)?;
        ensure_memory(width, height)?;
    }
    let image = input(source, settings)?;
    let work = tempfile::tempdir()?;
    let source_path = work.path().join("source.png");
    let result_path = work.path().join("result.png");
    let mut encoded = match options.mode {
        Mode::Artwork => {
            image.save_with_format(&source_path, image::ImageFormat::Png)?;
            let svg = work.path().join("result.svg");
            vtracer::convert_image_to_svg(&source_path, &svg, vtracer::Config::default())
                .map_err(anyhow::Error::msg)?;
            anyhow::ensure!(!cancel.stopped(), "Stopped");
            let bytes = fs::read(svg)?;
            resvg::usvg::Tree::from_data(&bytes, &Default::default())?;
            Ok::<_, anyhow::Error>(crate::web::Encoded {
                bytes,
                extension: "svg".into(),
                input_bytes,
            })
        }
        Mode::Photo => {
            let root = root.context("Download the AI engine first")?;
            let dimensions = image.dimensions();
            ensure_memory(dimensions.0, dimensions.1)?;
            let alpha = image.has_alpha().then(|| image.to_rgba8());
            DynamicImage::ImageRgb8(crate::convert::flatten_white(&image))
                .save_with_format(&source_path, image::ImageFormat::Png)?;
            engine::run(
                root,
                &source_path,
                &result_path,
                cancel,
                &work.path().join("engine.log"),
            )?;
            let mut result = crate::convert::load(&result_path)?;
            anyhow::ensure!(
                result.dimensions()
                    == (
                        dimensions
                            .0
                            .checked_mul(4)
                            .context("Image width overflow")?,
                        dimensions
                            .1
                            .checked_mul(4)
                            .context("Image height overflow")?
                    ),
                "AI engine returned incorrect dimensions"
            );
            let rgb = result.to_rgb8();
            anyhow::ensure!(
                rgb.as_raw().iter().any(|value| *value > 0)
                    || image.to_rgb8().as_raw().iter().all(|value| *value == 0),
                "AI engine produced an empty black image"
            );
            if options.scale == 2 {
                result = result.resize_exact(
                    dimensions.0 * 2,
                    dimensions.1 * 2,
                    image::imageops::FilterType::Lanczos3,
                );
            }
            if let Some(alpha) = alpha {
                let mask = image::GrayImage::from_fn(alpha.width(), alpha.height(), |x, y| {
                    image::Luma([alpha.get_pixel(x, y)[3]])
                });
                let mask = image::imageops::resize(
                    &mask,
                    result.width(),
                    result.height(),
                    image::imageops::FilterType::Triangle,
                );
                let mut rgba = result.into_rgba8();
                for (pixel, mask) in rgba.pixels_mut().zip(mask.pixels()) {
                    pixel[3] = mask[0];
                }
                result = DynamicImage::ImageRgba8(rgba);
            }
            let mut buffer = std::io::Cursor::new(Vec::new());
            result.write_to(&mut buffer, image::ImageFormat::Png)?;
            Ok::<_, anyhow::Error>(crate::web::Encoded {
                bytes: buffer.into_inner(),
                extension: "png".into(),
                input_bytes,
            })
        }
    }?;
    if let Some(settings) = settings {
        if let Some(template) = &settings.template {
            if encoded.extension == "svg" {
                encoded.bytes = template.svg(&encoded.bytes, settings.dpi)?;
            } else {
                let image = image::load_from_memory(&encoded.bytes)?;
                let image = template.apply(image, settings.dpi)?;
                let mut bytes = std::io::Cursor::new(Vec::new());
                image.write_to(&mut bytes, image::ImageFormat::Png)?;
                encoded.bytes = bytes.into_inner();
            }
        }
        if encoded.extension == "png" {
            crate::template::png_density(&mut encoded.bytes, settings.dpi)?;
        }
    }
    Ok(encoded)
}

#[derive(Serialize, Deserialize)]
struct Saved {
    key: String,
    path: PathBuf,
    hash: String,
}

fn key(
    source: &Source,
    options: Options,
    settings: Option<&crate::convert::Settings>,
) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(source.path.to_string_lossy().as_bytes());
    hash.update(digest(&source.path)?);
    hash.update(serde_json::to_vec(&options)?);
    hash.update(serde_json::to_vec(&settings)?);
    if let Some(bytes) = &source.edited_svg {
        hash.update(bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn export(
    sources: Vec<Source>,
    folder: PathBuf,
    options: Options,
    cancel: Cancel,
    events: crossbeam_channel::Sender<Event>,
    settings: Option<crate::convert::Settings>,
) -> Result<()> {
    fs::create_dir_all(&folder)?;
    let root = if options.mode == Mode::Photo
        && (settings.is_some()
            || sources
                .iter()
                .any(|source| vector(source).ok().flatten().is_none()))
    {
        Some(engine::install(&cancel, |bytes, total| {
            let _ = events.send(Event::Download(bytes, total));
        })?)
    } else {
        None
    };
    let journal = folder.join(".atlas-upscale-state.jsonl");
    let journal_text = fs::read_to_string(&journal).unwrap_or_default();
    let previous: std::collections::HashMap<String, Saved> = journal_text
        .lines()
        .filter_map(|line| serde_json::from_str::<Saved>(line).ok())
        .map(|saved| (saved.key.clone(), saved))
        .collect();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal)?;
    if !journal_text.is_empty() && !journal_text.ends_with('\n') {
        writeln!(log)?;
    }
    let common = crate::web::common_root(
        &sources
            .iter()
            .map(|source| source.path.clone())
            .collect::<Vec<_>>(),
    );
    let total = sources.len();
    let mut estimate = crate::progress::Exports::new(
        sources
            .iter()
            .map(|source| crate::progress::Work::file(&source.path))
            .collect(),
    );
    let mut files = 0;
    let mut failed = Vec::new();
    let mut errors = Vec::new();
    for (index, source) in sources.into_iter().enumerate() {
        if cancel.stopped() {
            break;
        }
        let started = Instant::now();
        let result = (|| -> Result<()> {
            let key = key(&source, options, settings.as_ref())?;
            if let Some(saved) = previous.get(&key)
                && saved.path.is_file()
                && saved
                    .path
                    .canonicalize()?
                    .starts_with(folder.canonicalize()?)
                && digest(&saved.path)? == saved.hash
            {
                return Ok(());
            }
            let encoded = enhance(
                &source,
                options,
                root.as_deref(),
                &cancel,
                settings.as_ref(),
            )?;
            anyhow::ensure!(!cancel.stopped(), "Stopped");
            let destination = crate::web::destination(&source.path, common.as_deref(), &folder);
            let output = crate::web::save(encoded, &source.path, &destination)?.path;
            let saved = Saved {
                key,
                hash: digest(&output)?,
                path: output,
            };
            serde_json::to_writer(&mut log, &saved)?;
            writeln!(log)?;
            log.sync_data()?;
            Ok(())
        })();
        let success = result.is_ok();
        if let Err(error) = result {
            if cancel.stopped() {
                break;
            }
            errors.push(format!("{}: {error}", source.path.display()));
            failed.push(source);
        } else {
            files += 1;
        }
        let (rate, eta) = estimate.finish(started.elapsed(), success);
        let _ = events.send(Event::Progress(index + 1, total, rate, eta));
    }
    fs::write(folder.join("upscale-errors.txt"), errors.join("\n"))?;
    let _ = events.send(Event::Completed {
        folder,
        files,
        failed,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn artwork() -> (tempfile::TempDir, Source) {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("sign.png");
        image::RgbaImage::from_fn(32, 32, |x, y| {
            if x < 8 || y < 8 {
                image::Rgba([255, 255, 255, 0])
            } else {
                image::Rgba([0, 100, 200, 255])
            }
        })
        .save(&path)
        .unwrap();
        (
            folder,
            Source {
                path,
                edited_svg: None,
            },
        )
    }
    #[test]
    fn artwork_export_is_real_vector_and_keeps_the_original() {
        let (_folder, source) = artwork();
        let original = fs::read(&source.path).unwrap();
        let encoded = enhance(
            &source,
            Options {
                mode: Mode::Artwork,
                scale: 2,
            },
            None,
            &Cancel::default(),
            None,
        )
        .unwrap();
        assert_eq!(encoded.extension, "svg");
        let text = String::from_utf8(encoded.bytes).unwrap();
        assert!(text.contains("<path"));
        assert!(!text.contains("data:image"));
        assert_eq!(fs::read(&source.path).unwrap(), original);
    }

    #[test]
    fn batch_color_edits_are_applied_before_upscaling() {
        let (_folder, source) = artwork();
        let settings = crate::convert::Settings {
            rules: vec![crate::convert::ColorRule {
                from: [0, 100, 200],
                to: [255, 255, 255],
                edges_only: false,
                ..Default::default()
            }],
            ..Default::default()
        };
        let encoded = enhance(
            &source,
            Options {
                mode: Mode::Artwork,
                scale: 2,
            },
            None,
            &Cancel::default(),
            Some(&settings),
        )
        .unwrap();
        let rendered = crate::web::preview(&encoded, &source.path)
            .unwrap()
            .to_rgba8();
        assert_eq!(rendered.get_pixel(20, 20).0, [255, 255, 255, 255]);
        assert_eq!(
            crate::convert::load(&source.path)
                .unwrap()
                .to_rgba8()
                .get_pixel(20, 20)
                .0,
            [0, 100, 200, 255]
        );
    }

    #[test]
    fn interrupted_journal_keeps_later_completed_exports_reusable() {
        let (folder, source) = artwork();
        let output = folder.path().join("output");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join(".atlas-upscale-state.jsonl"), "{broken record").unwrap();
        for _ in 0..2 {
            let (tx, _) = crossbeam_channel::unbounded();
            export(
                vec![source.clone()],
                output.clone(),
                Options {
                    mode: Mode::Artwork,
                    scale: 2,
                },
                Cancel::default(),
                tx,
                None,
            )
            .unwrap();
        }
        assert!(output.join("sign.svg").exists());
        assert!(!output.join("sign-2.svg").exists());
    }
    #[test]
    fn vectors_stay_vectors_and_resume_reuses_verified_outputs() {
        let (folder, mut source) = artwork();
        source.path = folder.path().join("vector.svg");
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\"/></svg>";
        fs::write(&source.path, svg).unwrap();
        let output = folder.path().join("output");
        for _ in 0..2 {
            let (tx, _) = crossbeam_channel::unbounded();
            export(
                vec![source.clone()],
                output.clone(),
                Options::default(),
                Cancel::default(),
                tx,
                None,
            )
            .unwrap();
        }
        assert_eq!(fs::read(output.join("vector.svg")).unwrap(), svg);
        assert!(!output.join("vector-2.svg").exists());
        assert_eq!(fs::read(&source.path).unwrap(), svg);
    }
    #[test]
    #[ignore = "Manual local GPU integration test"]
    fn real_ai_enlarges_pixels_and_preserves_transparency() {
        let (_folder, source) = artwork();
        let root = PathBuf::from(std::env::var_os("ATLAS_AI_ENGINE").unwrap());
        for scale in [2, 4] {
            let encoded = enhance(
                &source,
                Options {
                    mode: Mode::Photo,
                    scale,
                },
                Some(&root),
                &Cancel::default(),
                None,
            )
            .unwrap();
            let image = image::load_from_memory(&encoded.bytes).unwrap().to_rgba8();
            assert_eq!(image.dimensions(), (32 * scale, 32 * scale));
            assert_eq!(image.get_pixel(0, 0)[3], 0);
            assert_eq!(image.get_pixel(31 * scale, 31 * scale)[3], 255);
        }
    }
}
