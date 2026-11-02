use crate::model::Cancel;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

const URL: &str = "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip";
const CHECKSUM: &str = "abc02804e17982a3be33675e4d471e91ea374e65b70167abc09e31acb412802d";
const FILES: &[&str] = &[
    "realesrgan-ncnn-vulkan.exe",
    "vcomp140.dll",
    "models/realesrgan-x4plus.bin",
    "models/realesrgan-x4plus.param",
];

pub fn folder() -> PathBuf {
    crate::recovery::folder().join("engines/realesrgan-20220424")
}

pub fn ready() -> bool {
    let root = folder();
    root.join("verified.json").is_file() && FILES.iter().all(|name| root.join(name).is_file())
}

pub fn install(cancel: &Cancel, mut progress: impl FnMut(u64, Option<u64>)) -> Result<PathBuf> {
    anyhow::ensure!(
        cfg!(windows),
        "The local AI engine currently requires Windows and a Vulkan GPU"
    );
    static INSTALL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = loop {
        anyhow::ensure!(!cancel.stopped(), "Stopped");
        match INSTALL.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(80))
            }
            Err(error) => anyhow::bail!("AI engine installation failed: {error}"),
        }
    };
    if ready() {
        return Ok(folder());
    }
    let parent = folder().parent().unwrap().to_path_buf();
    fs::create_dir_all(&parent)?;
    let stage = tempfile::tempdir_in(&parent)?;
    let package = stage.path().join("engine.zip");
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()?;
    let mut response = client.get(URL).send()?.error_for_status()?;
    let total = response.content_length();
    let mut file = fs::File::create(&package)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    let mut bytes = 0;
    let mut last = std::time::Instant::now();
    loop {
        anyhow::ensure!(!cancel.stopped(), "Stopped");
        let length = response.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
        file.write_all(&buffer[..length])?;
        bytes += length as u64;
        if last.elapsed() >= Duration::from_millis(100) {
            progress(bytes, total);
            last = std::time::Instant::now();
        }
    }
    file.sync_all()?;
    drop(file);
    anyhow::ensure!(
        format!("{:x}", hash.finalize()) == CHECKSUM,
        "AI engine checksum mismatch; download discarded"
    );
    let mut archive = zip::ZipArchive::new(fs::File::open(package)?)?;
    let payload = stage.path().join("payload");
    fs::create_dir_all(&payload)?;
    for name in FILES {
        let mut entry = archive.by_name(name)?;
        let destination = payload.join(name);
        fs::create_dir_all(destination.parent().unwrap())?;
        let mut file = fs::File::create(destination)?;
        std::io::copy(&mut entry, &mut file)?;
        file.sync_all()?;
    }
    fs::write(
        payload.join("LICENSE.txt"),
        include_str!("../../THIRD-PARTY-LICENSES/Real-ESRGAN-ncnn-vulkan/LICENSE"),
    )?;
    fs::write(
        payload.join("MODEL-LICENSE.txt"),
        include_str!("../../THIRD-PARTY-LICENSES/Real-ESRGAN/LICENSE"),
    )?;
    fs::write(
        payload.join("NCNN-LICENSE.txt"),
        include_str!("../../THIRD-PARTY-LICENSES/ncnn-upscale/LICENSE"),
    )?;
    fs::write(
        payload.join("WEBP-LICENSE.txt"),
        include_str!("../../THIRD-PARTY-LICENSES/libwebp-upscale/LICENSE"),
    )?;
    fs::write(payload.join("verified.json"), serde_json::to_vec(CHECKSUM)?)?;
    let target = folder();
    if target.exists() {
        for name in FILES {
            let destination = target.join(name);
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::copy(payload.join(name), destination)?;
        }
        for name in [
            "LICENSE.txt",
            "MODEL-LICENSE.txt",
            "NCNN-LICENSE.txt",
            "WEBP-LICENSE.txt",
            "verified.json",
        ] {
            fs::copy(payload.join(name), target.join(name))?;
        }
    } else {
        fs::rename(payload, &target)?;
    }
    progress(bytes, total);
    Ok(target)
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn run(root: &Path, input: &Path, output: &Path, cancel: &Cancel, log: &Path) -> Result<()> {
    let stderr = fs::File::create(log)?;
    let mut command = Command::new(root.join("realesrgan-ncnn-vulkan.exe"));
    command
        .current_dir(root)
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .arg("-m")
        .arg(root.join("models"))
        .args([
            "-n",
            "realesrgan-x4plus",
            "-s",
            "4",
            "-t",
            "64",
            "-j",
            "1:1:1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = Process(
        command
            .spawn()
            .context("Could not start the local AI engine")?,
    );
    let mut last_activity = std::time::Instant::now();
    let mut log_length = 0;
    loop {
        anyhow::ensure!(!cancel.stopped(), "Stopped");
        let length = fs::metadata(log).map(|file| file.len()).unwrap_or(0);
        if length != log_length {
            log_length = length;
            last_activity = std::time::Instant::now();
        }
        anyhow::ensure!(
            last_activity.elapsed() < Duration::from_secs(180),
            "The AI engine stopped reporting progress. This file can be retried; completed exports are preserved."
        );
        if let Some(status) = child.0.try_wait()? {
            anyhow::ensure!(
                status.success() && output.is_file(),
                "AI upscaling failed ({status}). {}",
                fs::read_to_string(log)
                    .unwrap_or_default()
                    .chars()
                    .take(2000)
                    .collect::<String>()
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}
