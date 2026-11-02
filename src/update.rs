use crate::model::{Cancel, Event, UpdateRelease};
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
    Ok(latest(repository)?.filter(|release| {
        semver::Version::parse(&release.version).is_ok_and(|version| {
            version > semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("Package version")
        })
    }))
}

pub fn latest(repository: &str) -> Result<Option<UpdateRelease>> {
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
    semver::Version::parse(version)?;
    let assets = data["assets"].as_array().context("Release has no files")?;
    let archive = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .is_some_and(|n| n == format!("ImageAtlas-{version}-portable.zip"))
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .context("No Windows update package in release")?;
    let sums = assets
        .iter()
        .find(|a| a["name"] == "SHA256SUMS.txt")
        .and_then(|a| a["browser_download_url"].as_str())
        .context("Missing release checksums")?;
    anyhow::ensure!(
        archive.starts_with("https://github.com/") && sums.starts_with("https://github.com/"),
        "Unexpected update source"
    );
    let text = client.get(sums).send()?.error_for_status()?.text()?;
    let filename = archive.rsplit('/').next().unwrap_or("");
    let checksum = text
        .lines()
        .find(|l| {
            l.split_whitespace()
                .last()
                .is_some_and(|n| n.trim_start_matches('*') == filename)
        })
        .and_then(|l| l.split_whitespace().next())
        .context("Update checksum missing")?;
    anyhow::ensure!(
        checksum.len() == 64 && checksum.chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid checksum"
    );
    Ok(Some(UpdateRelease {
        version: version.into(),
        notes: data["body"].as_str().unwrap_or("").into(),
        archive: archive.into(),
        checksum: checksum.into(),
    }))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prepared {
    pub plan: PathBuf,
    pub version: String,
}

#[derive(Serialize, Deserialize)]
struct Plan {
    target: PathBuf,
    executable: std::ffi::OsString,
    files: Vec<(PathBuf, String)>,
    version: String,
    processes: Vec<u32>,
}

fn file_hash(path: &std::path::Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn allowed_file(path: &std::path::Path) -> bool {
    use std::path::Component;
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return false;
    }
    match path
        .components()
        .next()
        .and_then(|part| part.as_os_str().to_str())
    {
        Some("ImageAtlas.exe" | "LICENSE" | "NOTICE" | "ATTRIBUZIONI.txt" | "README.md") => {
            path.components().count() == 1
        }
        Some("THIRD-PARTY-LICENSES" | "docs") => path.components().count() > 1,
        _ => false,
    }
}

fn extract_package(
    archive: &std::path::Path,
    stage: &std::path::Path,
) -> Result<Vec<(PathBuf, String)>> {
    use std::io::Write;
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
    let mut files = Vec::new();
    let mut names = std::collections::HashSet::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        anyhow::ensure!(
            entry
                .unix_mode()
                .is_none_or(|mode| mode & 0o170000 != 0o120000),
            "Update contains a symbolic link"
        );
        let path = entry.enclosed_name().context("Unsafe update path")?;
        anyhow::ensure!(
            allowed_file(&path),
            "Unexpected file in update: {}",
            path.display()
        );
        anyhow::ensure!(
            names.insert(path.to_string_lossy().to_lowercase()),
            "Duplicate update file"
        );
        let output = stage.join(&path);
        fs::create_dir_all(output.parent().context("Missing update folder")?)?;
        let mut file = fs::File::create(&output)?;
        std::io::copy(&mut entry, &mut file)?;
        file.flush()?;
        file.sync_all()?;
        files.push((path, file_hash(&output)?));
    }
    let executable = stage.join("ImageAtlas.exe");
    let header = fs::read(&executable).context("Update does not contain ImageAtlas.exe")?;
    anyhow::ensure!(
        valid_executable(&header),
        "Update does not contain a valid Windows executable"
    );
    Ok(files)
}

fn valid_executable(bytes: &[u8]) -> bool {
    let Some(offset) = bytes
        .get(60..64)
        .and_then(|v| <[u8; 4]>::try_from(v).ok())
        .map(u32::from_le_bytes)
    else {
        return false;
    };
    bytes.starts_with(b"MZ") && bytes.get(offset as usize..offset as usize + 4) == Some(b"PE\0\0")
}

pub fn prepare(
    release: &UpdateRelease,
    cancel: Cancel,
    events: crossbeam_channel::Sender<Event>,
) -> Result<Prepared> {
    prepare_at(
        release,
        cancel,
        events,
        &std::env::current_exe()?,
        session_processes(),
    )
}

