use crate::model::{Asset, Cancel, Event, SiteMap, asset_path, digest};
use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    pub extension: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[serde(default)]
    pub response_ms: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Options {
    pub folder: PathBuf,
    pub bandwidth_mbps: u32,
    pub make_zip: bool,
}

#[derive(Debug)]
pub struct Adaptive {
    pub target: usize,
    previous_rate: f64,
    completed: usize,
}

impl Adaptive {
    pub fn new(hint: u32) -> Self {
        Self {
            target: if hint == 0 {
                4
            } else {
                (hint as usize / 100).clamp(2, 16)
            },
            previous_rate: 0.0,
            completed: 0,
        }
    }
    pub fn sample(&mut self, success: bool, rate: f64) {
        if !success {
            self.target = (self.target / 2).max(1);
            self.completed = 0;
            return;
        }
        self.completed += 1;
        if self.completed >= 8 {
            if self.previous_rate == 0.0 || rate > self.previous_rate * 1.03 {
                self.target += ((self.target as f64).sqrt().ceil() as usize).max(1);
            } else if rate < self.previous_rate * 0.85 {
                self.target = self.target.saturating_sub(1).max(1);
            }
            self.previous_rate = rate;
            self.completed = 0;
        }
    }
}

pub fn detected_extension(bytes: &[u8], mime: &str) -> Option<String> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(8192)]).to_ascii_lowercase();
    if text.trim_start().starts_with("<svg")
        || (text.contains("<svg")
            && (text.trim_start().starts_with("<?xml") || mime.contains("svg")))
    {
        return Some("svg".into());
    }
    if bytes.starts_with(b"%PDF-") {
        return Some("pdf".into());
    }
    image::guess_format(bytes)
        .ok()
        .and_then(|f| f.extensions_str().first().map(|s| s.to_string()))
}

fn read_index(path: &Path) -> HashMap<String, Saved> {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn checkpoint(path: &Path, values: &HashMap<String, Saved>) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(values)?)?;
    fs::rename(temp, path)?;
    Ok(())
}

