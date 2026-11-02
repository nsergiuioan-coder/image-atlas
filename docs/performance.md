# Resource measurements

Local comparison against version 0.1.2, using the same optimized Rust toolchain, three alternating runs per version and the median result. Image outputs had identical SHA-256 hashes in each comparison.

| Workload | 0.1.2 | 0.1.4 |
| --- | ---: | ---: |
| Three full-resolution JPEG exports, image processing | 0.817 s | 0.674 s |
| Peak working set during that workload | 139.9 MiB | 98.7 MiB |
| CPU time for that process | 0.859 s | 0.734 s |
| Three web-sized exports, image processing | 1.251 s | 1.162 s |
| Peak working set during web-sized workload | 138.4 MiB | 143.5 MiB |
| Crawl 25 local pages, 24 responses delayed by 60 ms | 1.588 s | 0.735 s |

The image fixture is a 4096×3072 RGBA colour gradient. Native exports use JPEG quality 85 and retain the dimensions. Web exports use automatic format selection and a 1600-pixel longest edge. The process also runs a 300-frame headless UI workload before its image work. Peak working set includes this test process; it does not measure the complete desktop window, GPU allocation, or every input format.

The new shaping engine increases UI layout cost: this fixture took about 36 ms for 300 headless UI frames, compared with 27 ms previously. Web-sized export peak memory was about 5 MiB higher. Full-resolution export avoids larger intermediate pixel buffers and therefore reduced peak memory. Results vary with the input image, fonts, machine load and renderer; these figures are examples, not a promise that every operation uses less memory.

The crawler now overlaps requests and parsing, saves pending and in-flight work durably, and reduces concurrency on transient failures or server throttling. The local crawl used about 13.7 MiB peak working set, compared with 8.6 MiB previously; CPU time increased from 0.078 to 0.203 seconds while elapsed time fell. Browser-rendered mapping uses one browser page at a time to avoid launching multiple browser processes. Real websites have different response times and rate limits, so the local timing does not predict a specific site's speed.

The ignored tests in `src/profiling.rs` reproduce image, local-server and headless language-rendering workloads. Set `IMAGE_ATLAS_SAMPLE` to an input file for the image workload and `IMAGE_ATLAS_PROFILE_NATIVE=1` for native-size JPEG export. Desktop resource profiling remains separate from these headless measurements.

## Dima rendering, 0.1.10

A local release-build comparison used a 700 × 1000 RGBA image fitted inside a 200 × 300 mm dima at 300 DPI. Three alternating old/new renders were performed for each corner radius. Every output pixel was compared, with identical results. Median elapsed time, including fitting and drawing the dima:

| Corner radius | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| 0 mm | 279.8 ms | 166.2 ms | 40.6% |
| 8 mm | 272.7 ms | 169.7 ms | 37.8% |

The renderer now processes edge bands rather than repeating border-distance calculations over untouched interior pixels. This is a rendering measurement, not an estimate of total export speed.

The AI path also stores the original transparency mask as one byte per input pixel instead of copying a four-byte RGBA image, and checks for black output without allocating a full RGB copy of the enlarged result. Opaque RGB inputs bypass an unnecessary RGBA conversion when flattening. Whole-process RAM savings were not measured in this pass.
