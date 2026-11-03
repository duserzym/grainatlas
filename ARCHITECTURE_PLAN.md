# grainmap: architecture plan and roadmap

**Status:** plan, 2026-11-03. Companion to `laicpmsdb` (the LA-ICP-MS session
database) and UPbRPy (the reduction engine). This document is the
specification for the first implementation; decisions recorded here are not
to be re-opened in code without an ADR in `docs/decisions/`.

## 1. Purpose

`grainmap` turns a folder of SEM tiles (CL, BSE or SE) of a grain mount into
one corrected, scaled, segmented, annotated image, and from that image produces
the two things a laser session needs: a spot list with names and coordinates,
and printable grain maps. It is the imaging module of a three-part workflow:

| Module | Owns | Repository |
| --- | --- | --- |
| grainmap | images, grains, spots, print layouts | this repository |
| laicpmsdb | samples, mounts, sessions, raw sweeps, reductions | `laicpmsdb` |
| UPbRPy Studio | data reduction | `UPbRPy` |

The end state is one unified application with one tab per module, where a
user can click a spot on a grain image and open the raw sweep and the reduced
result for that spot in the database tab. grainmap is built so that link is a
matter of identifiers, not of re-architecture (section 7).

## 2. Decisions

1. **Name:** `grainmap`. Free on GitHub under both `duserzym` and
   `Swanson-Hysell-Group`; the user picks the owner at publish time.
2. **One Rust core, two front ends.** Every pixel operation (reading, stitching,
   photometric correction, blending, segmentation, scale bars, print layout)
   lives in a Rust library crate that compiles natively (desktop, CLI) and to
   `wasm32` (browser). The web version is a static page: no server is needed
   to use it. The desktop version is the same UI inside a Tauri 2 shell with
   the core as native commands (multithreaded via rayon).
3. **Reuse photocraft-algo.** `storytold/photocraft` (MIT OR Apache-2.0, pure
   Rust, `unsafe` forbidden, builds for wasm) ships, in `crates/algo`, exactly
   the classical pipelines we need: `panorama` (features, RANSAC, bundle
   adjustment, gain and vignetting compensation, seams, multiband blending),
   `pyramid` (Burt-Adelson blending), `poisson` (membrane fill and seamless
   clone), `hdr` (Ward MTB alignment, Debevec response, radiance merge, tone
   mapping), `tone::hdr_toning`, `segment` (GrabCut, SLIC, max-flow, Select
   Subject, Otsu) and `features` (Harris + BRIEF registration). We depend on it
   as a pinned git dependency and write only what is specific to SEM mounts:
   stage-coordinate priors, translation-only drift solving, dark-background
   grain segmentation, scale bars, spots and print layout. Attribution goes in
   `ATTRIBUTION.md`. photocraft requires Rust 1.95; the laptop has 1.94 and
   must run `rustup update`.
4. **One UI build, same design language as laicpmsdb.** Svelte 5 + TypeScript
   + Vite with the same glass tokens (`:root` light and dark, `backdrop-filter`
   panels, opaque image surface). The infinite canvas is a custom 2D-canvas
   renderer over an image pyramid with an SVG overlay for spots and labels. No
   OpenSeadragon, no Pixi: fewer than a thousand lines, no dependency.
5. **Projects are folders, not databases.** A `.grainmap/` project folder holds
   a versioned `project.json`, content-addressed references to the source
   tiles, the stitched pyramid, masks, spots and layouts (section 5.4). Code
   lives on GitHub; images live on the NAS or the laptop, as in
   `OPERATIONS_PROTOCOL.md` of laicpmsdb. The repository carries only one small
   fixture area (6 tiles, 160 KB) for tests.
6. **Deterministic and reproducible.** Every automatic step records its
   parameters in `project.json`; re-running a project from the same tiles gives
   byte-identical outputs. Manual edits are stored as parameters (per-tile
   gain and offset, moved spots), never as baked pixels, so they survive a
   re-stitch.
