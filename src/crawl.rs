use crate::model::{Asset, Cancel, Event, SiteMap};
use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
use url::Url;

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CrawlOptions {
    pub website: String,
    pub render: bool,
    pub sitemap: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Checkpoint {
    options: CrawlOptions,
    map: SiteMap,
    queue: VecDeque<(String, Vec<String>)>,
    seen: HashSet<String>,
    scanned: usize,
}

fn checkpoint_path() -> PathBuf {
    crate::recovery::folder().join("mapping-job.json")
}

fn read_checkpoint(path: &std::path::Path) -> Option<Checkpoint> {
    serde_json::from_reader(std::fs::File::open(path).ok()?).ok()
}

pub fn pending_mapping() -> Option<(CrawlOptions, SiteMap)> {
    let job = read_checkpoint(&checkpoint_path())?;
    if job.queue.is_empty() {
        return None;
    }
    Some((job.options, job.map))
}

fn save_checkpoint(path: &std::path::Path, job: &impl serde::Serialize) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().context("Missing checkpoint folder")?)?;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    serde_json::to_writer(file.as_file_mut(), job)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[derive(Default)]
pub struct Page {
    pub assets: Vec<Asset>,
    pub links: Vec<(Url, Vec<String>)>,
}

pub fn web_url(base: &Url, value: &str) -> Option<Url> {
    let mut u = base.join(value.trim()).ok()?;
    if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some()
    {
        return None;
    }
    u.set_fragment(None);
    let pairs: Vec<_> = u
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_") && k != "fbclid" && k != "gclid")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    u.set_query(None);
    if !pairs.is_empty() {
        u.query_pairs_mut().extend_pairs(pairs);
    }
    Some(u)
}

fn asset_url(base: &Url, value: &str) -> Option<String> {
    if value.starts_with("data:image/") {
        Some(value.to_owned())
    } else {
        web_url(base, value).map(|u| u.to_string())
    }
}

fn image_url(u: &Url) -> bool {
    let p = u.path().to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".webp", ".gif", ".svg", ".avif", ".bmp", ".tif", ".tiff", ".ico",
    ]
    .iter()
    .any(|e| p.ends_with(e))
}

fn route(base: &Url, v: &Value) -> Option<Url> {
    let value = v
        .get("RouterLink")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .or_else(|| v.get("SeoUrl").and_then(Value::as_str))?;
    web_url(base, value)
}

fn name(v: &Value) -> String {
    v.get("Name")
        .and_then(Value::as_str)
        .unwrap_or("Images")
        .to_owned()
}

fn item_images(base: &Url, v: &Value, folders: &[String], label: &str, page: &mut Page) {
    let mut add = |text: &str| {
        if let Some(u) = web_url(base, text)
            && image_url(&u)
            && !u.path().to_ascii_lowercase().contains("noimage")
        {
            page.assets.push(Asset {
                url: u.to_string(),
                page: base.to_string(),
                label: label.into(),
                folders: folders.to_vec(),
                selected: true,
            });
        }
    };
    for key in [
        "OriginalImage",
        "FullImage",
        "Image",
        "Thumbnail",
        "ImmagineMenu",
        "ImageTop",
        "HeaderImage",
    ] {
        if let Some(s) = v.get(key).and_then(Value::as_str) {
            add(s);
        }
    }
    for key in [
        "PhotoGallery",
        "SliderImages",
        "Attachments",
        "ProductAttachments",
    ] {
        if let Some(images) = v.get(key).and_then(Value::as_array) {
            for image in images {
                if let Some(s) = image.as_str() {
                    add(s);
                }
                for field in [
                    "PhotoUrl",
                    "ImageUrl",
                    "Image",
                    "Url",
                    "FileUrl",
                    "AttachmentUrl",
                    "Path",
                    "OriginalUrl",
                ] {
                    if let Some(s) = image.get(field).and_then(Value::as_str) {
                        add(s);
                    }
                }
            }
        }
    }
    for key in ["ShortDescription", "LongDescription"] {
        if let Some(s) = v.get(key).and_then(Value::as_str) {
            let doc = Html::parse_fragment(s);
            if let Ok(selector) = Selector::parse("img[src], a[href]") {
                for e in doc.select(&selector) {
                    if let Some(src) = e.value().attr("src").or_else(|| e.value().attr("href")) {
                        add(src);
                    }
                }
            }
        }
    }
}

