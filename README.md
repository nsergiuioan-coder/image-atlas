# Image Atlas

A native Windows workbench for mapping website images, collecting originals into useful folders, and preparing local batches. Rust with egui; no webview or bundled browser runtime.

## Use

1. Enter the complete website URL in Website map. Enable JavaScript rendering for dynamic sites if needed; Edge or Chrome must already be installed.
2. Review the map before downloading. Filter entries, select images, edit product names and category paths, or save the map as JSON. Category paths can contain any number of levels separated by `/`.
3. Choose a destination in Downloads. Leave the connection hint at zero for automatic operation, or enter the connection speed in Mbps. The scheduler adjusts connections using measured throughput and server backpressure. The ETA is approximate and includes observed response delays; differently sized images and retries can change it.
4. Download selected images. The app keeps original bytes, detects the true extension, retains SVG vectors, resumes ETag-backed partial downloads, and reuses completed originals. The manifest records source URLs and hashes; failures are explicit. ZIP is optional.
5. In Batch studio, import files, a folder or dropped images. Source dimensions appear automatically. Zero width and height preserve dimensions; setting only one preserves aspect ratio. Physical width uses the selected DPI to calculate pixels. PDF page dimensions also use that DPI.
6. Browse the batch using Previous/Next or the slider. Scroll to zoom and drag to pan. Switch between Original and Edited. Add independent color rules for separate objects and the background, then use Pick from image. A rule can target edge-connected background, every matching color, or the connected area you click. Tolerance controls matching; feather softens matches near the tolerance boundary. Rules apply in order. Preview changes before exporting.

Batch exports: PNG, JPEG at quality 95, lossless WebP, TIFF, BMP, GIF, ICO, QOI, TGA, AVIF, PDF, embedded-raster SVG, and traced-vector SVG. SVG tracing is intended for flat artwork and can be slow on photographs. Embedded SVG retains pixels. PDF is export-only. Animated files are flattened to the decoded frame. AVIF export is supported; importing AVIF depends on the decoder features in the build. Raster-only formats flatten transparency onto white where required.

## What discovery covers

Public linked pages and sitemap entries, breadcrumbs, high-resolution responsive sources, lazy image attributes, image links, OpenGraph metadata, JSON-LD, inline SVG, inline CSS image URLs, and a public storefront catalog adapter. The optional isolated browser renderer handles many JavaScript sites. External CSS assets, images hidden behind logins, CAPTCHAs, scroll-only loading, unlinked originals, private media libraries and site-specific APIs may need an additional adapter. A public website backup cannot include files the website never exposes. Small originals are reported rather than enlarged and labelled high resolution.

The crawler stays on the starting website for page discovery; image hosts linked by those pages may be external. It does not extract browser credentials. Mapping is sequential to keep server load predictable. Downloads stream to disk and adapt concurrency, with one original reused across category placements. Batch conversion processes one image at a time to avoid holding the batch in RAM. Some encoders and vector tracing need more resources than ordinary downloads.

## Install and build

Use `ImageAtlas-0.1.0-setup.exe` for a per-user installation without administrator rights. The portable ZIP can be extracted anywhere. Windows 10/11 x64 is the packaged target. The initial build is unsigned.

Requires a current Rust MSVC toolchain, Python 3 and NSIS to build the installer:

```powershell
./scripts/build.ps1 -Nsis 'C:/Program Files (x86)/NSIS/makensis.exe'
```

The build runs formatting, release tests and Clippy, generates dependency notices, and produces the installer, portable ZIP and SHA-256 sums in `dist`.

## Updates and releases

Updates come from GitHub Releases, not directly from arbitrary source commits. Bump `version` in Cargo.toml and regenerate Cargo.lock before pushing an update. The Windows workflow builds and tests each relevant main-branch push, then creates a release only when that version does not already exist. It attaches the installer and checksum file.

The application checks `nsergiuioan-coder/image-atlas` by default; change owner/repository in Settings if the final repository name differs. Users choose Update, Later, or Don't remind me about this version, and can disable automatic checks. Update downloads are checked against release SHA-256 sums before launching the installer. Close Image Atlas when the installer asks to replace an existing executable. No public repository or release is created by building locally.

## License

Copyright (c) 2026 NIS. MIT, matching Etichette Studio. Dependency authors and licenses are in ATTRIBUZIONI.txt and THIRD-PARTY-LICENSES. Original code contains no comments and no copied third-party implementation.

## Vector editor

Open an SVG to select objects on the canvas or in Layers. Drag an object to move it, use Smaller/Larger to resize around its center, change fill and stroke, reorder siblings with Front/Back, delete objects, and undo/redo. Add rectangles, ellipses or text and save a valid SVG. A raster input is embedded as one image object; its pixels do not become separate vector objects. Node editing, pen curves, boolean geometry, CorelDRAW files and full CSS authoring are outside this first version.

The studio preview supports independent color rules with connected-area selection and an optional rectangle restriction. New rules are inactive until a color is picked. Area positions are relative and are reused across the batch, so review each image with the carousel before exporting mixed layouts.

## Interrupted downloads

The current download map and destination are saved before transfer starts. After restarting the app, an unfinished job is offered for resumption. Completed files are reused, and ETag-backed partial files can continue from their saved position. Transient connection and server failures trigger another pass after an increasing cooldown, from 15 seconds up to 5 minutes. Permanent errors remain listed for review. Stop keeps the saved job. The newest recovery changes have been compiled but have not yet had a prolonged network-outage test.
