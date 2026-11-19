# Print and open performance

## Preview first, print quality in the background

Make print sheets runs the existing planner and renderer with a preview image
provider. Physical rectangles, sheet assignment, labels, hatches and audits
use the original geometry. Each band reads the nearest coarser resident pyramid
level for a page with a 1600 px long side, then applies the display LUT. It does
not crop or enhance the full-resolution mosaic. Preview omits denoise,
sharpening and background stripping; a faint note explains this. Whole-area
and older grain/area layouts also use pyramid previews without changing their
physical layout or full-resolution export settings.

All preview SVGs arrive together. Only the selected page is mounted, and only
visited pages allocate SVG Blobs. After the first preview can paint, the
`PrintQuality` controller requests full-quality work in page order. Native
commands use `spawn_blocking`, with Rayon over bands and 32-row NLM chunks
inside each band. The web build uses its wasm worker. Each page request is a
cooperative boundary: canvas requests can run between pages, leaving Layout
pauses further requests, and returning resumes at the next page. Cancel also
pauses at a page boundary; a running band is allowed to finish. Input changes
stop preparation and block export until Make is used again. Replaced job ids,
changed pixel allocations and out-of-order pages are rejected by the core.

A non-modal instance of the existing Busy view reports Preparing print quality,
page k of n. Export uses `app.run` to wait for remaining preparation and then
reports Preparing PDF fonts and Writing PDF, page k of n, then Finishing PDF
and Saving PDF. The Busy view receives the current task explicitly, and stays
visible through the native save dialog/write. Cancelled or stale exports do not
download; partial PDF assembly is released on errors and cancellation, even
after source pixels change.
The older layout recalculates its inexpensive placement for each full-quality
page but crops only that page's images. Whole-area printing has one page.

Full-quality grain bands use the original crop, tone, enhancement and PNG
encoder. The last preparation step assembles the original complete renderer's
result from these images. Full-resolution page strings remain in the core;
they do not replace the on-screen previews or cross IPC during preparation.
The full result is retrieved only for the browser print fallback. PDF assembly
uses it directly in the core. Registration statistics and background decisions
are updated from the full-quality summary when preparation finishes.

## Caches and lifetime

A native session keeps a 512 MiB LRU of enhanced band PNGs; wasm uses 256 MiB.
Keys include area, mosaic rectangle, pixel allocation revision, dimensions,
calibration, display LUT and every enhancement parameter. Background stripping
also keys on grain geometry. A weak reference preserves allocation identity
without retaining replaced mosaics. Hits update recency, eviction removes the
least recently used images, and an oversized image is returned without caching.
Packing, paper and scale changes reuse bands when their source rectangles
remain unchanged. PDF reuses the PNGs already assembled into full-quality SVGs.
The active result and its temporary assembly buffers are separate from this
band-cache budget; paused jobs also count toward the UI result-cache budget.

The UI result cache belongs to the app session and project. Immutable grain,
spot and label arrays supply cheap revisions. Its remaining fingerprint covers
print controls, mosaic and placement revisions, metadata, tone, saved edits,
package identity and published revision. Returning with changed inputs leaves
the preview empty. Unchanged inputs preserve pages, editor selection, split
controls, statistics, undo/redo history and preparation progress.

At most two idle results share a 300 MiB page-data budget. The estimate
allows two bytes per JavaScript code unit and three for its UTF-8 Blob. The
core reports resident preview/full SVG and temporary band PNG bytes after each
page. These also count toward the idle-result budget. If a running page
finishes after leaving Layout, its cached entry is reweighed and evicted if
necessary; a taken or replaced entry is never reinserted. Taking a result
transfers URL ownership to the mounted screen. Eviction, replacement
and project close revoke its URLs and release the corresponding core job.
Core project close also releases band images and PDF assembly state.

## One PDF export

Grain sheets, whole-area sheets and older layouts offer one multi-page PDF.
The desktop uses one save dialog; the web uses one download. The automatic name
is `<mount>_print_<YYYY-MM-DD>.pdf`, with Windows/macOS unsafe characters and
trailing dots/spaces removed. Every grain/whole PDF embeds its complete
registration JSON in the PDF catalog’s `/Names/EmbeddedFiles` tree. The web
build downloads only this PDF. The desktop sends one binary packet, asks for
the PDF path once, and atomically writes the PDF and
`<chosen PDF stem>_registration.json` beside it. Cancelling the dialog writes
neither file. The Registration button still offers an explicit separate export.
Older layouts have no print registration to attach.
Per-page SVG export was the only UI download loop found and has been removed.
Grain export already chooses one folder or downloads one ZIP. Other UI exports
produce one file each.

