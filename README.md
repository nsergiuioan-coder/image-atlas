# Image Atlas

Image Atlas is a Windows desktop app for collecting website images, organizing them into folders, and preparing image batches. Review what a website exposes before downloading, adjust the folder structure, then edit and export your files from one workspace.

## Download

Get the Windows installer or portable ZIP from [Releases](https://github.com/nsergiuioan-coder/image-atlas/releases).

The installer runs per user without administrator rights. The portable version runs from an extracted folder. Windows 10 or 11, 64-bit, is required. The current release is an unsigned preview.

## Website images

- Map public pages and sitemaps before starting a download.
- Review image sources, select files, and edit category and subcategory folders.
- Save maps and reopen them later.
- Download available originals without converting them. File contents determine the extension, so an SVG remains an SVG and a JPEG is saved as a JPEG.
- Adapt connection counts to observed throughput and server responses, with an approximate time remaining.
- Resume saved jobs, reuse completed downloads, and continue partial downloads when the server supports ETag-based resumption.
- Export a ZIP archive and a CSV manifest containing source URLs and file hashes.

Optional browser rendering supports many JavaScript-based websites and requires an installed copy of Edge or Chrome.

## Batch editor

Import images or a folder, then browse the batch with Previous, Next, or the image slider. Zoom, pan, and compare the original with the edited preview.

Resize by pixel dimensions or calculate a physical width using DPI. Replace colors with independent rules for a connected area, the background at the edges, or matching colors throughout the image. Pick colors directly from the preview and restrict a rule to a rectangle to protect nearby artwork. Tolerance and feather controls adjust the match.

Export PNG, JPEG, WebP, TIFF, BMP, GIF, ICO, QOI, TGA, AVIF, PDF, or SVG. SVG export offers two options: embedding a raster image or tracing artwork into vector shapes.

## Vector editor

Select SVG objects on the canvas or in the layer list. Move and resize them, change fill and stroke, adjust their stacking order, or delete them. Add rectangles, ellipses, and text, use undo and redo, then save the result as SVG.

A PNG or JPEG opens as one image object. Tracing can create editable shapes; it does not recover the original document's layers or text.

## Preview limitations

Image discovery depends on what a website makes publicly accessible. Private libraries, access challenges, externally defined CSS images, and some scrolling or site-specific interfaces are not covered. A small source image cannot supply a higher-resolution original.

PDF is export-only. AVIF export is supported, but AVIF import is not enabled in the packaged build. Animated inputs are flattened to the decoded frame. Color selection regions are reused at the same relative position across a batch, so check the preview when layouts differ.

The vector editor does not support CorelDRAW documents, path-node editing, pen tools, or boolean geometry. Automatic recovery from prolonged network outages is experimental.

## Build from source

The application uses Rust and egui. Building the Windows installer also requires Python 3, NSIS, and the Rust MSVC toolchain.

```powershell
cargo run --release
```

To run the checks and build the installer and portable package:

```powershell
./scripts/build.ps1 -Nsis 'C:/Program Files (x86)/NSIS/makensis.exe'
```

Build outputs and SHA-256 checksums are written to `dist`.

## Updates

Published GitHub releases supply app updates. Users can install an update, postpone it, skip a version, or turn off automatic checks. Installer downloads are verified against the release checksums.

For maintainers, the Windows workflow builds and tests relevant pushes to `main`. Bumping the package version creates a new release when that version has not already been published.

## License

Copyright (c) 2026 NIS. Licensed under the [MIT License](LICENSE).

Third-party authors and licenses are listed in [ATTRIBUZIONI.txt](ATTRIBUZIONI.txt) and [THIRD-PARTY-LICENSES](THIRD-PARTY-LICENSES).

Vibe coded.