fn session_processes() -> Vec<u32> {
    let mut processes = vec![std::process::id()];
    if let Ok(pid) = std::env::var("IMAGE_ATLAS_SUPERVISOR_PID")
        && let Ok(pid) = pid.parse::<u32>()
    {
        processes.push(pid);
    }
    processes
}

pub fn prepare_at(
    release: &UpdateRelease,
    cancel: Cancel,
    events: crossbeam_channel::Sender<Event>,
    target: &std::path::Path,
    processes: Vec<u32>,
) -> Result<Prepared> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let url = url::Url::parse(&release.archive)?;
    anyhow::ensure!(
        url.scheme() == "https" && url.host_str() == Some("github.com"),
        "Unexpected update source"
    );
    let cache = crate::recovery::folder().join("updates");
    fs::create_dir_all(&cache)?;
    let stage = tempfile::Builder::new()
        .prefix("update-")
        .tempdir_in(cache)?;
    let archive = stage.path().join("package.zip");
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .user_agent("ImageAtlas")
        .build()?;
    let mut response = client.get(&release.archive).send()?.error_for_status()?;
    let total = response.content_length();
    let mut file = fs::File::create(&archive)?;
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 65536];
    let mut last_progress = std::time::Instant::now();
    loop {
        anyhow::ensure!(!cancel.stopped(), "Update cancelled");
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
        bytes += count as u64;
        if last_progress.elapsed() >= Duration::from_millis(100) {
            let _ = events.send(Event::UpdateProgress { bytes, total });
            last_progress = std::time::Instant::now();
        }
    }
    file.sync_all()?;
    drop(file);
    anyhow::ensure!(
        format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&release.checksum),
        "Update checksum did not match; download discarded"
    );
    let payload = stage.path().join("payload");
    let files = extract_package(&archive, &payload)?;
    let executable = target.canonicalize()?;
    let plan = Plan {
        target: executable
            .parent()
            .context("Missing Atlas folder")?
            .to_path_buf(),
        executable: executable
            .file_name()
            .context("Missing executable name")?
            .to_owned(),
        files,
        version: release.version.clone(),
        processes,
    };
    fs::write(stage.path().join("plan.json"), serde_json::to_vec(&plan)?)?;
    fs::copy(
        std::env::current_exe()?,
        stage.path().join("AtlasUpdate.exe"),
    )?;
    let path = stage.keep();
    let _ = events.send(Event::UpdateProgress {
        bytes,
        total: Some(bytes),
    });
    Ok(Prepared {
        plan: path.join("plan.json"),
        version: release.version.clone(),
    })
}

pub fn start(prepared: &Prepared) -> Result<()> {
    let root = prepared.plan.parent().context("Missing update stage")?;
    let mut command = std::process::Command::new(root.join("AtlasUpdate.exe"));
    command.arg("--apply-update").arg(&prepared.plan);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .context("Could not start the update helper")?;
    Ok(())
}

#[cfg(windows)]
fn wait_for_process(pid: u32) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(87) {
            return Ok(());
        }
        return Err(error.into());
    }
    let status = unsafe { WaitForSingleObject(handle, 120_000) };
    unsafe {
        CloseHandle(handle);
    }
    anyhow::ensure!(
        status == WAIT_OBJECT_0,
        "Atlas is still running. Close it and try the update again."
    );
    Ok(())
}

#[cfg(not(windows))]
fn wait_for_process(_pid: u32) -> Result<()> {
    Ok(())
}