fn storefront(source: &str, base: &Url, inherited: &[String]) -> Option<Page> {
    let marker = source.find("window.UNIVERSAL_CACHE")?;
    let start = source[marker..].find('{')? + marker;
    let outer: Value = serde_json::Deserializer::from_str(&source[start..])
        .into_iter::<Value>()
        .next()?
        .ok()?;
    let cache: Value = serde_json::from_str(outer.get("CacheService")?.as_str()?).ok()?;
    let values = cache.as_object()?;
    let mut page = Page::default();
    if let Some((_, item)) = values.iter().find(|(k, _)| {
        k.contains("/api/inventory/getcategory") || k.contains("/api/inventory/getproduct")
    }) {
        let is_category = item.get("SubCategories").is_some()
            && item.get("Type").and_then(Value::as_str).is_none();
        let mut folders = inherited.to_vec();
        if let Some(crumbs) = item.get("BreadcrumbsInfo").and_then(Value::as_array) {
            let mut names: Vec<_> = crumbs
                .iter()
                .skip(1)
                .filter_map(|c| c.get("BreadcrumbName").and_then(Value::as_str))
                .map(str::to_owned)
                .collect();
            if !is_category && !names.is_empty() {
                names.pop();
            }
            if !names.is_empty() {
                folders = names;
            }
        }
        if folders.is_empty() {
            folders.push(base.host_str().unwrap_or("Website").into());
        }
        item_images(
            base,
            item,
            &folders,
            if is_category {
                "Category"
            } else {
                item.get("Name").and_then(Value::as_str).unwrap_or("Images")
            },
            &mut page,
        );
        for key in ["SubCategories", "SubProducts"] {
            if let Some(children) = item.get(key).and_then(Value::as_array) {
                for child in children {
                    if child.get("Active") == Some(&Value::Bool(false)) {
                        continue;
                    }
                    let mut path = folders.clone();
                    if key == "SubCategories" {
                        path.push(name(child));
                    }
                    item_images(base, child, &folders, &name(child), &mut page);
                    if let Some(u) = route(base, child) {
                        page.links.push((u, path));
                    }
                }
            }
        }
    } else if let Some((_, menu)) = values.iter().find(|(k, _)| k.contains("/getsidemenudata/"))
        && let Some(categories) = menu.get("Categories").and_then(Value::as_array)
    {
        for category in categories {
            if let Some(u) = route(base, category) {
                page.links.push((u, vec![name(category)]));
            }
        }
    }
    Some(page)
}

fn hidden(element: ElementRef<'_>) -> bool {
    element.ancestors().filter_map(ElementRef::wrap).any(|e| {
        let v = e.value();
        matches!(v.name(), "header" | "footer" | "nav")
            || v.attr("aria-hidden") == Some("true")
            || v.attr("hidden").is_some()
            || v.attr("class")
                .unwrap_or("")
                .split_whitespace()
                .any(|c| c == "d-none" || c == "hidden")
            || v.attr("id") == Some("main-sidebar")
    })
}