7. **Identifiers:** UUIDv7 for projects, areas, grains and spots, the same
   scheme as laicpmsdb, so a spot id minted here can become a
   `physical_spot.id` there without translation.
8. **Platforms:** macOS arm64 and Windows x86-64 desktop builds are
   first-class, Linux best-effort; any evergreen browser for the web build.
9. **License:** MIT, matching laicpmsdb. photocraft's Apache-2.0 OR MIT dual
   license is compatible under MIT.

## 3. Evidence from the example data

Example package: `2026_Boise/SEM/Sweden_XS_150X` from a Hitachi TM4000 table
top SEM with Hitachi's Zigzag multi-capture (Tsunagi) software. One mount, five
areas, 150x, SE signal, 88 tiles, 20 MB.

| File | Encoding | Content |
| --- | --- | --- |
| `<mount>.zag` | UTF-8 XML, BOM, CRLF | all areas: name, folder, grid (`CaptureNumX/Y`), pitch, start and end stage points, brightness and contrast history |
| `<mount>.bmp` | 1280x960 BGRA | optical navigation image of the mount with stage calibration (top-left and bottom-right stage positions in nm, pixel size) |
| `<area>/<area>_.ini` and `.xml` | UTF-16LE text / UTF-8 XML | grid `wXCount x wYCount`, field size `dXSize x dYSize` um, pitch `dXPitch x dYPitch` um, per-view positions, stage corners |
| `<area>/_<area>_.xml` | UTF-8 XML | capture management: margin index, brightness, contrast, focus state |
| `<area>/<area>_NNNN.jpg` | 1280x960 8-bit grey baseline JPEG, no burned-in data bar | one tile, row-major order (x fastest) |
| `<area>/<area>_NNNN.txt` | UTF-16LE `[SemImageFile]` ini | `PixelSize` (nm per pixel: 661.4583 at 150x), `StagePositionX/Y` (nm), magnification, voltage, working distance, signal, scan speed, date and time |
| `<area>/<name>_stitch.jpg` | RGB JPEG | Zigzag's stitch (4443x6346 for the 4x8 area): visible seams and per-tile brightness steps |

Numbers that drive the design:

| Quantity | Value |
| --- | --- |
| Tile field | 846.666 x 635.000 um |
| Pitch | 676 x 508 um, so 170 x 127 um overlap (20 percent) |
| Stage step between neighbours | 676000 nm in x (stage x decreases to the right), 508000 nm in y |
| Largest area in the example | 4 x 8 tiles, 32 tiles, 5.6 x 5.2 mm, about 40 megapixels at full resolution |
| Grains per area | 100 to 200 zircons of 50 to 200 um on a black epoxy background |

The hand-drawn `grain_maps.pdf` in the same folder (mount outlines with sample
rows named by hand) is what the print layout replaces.

Consequences. The pixel scale and tile positions come from metadata, so scale
bars never need OCR for this instrument and stitching starts from a nearly
correct placement and only has to solve small drifts. The background is dark
and uniform, so grains segment with a threshold plus morphology before any
graph cut. Other labs' packages (Zeiss, JEOL, FEI TIFF with burned-in data bars
and XML/TIFF-tag metadata) must plug in as readers behind the same `Package`
trait; the burned-in-bar case is handled by bar detection plus a typed length.

## 4. Requirements

Functional:

1. Open a package folder (TM4000 first) or a plain folder of images plus a
   positions CSV; show areas, tiles and metadata without stitching.
2. Auto-stitch an area: stage prior, drift refinement, global solve, seams,
   blending; under 10 s native and under 60 s in the browser for 32 tiles.
3. Auto brightness and contrast: per-tile gain and offset compensation across
   overlaps, global percentile stretch, optional local contrast (CLAHE-like),
   16-bit input support, HDR merge when the same field exists at several
   brightness or contrast settings; all with manual per-tile and global
   overrides and a histogram.
4. Background leveling: estimate and remove the slow background trend of each
   tile (charging, detector drift) so the epoxy is uniform across the mosaic.