fn replace_files(
    plan: &Plan,
    root: &std::path::Path,
    launch: impl FnOnce(&std::path::Path) -> Result<()>,
) -> Result<()> {
    let target = plan.target.canonicalize()?;
    anyhow::ensure!(
        std::path::Path::new(&plan.executable).components().count() == 1
            && matches!(
                std::path::Path::new(&plan.executable).components().next(),
                Some(std::path::Component::Normal(_))
            ),
        "Invalid target executable"
    );
    let payload = root.join("payload");
    for (relative, expected) in &plan.files {
        anyhow::ensure!(allowed_file(relative), "Unsafe update path");
        anyhow::ensure!(
            file_hash(&payload.join(relative))? == *expected,
            "Staged update changed; nothing was replaced"
        );
    }
    let backup = root.join("backup");
    fs::create_dir_all(&backup)?;
    let mut replaced = Vec::new();
    let result = (|| -> Result<()> {
        for (relative, _) in &plan.files {
            let installed = if relative == std::path::Path::new("ImageAtlas.exe") {
                PathBuf::from(&plan.executable)
            } else {
                relative.clone()
            };
            let destination = target.join(&installed);
            let parent = destination.parent().context("Missing destination folder")?;
            fs::create_dir_all(parent)?;
            anyhow::ensure!(
                parent.canonicalize()?.starts_with(&target),
                "Update destination leaves the Atlas folder"
            );
            let mut candidate = tempfile::NamedTempFile::new_in(parent)?;
            std::io::copy(
                &mut fs::File::open(payload.join(relative))?,
                candidate.as_file_mut(),
            )?;
            candidate.as_file().sync_all()?;
            let existed = destination.exists();
            if existed {
                anyhow::ensure!(destination.is_file(), "Update destination is not a file");
                let saved = backup.join(&installed);
                fs::create_dir_all(saved.parent().unwrap())?;
                fs::rename(&destination, saved)?;
            }
            replaced.push((installed.clone(), existed));
            candidate.persist(&destination).map_err(|e| e.error)?;
        }
        launch(&target.join(&plan.executable))?;
        Ok(())
    })();
    if let Err(error) = result {
        let mut rollback_errors = Vec::new();
        for (relative, existed) in replaced.into_iter().rev() {
            let destination = target.join(&relative);
            if destination.exists()
                && let Err(e) = fs::remove_file(&destination)
            {
                rollback_errors.push(e.to_string());
                continue;
            }
            if existed && let Err(e) = fs::rename(backup.join(&relative), destination) {
                rollback_errors.push(e.to_string());
            }
        }
        anyhow::ensure!(
            rollback_errors.is_empty(),
            "Update failed: {error}. Rollback: {}",
            rollback_errors.join("; ")
        );
        anyhow::bail!("Update failed; the previous version was restored: {error}");
    }
    Ok(())
}

pub fn apply(path: &std::path::Path) -> Result<()> {
    let root = path.parent().context("Missing update stage")?;
    let plan: Plan = serde_json::from_reader(fs::File::open(path)?)?;
    for pid in &plan.processes {
        wait_for_process(*pid)?;
    }
    replace_files(&plan, root, |executable| {
        let health = root.join("health.json");
        if health.exists() {
            fs::remove_file(&health)?;
        }
        let child = std::process::Command::new(executable)
            .env("IMAGE_ATLAS_UPDATE_HEALTH", &health)
            .spawn()
            .context("Could not restart Atlas")?;
        wait_for_health(child, &health, &plan.version, Duration::from_secs(60))
    })
    .inspect_err(|_| {
        let _ = std::process::Command::new(plan.target.join(&plan.executable)).spawn();
    })?;
    update_installed_version(&plan.target, &plan.version);
    fs::write(
        root.join("completed.json"),
        serde_json::to_vec(&plan.version)?,
    )?;
    Ok(())
}

#[cfg(windows)]
pub fn installed_executable() -> Option<PathBuf> {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
        },
    };
    let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide("Software\\NIS\\ImageAtlas").as_ptr(),
            0,
            KEY_READ,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut value = vec![0u16; 32768];
        let mut size = (value.len() * 2) as u32;
        let mut kind = 0;
        let result = RegQueryValueExW(
            key,
            wide("InstallDir").as_ptr(),
            std::ptr::null(),
            &mut kind,
            value.as_mut_ptr().cast(),
            &mut size,
        );
        RegCloseKey(key);
        if result != ERROR_SUCCESS || kind != REG_SZ {
            return None;
        }
        let length = value
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(value.len());
        let path = PathBuf::from(String::from_utf16_lossy(&value[..length])).join("ImageAtlas.exe");
        path.is_file().then_some(path)
    }
}

#[cfg(not(windows))]
pub fn installed_executable() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
pub fn running_instances(target: &std::path::Path) -> Vec<u32> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    };
    let Ok(target) = target.canonicalize() else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return ids;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                if !handle.is_null() {
                    let mut buffer = vec![0u16; 32768];
                    let mut length = buffer.len() as u32;
                    if QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) != 0
                    {
                        let path =
                            PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
                        if path.canonicalize().is_ok_and(|path| {
                            path.to_string_lossy()
                                .eq_ignore_ascii_case(&target.to_string_lossy())
                        }) {
                            ids.push(entry.th32ProcessID);
                        }
                    }
                    CloseHandle(handle);
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    ids
}