fn structured_images(v: &Value, base: &Url, folders: &[String], title: &str, page: &mut Page) {
    match v {
        Value::Array(a) => {
            for child in a {
                structured_images(child, base, folders, title, page);
            }
        }
        Value::Object(o) => {
            for (key, value) in o {
                if matches!(key.as_str(), "image" | "contentUrl" | "thumbnailUrl") {
                    let strings: Vec<_> = if let Some(s) = value.as_str() {
                        vec![s]
                    } else {
                        value
                            .as_array()
                            .map(|a| a.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default()
                    };
                    for s in strings {
                        if let Some(u) = web_url(base, s) {
                            page.assets.push(Asset {
                                url: u.to_string(),
                                page: base.to_string(),
                                label: title.into(),
                                folders: folders.to_vec(),
                                selected: true,
                            });
                        }
                    }
                }
                structured_images(value, base, folders, title, page);
            }
        }
        _ => {}
    }
}

pub fn parse_page(source: &str, base: &Url, inherited: &[String]) -> Page {
    if let Some(page) = storefront(source, base, inherited) {
        return page;
    }
    let doc = Html::parse_document(source);
    let mut page = Page::default();
    let title_selector = Selector::parse("h1, title").unwrap();
    let title = doc
        .select(&title_selector)
        .find(|e| e.value().name() == "h1")
        .or_else(|| doc.select(&title_selector).next())
        .map(|e| e.text().collect::<String>().trim().to_owned())
        .unwrap_or_else(|| base.path().into());
    let crumb_selector = Selector::parse("[aria-label*=readcrumb] a, .breadcrumb a, .breadcrumbs a, [itemprop=itemListElement] [itemprop=name]").unwrap();
    let crumbs: Vec<_> = doc
        .select(&crumb_selector)
        .map(|e| e.text().collect::<String>().trim().to_owned())
        .filter(|s| {
            !s.is_empty()
                && !["home", "homepage", "start"].contains(&s.to_ascii_lowercase().as_str())
        })
        .collect();
    let folders = if !crumbs.is_empty() {
        crumbs
    } else if !inherited.is_empty() {
        inherited.to_vec()
    } else {
        let mut path: Vec<_> = base
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .map(|s| s.replace('-', " "))
            .collect();
        path.pop();
        if path.is_empty() {
            path.push(base.host_str().unwrap_or("Website").into());
        }
        path
    };
    for image in doc
        .select(&Selector::parse("img").unwrap())
        .filter(|e| !hidden(*e))
    {
        let srcset = image
            .value()
            .attr("srcset")
            .or_else(|| image.value().attr("data-srcset"));
        let best = srcset.and_then(|s| {
            s.split(',')
                .filter_map(|entry| {
                    let mut parts = entry.split_whitespace();
                    let url = parts.next()?;
                    let size = parts
                        .next()
                        .unwrap_or("1x")
                        .trim_end_matches(['w', 'x'])
                        .parse::<f64>()
                        .unwrap_or(1.0);
                    Some((url, size))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(url, _)| url)
        });
        let src = image
            .value()
            .attr("data-original")
            .or_else(|| image.value().attr("data-full"))
            .or(best)
            .or_else(|| image.value().attr("data-src"))
            .or_else(|| image.value().attr("src"));
        if let Some(u) = src.and_then(|s| asset_url(base, s)) {
            let label = image
                .value()
                .attr("alt")
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&title);
            page.assets.push(Asset {
                url: u.to_string(),
                page: base.to_string(),
                label: label.into(),
                folders: folders.clone(),
                selected: true,
            });
        }
    }

    for svg in doc
        .select(&Selector::parse("svg").unwrap())
        .filter(|e| !hidden(*e))
    {
        use base64::Engine;
        let source = svg.html();
        let source = if source.contains("xmlns=") {
            source
        } else {
            source.replacen("<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"", 1)
        };
        page.assets.push(Asset {
            url: format!(
                "data:image/svg+xml;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(source)
            ),
            page: base.to_string(),
            label: title.clone(),
            folders: folders.clone(),
            selected: true,
        });
    }
    for element in doc
        .select(&Selector::parse("[style]").unwrap())
        .filter(|e| !hidden(*e))
    {
        let style = element.value().attr("style").unwrap_or("");
        for part in style.split("url(").skip(1) {
            if let Some(value) = part.split(')').next()
                && let Some(url) = asset_url(base, value.trim().trim_matches(['\'', '"']))
            {
                page.assets.push(Asset {
                    url,
                    page: base.to_string(),
                    label: title.clone(),
                    folders: folders.clone(),
                    selected: true,
                });
            }
        }
    }
    for meta in doc
        .select(&Selector::parse("meta[property='og:image'], meta[name='twitter:image']").unwrap())
    {
        if let Some(u) = meta.value().attr("content").and_then(|s| web_url(base, s)) {
            page.assets.push(Asset {
                url: u.to_string(),
                page: base.to_string(),
                label: title.clone(),
                folders: folders.clone(),
                selected: true,
            });
        }
    }
    for script in doc.select(&Selector::parse("script[type='application/ld+json']").unwrap()) {
        if let Ok(v) = serde_json::from_str::<Value>(&script.inner_html()) {
            structured_images(&v, base, &folders, &title, &mut page);
        }
    }
    for link in doc.select(&Selector::parse("a[href], loc").unwrap()) {
        let text = link.text().collect::<String>();
        let href = link.value().attr("href").unwrap_or(text.trim());
        if let Some(u) = web_url(base, href) {
            if image_url(&u) {
                page.assets.push(Asset {
                    url: u.to_string(),
                    page: base.to_string(),
                    label: title.clone(),
                    folders: folders.clone(),
                    selected: true,
                });
            } else if u.origin() == base.origin()
                && ![
                    "logout",
                    "signout",
                    "cart",
                    "checkout",
                    "login",
                    "register",
                    "backoffice",
                    "wp-admin",
                ]
                .iter()
                .any(|x| u.path().to_ascii_lowercase().contains(x))
            {
                let ext = std::path::Path::new(u.path())
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                if ext.is_empty() || ["html", "htm", "php", "aspx", "xml"].contains(&ext) {
                    page.links.push((u, Vec::new()));
                }
            }
        }
    }
    page
}

pub fn browser_binary() -> Option<PathBuf> {
    [
        std::env::var_os("PROGRAMFILES(X86)"),
        std::env::var_os("PROGRAMFILES"),
        std::env::var_os("LOCALAPPDATA"),
    ]
    .into_iter()
    .flatten()
    .flat_map(|p| {
        [
            PathBuf::from(&p).join("Microsoft/Edge/Application/msedge.exe"),
            PathBuf::from(p).join("Google/Chrome/Application/chrome.exe"),
        ]
    })
    .find(|p| p.exists())
}

fn render(url: &Url) -> Result<String> {
    let browser =
        browser_binary().context("Install Edge or Chrome to enable JavaScript rendering")?;
    let profile = tempfile::tempdir()?;
    let mut cmd = Command::new(browser);
    cmd.args([
        "--headless",
        "--disable-gpu",
        "--no-first-run",
        "--no-default-browser-check",
        "--blink-settings=imagesEnabled=false",
        "--dump-dom",
        "--virtual-time-budget=5000",
    ])
    .arg(format!("--user-data-dir={}", profile.path().display()))
    .arg(url.as_str())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().context("Browser output unavailable")?;
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut bytes = Vec::new();
        let mut stdout = stdout;
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let start = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(45) {
            child.kill()?;
            anyhow::bail!("Browser rendering timed out");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Browser output reader failed"))??;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[derive(Default)]
struct MappingEstimate {
    samples: VecDeque<f64>,
}
impl MappingEstimate {
    fn observe(&mut self, seconds: f64) {
        self.samples.push_back(seconds.max(0.001));
        if self.samples.len() > 20 {
            self.samples.pop_front();
        }
    }
    fn remaining(&self, pages: usize) -> Option<f64> {
        if pages == 0 {
            return Some(0.0);
        }
        if self.samples.len() < 2 {
            return None;
        }
        let (weighted, weights) =
            self.samples
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(sum, weights), (i, time)| {
                    let weight = (i + 1) as f64;
                    (sum + time * weight, weights + weight)
                });
        Some(weighted / weights * pages as f64)
    }
}

pub fn map_site(options: CrawlOptions, cancel: Cancel, events: Sender<Event>) -> Result<SiteMap> {
    map_site_at(options, cancel, events, &checkpoint_path())
}

fn map_site_at(
    options: CrawlOptions,
    cancel: Cancel,
    events: Sender<Event>,
    checkpoint: &std::path::Path,
) -> Result<SiteMap> {
    let base =
        Url::parse(&options.website).context("Enter a complete http:// or https:// website URL")?;
    anyhow::ensure!(
        matches!(base.scheme(), "http" | "https"),
        "Only HTTP and HTTPS websites are supported"
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(40))
        .user_agent("ImageAtlas/0.1 site-owner image mapper")
        .build()?;
    let mut queue = VecDeque::from([(base.clone(), Vec::new())]);
    if options.sitemap
        && let Ok(u) = base.join("/sitemap.xml")
    {
        queue.push_back((u, Vec::new()));
    }
    let mut discovered = HashSet::new();
    queue.retain(|(url, _)| discovered.insert(url.to_string()));
    let mut estimate = MappingEstimate::default();
    let mut scanned = 0;
    let mut seen = HashSet::new();
    let mut unique = HashSet::new();
    let mut map = SiteMap {
        website: base.to_string(),
        ..Default::default()
    };
    if let Some(saved) = read_checkpoint(checkpoint).filter(|job| job.options == options) {
        queue = saved
            .queue
            .into_iter()
            .map(|(url, path)| Ok((Url::parse(&url)?, path)))
            .collect::<Result<_>>()?;
        seen = saved.seen;
        scanned = saved.scanned;
        map = saved.map;
        discovered = seen.clone();
        discovered.extend(queue.iter().map(|(url, _)| url.to_string()));
        unique.extend(map.assets.iter().map(|asset| {
            (
                asset.url.clone(),
                asset.folders.clone(),
                asset.label.clone(),
            )
        }));
    }
    #[derive(serde::Serialize)]
    struct Snapshot<'a> {
        options: &'a CrawlOptions,
        map: &'a SiteMap,
        queue: Vec<(&'a str, &'a Vec<String>)>,
        seen: &'a HashSet<String>,
        scanned: usize,
    }
    let persist =
        |queue: &VecDeque<(Url, Vec<String>)>, seen: &HashSet<String>, scanned, map: &SiteMap| {
            save_checkpoint(
                checkpoint,
                &Snapshot {
                    options: &options,
                    map,
                    queue: queue
                        .iter()
                        .map(|(url, path)| (url.as_str(), path))
                        .collect(),
                    seen,
                    scanned,
                },
            )
            .context("Could not save mapping recovery progress")
        };
    persist(&queue, &seen, scanned, &map)?;
    while !queue.is_empty() {
        if cancel.stopped() {
            map.notices
                .push("Mapping stopped by user; the partial map is available for review".into());
            break;
        }
        let (url, folders) = queue.pop_front().unwrap();
        if !seen.insert(url.to_string()) {
            continue;
        }
        let request_started = std::time::Instant::now();
        let _ = events.send(Event::MappingProgress {
            scanned,
            queued: queue.len() + 1,
            images: map.assets.len(),
            eta_seconds: estimate.remaining(queue.len() + 1),
            current_url: url.to_string(),
        });
        let source = if options.render && !url.path().ends_with(".xml") {
            render(&url)
        } else {
            client
                .get(url.clone())
                .send()
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.text())
                .map_err(Into::into)
        };
        match source {
            Ok(source) => {
                let page = parse_page(&source, &url, &folders);
                map.pages += 1;
                for asset in page.assets {
                    let key = (
                        asset.url.clone(),
                        asset.folders.clone(),
                        asset.label.clone(),
                    );
                    if unique.insert(key) {
                        map.assets.push(asset);
                    }
                }
                for (next, path) in page.links {
                    if next.origin() == base.origin() && discovered.insert(next.to_string()) {
                        queue.push_back((next, path));
                    }
                }
            }
            Err(e) => map.notices.push(format!("{}: {}", url, e)),
        }
        scanned += 1;
        persist(&queue, &seen, scanned, &map)?;
        estimate.observe(request_started.elapsed().as_secs_f64());
        let _ = events.send(Event::MappingProgress {
            scanned,
            queued: queue.len(),
            images: map.assets.len(),
            eta_seconds: estimate.remaining(queue.len()),
            current_url: url.to_string(),
        });
    }
    if queue.is_empty() {
        std::fs::remove_file(checkpoint).context("Could not clear completed mapping recovery")?;
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping_resumes_pending_page_without_refetching_completed_pages() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let website = format!("http://{}/", server.local_addr().unwrap());
        let pending = format!("{website}pending");
        let options = CrawlOptions {
            website: website.clone(),
            render: false,
            sitemap: false,
        };
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("mapping.json");
        let map = SiteMap {
            website: website.clone(),
            pages: 7,
            ..Default::default()
        };
        save_checkpoint(
            &path,
            &Checkpoint {
                options: options.clone(),
                map,
                queue: VecDeque::from([(pending.clone(), vec!["Category".into()])]),
                seen: HashSet::from([website.clone()]),
                scanned: 7,
            },
        )
        .unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0; 4096];
            let length = socket.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..length]).starts_with("GET /pending "));
            let body = "<h1>Recovered</h1><img src='/original.svg'>";
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let (tx, _) = crossbeam_channel::unbounded();
        let map = map_site_at(options, Cancel::default(), tx, &path).unwrap();
        worker.join().unwrap();
        assert_eq!(map.pages, 8);
        assert_eq!(map.assets.len(), 1);
        assert!(!path.exists());
    }
    #[test]
    fn cancelled_mapping_keeps_pending_queue_and_atomic_replacements_are_readable() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("mapping.json");
        let options = CrawlOptions {
            website: "https://example.test/".into(),
            render: false,
            sitemap: true,
        };
        let cancel = Cancel::default();
        cancel.stop();
        let (tx, _) = crossbeam_channel::unbounded();
        map_site_at(options.clone(), cancel.clone(), tx.clone(), &path).unwrap();
        let saved = read_checkpoint(&path).unwrap();
        assert_eq!(saved.queue.len(), 2);
        map_site_at(options, cancel, tx, &path).unwrap();
        assert_eq!(read_checkpoint(&path).unwrap().queue.len(), 2);
    }
    #[test]
    fn generic_mapping_uses_original_and_breadcrumb() {
        let source = r#"<html><title>Shop</title><nav><img src="logo.png"></nav><div class="breadcrumb"><a>Home</a><a>Prints</a></div><h1>Poster</h1><img src="small.jpg" srcset="small.jpg 100w, full.jpg 1200w" alt="Poster"><a href="/next">Next</a></html>"#;
        let p = parse_page(
            source,
            &Url::parse("https://example.test/shop/poster").unwrap(),
            &[],
        );
        assert_eq!(p.assets.len(), 1);
        assert!(p.assets[0].url.ends_with("full.jpg"));
        assert_eq!(p.assets[0].folders, vec!["Prints"]);
        assert_eq!(p.links.len(), 1);
    }
    #[test]
    fn rejects_non_web_urls() {
        let base = Url::parse("https://example.test").unwrap();
        assert!(web_url(&base, "javascript:alert(1)").is_none());
        assert!(web_url(&base, "file:///etc/passwd").is_none());
    }
    #[test]
    fn mapping_eta_uses_measured_times_and_current_queue() {
        let mut estimate = MappingEstimate::default();
        assert_eq!(estimate.remaining(5), None);
        estimate.observe(1.0);
        assert_eq!(estimate.remaining(5), None);
        estimate.observe(3.0);
        let earlier = estimate.remaining(3).unwrap();
        assert!((earlier - 7.0).abs() < 0.001);
        assert!((estimate.remaining(6).unwrap() - earlier * 2.0).abs() < 0.001);
        estimate.observe(15.0);
        assert!(estimate.remaining(3).unwrap() > earlier);
        assert_eq!(estimate.remaining(0), Some(0.0));
    }
    #[test]
    fn mapping_reports_unique_pending_pages_and_completion() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let website = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for number in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0u8; 4096];
                socket.read(&mut request).unwrap();
                std::thread::sleep(Duration::from_millis(15 * (number + 1)));
                let body = if number == 0 {
                    "<h1>Home</h1><a href='/one'>One</a><a href='/one'>Duplicate</a><a href='/two'>Two</a>"
                } else {
                    "<h1>Product</h1><img src='/original.svg'>"
                };
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let (tx, rx) = crossbeam_channel::unbounded();
        let temp = tempfile::tempdir().unwrap();
        let map = map_site_at(
            CrawlOptions {
                website,
                render: false,
                sitemap: false,
            },
            Cancel::default(),
            tx,
            &temp.path().join("mapping.json"),
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(map.pages, 3);
        let samples = rx
            .try_iter()
            .filter_map(|event| {
                if let Event::MappingProgress {
                    scanned,
                    queued,
                    eta_seconds,
                    ..
                } = event
                {
                    Some((scanned, queued, eta_seconds))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(
            samples
                .iter()
                .any(|(scanned, queued, _)| *scanned == 1 && *queued == 2)
        );
        assert!(
            samples
                .iter()
                .any(|(_, queued, eta)| *queued > 0 && eta.is_some_and(|s| s > 0.0))
        );
        assert_eq!(samples.last().unwrap(), &(3, 0, Some(0.0)));
    }
}