5. Segmentation: find grains on the background, split touching grains, give
   each a polygon, bounding box, centroid, area, axes and orientation, and a
   number in reading order; GrabCut refinement and manual add, split, merge,
   delete.
6. Scale registration: a true-scale coordinate system in micrometres for the
   mosaic and for each grain, with scale bars drawn per grain and per page.
7. Infinite canvas: pan, zoom, rotate; pixel-accurate at any zoom; grain
   outlines, numbers and spots as overlays that can be toggled.
8. Spot placement: toolbar with select, spot, line, raster, measure, label and
   delete; spot diameter in micrometres from presets or typed; spots render at
   true scale so they resize with zoom; numbering propagates from a pattern
   such as `{sample}-{grain}-{n}` set by the user; snapping to grain centroid;
   floating, movable metadata panel for the hovered or selected object.
9. Print layout: choose paper (Letter, Tabloid 11x17, A4, A3, custom), scale
   (um per mm or fit), and get pages that pack grains or areas automatically
   with labels, scale bars, cut marks and page numbers; export PDF and SVG.
10. Exports: spot list CSV (name, grain, x and y in um and in pixels, diameter,
    comment), grain table CSV, GeoJSON of outlines and spots, the mosaic as
    PNG, TIFF (8 or 16-bit) and a Deep Zoom pyramid.
11. Same interface two ways: a static web page and a desktop app from one UI
    build, with identical results from the same core.
12. Link-ready: every spot and grain has a UUIDv7, and the project records the
    laicpmsdb `mount` id when known; spot exports carry those ids.

Non-functional: open source, MIT; offline; deterministic; no telemetry; a
40-megapixel area must stay under 1.5 GB of memory in the browser; keyboard
shortcuts for every tool; undo and redo for all edits.

## 5. Architecture

### 5.1 Components

| Component | Technology | Why |
| --- | --- | --- |
| `grainmap-core` (Rust crate) | Rust 2024, `image`, `rustfft`, `photocraft-algo`, `serde`, `uuid` v7 | all algorithms; native and wasm; no I/O except through a `Package` trait and byte buffers |
| `grainmap-wasm` | `wasm-bindgen`, `wasm-pack` | the core for the browser: typed JS API, transferable buffers, progress callbacks, cancellation |
| `grainmap-cli` | `clap` | `grainmap stitch`, `segment`, `export`, `print` for batch work and tests |
| `ui` | Svelte 5, TypeScript, Vite | the one interface; talks to the core through a `Core` interface with two implementations (wasm worker, Tauri invoke) |
| `tauri` | Tauri 2 | desktop shell; file dialogs, native file system, multithreaded core |
| `fixtures` | one TM4000 area, 6 tiles | tests and the demo |

### 5.2 Repository layout

```text
grainmap/
  crates/
    grainmap-core/      readers/, mosaic/, photometric/, blend/, segment/, scale/, spots/, layout/, project/
    grainmap-wasm/
    grainmap-cli/
  ui/                   Svelte app; src/lib/core/{wasm,tauri}.ts implement the Core interface
  tauri/                Tauri 2 shell (src-tauri)
  tests/fixtures/       tm4000/001_HJ-GC1 (6 tiles + sidecars + .zag excerpt)
  docs/decisions/       ADRs
  ARCHITECTURE_PLAN.md  this file
  ATTRIBUTION.md        photocraft and other reused code
```

### 5.3 Data flow

```text
package folder --read--> Area{tiles, positions_um, pixel_size_nm, meta}
  --mosaic--> Placement[] (translation per tile, drift-corrected)
  --photometric--> Gain/Offset[] + background models
  --blend--> Mosaic (f32 grey or RGB, w x h) + Pyramid (levels 0..n, WebP or PNG tiles 512 px)
  --segment--> Grain[] (polygon, bbox, centroid, stats, number)
  --spots (user)--> Spot[] (uuid, name, grain, x_um, y_um, diameter_um)
  --layout--> Page[] (SVG) --> PDF
```

