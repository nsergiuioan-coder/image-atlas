use anyhow::Result;
use image::{DynamicImage, Rgba};
use serde::{Deserialize, Serialize};

pub fn parse_measurement(value: &str) -> Option<f64> {
    value
        .trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    Margin,
    Fixed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub layout: Layout,
    pub width: f32,
    pub height: f32,
    pub margin: f32,
    pub radius: f32,
    pub fill: [u8; 3],
    pub stroke: [u8; 3],
    pub stroke_width: f32,
}

impl Default for Template {
    fn default() -> Self {
        Self {
            layout: Layout::Margin,
            width: 200.0,
            height: 300.0,
            margin: 5.0,
            radius: 0.0,
            fill: [255; 3],
            stroke: [32; 3],
            stroke_width: 0.5,
        }
    }
}

impl Template {
    pub fn ui(&mut self, ui: &mut eframe::egui::Ui, dpi: &mut f32) {
        use eframe::egui;
        ui.selectable_value(
            &mut self.layout,
            Layout::Margin,
            crate::i18n::text("Margin around artwork"),
        );
        ui.selectable_value(
            &mut self.layout,
            Layout::Fixed,
            crate::i18n::text("Fixed width and height"),
        );
        if self.layout == Layout::Fixed {
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::text("Width"));
                ui.add(
                    egui::DragValue::new(&mut self.width)
                        .max_decimals(3)
                        .custom_parser(parse_measurement)
                        .range(0.1..=f32::MAX)
                        .speed(0.5)
                        .suffix(" mm"),
                );
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::text("Height"));
                ui.add(
                    egui::DragValue::new(&mut self.height)
                        .max_decimals(3)
                        .custom_parser(parse_measurement)
                        .range(0.1..=f32::MAX)
                        .speed(0.5)
                        .suffix(" mm"),
                );
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::i18n::text("Margin around artwork"));
            ui.add(
                egui::DragValue::new(&mut self.margin)
                    .max_decimals(3)
                    .custom_parser(parse_measurement)
                    .range(0.0..=f32::MAX)
                    .speed(0.1)
                    .suffix(" mm"),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::i18n::text("Corner radius"));
            ui.add(
                egui::DragValue::new(&mut self.radius)
                    .max_decimals(3)
                    .custom_parser(parse_measurement)
                    .range(0.0..=f32::MAX)
                    .speed(0.1)
                    .suffix(" mm"),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::i18n::text("Fill"));
            ui.color_edit_button_srgb(&mut self.fill);
            ui.label(crate::i18n::text("Stroke"));
            ui.color_edit_button_srgb(&mut self.stroke);
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::i18n::text("Width"));
            ui.add(
                egui::DragValue::new(&mut self.stroke_width)
                    .max_decimals(3)
                    .custom_parser(parse_measurement)
                    .range(0.0..=f32::MAX)
                    .speed(0.1)
                    .suffix(" mm"),
            );
        });
        ui.add(
            egui::DragValue::new(dpi)
                .max_decimals(3)
                .custom_parser(parse_measurement)
                .range(1.0..=65535.0)
                .suffix(" DPI"),
        );
    }

    pub fn apply(&self, image: DynamicImage, dpi: f32) -> Result<DynamicImage> {
        anyhow::ensure!(
            dpi.is_finite() && dpi > 0.0,
            "DPI must be greater than zero"
        );
        for value in [
            self.width,
            self.height,
            self.margin,
            self.radius,
            self.stroke_width,
        ] {
            anyhow::ensure!(
                value.is_finite() && value >= 0.0,
                "Dima measurements must be finite and non-negative"
            );
        }
        let pixels = dpi as f64 / 25.4;
        let margin = self.margin as f64 * pixels;
        let stroke = self.stroke_width as f64 * pixels;
        let inset = margin + stroke;
        let (width, height) = match self.layout {
            Layout::Margin => (
                image.width() as f64 + inset * 2.0,
                image.height() as f64 + inset * 2.0,
            ),
            Layout::Fixed => (self.width as f64 * pixels, self.height as f64 * pixels),
        };
        anyhow::ensure!(
            width >= 1.0 && height >= 1.0 && width <= u32::MAX as f64 && height <= u32::MAX as f64,
            "Invalid dima dimensions"
        );
        anyhow::ensure!(
            width > inset * 2.0 && height > inset * 2.0,
            "The dima is too small for its margin and outline"
        );
        let (width, height) = (width.round() as u32, height.round() as u32);
        check_memory(width, height)?;
        let available_width = (width as f64 - inset * 2.0).floor().max(1.0) as u32;
        let available_height = (height as f64 - inset * 2.0).floor().max(1.0) as u32;
        let image = if self.layout == Layout::Fixed {
            image.resize(
                available_width,
                available_height,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            image
        };
        let mut canvas = image::RgbaImage::from_pixel(
            width,
            height,
            Rgba([self.fill[0], self.fill[1], self.fill[2], 255]),
        );
        image::imageops::overlay(
            &mut canvas,
            &image,
            ((width - image.width()) / 2) as i64,
            ((height - image.height()) / 2) as i64,
        );
        let radius = (self.radius as f64 * pixels).min(width.min(height) as f64 / 2.0);
        let band = ((radius + stroke + 1.0).ceil() as usize).min(width as usize);
        let paint = |x: usize, y: usize, pixel: &mut [u8]| {
            let qx = (x as f64 + 0.5 - width as f64 / 2.0).abs() - (width as f64 / 2.0 - radius);
            let qy = (y as f64 + 0.5 - height as f64 / 2.0).abs() - (height as f64 / 2.0 - radius);
            let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
            let alpha = (0.5 - distance).clamp(0.0, 1.0);
            let outline = (distance + stroke + 0.5).clamp(0.0, 1.0) * (stroke > 0.0) as u8 as f64;
            for (channel, value) in pixel.iter_mut().take(3).enumerate() {
                *value = (*value as f64 * (1.0 - outline) + self.stroke[channel] as f64 * outline)
                    .round() as u8;
            }
            pixel[3] = (pixel[3] as f64 * alpha).round() as u8;
        };
        for (y, row) in canvas
            .as_mut()
            .chunks_exact_mut(width as usize * 4)
            .enumerate()
        {
            if y < band || y + band >= height as usize || band * 2 >= width as usize {
                for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    paint(x, y, pixel);
                }
            } else {
                for (x, pixel) in row[..band * 4]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .enumerate()
                {
                    paint(x, y, pixel);
                }
                let start = width as usize - band;
                for (x, pixel) in row[start * 4..]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .enumerate()
                {
                    paint(start + x, y, pixel);
                }
            }
        }
        Ok(DynamicImage::ImageRgba8(canvas))
    }

    pub fn svg(&self, bytes: &[u8], dpi: f32) -> Result<Vec<u8>> {
        let tree = resvg::usvg::Tree::from_data(bytes, &Default::default())?;
        let size = tree.size();
        let pixels = dpi as f64 / 25.4;
        anyhow::ensure!(pixels.is_finite() && pixels > 0.0, "Invalid DPI");
        let inset = (self.margin + self.stroke_width) as f64 * pixels;
        let (width, height) = match self.layout {
            Layout::Margin => (
                size.width() as f64 + 2.0 * inset,
                size.height() as f64 + 2.0 * inset,
            ),
            Layout::Fixed => (self.width as f64 * pixels, self.height as f64 * pixels),
        };
        anyhow::ensure!(
            width.is_finite() && height.is_finite() && width > 2.0 * inset && height > 2.0 * inset,
            "The dima is too small for its margin and outline"
        );
        let scale = ((width - 2.0 * inset) / size.width() as f64)
            .min((height - 2.0 * inset) / size.height() as f64);
        let content_width = size.width() as f64 * scale;
        let content_height = size.height() as f64 * scale;
        let radius = (self.radius as f64 * pixels).min(width.min(height) / 2.0);
        let stroke = self.stroke_width as f64 * pixels;
        let fill = format!(
            "#{:02x}{:02x}{:02x}",
            self.fill[0], self.fill[1], self.fill[2]
        );
        let line = format!(
            "#{:02x}{:02x}{:02x}",
            self.stroke[0], self.stroke[1], self.stroke[2]
        );
        let mut content = xmltree::Element::parse(bytes)?;
        content
            .attributes
            .insert("x".into(), ((width - content_width) / 2.0).to_string());
        content
            .attributes
            .insert("y".into(), ((height - content_height) / 2.0).to_string());
        content
            .attributes
            .insert("width".into(), content_width.to_string());
        content
            .attributes
            .insert("height".into(), content_height.to_string());
        content
            .attributes
            .entry("viewBox".into())
            .or_insert_with(|| format!("0 0 {} {}", size.width(), size.height()));
        let mut content_bytes = Vec::new();
        content.write_with_config(
            &mut content_bytes,
            xmltree::EmitterConfig::new().write_document_declaration(false),
        )?;
        let content = String::from_utf8(content_bytes)?;
        let outline_width = (width - stroke).max(0.0);
        let outline_height = (height - stroke).max(0.0);
        Ok(format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}mm" height="{}mm" viewBox="0 0 {width} {height}"><defs><clipPath id="atlas-dima-clip"><rect width="{width}" height="{height}" rx="{radius}"/></clipPath></defs><g clip-path="url(#atlas-dima-clip)"><rect width="{width}" height="{height}" rx="{radius}" fill="{fill}"/>{content}</g><rect x="{}" y="{}" width="{outline_width}" height="{outline_height}" rx="{}" fill="none" stroke="{line}" stroke-width="{stroke}"/></svg>"##,width/pixels,height/pixels,stroke/2.0,stroke/2.0,(radius-stroke/2.0).max(0.0)).into_bytes())
    }
}

