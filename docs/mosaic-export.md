# Export stitched mosaics

In the grain panel, choose **Export mosaic...** beside **Export grains...**.
Choose the current area, chosen areas (one image each), or the whole mount.
Original export reads source fields directly, including areas whose mosaic was evicted or never built. As displayed needs a previously built base image (or loaded published pixels); there is no displayed image before that base exists.
This export does not edit the project, registered data, mirror or outbox.

**Original values** writes blended, corrected intensities before tone,
rounded to the source depth (16-bit when any source tile is 16-bit, otherwise
8-bit). Corrections include configured gain/offset, background correction,
tile overrides and HDR merge. **As displayed** writes the current 8-bit base
mosaic through the area's display LUT, including the base stretch, CLAHE and
per-grain normalisation already applied on screen. Published bundles contain
only 8-bit pixels; exporting them cannot recover original source precision.
The current application pipeline converts source RGB to grayscale before
stitching, so application exports are grayscale. The TIFF writer itself also
supports RGB; preserving RGB through the application pipeline is not implemented.

Each TIFF has 512 × 512 uncompressed tiles, including zero padding outside the
image dimensions. Uncompressed output permits sequential writes with known
offsets, without seeking or a file-sized compression buffer. The current
`tiff` crate encoder has BigTIFF support but no tiled image encoder. Our small
writer emits classic TIFF below a conservative 3.5 GB threshold and BigTIFF
above it. The threshold estimate includes edge padding, offset arrays and
1 MiB metadata headroom. TIFF resolution tags use pixels per centimetre,
converted from the pixel size in µm; sample format is unsigned integer.
`ImageDescription` contains short JSON identifying the project, areas, values
mode, version and JSON sidecar.

Names are `<mount>_<area>_mosaic.<ext>` per area and
`<mount>_mount_mosaic.<ext>` for a whole mount. Each name part is limited to
80 UTF-8 bytes so compound names and split suffixes fit common filesystem
component limits. Windows-invalid characters are
replaced and reserved device names are prefixed. Empty names become `mosaic`.
Case-insensitive collisions get `-2`, `-3`, etc. Actual names and identifiers
remain verbatim in JSON. The JSON records pixel size, dimensions, depth,
coordinate convention, source package identity, source tile hashes, solved
placement, photometric correction, version and both tone stages for displayed
exports. Names and source paths in these exported files are user data.

## Coordinates

Every image has `<name>.json` and a world file: `.tfw` for TIFF, `.pgw` for PNG,
`.jgw` for JPG. Units are µm, never degrees.
The JSON affine `[a,b,c,d,e,f]` maps pixel coordinates to instrument stage / mount
coordinates with `X = a*col + b*row + c`, `Y = d*col + e*row + f`.
Integer `(col,row)` denotes the pixel's upper-left corner. Pixel samples are
at `(col+0.5,row+0.5)`; pixel `(0,0)` has centre `(0.5,0.5)`.

The world file's six lines are `a`, `d`, `b`, `e`, `c+(a+b)/2`, `f+(d+e)/2`.
Its last two lines give the stage coordinates of the first sample centre.
Per-area image axes are right/down; stage-axis signs follow the instrument,
exactly as in grain export's `outlines.geojson`. The mapping uses the same
solved subpixel tile centres and stage positions as `AreaRecord::stage_of`.
All real tiles must agree within 0.1 pixel for a single affine to be reported.
Individual tile transforms can differ within that tolerance; the JSON includes
the source placements. HDR-grouped placements without one-to-one tile references
are not assigned an invented stage affine.

When stage metadata is absent or inconsistent, `pixel_to_stage_um` is null,
the reason is recorded, and the world file explicitly describes an **area-local**
right/down µm frame. Such a file is not registered to the mount or GeoJSON.
The dialog states this before export. Do not overlay those world files on stage
coordinates without a separate registration.

A whole-mount image requires a consistent stage map for every area and the same
pixel size (relative tolerance 1e-9). Otherwise the export offers per-area images
and reports the reason. The whole-mount grid increases along stage +x and +y;
areas with decreasing stage axes are flipped. The bounding box encloses every
area, including gaps. Sampling selects the containing source pixel by its
centre (nearest neighbour); the first selected area wins overlapping area
rectangles, gaps are zero, and mixed 8/16-bit inputs use 16-bit output. The JSON
records that policy. There is no second blend between independently stitched
areas. Neighbouring source pixels retain their original sampling interval.

## Saving, memory and cancellation

Desktop output goes into a new `mosaic-export-<timestamp>/` folder inside the
chosen native folder. Rust runs on a blocking worker and emits tile counts and
fractional progress. Cancellation uses the tab's shared atomic flag. Failed or
cancelled jobs remove files created by that job; existing files are never
replaced. Cleanup errors report the incomplete destination.