Everything after `read` is a pure function of its inputs and the parameters
stored in `project.json`.

### 5.4 Project folder

```text
<name>.grainmap/
  project.json          schema_version, uuid, created, package path and reader, laicpmsdb mount id,
                        areas[] with tile list (filename, sha256, position, pixel size),
                        parameters for every step, per-tile manual overrides
  mosaic/<area>/        level-0 tiles 512x512 (lossless WebP or PNG), levels 1..n, mosaic.json (w, h, um per px, origin)
  masks/<area>/         grains.png (label image, u16) and grains.json (polygons and stats)
  spots.json            all spots and labels, with history for undo across sessions
  layouts/<name>.json   paper, scale, page arrangement
  exports/              last PDF, SVG, CSV written
```

Source tiles are never copied into the project; they are referenced by path
and SHA-256 so a moved package can be re-linked by hash.

### 5.5 Coordinate systems

| Frame | Unit | Origin | Used for |
| --- | --- | --- | --- |
| tile pixel | px | tile top-left | reading, features |
| mosaic pixel | px | mosaic top-left | pyramid, masks, canvas |
| mosaic micrometre | um | mosaic top-left, y down | spots, grains, scale bars, exports |
| stage | nm | instrument stage | re-finding a grain on the SEM; later, laser stage after a 3-point affine registration |
| page | mm | page top-left | print layout |

Mosaic micrometre = mosaic pixel x (pixel size nm / 1000). Stage from mosaic
through the per-tile stage positions (sign flip in x for the TM4000).

### 5.6 UI

Screens: Open (package picker, recent projects), Area (tile grid preview,
metadata, stitch and correction controls with live histogram), Canvas (the
infinite canvas with toolbar, layers panel, floating metadata panel, grain
list), Layout (paper, scale, page preview, export) and Settings (naming
pattern, spot presets, theme).

Design tokens are copied from `laicpmsdb/ui/src/app.css` so the two apps look
like one. The image surface is opaque; glass is for panels and the toolbar
only. Dark theme is the default for CL work.

Rendering: the pyramid levels are `ImageBitmap`s; the canvas draws the level
closest to the current zoom with `drawImage` at device pixel ratio; the SVG
overlay holds grain outlines (simplified at low zoom), numbers and spots, with
`vector-effect: non-scaling-stroke`. Wasm runs in a Web Worker; results move
with transferable `ArrayBuffer`s.

## 6. Pipeline design

### 6.1 Readers

`trait Package { fn areas(&self) -> Vec<AreaMeta>; fn tile(&self, id) -> Tile }`
with `TM4000Package` first (`.zag`, `.ini`, `.txt` UTF-16LE, `.bmp`
navigation) and `GenericPackage` (images plus `positions.csv` with
`file,x_um,y_um,pixel_size_nm`). A reader must never fail on an unknown key;
unknown metadata is kept as a string map and shown in the panel.

### 6.2 Stitching (drift, not perspective)

Tiles from a stage are related by translation only. The pipeline is:

1. Prior placement from stage positions and pixel size.
2. For each neighbouring pair (left-right, up-down), phase correlation
   (`rustfft`) on the overlap band at half resolution, then a 3x3 sub-pixel
   peak fit; keep the pair if the peak-to-sidelobe ratio passes a threshold,
   otherwise fall back to the prior. Pairs with no texture (empty epoxy) fall
   back automatically.
3. Global least-squares over all pairwise offsets (sparse normal equations,
   conjugate gradient) with the first tile fixed; residuals above 3 px are
   dropped and the solve repeated once.
4. Optional feature-based verification with `photocraft_algo::features`
   (Harris + BRIEF, RANSAC translation) for packages without stage metadata.

Output: integer-aligned placements plus the sub-pixel residual, the residual
map in the UI, and the solved drift per tile stored in `project.json`.

### 6.3 Photometric correction, HDR, background

