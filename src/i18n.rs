use eframe::egui;
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Catalog {
    code: &'static str,
    name: &'static str,
    source: &'static str,
    strings: OnceLock<HashMap<String, String>>,
}

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static CATALOGS: [Catalog; 36] = [
    Catalog {
        code: "en",
        name: "English",
        source: include_str!("../locales/en.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "it",
        name: "Italiano",
        source: include_str!("../locales/it.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "fr",
        name: "Français",
        source: include_str!("../locales/fr.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "de",
        name: "Deutsch",
        source: include_str!("../locales/de.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "es",
        name: "Español",
        source: include_str!("../locales/es.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "pt",
        name: "Português",
        source: include_str!("../locales/pt.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "nl",
        name: "Nederlands",
        source: include_str!("../locales/nl.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "pl",
        name: "Polski",
        source: include_str!("../locales/pl.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "ro",
        name: "Română",
        source: include_str!("../locales/ro.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "cs",
        name: "Čeština",
        source: include_str!("../locales/cs.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "sk",
        name: "Slovenčina",
        source: include_str!("../locales/sk.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "hu",
        name: "Magyar",
        source: include_str!("../locales/hu.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "sv",
        name: "Svenska",
        source: include_str!("../locales/sv.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "da",
        name: "Dansk",
        source: include_str!("../locales/da.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "fi",
        name: "Suomi",
        source: include_str!("../locales/fi.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "no",
        name: "Norsk",
        source: include_str!("../locales/no.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "el",
        name: "Greek",
        source: include_str!("../locales/el.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "ru",
        name: "Russian",
        source: include_str!("../locales/ru.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "uk",
        name: "Ukrainian",
        source: include_str!("../locales/uk.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "tr",
        name: "Türkçe",
        source: include_str!("../locales/tr.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "id",
        name: "Indonesia",
        source: include_str!("../locales/id.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "ms",
        name: "Melayu",
        source: include_str!("../locales/ms.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "vi",
        name: "Tiếng Việt",
        source: include_str!("../locales/vi.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "th",
        name: "Thai",
        source: include_str!("../locales/th.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "zh-CN",
        name: "Chinese (Simplified)",
        source: include_str!("../locales/zh-CN.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "zh-TW",
        name: "Chinese (Traditional)",
        source: include_str!("../locales/zh-TW.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "ja",
        name: "Japanese",
        source: include_str!("../locales/ja.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "ko",
        name: "Korean",
        source: include_str!("../locales/ko.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "hi",
        name: "Hindi",
        source: include_str!("../locales/hi.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "bn",
        name: "Bengali",
        source: include_str!("../locales/bn.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "ta",
        name: "Tamil",
        source: include_str!("../locales/ta.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "te",
        name: "Telugu",
        source: include_str!("../locales/te.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "ar",
        name: "Arabic",
        source: include_str!("../locales/ar.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "fa",
        name: "Persian",
        source: include_str!("../locales/fa.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "ur",
        name: "Urdu",
        source: include_str!("../locales/ur.json"),
        strings: OnceLock::new(),
    },
    Catalog {
        code: "he",
        name: "Hebrew",
        source: include_str!("../locales/he.json"),
        strings: OnceLock::new(),
    },
];

pub fn languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    CATALOGS.iter().map(|entry| (entry.code, entry.name))
}

fn resolve(code: &str) -> usize {
    let code = code.replace('_', "-").to_lowercase();
    let normalized = if code.starts_with("zh") {
        if code.contains("tw") || code.contains("hk") || code.contains("hant") {
            "zh-tw"
        } else {
            "zh-cn"
        }
    } else {
        code.split('-').next().unwrap_or("en")
    };
    CATALOGS
        .iter()
        .position(|entry| entry.code.eq_ignore_ascii_case(normalized))
        .unwrap_or(0)
}

pub fn set_language(code: &str) {
    let code = if code.is_empty() || code == "system" {
        system_language()
    } else {
        code.to_owned()
    };
    ACTIVE.store(resolve(&code), Ordering::Relaxed);
}

#[cfg(windows)]
fn system_language() -> String {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
        fn LCIDToLocaleName(locale: u32, name: *mut u16, length: i32, flags: u32) -> i32;
    }
    let mut name = [0u16; 85];
    let length = unsafe {
        LCIDToLocaleName(
            GetUserDefaultUILanguage() as u32,
            name.as_mut_ptr(),
            name.len() as i32,
            0,
        )
    };
    if length > 1 {
        String::from_utf16_lossy(&name[..length as usize - 1])
    } else {
        "en".into()
    }
}

#[cfg(not(windows))]
fn system_language() -> String {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "en".into())
        .split('.')
        .next()
        .unwrap_or("en")
        .into()
}

pub fn text(source: &str) -> &str {
    let index = ACTIVE.load(Ordering::Relaxed);
    if index == 0 {
        return source;
    }
    let strings = CATALOGS[index].strings.get_or_init(|| {
        serde_json::from_str(CATALOGS[index].source).expect("Validated language catalog")
    });
    strings.get(source).map(String::as_str).unwrap_or(source)
}

fn parts(template: &str) -> Vec<(String, Option<String>)> {
    let mut parts = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let end = start + end + 1;
        parts.push((rest[..start].into(), Some(rest[start..end].into())));
        rest = &rest[end..];
    }
    parts.push((rest.into(), None));
    parts
}

fn substitute(source: &str, translated: &str, rendered: &str) -> String {
    if source == translated {
        return rendered.to_owned();
    }
    let source_parts = parts(source);
    let mut rest = rendered;
    let mut values: Vec<(&str, &str)> = Vec::new();
    for (index, (literal, token)) in source_parts.iter().enumerate() {
        let Some(after_literal) = rest.strip_prefix(literal.as_str()) else {
            return rendered.to_owned();
        };
        rest = after_literal;
        if let Some(token) = token {
            let next = &source_parts[index + 1].0;
            let end = if next.is_empty() {
                rest.len()
            } else if let Some(end) = rest.find(next) {
                end
            } else {
                return rendered.to_owned();
            };
            values.push((token, &rest[..end]));
            rest = &rest[end..];
        }
    }
    let mut used = vec![false; values.len()];
    let mut result = String::new();
    for (literal, token) in parts(translated) {
        result.push_str(&literal);
        if let Some(token) = token {
            let Some(index) = values
                .iter()
                .enumerate()
                .position(|(index, (candidate, _))| !used[index] && *candidate == token)
            else {
                return rendered.to_owned();
            };
            used[index] = true;
            result.push_str(values[index].1);
        }
    }
    result
}

pub fn formatted(source: &str, rendered: String) -> String {
    let translated = text(source);
    if source == translated {
        rendered
    } else {
        substitute(source, translated, &rendered)
    }
}

pub fn configure_fonts(ctx: &egui::Context) {
    configure_locale_fonts(ctx, CATALOGS[ACTIVE.load(Ordering::Relaxed)].code);
}

pub(crate) fn configure_locale_fonts(ctx: &egui::Context, locale: &str) {
    static PROVIDER: OnceLock<std::sync::Arc<egui_system_font_provider::SystemFontProvider>> =
        OnceLock::new();
    let provider = PROVIDER
        .get_or_init(|| std::sync::Arc::new(egui_system_font_provider::SystemFontProvider::new()));
    ctx.set_font_providers(vec![provider.clone()]);
    let mut fonts = egui::FontDefinitions::default();
    let fallback = match locale {
        "zh-CN" => Some("msyh.ttc"),
        "zh-TW" => Some("msjh.ttc"),
        "ja" => Some("YuGothR.ttc"),
        "ko" => Some("malgun.ttf"),
        "hi" | "bn" | "ta" | "te" => Some("Nirmala.ttc"),
        "th" => Some("LeelawUI.ttf"),
        _ => None,
    };
    for (name, filename, first) in [
        ("segoe", Some("segoeui.ttf"), true),
        ("locale", fallback, false),
        ("windows-icons", Some("segmdl2.ttf"), false),
    ] {
        let Some(filename) = filename else {
            continue;
        };
        let path = std::path::PathBuf::from(
            std::env::var_os("WINDIR").unwrap_or_else(|| "C:/Windows".into()),
        )
        .join("Fonts")
        .join(filename);
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                name.into(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                let names = fonts.families.entry(family).or_default();
                if first {
                    names.insert(0, name.into());
                } else {
                    names.push(name.into());
                }
            }
        }
    }
    ctx.set_fonts(fonts);
}

pub fn trailing_layout(align: egui::Align) -> egui::Layout {
    if is_rtl() {
        egui::Layout::left_to_right(align)
    } else {
        egui::Layout::right_to_left(align)
    }
}

pub fn text_alignment() -> egui::Align {
    if is_rtl() {
        egui::Align::RIGHT
    } else {
        egui::Align::LEFT
    }
}

pub fn is_rtl() -> bool {
    matches!(
        CATALOGS[ACTIVE.load(Ordering::Relaxed)].code,
        "ar" | "fa" | "ur" | "he"
    )
}

#[macro_export]
macro_rules! tr {
    ($template:literal $(, $argument:expr)* $(,)?) => {
        $crate::i18n::formatted($template, format!($template $(, $argument)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout_sample(ctx: &egui::Context, text: &str, width: f32) -> std::sync::Arc<egui::Galley> {
        let mut result = None;
        let _ = ctx.run_pass(egui::RawInput::default(), |ctx| {
            result = Some(ctx.fonts_mut(|fonts| {
                fonts.layout(
                    text.into(),
                    egui::FontId::proportional(20.0),
                    egui::Color32::WHITE,
                    width,
                )
            }));
        });
        result.unwrap()
    }
    #[test]
    #[cfg(windows)]
    fn rtl_text_field_accepts_mixed_text_and_edits_at_the_visual_cursor() {
        let ctx = egui::Context::default();
        configure_locale_fonts(&ctx, "ar");
        let mut text = String::new();
        let mut render = |events, focus| {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut text).id_salt("mixed-text-test"));
                    if focus {
                        response.request_focus();
                    }
                },
            );
        };
        render(Vec::new(), true);
        render(vec![egui::Event::Text("ملف PDF".into())], false);
        render(
            vec![
                egui::Event::Key {
                    key: egui::Key::ArrowLeft,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
                egui::Event::Text("x".into()),
            ],
            false,
        );
        assert_eq!(text, "ملف PDxF");
    }
    #[test]
    #[cfg(windows)]
    fn every_catalog_renders_with_real_fonts_without_missing_glyphs() {
        for catalog in &CATALOGS {
            let ctx = egui::Context::default();
            configure_locale_fonts(&ctx, catalog.code);
            ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Panic);
            let values: HashMap<String, String> = serde_json::from_str(catalog.source).unwrap();
            for value in values.values() {
                let sample: String = parts(value)
                    .into_iter()
                    .map(|(literal, token)| literal + if token.is_some() { "123" } else { "" })
                    .collect();
                let galley = layout_sample(&ctx, &sample, 400.0);
                assert_eq!(
                    galley.end().index.0,
                    sample.chars().count(),
                    "{}: {}",
                    catalog.code,
                    sample
                );
                let actual: String = galley.rows.iter().map(|row| row.text()).collect();
                assert_eq!(actual, sample.replace('\n', ""), "{}", catalog.code);
            }
        }
    }
    #[test]
    #[cfg(windows)]
    fn mixed_rtl_text_preserves_digits_marks_logical_characters_and_end_cursor() {
        let ctx = egui::Context::default();
        configure_locale_fonts(&ctx, "ar");
        for text in [
            "ملف PDF",
            "بِسْمِ",
            "صفحة 12 من 340",
            "Open ملف now",
            "שלום 123 עולם",
            "لا",
            "مرحبا بالعالم",
            "தமிழ்",
            "क्षेत्र",
            "বাংলা",
            "తెలుగు",
            "ภาษาไทย",
        ] {
            let galley = layout_sample(&ctx, text, f32::INFINITY);
            assert_eq!(galley.rows[0].text(), text);
            assert_eq!(galley.end().index.0, text.chars().count());
            assert!(galley.mesh_bounds.is_finite());
        }
        let galley = layout_sample(&ctx, "ملف PDF", f32::INFINITY);
        let row = &galley.rows[0];
        let last = row.glyphs.last().unwrap();
        assert!(!last.is_rtl());
        assert!(
            (row.x_offset(egui::epaint::text::CharIndex(row.glyphs.len())) - last.max_x()).abs()
                < 0.1
        );
        let galley = layout_sample(&ctx, "אב 123", f32::INFINITY);
        let row = &galley.rows[0];
        let x = |chr| {
            row.glyphs
                .iter()
                .find(|glyph| glyph.chr == chr)
                .unwrap()
                .pos
                .x
        };
        assert!(x('1') < x('2') && x('2') < x('3'));
        assert!(x('ב') < x('א'));
        assert!(x('3') < x('ב'));
        let mut cursor = egui::epaint::text::cursor::CCursor::new(0);
        for _ in 0..row.glyphs.len() {
            let next = galley.cursor_left_one_character(&cursor);
            assert!(galley.pos_from_cursor(next).min.x <= galley.pos_from_cursor(cursor).min.x);
            cursor = next;
        }
    }
    #[test]
    fn locale_matching_covers_regional_variants() {
        assert_eq!(CATALOGS[resolve("it-IT")].code, "it");
        assert_eq!(CATALOGS[resolve("zh-Hant-HK")].code, "zh-TW");
        assert_eq!(CATALOGS[resolve("pt_BR")].code, "pt");
        assert_eq!(resolve("unknown"), 0);
    }
    #[test]
    fn translated_templates_preserve_values_and_do_not_translate_file_names() {
        assert_eq!(
            substitute("Saved {}", "Salvato {}", "Saved Download.svg"),
            "Salvato Download.svg"
        );
        assert_eq!(
            substitute(
                "Download {count} images",
                "Scarica {count} immagini",
                "Download 35 images"
            ),
            "Scarica 35 immagini"
        );
        assert_eq!(
            substitute("ETA ~{}m {:02}s", "Tempo ~{}m {:02}s", "ETA ~2m 05s"),
            "Tempo ~2m 05s"
        );
        assert_eq!(
            substitute(
                "Skipped {}: {e}",
                "{e}: {} ignorato",
                "Skipped x.svg: broken"
            ),
            "broken: x.svg ignorato"
        );
    }
    #[test]
    fn catalogs_are_valid_and_preserve_format_placeholders() {
        let source: HashMap<String, String> = serde_json::from_str(CATALOGS[0].source).unwrap();
        for catalog in &CATALOGS {
            let values: HashMap<String, String> = serde_json::from_str(catalog.source).unwrap();
            assert_eq!(
                values.len(),
                source.len(),
                "Incomplete catalog: {}",
                catalog.code
            );
            for (key, value) in values {
                assert!(
                    source.contains_key(&key),
                    "Unknown key in {}: {}",
                    catalog.code,
                    key
                );
                let mut expected: Vec<_> = parts(&key)
                    .into_iter()
                    .filter_map(|(_, token)| token)
                    .collect();
                let mut actual: Vec<_> = parts(&value)
                    .into_iter()
                    .filter_map(|(_, token)| token)
                    .collect();
                expected.sort();
                actual.sort();
                assert_eq!(
                    expected, actual,
                    "Invalid placeholders in {}: {}",
                    catalog.code, key
                );
            }
        }
    }
}
