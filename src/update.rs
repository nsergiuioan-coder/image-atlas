use crate::model::{UpdateRelease, digest};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MappingNotification {
    #[default]
    Quick,
    Modal,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub language: String,
    pub skip_version: String,
    pub disable_updates: bool,
    pub repository: String,
    #[serde(default)]
    pub mapping_notification: MappingNotification,
}
impl Preferences {
    fn path() -> PathBuf {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
            .join("ImageAtlas/preferences.json")
    }
    pub fn load() -> Self {
        fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| Self {
                repository: "nsergiuioan-coder/image-atlas".into(),
                ..Default::default()
            })
    }
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
pub fn check(repository: &str) -> Result<Option<UpdateRelease>> {
    anyhow::ensure!(
        repository.split('/').count() == 2
            && repository
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_/ .".contains(c))
            && !repository.contains(' '),
        "Enter owner/repository"
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("ImageAtlas")
        .build()?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{repository}/releases/latest"
        ))
        .send()?;
    if response.status() == 404 {
        return Ok(None);
    }
    let data: serde_json::Value = response.error_for_status()?.json()?;
    let version = data["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v');
    if semver::Version::parse(version)? <= semver::Version::parse(env!("CARGO_PKG_VERSION"))? {
        return Ok(None);
    }
    let assets = data["assets"].as_array().context("Release has no files")?;
    let installer = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .is_some_and(|n| n.ends_with("-setup.exe"))
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .context("No Windows installer in release")?;
    let sums = assets
        .iter()
        .find(|a| a["name"] == "SHA256SUMS.txt")
        .and_then(|a| a["browser_download_url"].as_str())
        .context("Missing release checksums")?;
    anyhow::ensure!(
        installer.starts_with("https://github.com/") && sums.starts_with("https://github.com/"),
        "Unexpected update source"
    );
    let text = client.get(sums).send()?.error_for_status()?.text()?;
    let filename = installer.rsplit('/').next().unwrap_or("");
    let checksum = text
        .lines()
        .find(|l| {
            l.split_whitespace()
                .last()
                .is_some_and(|n| n.trim_start_matches('*') == filename)
        })
        .and_then(|l| l.split_whitespace().next())
        .context("Installer checksum missing")?;
    anyhow::ensure!(
        checksum.len() == 64 && checksum.chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid checksum"
    );
    Ok(Some(UpdateRelease {
        version: version.into(),
        notes: data["body"].as_str().unwrap_or("").into(),
        installer: installer.into(),
        checksum: checksum.into(),
    }))
}
pub fn install(release: &UpdateRelease) -> Result<()> {
    anyhow::ensure!(
        release.installer.starts_with("https://github.com/"),
        "Unexpected update source"
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("ImageAtlas")
        .build()?;
    let bytes = client
        .get(&release.installer)
        .send()?
        .error_for_status()?
        .bytes()?;
    anyhow::ensure!(
        digest(&bytes).eq_ignore_ascii_case(&release.checksum),
        "Installer checksum did not match; download discarded"
    );
    let path = std::env::temp_dir().join(format!("ImageAtlas-{}-setup.exe", release.version));
    fs::write(&path, &bytes)?;
    std::process::Command::new(path).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_setting_is_backward_compatible_and_persists() {
        let old = r#"{"skip_version":"","disable_updates":false,"repository":"owner/app"}"#;
        let mut preferences: Preferences = serde_json::from_str(old).unwrap();
        assert_eq!(preferences.mapping_notification, MappingNotification::Quick);
        assert_eq!(preferences.language, "");
        preferences.language = "it".into();
        preferences.mapping_notification = MappingNotification::Modal;
        let restored: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert_eq!(restored.mapping_notification, MappingNotification::Modal);
        assert_eq!(restored.language, "it");
    }
}