#[cfg(not(windows))]
pub fn running_instances(_target: &std::path::Path) -> Vec<u32> {
    Vec::new()
}

pub fn report_healthy() {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if let Some(path) = std::env::var_os("IMAGE_ATLAS_UPDATE_HEALTH") {
        let path = PathBuf::from(path);
        let cache = crate::recovery::folder().join("updates");
        if path.file_name().is_some_and(|name| name == "health.json") && path.starts_with(cache) {
            let _ = fs::write(path, env!("CARGO_PKG_VERSION"));
        }
    }
}

fn wait_for_health(
    mut child: std::process::Child,
    health: &std::path::Path,
    version: &str,
    timeout: Duration,
) -> Result<()> {
    let started = std::time::Instant::now();
    loop {
        if fs::read_to_string(health).is_ok_and(|value| value == version) {
            return Ok(());
        }
        if child.try_wait()?.is_some() {
            stop_update_processes(child.id());
            anyhow::bail!("The updated version closed before it was ready");
        }
        if started.elapsed() >= timeout {
            stop_update_processes(child.id());
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("The updated version did not start correctly");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(windows)]
fn stop_update_processes(root: u32) {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
                WaitForSingleObject,
            },
        },
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut all = Vec::new();
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                all.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        let mut ids = vec![root];
        loop {
            let added: Vec<_> = all
                .iter()
                .filter(|(pid, parent)| ids.contains(parent) && !ids.contains(pid))
                .map(|(pid, _)| *pid)
                .collect();
            if added.is_empty() {
                break;
            }
            ids.extend(added);
        }
        for pid in ids.into_iter().rev() {
            let handle = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
            if !handle.is_null() {
                TerminateProcess(handle, 1);
                WaitForSingleObject(handle, 5000);
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(not(windows))]
fn stop_update_processes(_root: u32) {}

#[cfg(windows)]
fn update_installed_version(target: &std::path::Path, version: &str) {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW,
            RegQueryValueExW, RegSetValueExW,
        },
    };
    let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide("Software\\NIS\\ImageAtlas").as_ptr(),
            0,
            KEY_READ,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return;
        }
        let mut value = vec![0u16; 32768];
        let mut size = (value.len() * 2) as u32;
        let mut kind = 0;
        let result = RegQueryValueExW(
            key,
            wide("InstallDir").as_ptr(),
            std::ptr::null(),
            &mut kind,
            value.as_mut_ptr().cast(),
            &mut size,
        );
        RegCloseKey(key);
        if result != ERROR_SUCCESS || kind != REG_SZ {
            return;
        }
        let length = value
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(value.len());
        let installed = PathBuf::from(String::from_utf16_lossy(&value[..length]));
        if installed.canonicalize().ok() != target.canonicalize().ok() {
            return;
        }
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ImageAtlas").as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        ) == ERROR_SUCCESS
        {
            let value = wide(version);
            RegSetValueExW(
                key,
                wide("DisplayVersion").as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            );
            RegCloseKey(key);
        }
    }
}

#[cfg(not(windows))]
fn update_installed_version(_target: &std::path::Path, _version: &str) {}

pub fn check_due(elapsed: Duration, returned_to_app: bool) -> bool {
    elapsed >= Duration::from_secs(900) || returned_to_app && elapsed >= Duration::from_secs(60)
}