fn check_memory(width: u32, height: u32) -> Result<()> {
    let bytes = width as u64 * height as u64 * 4;
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut memory = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        anyhow::ensure!(
            unsafe { GlobalMemoryStatusEx(&mut memory) } != 0 && bytes < memory.ullAvailPhys / 3,
            "Not enough free memory for this dima at the selected DPI"
        );
    }
    #[cfg(not(windows))]
    anyhow::ensure!(
        bytes <= isize::MAX as u64,
        "Dima dimensions exceed available memory"
    );
    Ok(())
}

pub fn png_density(bytes: &mut Vec<u8>, dpi: f32) -> Result<()> {
    anyhow::ensure!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 33,
        "Invalid PNG"
    );
    let density = (dpi as f64 / 0.0254).round() as u32;
    let mut data = Vec::from(*b"pHYs");
    data.extend_from_slice(&density.to_be_bytes());
    data.extend_from_slice(&density.to_be_bytes());
    data.push(1);
    let crc = crc32fast::hash(&data);
    let mut chunk = Vec::from(9u32.to_be_bytes());
    chunk.extend(data);
    chunk.extend_from_slice(&crc.to_be_bytes());
    let mut offset = 8;
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into()?) as usize;
        let end = offset
            .checked_add(length + 12)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| anyhow::anyhow!("Invalid PNG chunk"))?;
        if &bytes[offset + 4..offset + 8] == b"pHYs" {
            bytes.drain(offset..end);
            break;
        }
        offset = end;
    }
    bytes.splice(33..33, chunk);
    Ok(())
}