1. Gain and offset per tile from overlap statistics (least squares in log
   space, `photocraft_algo::panorama::photometric` with vignetting off, or our
   own affine fit when offsets matter, which they do for SE and CL).
2. Background model per tile: mask grains (coarse threshold), fit a low-order
   polynomial or a heavily smoothed surface to the background pixels, subtract
   it, add the mosaic-wide median background back. This is the "background
   blending smoothing" for drift and charging.
3. Global tone: percentile stretch (0.1 and 99.9 by default) with a histogram
   and black, white and gamma handles; optional local contrast (CLAHE on
   64 px tiles, implemented in the core; `tone::hdr_toning` from photocraft as
   the alternative).
4. HDR: when a field was captured more than once at different brightness or
   contrast, or in 16-bit, align with `hdr::mtb_offset`, estimate exposures
   with `hdr::estimate_exposures`, merge with `hdr::merge` and tone-map with
   `hdr::tone_map`. For CL this recovers both dim cores and bright rims.
5. Manual: per-tile gain and offset sliders and a "match neighbours" button;
   everything stored as parameters.

### 6.4 Seams and blending

Seam labels through the overlap on the colour difference
(`panorama::seam_labels`), then multiband blending
(`panorama::multiband`, levels from `blend_levels`). A "fast" mode uses
linear feathering for previews. Holes (missing tiles) are filled with the
median background, not with inpainting, so nothing is invented.

### 6.5 Segmentation

1. Otsu threshold on the smoothed mosaic (grains bright on dark epoxy for CL
   and BSE; inverted for transmitted light), morphological open and close,
   fill holes.
2. Distance transform and marker-based watershed to split touching grains;
   drop components under a minimum area (default 20 um across).
3. Optional refinement per grain with `segment::grabcut` on the bounding box
   with a dilated trimap, and `segment::slic` plus saliency for difficult
   cases.
4. Per grain: polygon (Douglas-Peucker simplified), bbox, centroid, area,
   equivalent diameter, major and minor axes, orientation, mean intensity.
5. Numbering in reading order (rows by centroid y with a tolerance, then x),
   stable across re-runs by matching centroids to the previous labels.
6. Manual tools: add (click-grow), split (draw a line), merge, delete,
   renumber from here.

### 6.6 Scale registration

Pixel size comes from metadata. When absent, the user clicks two points and
types a length, or the bar detector finds the longest horizontal white run in
the bottom band of the image and asks for its length once per package. Each
grain gets a scale bar whose length is the largest of 10, 20, 50, 100, 200 or
500 um that fits a third of its bbox width.

### 6.7 Spots

A spot is `{id, name, grain_id, x_um, y_um, diameter_um, kind (spot, line,
raster), comment, created, created_by}`. The naming pattern is a template with
`{sample}`, `{grain}`, `{n}` and `{n:03}`; `n` continues from the highest
existing number in the project. Snap targets: grain centroid, grain long axis
ends, previous spot. Spots and lines are drawn at true scale. Export columns
match what the lab's laser software imports; an adapter per laser system
converts micrometres to stage coordinates once a 3-point registration is
entered.

### 6.8 Print layout