pub fn cleanup_completed() {
    let cache = crate::recovery::folder().join("updates");
    let Ok(cache) = cache.canonicalize() else {
        return;
    };
    let Ok(entries) = fs::read_dir(&cache) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            continue;
        }
        if !entry.file_name().to_string_lossy().starts_with("update-")
            || !path.join("completed.json").is_file()
        {
            continue;
        }
        if path
            .canonicalize()
            .is_ok_and(|path| path.parent() == Some(cache.as_path()))
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package_fixture() -> (tempfile::TempDir, Plan) {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Atlas folder");
        let payload = root.path().join("payload");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&payload).unwrap();
        fs::write(target.join("Atlas.exe"), b"old executable").unwrap();
        fs::write(target.join("personal.png"), b"user image").unwrap();
        fs::write(target.join("NOTICE"), b"old notice").unwrap();
        fs::write(payload.join("ImageAtlas.exe"), b"new executable").unwrap();
        fs::write(payload.join("NOTICE"), b"new notice").unwrap();
        let files = ["ImageAtlas.exe", "NOTICE"]
            .into_iter()
            .map(|name| (PathBuf::from(name), file_hash(&payload.join(name)).unwrap()))
            .collect();
        let plan = Plan {
            target,
            executable: "Atlas.exe".into(),
            files,
            version: "9.0.0".into(),
            processes: Vec::new(),
        };
        (root, plan)
    }
    #[test]
    fn update_replaces_the_current_copy_and_keeps_unrelated_files() {
        let (root, plan) = package_fixture();
        replace_files(&plan, root.path(), |executable| {
            assert_eq!(
                executable,
                plan.target.join("Atlas.exe").canonicalize().unwrap()
            );
            assert_eq!(fs::read(executable).unwrap(), b"new executable");
            Ok(())
        })
        .unwrap();
        assert!(!plan.target.join("ImageAtlas.exe").exists());
        assert_eq!(
            fs::read(plan.target.join("personal.png")).unwrap(),
            b"user image"
        );
        assert_eq!(fs::read(plan.target.join("NOTICE")).unwrap(), b"new notice");
    }
    #[test]
    fn failed_restart_rolls_back_every_replaced_file() {
        let (root, plan) = package_fixture();
        let result = replace_files(&plan, root.path(), |_| anyhow::bail!("startup failed"));
        assert!(result.is_err());
        assert_eq!(
            fs::read(plan.target.join("Atlas.exe")).unwrap(),
            b"old executable"
        );
        assert_eq!(fs::read(plan.target.join("NOTICE")).unwrap(), b"old notice");
        assert_eq!(
            fs::read(plan.target.join("personal.png")).unwrap(),
            b"user image"
        );
    }
    #[test]
    fn altered_staged_files_are_rejected_before_replacing_anything() {
        let (root, plan) = package_fixture();
        fs::write(root.path().join("payload/NOTICE"), b"tampered").unwrap();
        assert!(replace_files(&plan, root.path(), |_| panic!("must not launch")).is_err());
        assert_eq!(
            fs::read(plan.target.join("Atlas.exe")).unwrap(),
            b"old executable"
        );
    }
    #[test]
    fn package_paths_cannot_write_outside_the_application() {
        for value in [
            "../ImageAtlas.exe",
            "C:/ImageAtlas.exe",
            "docs/../../user.txt",
            "personal.png",
            "session.lock",
            "user-images/picture.jpg",
        ] {
            assert!(!allowed_file(std::path::Path::new(value)), "{value}");
        }
        assert!(allowed_file(std::path::Path::new(
            "THIRD-PARTY-LICENSES/example/LICENSE"
        )));
        let folder = tempfile::tempdir().unwrap();
        let archive = folder.path().join("update.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("../escape.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, b"bad").unwrap();
        zip.finish().unwrap();
        assert!(extract_package(&archive, &folder.path().join("stage")).is_err());
        assert!(!folder.path().join("escape.exe").exists());
    }
    #[test]
    fn automatic_checks_run_periodically_and_when_returning_to_the_app() {
        assert!(!check_due(Duration::from_secs(59), true));
        assert!(check_due(Duration::from_secs(60), true));
        assert!(!check_due(Duration::from_secs(899), false));
        assert!(check_due(Duration::from_secs(900), false));
    }
    #[test]
    #[ignore = "Child process used by update health-check tests"]
    fn health_probe() {
        if let Some(path) = std::env::var_os("ATLAS_TEST_HEALTH") {
            if std::env::var_os("ATLAS_TEST_NO_HEALTH").is_none() {
                fs::write(path, "test-version").unwrap();
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    #[test]
    fn restart_waits_for_the_new_version_to_report_ready() {
        let folder = tempfile::tempdir().unwrap();
        let health = folder.path().join("health.json");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["update::tests::health_probe", "--exact", "--ignored"])
            .env("ATLAS_TEST_HEALTH", &health)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert!(wait_for_health(child, &health, "test-version", Duration::from_secs(5)).is_ok());
    }
    #[test]
    fn a_version_that_never_reports_ready_is_stopped_and_rejected() {
        let folder = tempfile::tempdir().unwrap();
        let health = folder.path().join("health.json");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["update::tests::health_probe", "--exact", "--ignored"])
            .env("ATLAS_TEST_HEALTH", &health)
            .env("ATLAS_TEST_NO_HEALTH", "1")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert!(
            wait_for_health(child, &health, "test-version", Duration::from_millis(200)).is_err()
        );
    }
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