async fn one(
    client: reqwest::Client,
    url: String,
    cache: PathBuf,
    existing: Option<Saved>,
    cancel: Cancel,
) -> Result<(String, Saved)> {
    if let Some(saved) = existing
        && cache
            .join(&saved.file)
            .metadata()
            .is_ok_and(|m| m.len() == saved.bytes)
    {
        return Ok((url, saved));
    }
    let key = digest(url.as_bytes());

    if url.starts_with("data:image/") {
        use base64::Engine;
        let (header, payload) = url.split_once(',').context("Invalid inline image")?;
        let bytes = if header.ends_with(";base64") {
            base64::engine::general_purpose::STANDARD.decode(payload)?
        } else {
            percent_encoding::percent_decode_str(payload).collect::<Vec<_>>()
        };
        let extension = detected_extension(&bytes, header).context("Invalid inline image data")?;
        let file = format!("{key}.{extension}");
        fs::write(cache.join(&file), &bytes)?;
        let size = image::load_from_memory(&bytes)
            .ok()
            .map(|i| (i.width(), i.height()));
        return Ok((
            url,
            Saved {
                file,
                bytes: bytes.len() as u64,
                sha256: digest(&bytes),
                extension,
                width: size.map(|s| s.0),
                height: size.map(|s| s.1),
                response_ms: 0,
            },
        ));
    }
    let part = cache.join(format!("{key}.part"));
    let side = cache.join(format!("{key}.part.json"));
    let mut last = String::new();
    for attempt in 0..4 {
        anyhow::ensure!(!cancel.stopped(), "Cancelled");
        let mut request = client
            .get(&url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity");
        let etag = fs::read_to_string(&side).ok();
        let offset = if etag.is_some() {
            part.metadata().map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };
        if offset > 0 {
            request = request
                .header(reqwest::header::RANGE, format!("bytes={offset}-"))
                .header(reqwest::header::IF_RANGE, etag.clone().unwrap_or_default());
        }
        let request_started = Instant::now();
        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                last = e.to_string();
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                continue;
            }
        };
        let response_ms = request_started.elapsed().as_millis() as u64;
        let status = response.status();
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let _ = fs::remove_file(&part);
            let _ = fs::remove_file(&side);
            last = "Restarting an invalid partial download".into();
            continue;
        }
        if status.as_u16() == 429 || status.is_server_error() {
            let seconds = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(2_u64 << attempt);
            last = format!("HTTP {status}");
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            continue;
        }
        anyhow::ensure!(status.is_success(), "HTTP {} for {}", status, url);
        let append = offset > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
        if append {
            let range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            anyhow::ensure!(
                range.starts_with(&format!("bytes {offset}-")),
                "Unexpected resume range"
            );
        }
        if let Some(tag) = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
        {
            fs::write(&side, tag)?;
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&part)?;
        let mut stream = response.bytes_stream();
        let mut failed = None;
        while let Some(chunk) = stream.next().await {
            anyhow::ensure!(!cancel.stopped(), "Cancelled");
            match chunk {
                Ok(bytes) => file.write_all(&bytes)?,
                Err(e) => {
                    failed = Some(e.to_string());
                    break;
                }
            }
        }
        file.flush()?;
        drop(file);
        if let Some(error) = failed {
            last = error;
            continue;
        }
        let mut file = File::open(&part)?;
        let mut head = [0u8; 8192];
        let count = file.read(&mut head)?;
        let extension = detected_extension(&head[..count], &mime)
            .context("The server returned a non-image response")?;
        let (width, height) = if extension == "svg" {
            (None, None)
        } else {
            image::ImageReader::open(&part)
                .ok()
                .and_then(|r| r.with_guessed_format().ok())
                .and_then(|r| r.into_dimensions().ok())
                .ok_or(())
                .map(|(w, h)| (Some(w), Some(h)))
                .unwrap_or((None, None))
        };
        let mut hash = Sha256::new();
        hash.update(&head[..count]);
        let mut block = [0u8; 65536];
        loop {
            let n = file.read(&mut block)?;
            if n == 0 {
                break;
            }
            hash.update(&block[..n]);
        }
        let bytes = file.metadata()?.len();
        drop(file);
        let name = format!("{key}.{extension}");
        let final_path = cache.join(&name);
        if final_path.exists() {
            fs::remove_file(&final_path)?;
        }
        fs::rename(&part, &final_path)?;
        let _ = fs::remove_file(&side);
        return Ok((
            url,
            Saved {
                file: name,
                bytes,
                sha256: format!("{:x}", hash.finalize()),
                extension,
                width,
                height,
                response_ms,
            },
        ));
    }
    Err(TransientFailure(format!("{url}: {last}")).into())
}

