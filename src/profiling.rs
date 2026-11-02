use eframe::egui;
use std::time::Instant;

#[test]
#[ignore = "Manual resource comparison using IMAGE_ATLAS_SAMPLE"]
fn profile_resource_usage() {
    let source = std::path::PathBuf::from(
        std::env::var_os("IMAGE_ATLAS_SAMPLE").expect("Set IMAGE_ATLAS_SAMPLE"),
    );
    let ctx = egui::Context::default();
    let started = Instant::now();
    for frame in 0..300 {
        let _ = ctx.run_pass(egui::RawInput::default(), |ctx| {
            egui::Window::new("Resource workload").show(ctx, |ui| {
                ui.heading("Image collection");
                for row in 0..30 {
                    ui.horizontal(|ui| {
                        ui.label(format!("Image {row}"));
                        let _ = ui.button("Edit image");
                        ui.label(format!("{} files ready", frame % 10));
                    });
                }
            });
        });
    }
    let ui_seconds = started.elapsed().as_secs_f64();
    let native = std::env::var_os("IMAGE_ATLAS_PROFILE_NATIVE").is_some();
    let started = Instant::now();
    let mut last_hash = String::new();
    let mut bytes = 0;
    for _ in 0..3 {
        let image = crate::convert::load(&source).unwrap();
        let settings = crate::convert::Settings {
            rules: vec![crate::convert::ColorRule {
                enabled: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let image = crate::convert::prepare(image, &settings);
        let encoded = crate::web::encode_raster(
            image,
            &source,
            &crate::web::Options {
                format: if native {
                    crate::web::Format::Jpeg
                } else {
                    crate::web::Format::Automatic
                },
                max_edge: if native { 0 } else { 1600 },
                ..Default::default()
            },
            false,
        )
        .unwrap();
        bytes = encoded.bytes.len();
        last_hash = crate::model::digest(&encoded.bytes);
    }
    println!(
        "ATLAS_PROFILE {}",
        serde_json::json!({"ui_seconds":ui_seconds,"image_seconds":started.elapsed().as_secs_f64(),"output_bytes":bytes,"output_hash":last_hash})
    );
}

#[test]
#[ignore = "Manual headless language rendering"]
fn render_language_samples() {
    let folder = std::path::PathBuf::from(
        std::env::var_os("IMAGE_ATLAS_RENDER_FOLDER").expect("Set IMAGE_ATLAS_RENDER_FOLDER"),
    );
    std::fs::create_dir_all(&folder).unwrap();
    for (code, heading, text) in [
        (
            "ar",
            "العربية",
            "بِسْمِ — ملف PDF — صفحة 12 من 340 — مرحبا بالعالم",
        ),
        ("fa", "فارسی", "ویرایش تصویر — فایل PDF — تصویر ۱۲۳"),
        ("ur", "اردو", "تصاویر کی فہرست — فائل PDF — 123"),
        ("he", "עברית", "עריכת תמונות — קובץ PDF — שלום 123 עולם"),
        ("hi", "हिन्दी", "चित्र संपादक — क्षेत्र — क्षत्रिय — 123"),
        ("bn", "বাংলা", "ছবি সম্পাদনা — স্বাগতম — 123"),
        ("ta", "தமிழ்", "படங்களைத் திருத்து — வணக்கம் — 123"),
        ("te", "తెలుగు", "చిత్రాలను సవరించండి — స్వాగతం — 123"),
        ("th", "ไทย", "แก้ไขภาพ — บันทึกตำแหน่ง — 123"),
    ] {
        let locale = code.to_owned();
        let mut harness = egui_kittest::Harness::builder()
            .with_size([900.0, 300.0])
            .with_theme(egui::Theme::Dark)
            .with_accessibility_check(false)
            .with_context_setup(move |ctx| {
                crate::i18n::configure_locale_fonts(ctx, &locale);
            })
            .build_ui(|ui| {
                ui.add_space(20.0);
                ui.heading(heading);
                ui.add_space(20.0);
                ui.label(egui::RichText::new(text).size(25.0));
                ui.add_space(15.0);
                ui.add(egui::Label::new(egui::RichText::new(text).size(20.0)).wrap());
            });
        harness.run();
        harness
            .render()
            .unwrap()
            .save(folder.join(format!("{code}.png")))
            .unwrap();
    }
}

#[test]
#[ignore = "Manual local-server mapping comparison"]
fn profile_mapping_schedule() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let website = format!("http://{}/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut threads = Vec::new();
        for _ in 0..25 {
            let (mut socket, _) = listener.accept().unwrap();
            threads.push(std::thread::spawn(move || {
                let mut request = [0; 4096];
                let n = socket.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                let body = if request.starts_with("GET / ") {
                    (0..24)
                        .map(|index| format!("<a href='/product/{index}'>Product</a>"))
                        .collect::<String>()
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                    "<h1>Product</h1><img src='/original.svg'>".into()
                };
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
    });
    let (tx, _) = crossbeam_channel::unbounded();
    let started = Instant::now();
    let map = crate::crawl::map_site(
        crate::crawl::CrawlOptions {
            website,
            render: false,
            sitemap: false,
        },
        crate::model::Cancel::default(),
        tx,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(map.pages, 25);
    println!(
        "ATLAS_MAPPING {}",
        serde_json::json!({"seconds":started.elapsed().as_secs_f64(),"pages":map.pages,"images":map.assets.len()})
    );
}