Items are grains (cropped from the mosaic with a margin), or whole areas.
Given paper size, margins and scale, a shelf packing (next-fit decreasing
height, rows of similar height) places items with their labels, scale bars
and spots; items that do not fit on the current page start a new one; a grain
larger than a page is tiled with overlap and registration marks. Output is
one SVG per page rendered in the core and converted to PDF with `svg2pdf`
(wasm-capable); the browser print dialog is the fallback. Page templates:
Letter, Tabloid 11x17 (the lab's default), A4, A3, custom.

## 7. Integration with laicpmsdb and the unified application

- A project may carry `laicpmsdb.mount_id`; grains map to `grain` rows and
  spots to `physical_spot` rows there. The link is one `POST
  /v1/mounts/{id}/spots` of the spot export when the user chooses to sync.
- A spot carries its UUIDv7 from birth; the laser session labels are
  generated from spot names, so `acquisition.label` in laicpmsdb equals the
  spot name and the join is exact.
- Deep links: `grainmap://project/<uuid>/spot/<uuid>` and
  `laicpmsdb://acquisition/<uuid>`; each app registers its scheme in Tauri and
  the web builds accept the same path as a route.
- The unified shell is a later Tauri app whose tabs load the laicpmsdb web
  build, the grainmap web build and UPbRPy Studio; nothing in grainmap may
  assume it is the only app in the window (no global CSS, no `window.name`).

## 8. Roadmap

| Phase | Scope | Done when |
| --- | --- | --- |
| 0 | repository, plan, ADRs, fixture, CI skeleton | this document committed; fixture area under `tests/fixtures/` |
| 1 | core readers and stitching | `grainmap stitch <area>` writes a mosaic and pyramid for the 4x8 example with no visible seams; residuals under 1 px; tests on the fixture |
| 2 | photometric, background, HDR | per-tile gain and offset, background leveling, percentile stretch, CLAHE; HDR merge tested on synthetic exposures |
| 3 | web UI | Svelte app with wasm worker: open package, stitch, histogram and correction sliders, infinite canvas with pyramid; `vite build` static page works from `file://` and from any static host |
| 4 | segmentation and scale | grain outlines and numbers on the example areas, scale bars, grain table export |
| 5 | spots and metadata panel | toolbar, spot presets, naming pattern, snapping, floating panel, CSV and GeoJSON export, undo and redo |
| 6 | print layout | Tabloid and Letter pages with auto packing, PDF export, cut marks |
| 7 | desktop | Tauri 2 build on macOS and Windows using the native core; file dialogs; drag and drop of a package folder |
| 8 | link | mount id, spot sync to laicpmsdb, deep links; readers for a second instrument when example files arrive |

## 9. Verification

- Unit tests per core module on the fixture area and on synthetic tiles with
  known offsets and gains (recovered within 0.25 px and 1 percent).
- Golden images: the fixture mosaic is committed as a hash, not as pixels; a
  change in output must be deliberate and update the hash.
- Browser and native parity: the wasm and native builds produce identical
  hashes for the fixture.
- UI tests with Playwright on the static build: open fixture, stitch, place
  three spots, export CSV, print one page.

## 10. Risks

| Risk | Mitigation |
| --- | --- |
| photocraft-algo API changes | pin the git rev; copy a function with attribution if it moves |
| Browser memory for large mosaics | process at half resolution for preview, full resolution per 2048 px block; pyramid tiles stream from OPFS |
| Zigzag order or sign conventions differ between instruments | readers compute placement from per-tile stage positions, never from index order |
| Grains touching or on a bright background | watershed plus GrabCut refinement, manual split and merge |
| Print scale errors | every page carries a 10 mm scale bar at paper scale and a um scale bar at image scale; a test prints the fixture and checks the bar in the PDF |

## 11. References

- Brown and Lowe, Automatic Panoramic Image Stitching using Invariant
  Features, IJCV 2007. Burt and Adelson, A Multiresolution Spline, ACM TOG
  1983. Perez, Gangnet and Blake, Poisson Image Editing, SIGGRAPH 2003.
  Debevec and Malik, SIGGRAPH 1997. Rother, Kolmogorov and Blake, GrabCut,
  SIGGRAPH 2004. Achanta et al., SLIC, PAMI 2012.
- Kuglin and Hines, The Phase Correlation Image Alignment Method, 1975.
- Preibisch, Saalfeld and Tomancak, Globally optimal stitching of tiled 3D
  microscopic image acquisitions, Bioinformatics 2009 (the stage-prior plus
  phase-correlation plus global least-squares design used here).
- storytold/photocraft, `crates/algo` (MIT OR Apache-2.0).
- laicpmsdb `DATABASE_ARCHITECTURE_PLAN.md` sections 5.3 (mounts, grains,
  physical spots) and 6.3 (design language).