Chromium streams each worker chunk into a new directory where directory access
is available. If only `showSaveFilePicker` is available, it streams a ZIP64 STORE
archive containing images and sidecars. Closing/extracting the archive places
the sidecars beside each image. Hosted pages use the same browser capability checks. Browsers without a picker can download a ZIP only below 512 MiB; the estimate is
checked before starting and the byte limit is enforced during writing.
Only one worker chunk is requested at a time, after the previous disk write
finishes. Cancel aborts the open stream, removes the new export directory or
discards unpublished Blob chunks, and releases the job without destroying the
worker or its published source pixels. A save-picker abort discards pending
changes; browsers may leave an empty newly selected archive file.

Image buffers are bounded by one 512-row source strip, the overlapping source
fields and their multiband scratch. The strip keeps the global pyramid grid and
includes all levels' expansion neighbours. Source fields are decoded individually;
HDR holds only one exposure group. Photometric fitting runs over reloadable tiles,
retaining correction recipes and solve observations, rather than corrected image
planes. Original values use the same fit, background fallback guard, seams and
blend arithmetic as the canvas. Seam ownership is stored as row spans; its DP
scratch covers only the incoming coarse tile, including for very tall mosaics.
Global solve metadata scales with source count and overlap observations; it is
not counted as pixel-plane memory. Nothing calls `ensure_heavy` during export.
Displayed and published exports borrow existing 8-bit pixels. A source-free
project cannot recover raw pixels from a published bundle.

Output TIFF chunks contain one padded tile and a coverage bitmap (786,432 bytes
for 16-bit grayscale). PNG chunks consume 64 scanlines through `png`'s
`StreamWriter`, which retains its deflater and row filters. PNG 16-bit uses
original values; 8-bit supports either As displayed or original values clamped
to [0, 1], multiplied by 255 and rounded without a tone curve. JPG always uses
As displayed, quality 90 by default (1–100). The existing MIT/Apache `image`
encoder encodes one eight-row grayscale restart interval at a time, with
identical tables and a restart marker between intervals. It never receives a
whole image. No new encoder dependency was added; `png` is MIT/Apache, `image`
is MIT/Apache, and their existing Deflate support is used for PNG. TIFF remains
uncompressed to retain fixed, seek-free tiled offsets.

PNG and JPG have a practical export limit of 65,535 px on each side. The plan
reports this before writing. Choose a power-of-two downsample factor that fits,
or split into files named `<name>_r<row>_c<col>.<ext>` (one-based row/column).
Each split has its own JSON and world file with the translated origin.
Downsampling uses nearest-neighbour sampling at the output pixel centre,
multiplies pixel size and affine basis vectors by the factor, and preserves the
outer upper-left corner. Edge dimensions round upwards; partial per-area
edge samples clamp to the nearest source edge. All formats use the
same area and whole-mount coordinate rules.

Only one worker request/write is pending at a time. Browser cancellation is
cooperative between chunks; an in-flight source fitting/strip request finishes
before the worker receives abort. The page stays responsive because all core
work runs in the worker. Native cancellation also checks the atomic flag during
source loading, fitting passes, strip rendering and output writes.

## Verification

Synthetic tests independently decode classic TIFF and BigTIFF with the MIT
`tiff` crate at 8/16 bits, grayscale/RGB, multiple tiles and padded edges.
PNG round trips cover 8/16-bit samples; JPEG restart intervals are decoded
independently and checked within a grey-level tolerance. Limit tests cover
full, reduced and split planning, updated pixel sizes and translated affines.
Other tests compare an actual synthetic stitch pixel for pixel, verify the
GeoJSON stage frame at a grain centroid, check displayed LUTs, whole-mount axis
flips, fallback reasons, file cleanup, source depth and encoder allocations. Reloadable photometric output matches the
canvas for all gain models and both background methods. Strip blending matches
the full canvas bit for bit for feather and multiple multiband levels. A tall
100,000-row virtual mosaic retains the same pixel working set as 10,000 rows;
evicted and never-built source tests assert no full mosaic is restored.
The canvas regression also runs the complete open, stitch, correction, blend and
tone pipeline on procedural overlapping tiles. Seven parameter sets are pinned
to SHA-256 hashes from the separately built pre-export core and compared with a
test-only frozen copy of its seam and multiband algorithms, independently of
the streaming renderer.
Browser unit tests cover estimates, portable names, backpressure, cancellation,
write/close failures and ZIP64 CRC/offsets through Python's ZIP decoder.

```sh
cargo test -p grainatlas-core mosaic_export
cargo test -p grainatlas-core mosaic_export --lib -- --include-ignored --nocapture
cd ui && node --test src/lib/mosaic-export.test.mjs
cd ui && npx playwright test e2e/mosaic-export.spec.ts
```

Playwright uses the invented large bundle, checks the 1280 × 800 dialog, validates
a downloaded TIFF package and exercises folder streaming, cancellation, retry,
PNG multi-area automatic naming with one folder pick, and JPG size preflight.
The supervisor runs it with the usual preview server.

Canvas image recipes do not crop, mask, rotate or flip exported mosaics. Mosaic
exports retain original stage-aligned pixels, dimensions and all grain geometry,
including grains left off print sheets. “As displayed” applies the recipe's
black and white levels after the display LUT; it applies no spatial image edits.
Original-value modes ignore the image recipe entirely.