pub fn density(bytes: &mut Vec<u8>, extension: &str, dpi: f32) -> Result<()> {
    anyhow::ensure!(
        dpi.is_finite() && dpi > 0.0,
        "DPI must be greater than zero"
    );
    match extension {
        "png" => png_density(bytes, dpi),
        "jpg" | "jpeg" => {
            if let Some(start) = bytes.windows(5).position(|value| value == b"JFIF\0") {
                anyhow::ensure!(start + 12 <= bytes.len(), "Invalid JPEG density metadata");
                bytes[start + 7] = 1;
                let value = (dpi.round().clamp(1.0, 65535.0) as u16).to_be_bytes();
                bytes[start + 8..start + 10].copy_from_slice(&value);
                bytes[start + 10..start + 12].copy_from_slice(&value);
            }
            Ok(())
        }
        "webp" => webp_density(bytes, dpi),
        _ => Ok(()),
    }
}

fn webp_density(bytes: &mut Vec<u8>, dpi: f32) -> Result<()> {
    anyhow::ensure!(
        bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "Invalid WebP"
    );
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(&*bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    let mut chunks = Vec::new();
    let mut extended = None;
    let mut alpha = false;
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into()?) as usize;
        let end = offset
            .checked_add(8 + size + (size % 2))
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| anyhow::anyhow!("Invalid WebP chunk"))?;
        let kind = &bytes[offset..offset + 4];
        if kind == b"VP8X" {
            anyhow::ensure!(size == 10, "Invalid WebP extended header");
            extended = Some(bytes[offset + 8..offset + 18].to_vec());
        } else if kind != b"EXIF" {
            if kind == b"ALPH" || (kind == b"VP8L" && size >= 5 && bytes[offset + 12] & 0x10 != 0) {
                alpha = true;
            }
            chunks.extend_from_slice(&bytes[offset..end]);
        }
        offset = end;
    }
    let mut header = extended.unwrap_or_else(|| {
        let mut data = vec![0; 10];
        if alpha {
            data[0] |= 0x10;
        }
        data[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
        data[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
        data
    });
    header[0] |= 0x08;
    let mut exif = Vec::from(*b"II\x2a\0\x08\0\0\0");
    exif.extend_from_slice(&3u16.to_le_bytes());
    for (tag, kind, value) in [(0x11au16, 5u16, 50u32), (0x11b, 5, 58), (0x128, 3, 2)] {
        exif.extend_from_slice(&tag.to_le_bytes());
        exif.extend_from_slice(&kind.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        exif.extend_from_slice(&value.to_le_bytes());
    }
    exif.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..2 {
        exif.extend_from_slice(&((dpi as f64 * 1000.0).round() as u32).to_le_bytes());
        exif.extend_from_slice(&1000u32.to_le_bytes());
    }
    let mut result = Vec::from(*b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0");
    result.extend(header);
    result.extend(chunks);
    result.extend_from_slice(b"EXIF");
    result.extend_from_slice(&(exif.len() as u32).to_le_bytes());
    result.extend(exif);
    if result.len() % 2 != 0 {
        result.push(0);
    }
    let length = u32::try_from(result.len() - 8)?;
    result[4..8].copy_from_slice(&length.to_le_bytes());
    *bytes = result;
    Ok(())
}

pub fn svg_size(bytes: &[u8], width: u32, height: u32, dpi: f32) -> Result<Vec<u8>> {
    let mut root = xmltree::Element::parse(bytes)?;
    root.attributes.insert(
        "width".into(),
        format!("{}mm", width as f64 * 25.4 / dpi as f64),
    );
    root.attributes.insert(
        "height".into(),
        format!("{}mm", height as f64 * 25.4 / dpi as f64),
    );
    root.attributes
        .entry("viewBox".into())
        .or_insert_with(|| format!("0 0 {width} {height}"));
    let mut bytes = Vec::new();
    root.write(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    #[test]
    fn measurements_accept_comma_and_dot_without_losing_precision() {
        assert_eq!(parse_measurement("246,944"), Some(246.944));
        assert_eq!(parse_measurement("246.944"), Some(246.944));
        assert_eq!(parse_measurement("0,125"), Some(0.125));
        assert_eq!(parse_measurement("NaN"), None);
    }
    #[test]
    fn margin_adds_mm_and_fixed_size_fits_without_cropping() {
        let source = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            100,
            50,
            Rgba([0, 100, 200, 255]),
        ));
        let mut template = Template {
            margin: 5.0,
            stroke_width: 0.0,
            ..Default::default()
        };
        let margin = template.apply(source.clone(), 254.0).unwrap();
        assert_eq!(margin.dimensions(), (200, 150));
        template.layout = Layout::Fixed;
        template.width = 30.0;
        template.height = 20.0;
        let fixed = template.apply(source, 254.0).unwrap().to_rgba8();
        assert_eq!(fixed.dimensions(), (300, 200));
        assert_eq!(fixed.get_pixel(150, 100).0, [0, 100, 200, 255]);
        assert_eq!(fixed.get_pixel(1, 1).0, [255, 255, 255, 255]);
    }
    #[test]
    fn png_contains_real_dpi_and_repeated_tagging_is_stable() {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::new(10, 10));
        let mut buffer = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        let mut bytes = buffer.into_inner();
        png_density(&mut bytes, 300.0).unwrap();
        let tagged = bytes.clone();
        png_density(&mut bytes, 300.0).unwrap();
        assert_eq!(bytes, tagged);
        assert_eq!(u32::from_be_bytes(bytes[41..45].try_into().unwrap()), 11811);
        assert!(image::load_from_memory(&bytes).is_ok());
    }

    #[test]
    fn vector_dima_keeps_paths_and_explicit_mm() {
        let template = Template {
            layout: Layout::Fixed,
            width: 200.0,
            height: 300.0,
            ..Default::default()
        };
        let input = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\"><path d=\"M0 0 L100 0 L100 100Z\" fill=\"blue\"/></svg>";
        let bytes = template.svg(input, 300.0).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("width=\"200mm\""));
        assert!(text.contains("height=\"300mm\""));
        assert!(text.contains("<path"));
        assert!(!text.contains("data:image"));
        assert!(resvg::usvg::Tree::from_data(&bytes, &Default::default()).is_ok());
    }

    #[test]
    fn webp_dpi_preserves_pixels_and_transparency() {
        use image::ImageDecoder;
        let source = image::RgbaImage::from_fn(20, 10, |x, y| {
            Rgba([x as u8 * 10, y as u8 * 20, 100, if x < 5 { 0 } else { 255 }])
        });
        let mut buffer = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(source.clone())
            .write_to(&mut buffer, image::ImageFormat::WebP)
            .unwrap();
        let mut bytes = buffer.into_inner();
        let original = image::load_from_memory(&bytes).unwrap().to_rgba8();
        density(&mut bytes, "webp", 254.0).unwrap();
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().to_rgba8(),
            original
        );
        let mut decoder = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        let exif = decoder.exif_metadata().unwrap().unwrap();
        drop(decoder);
        assert!(exif.starts_with(b"II\x2a\0"));
        assert_eq!(u32::from_le_bytes(exif[50..54].try_into().unwrap()), 254000);
        let once = bytes.clone();
        density(&mut bytes, "webp", 254.0).unwrap();
        assert_eq!(bytes, once);
    }
}