fn place(root: &Path, cache: &Path, asset: &Asset, saved: &Saved) -> Result<PathBuf> {
    let mut relative = asset_path(asset);
    anyhow::ensure!(
        crate::model::safe_relative(&relative.to_string_lossy()),
        "Invalid output path"
    );
    relative.set_extension(&saved.extension);
    let destination = root.join(&relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if !destination.metadata().is_ok_and(|m| m.len() == saved.bytes) {
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        if fs::hard_link(cache.join(&saved.file), &destination).is_err() {
            fs::copy(cache.join(&saved.file), &destination)?;
        }
    }
    Ok(relative)
}

async fn download_pass(
    map: SiteMap,
    options: Options,
    cancel: Cancel,
    events: Sender<Event>,
) -> Result<(usize, bool)> {
    fs::create_dir_all(&options.folder)?;
    let cache = options.folder.join(".atlas-cache");
    fs::create_dir_all(&cache)?;
    let index_path = cache.join("downloads.json");
    let mut index = read_index(&index_path);
    let mut groups: HashMap<String, Vec<Asset>> = HashMap::new();
    for asset in map.assets.into_iter().filter(|a| a.selected) {
        groups.entry(asset.url.clone()).or_default().push(asset);
    }
    let mut queue: VecDeque<_> = groups.keys().cloned().collect();
    let total = queue.len();
    let mut jobs = FuturesUnordered::new();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(90))
        .user_agent("ImageAtlas/0.1 original image backup")
        .build()?;
    let mut adaptive = Adaptive::new(options.bandwidth_mbps);
    let mut done = 0;
    let mut bytes = 0;
    let started = Instant::now();
    let mut failures = Vec::new();
    let mut retryable = false;
    let mut exported = Vec::new();
    let mut response_total = 0u64;
    let mut successful = 0usize;
    while !queue.is_empty() || !jobs.is_empty() {
        if cancel.stopped() {
            break;
        }
        while jobs.len() < adaptive.target && !queue.is_empty() {
            let url = queue.pop_front().unwrap();
            jobs.push(one(
                client.clone(),
                url.clone(),
                cache.clone(),
                index.get(&url).cloned(),
                cancel.clone(),
            ));
        }
        if let Some(result) = jobs.next().await {
            done += 1;
            let success = result.is_ok();
            match result {
                Ok((url, saved)) => {
                    bytes += saved.bytes;
                    response_total += saved.response_ms;
                    successful += 1;
                    for asset in &groups[&url] {
                        match place(&options.folder, &cache, asset, &saved) {
                            Ok(path) => exported.push((asset.clone(), path, saved.clone())),
                            Err(e) => failures.push(e.to_string()),
                        }
                    }
                    if saved.width.is_some_and(|w| w < 500) || saved.height.is_some_and(|h| h < 500)
                    {
                        let _ = events.send(Event::Log(format!(
                            "Low-resolution source: {} × {} — {}",
                            saved.width.unwrap_or(0),
                            saved.height.unwrap_or(0),
                            url
                        )));
                    }
                    index.insert(url, saved);
                }
                Err(e) => {
                    retryable |= e.downcast_ref::<TransientFailure>().is_some();
                    let _ = events.send(Event::Log(e.to_string()));
                    failures.push(e.to_string());
                }
            }
            let speed = bytes as f64 / started.elapsed().as_secs_f64().max(0.01);
            adaptive.sample(success, speed);
            let _ = events.send(Event::Progress {
                done,
                total,
                bytes,
                active: adaptive.target,
                speed,
                eta_seconds: estimate_eta(
                    total.saturating_sub(done),
                    bytes,
                    speed,
                    response_total,
                    successful,
                    adaptive.target,
                    started.elapsed().as_secs_f64(),
                ),
            });
            if done % 10 == 0 {
                checkpoint(&index_path, &index)?;
            }
        }
    }
    checkpoint(&index_path, &index)?;
    let mut manifest = File::create(options.folder.join("manifest.csv"))?;
    manifest.write_all(
        b"\xef\xbb\xbfcategory,product,source,page,file,bytes,sha256,width,height,format\r\n",
    )?;
    for (asset, path, saved) in &exported {
        let values = [
            asset.folders.join(" / "),
            asset.label.clone(),
            asset.url.clone(),
            asset.page.clone(),
            path.to_string_lossy().into_owned(),
            saved.bytes.to_string(),
            saved.sha256.clone(),
            saved.width.map(|v| v.to_string()).unwrap_or_default(),
            saved.height.map(|v| v.to_string()).unwrap_or_default(),
            saved.extension.clone(),
        ];
        writeln!(
            manifest,
            "{}",
            values
                .iter()
                .map(|v| format!("\"{}\"", v.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(",")
        )?;
    }
    fs::write(
        options.folder.join("failures.json"),
        serde_json::to_vec_pretty(&failures)?,
    )?;
    if options.make_zip && !cancel.stopped() {
        let file = File::create(options.folder.with_extension("zip"))?;
        let mut archive = zip::ZipWriter::new(file);
        let settings = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut zipped = std::collections::HashSet::new();
        for (_, relative, _) in &exported {
            if !zipped.insert(relative.clone()) {
                continue;
            }
            archive.start_file(relative.to_string_lossy().replace('\\', "/"), settings)?;
            let mut source = File::open(options.folder.join(relative))?;
            std::io::copy(&mut source, &mut archive)?;
        }
        for name in ["manifest.csv", "failures.json"] {
            archive.start_file(name, settings)?;
            archive.write_all(&fs::read(options.folder.join(name))?)?;
        }
        archive.finish()?;
    }
    Ok((failures.len(), retryable))
}

pub fn downloaded_files(map: &SiteMap, folder: &Path) -> Vec<PathBuf> {
    let index = read_index(&folder.join(".atlas-cache/downloads.json"));
    let mut seen = std::collections::HashSet::new();
    map.assets
        .iter()
        .filter(|a| a.selected)
        .filter_map(|asset| {
            let saved = index.get(&asset.url)?;
            let mut relative = asset_path(asset);
            relative.set_extension(&saved.extension);
            let path = folder.join(relative);
            if path
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() == saved.bytes)
                && seen.insert(path.clone())
            {
                Some(path)
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug)]
struct TransientFailure(String);
impl std::fmt::Display for TransientFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for TransientFailure {}

#[derive(Clone, Serialize, Deserialize)]
pub struct DownloadJob {
    pub map: SiteMap,
    pub options: Options,
}
fn job_path() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
        .join("ImageAtlas/download-job.json")
}
pub fn pending_job() -> Option<DownloadJob> {
    fs::read(job_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}
pub async fn download_map(
    map: SiteMap,
    options: Options,
    cancel: Cancel,
    events: Sender<Event>,
) -> Result<usize> {
    let path = job_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let job = DownloadJob {
        map: map.clone(),
        options: options.clone(),
    };
    let temp = path.with_extension("tmp");
    fs::write(&temp, serde_json::to_vec(&job)?)?;
    fs::rename(temp, &path)?;
    fs::create_dir_all(&options.folder)?;
    fs::write(
        options.folder.join("website-map.json"),
        serde_json::to_vec_pretty(&map)?,
    )?;
    let mut delay = 15u64;
    loop {
        let (failures, retryable) =
            download_pass(map.clone(), options.clone(), cancel.clone(), events.clone()).await?;
        if cancel.stopped() {
            return Ok(failures);
        }
        if failures == 0 {
            let _ = fs::remove_file(&path);
            return Ok(0);
        }
        if !retryable {
            return Ok(failures);
        }
        let _=events.send(Event::Log(format!("Server unavailable. Retrying failed files in {delay} seconds; completed downloads are safe.")));
        for _ in 0..delay {
            if cancel.stopped() {
                return Ok(failures);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        delay = (delay * 2).min(300);
    }
}

fn estimate_eta(
    remaining: usize,
    bytes: u64,
    speed: f64,
    response_ms: u64,
    successful: usize,
    connections: usize,
    elapsed: f64,
) -> Option<f64> {
    if remaining == 0 {
        return Some(0.0);
    }
    if successful < 3 || elapsed < 2.0 || speed <= 0.0 {
        return None;
    }
    let remaining = remaining as f64;
    let transfer = remaining * (bytes as f64 / successful as f64) / speed;
    let response =
        remaining * (response_ms as f64 / successful as f64 / 1000.0) / connections.max(1) as f64;
    Some(transfer.max(response).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_remains_vector_despite_wrong_extension() {
        assert_eq!(
            detected_extension(
                b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
                "image/svg+xml"
            ),
            Some("svg".into())
        );
    }
    #[test]
    fn network_backpressure_reduces_connections() {
        let mut a = Adaptive::new(1000);
        let before = a.target;
        a.sample(false, 1.0);
        assert!(a.target < before);
        for _ in 0..16 {
            a.sample(true, 100.0);
        }
        assert!(a.target > 1);
    }
    #[test]
    fn resumes_original_bytes_and_writes_detected_extension() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/wrong.jpg", listener.local_addr().unwrap());
        let svg=b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"16\"><rect width=\"32\" height=\"16\" fill=\"red\"/></svg>";
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let size = socket.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.to_lowercase().contains("range: bytes=20-"));
            write!(socket,"HTTP/1.1 206 Partial Content\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nContent-Range: bytes 20-{}/{}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n",svg.len()-20,svg.len()-1,svg.len()).unwrap();
            socket.write_all(&svg[20..]).unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join(".atlas-cache");
        fs::create_dir_all(&cache).unwrap();
        let key = digest(url.as_bytes());
        fs::write(cache.join(format!("{key}.part")), &svg[..20]).unwrap();
        fs::write(cache.join(format!("{key}.part.json")), "\"v1\"").unwrap();
        let asset = Asset {
            url: url.clone(),
            page: "http://test/product".into(),
            label: "Logo".into(),
            folders: vec![
                "Category".into(),
                "Subcategory".into(),
                "Third level".into(),
            ],
            selected: true,
        };
        let map = SiteMap {
            assets: vec![asset.clone(), asset],
            ..Default::default()
        };
        let (tx, _) = crossbeam_channel::unbounded();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(
            runtime
                .block_on(download_pass(
                    map,
                    Options {
                        folder: dir.path().into(),
                        bandwidth_mbps: 0,
                        make_zip: true
                    },
                    Cancel::default(),
                    tx
                ))
                .unwrap(),
            (0, false)
        );
        server.join().unwrap();
        let output = walkdir::WalkDir::new(dir.path().join("Category"))
            .into_iter()
            .filter_map(Result::ok)
            .find(|e| e.file_type().is_file())
            .unwrap();
        assert_eq!(output.path().extension().unwrap(), "svg");
        assert_eq!(fs::read(output.path()).unwrap(), svg);
        let mut zip =
            zip::ZipArchive::new(File::open(dir.path().with_extension("zip")).unwrap()).unwrap();
        assert_eq!(zip.len(), 3);
        let mut contents = String::new();
        zip.by_index(0)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert!(contents.starts_with("<svg"));
        drop(zip);
        fs::remove_file(dir.path().with_extension("zip")).unwrap();
    }
    #[test]
    fn eta_accounts_for_speed_and_server_latency() {
        assert!(estimate_eta(9, 100, 100.0, 10, 1, 4, 1.0).is_none());
        let fast = estimate_eta(5, 5000, 1000.0, 50, 5, 4, 5.0).unwrap();
        let slow = estimate_eta(5, 5000, 100.0, 50, 5, 4, 5.0).unwrap();
        assert!(slow > fast);
        let delayed = estimate_eta(5, 5000, 1000.0, 100_000, 5, 1, 5.0).unwrap();
        assert!(delayed > fast);
        assert_eq!(estimate_eta(0, 5000, 1000.0, 50, 5, 4, 5.0), Some(0.0));
    }
    #[test]
    fn jpeg_bytes_are_never_saved_as_svg() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(32, 24)
            .write_to(&mut bytes, image::ImageFormat::Jpeg)
            .unwrap();
        let extension = detected_extension(bytes.get_ref(), "image/svg+xml").unwrap();
        assert!(matches!(extension.as_str(), "jpg" | "jpeg"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wrong.svg");
        fs::write(&path, bytes.into_inner()).unwrap();
        assert_eq!(crate::convert::dimensions(&path).unwrap(), (32, 24));
        let output = crate::convert::export(
            &path,
            &dir.path().join("out"),
            &crate::convert::Settings {
                format: "svg embedded".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let svg = fs::read(&output).unwrap();
        assert!(resvg::usvg::Tree::from_data(&svg, &Default::default()).is_ok());
        assert!(crate::convert::load(&output).is_ok());
    }
    #[test]
    fn editor_import_contains_only_completed_selected_files() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join(".atlas-cache");
        fs::create_dir_all(&cache).unwrap();
        let asset = Asset {
            url: "https://example.test/wrong.svg".into(),
            page: "https://example.test".into(),
            label: "Product".into(),
            folders: vec!["Category".into()],
            selected: true,
        };
        let mut missing = asset.clone();
        missing.url = "https://example.test/missing.svg".into();
        let mut excluded = asset.clone();
        excluded.url = "https://example.test/excluded.svg".into();
        excluded.selected = false;
        let saved = Saved {
            file: "cached.jpg".into(),
            bytes: 3,
            sha256: "hash".into(),
            extension: "jpg".into(),
            width: None,
            height: None,
            response_ms: 0,
        };
        let index = HashMap::from([
            (asset.url.clone(), saved.clone()),
            (missing.url.clone(), saved.clone()),
            (excluded.url.clone(), saved),
        ]);
        checkpoint(&cache.join("downloads.json"), &index).unwrap();
        let mut relative = asset_path(&asset);
        relative.set_extension("jpg");
        let path = dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"jpg").unwrap();
        let map = SiteMap {
            assets: vec![asset.clone(), asset, missing, excluded],
            ..Default::default()
        };
        assert_eq!(downloaded_files(&map, dir.path()), vec![path]);
    }
}