`PdfBuilder` converts vector layers in their original SVG order and embeds the
existing grayscale PNG IDAT streams using PDF's PNG predictor. It does not
decode, expand to RGB or recompress those image pixels. Unsupported image
shapes/formats retain the svg2pdf conversion path. Incremental page assembly
runs in the native blocking pool or wasm worker; bytes cross to the UI once.
PDF page ordering and completion are checked before finishing. A page-by-page
font preflight collects shaping glyphs; verified hidden samples give every
vector layer the same font subset and CID mappings. Context-sensitive scripts
retain complete shaping samples when a compact cluster alphabet is insufficient.
The generated resource graph is interned by content, including font dependency
trees and repeated XObjects. Parsing uses pdf-writer’s own object offsets and
reference rewriting, so image streams and literal strings are preserved.
Repeated grayscale PNG IDAT streams also share one image XObject.

## Method and results

`print_bench` reads the ignored synthetic bundle or a private source package
without writing either. Package mode stitches and segments with default
parameters. Both modes use default sheet parameters with denoise and sharpening
and an identity display LUT. It separates planning, crop/enhancement, PNG,
base64, serialization, preview, full preparation, cached repacking and PDF bytes.
`band_cpu_ms` and `png_cpu_ms` sum elapsed band timings, including nested Rayon
waits; they are not process CPU measurements. Elapsed render includes those
stages, SVG construction and audits. Staged quality includes the final original
renderer assembly. Staged PDF includes per-page dispatch inside the core probe.

The second-pass native comparison alternated three before/after release runs per
workload, with `RAYON_NUM_THREADS=4`, on the same macOS host without concurrent
builds. Before is the preserved first-pass executable; after is this staged
implementation. Values below are medians. The read-only private workload has
13 bands on 13 pages; its identity and path are deliberately omitted.

| Measurement | Synthetic before | Synthetic after | Private before | Private after |
| --- | ---: | ---: | ---: | ---: |
| Planning | 12.1 ms | 12.7 ms | 37.7 ms | 37.8 ms |
| All previews (previously full render before display) | 4,280 ms | 126 ms | 10,335 ms | 196 ms |
| Cold full renderer | 4,280 ms | 4,590 ms | 10,335 ms | 10,757 ms |
| Background full preparation, including final assembly | — | 5,064 ms | — | 11,132 ms |
| Repack with enhanced-band cache | — | 63 ms | — | 211 ms |
| Same full SVG pages to PDF bytes | 1,495 ms | 1,486 ms | 1,157 ms | 143 ms |
| Incremental staged PDF bytes | — | 1,575 ms | — | 143 ms |
| PDF bytes, same full SVG input | 19,583,028 | 30,561,941 | 13,311,455 | 13,040,378 |
| Full-result JSON bytes | 14,935,891 | 14,935,891 | 17,011,066 | 17,011,066 |
| Preview SVG bytes | — | 12,670,004 | — | 1,679,035 |

Synthetic preview range was 124–131 ms; private preview range was 192–236 ms.
The synthetic workload has 763 bands on 672 pages. Background preparation is
not a reduction in cold enhancement CPU time: the point is to remove that work
from the preview wait and reuse it after edits. The cold renderer was 7% slower
on the synthetic workload and 4% slower on the private workload in these runs.
Page dispatch and final assembly add further overhead. The image-heavy private
PDF conversion is about 8 times faster. On the many-page synthetic workload,
vector/font conversion dominates and PDF time is effectively unchanged; its
PDF grew by 56% because separate vector layers repeated font resources. The
third-pass sharing below removes that regression; the PDF rows in this table
are historical second-pass results.
Staged PDFs include the original session's additional registration metadata,
so their sizes differ slightly from the direct renderer's benchmark PDFs.

### Third pass: export failure and resource sharing

The supervisor’s 20-minute timeout was waiting to click a disabled export, not
running PDF conversion. The default synthetic automatic scale is 4 µm/mm;
its unchanged plan has one `outside_body` band on sheet 164, ending at
299.3 mm on 279.4 mm-high paper. `exportBlocked` correctly rejected this plan,
but its reason was hidden outside the strip editor. Layout now shows the first
blocking problem next to the preview, with scale/paper/edit guidance. No layout
rule or export validation was weakened. An ordinary e2e assertion covers this
visible blocker.

