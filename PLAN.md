# Image Atlas

## Workflow

1. Enter a website URL and map its public pages before downloading.
2. Review images by category, subcategory, product/page and source URL. Edit folders, exclude items, and save or import the map.
3. Download selected originals with resume, duplicate reuse, cancellation, an adaptive scheduler and a CSV manifest.
4. Import local images or the download folder. Inspect pixel dimensions, choose pixels or physical size plus DPI, and preview batch changes.
5. Export common raster formats, lossless PDF pages, embedded-image SVG or traced vector SVG.
6. Replace a chosen background color using tolerance, optional edge-connected selection and feathering.

## Modules

- `model`: saved maps, item identity, safe relative paths and job events.
- `crawl`: generic HTML, sitemap, structured data, storefront adapter and optional isolated browser rendering.
- `download`: streaming HTTP, resume metadata, integrity hashes, adaptive concurrency and archive creation.
- `convert`: decode, dimension detection, resizing, background replacement and encoders.
- `app`: small native interface with separate mapping, download and batch workspaces.

## Delivery

Windows native executable, per-user installer, MIT license matching Etichette Studio, dependency attributions, source, tests, and release instructions. No GitHub push or public release before the owner's approval. Original code has no comments. Any copied third-party source must preserve its author and license at the top.

## Validation

Path traversal and reserved-name tests; generic HTML and storefront mapping fixtures; local HTTP download/resume and throttling tests; pixel-level background and aspect-ratio tests; real format exports and installer smoke test. Network adaptation uses observed transfer rate, latency and server back-pressure rather than an assumed ISP speed. Sites requiring access checks or special navigation need an explicit adapter or user-assisted access; no universal crawler can guarantee every website.
