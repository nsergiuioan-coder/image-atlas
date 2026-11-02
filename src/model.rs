use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub url: String,
    pub page: String,
    pub label: String,
    pub folders: Vec<String>,
    pub selected: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SiteMap {
    pub website: String,
    pub pages: usize,
    pub assets: Vec<Asset>,
    pub notices: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Event {
    Log(String),
    Mapped(SiteMap),
    MappingProgress {
        scanned: usize,
        queued: usize,
        images: usize,
        eta_seconds: Option<f64>,
        current_url: String,
    },
    DownloadFinished {
        folder: PathBuf,
        files: Vec<PathBuf>,
        failures: usize,
    },
    Progress {
        done: usize,
        total: usize,
        bytes: u64,
        active: usize,
        speed: f64,
        eta_seconds: Option<f64>,
    },
    Finished {
        folder: PathBuf,
        failures: usize,
    },
    Optimized {
        folder: PathBuf,
        files: usize,
        failures: usize,
        before: u64,
        after: u64,
    },
    Failed(String),
    Update(UpdateRelease),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateRelease {
    pub version: String,
    pub notes: String,
    pub installer: String,
    pub checksum: String,
}

#[derive(Clone, Default)]
pub struct Cancel(pub Arc<AtomicBool>);

impl Cancel {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub fn safe_segment(value: &str) -> String {
    let mut s: String = value
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    s = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches([' ', '.'])
        .chars()
        .take(100)
        .collect();
    s = s.trim_end_matches([' ', '.']).to_owned();
    if s.is_empty() || s == "." || s == ".." {
        s = "Untitled".into();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|c: char| c.is_ascii_digit()))
    {
        s.insert(0, '_');
    }
    s
}

pub fn safe_relative(value: &str) -> bool {
    !Path::new(value).is_absolute()
        && !value.contains(':')
        && Path::new(value)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

pub fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub fn asset_path(asset: &Asset) -> PathBuf {
    let mut out = PathBuf::new();
    for folder in &asset.folders {
        out.push(safe_segment(folder));
    }
    out.push(safe_segment(&asset.label));
    let name = url::Url::parse(&asset.url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "image".into());
    let decoded = percent_encoding::percent_decode_str(&name).decode_utf8_lossy();
    let p = Path::new(decoded.as_ref());
    let stem = safe_segment(p.file_stem().and_then(|s| s.to_str()).unwrap_or("image"));
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("img");
    out.push(format!(
        "{}--{}.{}",
        stem.chars().take(70).collect::<String>(),
        &digest(asset.url.as_bytes())[..8],
        safe_segment(ext)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_names_and_traversal() {
        assert_eq!(safe_segment("CON"), "_CON");
        assert_eq!(safe_segment("a/b:c?"), "a_b_c_");
        assert!(!safe_relative("../escape"));
        assert!(!safe_relative("C:\\escape"));
        assert!(safe_relative("images/photo.png"));
    }
    #[test]
    fn collision_resistant_names() {
        let mut a = Asset {
            url: "https://a.test/pic.png?a=1".into(),
            page: String::new(),
            label: "Product".into(),
            folders: vec!["Shop".into()],
            selected: true,
        };
        let first = asset_path(&a);
        a.url = "https://a.test/pic.png?a=2".into();
        assert_ne!(first, asset_path(&a));
    }
    #[test]
    fn truncated_folder_names_cannot_end_with_windows_trimmed_characters() {
        let name = format!("{} trailing", "x".repeat(99));
        let safe = safe_segment(&name);
        assert_eq!(safe, "x".repeat(99));
        let name = format!("{}.tail", "x".repeat(99));
        assert_eq!(safe_segment(&name), "x".repeat(99));
    }
}