The perf spec now explicitly uses 5 µm/mm: 487 valid pages on the same bundle.
It checks that export is enabled before clicking, bounds click/download waits,
and emits a `before-pdf` JSON line with open, preview, quality and cached-return
milestones before attempting export. A `complete` line adds PDF bytes readiness
and asserts that progress remains visible throughout conversion. Page count
and scale are recorded, so these browser numbers must not be compared as if
this were the previous 672-page layout.

The original 672-page full-resolution workload was retained for direct wasm
measurements in Node, without browser throttling or UI/IPC. Both runs used the
same project and enhanced pages on this host:

| Measurement | Before sharing | After sharing |
| --- | ---: | ---: |
| Wasm PDF bytes ready | 2,249 ms | 4,422 ms |
| Font preflight (included above) | — | 1,190 ms |
| Staged PDF bytes, registration included | 30,804,346 | 13,525,726 |
| Font objects in complete PDF | repeated per layer | 1 |

The PDF is 56.1% smaller than the second-pass staged PDF and below the original
converter’s 19,583,028-byte output. Sharing adds conversion CPU work: the wasm
PDF takes about 2.17 s longer, rather than a 20-minute stall. A compact verified
glyph seed reduced the first sharing implementation from 10.04 s to 4.42 s.
The final wasm run also measured 264 ms for previews and 28.52 s for full
quality. These are host measurements, not Windows/WebView2 or 4x-throttled
browser results.

One fresh release native run with four Rayon threads measured 122 ms for all
previews, 4,744 ms for full preparation and 63 ms for cached repacking. PDF
conversion of the direct renderer’s identical full SVGs took 2,860 ms and
produced 12,431,099 bytes (previously 1,486 ms and 30,561,941 bytes). Staged PDF,
including the session registration attachment, took 2,992 ms and produced
13,527,967 bytes (previously 1,575 ms and 30,804,346 bytes). These third-pass
numbers are individual runs; the earlier table contains three-run medians.
The native full-result hash below remained unchanged. Raw measurements and
before/after PDF artifacts remain under ignored `target/perf/`.

Opening is unchanged in this pass. In the paired runs, synthetic bundle
mosaic/pyramid construction was 649 ms before and after; tile decoding was
261 ms before and 255 ms after. The private package's summed stitch/correction/
blend/tone/pyramid steps were 3,427 ms before and 3,436 ms after. These timings
exclude browser first paint and package segmentation.

The complete synthetic full-resolution render-result SHA-256 remains:
`a7d1db6e44399767b34f93abbd1e87774659ba2b016d62390a964182479f5736`.
New integration tests compare every full-result byte, including PNGs, between
the original and staged grain-sheet paths, including strip edits and background
stripping. Whole-area and older layouts compare complete result values, allowing
JSON object key order to differ. Synthetic PDF pages 1, 164 and 672 rendered through Poppler with a 1600 px
long side were pixel-identical before and after resource sharing, including
hatches, labels and scale bars. Poppler also found the registration attachment. PDF bytes
are not used as an equality oracle because svg2pdf resource ordering varies.

The supervisor's first-pass browser result was 803 ms to the first canvas image,
28,618 ms to the first print page and 23 ms for cached return. Its 89,493 ms
all-pages metric included 672 Playwright pagination/decode round trips. The
updated spec measures all preview SVGs available separately from the first
painted/decoded page, then full-quality readiness and PDF bytes after clicking
Export PDF. It keeps 4x CDP CPU throttling and now prints the earlier milestones
before attempting PDF, followed by a complete JSON line on success.

First-pass native open medians were 2.52 s to 1.70 s for bundle mosaic/pyramid
construction and 7.06 s to 6.76 s for a source-package pipeline. This second pass
does not alter opening. Browser paint, installed WKWebView/WebView2 behaviour
and Windows output still require supervisor checks. Commands are in
[developing.md](developing.md#print-performance-checks).

## Verification

`make full` passed: the existing layout properties and sheet tests, full-result
parity tests, desktop compilation, wasm release rebuild, Svelte checking,
176 Node tests and 59 Vitest tests. Both native PDF/sidecar save tests passed. Core and wasm clippy passed with warnings
denied. The static UI build passed. Playwright specs collect successfully;
runtime browser tests and installed macOS/Windows checks are left to the
supervisor because this workspace cannot bind ports or run the desktop app.
The strip-editor export assertion now checks that SVG export is absent and
that all remaining export actions stay blocked when strips have problems.
