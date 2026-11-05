# Attribution

## Reused code

- `photocraft-algo` and sibling crates from
  https://github.com/storytold/photocraft, copyright the PhotoCraft
  contributors, licensed MIT OR Apache-2.0. Used as a dependency pinned at
  commit `81bd97376f881ee3561371b7da52780844d3dfc7` (no functions are copied
  into this repository). grainatlas calls:
  - `panorama::seam_labels`, `panorama::multiband`, `panorama::blend_levels`
    and `panorama::RoiImage` for seams and multiband blending
    (`crates/grainatlas-core/src/blend/mod.rs`);
  - `panorama::photometric` for the gain-only photometric model
    (`crates/grainatlas-core/src/photometric/mod.rs`);
  - `hdr::mtb_offset`, `hdr::estimate_exposures`, `hdr::merge` and
    `hdr::tone_map` for HDR merges (`crates/grainatlas-core/src/photometric/hdr.rs`);
  - `segment::grabcut::grabcut` and `segment::RgbImage` for grain refinement
    (`crates/grainatlas-core/src/segment/shape.rs`);
  - `magnetic::Tracer` (the Magnetic Lasso's live wire) to snap hand-drawn
    grain outlines to edges, and `segment::quick::quick_select` (Quick
    Selection) for the grain brush (`crates/grainatlas-core/src/segment/edit.rs`),
    with `photocraft-geom`'s `Rect` at the same commit.
  Any function copied in the future keeps a comment naming its origin and
  the commit.
- `svg2pdf` and `pdf-writer` (typst project, MIT OR Apache-2.0) convert the
  SVG print pages to PDF (`crates/grainatlas-core/src/layout/pdf.rs`).

- `image-webp` 0.2.4 (https://github.com/image-rs/image-webp, the image-rs
  developers, MIT OR Apache-2.0): its lossless WebP (VP8L) encoder,
  `src/encoder.rs`, is copied into
  `crates/grainatlas-core/src/atlas/vp8l.rs` for the atlas bundle tiles, with
  two changes marked in the file: a plain error enum instead of `quick_error`,
  and a stable sort when code lengths are limited (the unstable sort ordered
  equal frequencies differently on wasm32, so native and browser tiles
  differed). The crate itself is a test dependency (decoding).

## Icon

- The application icon (`tauri/assets-icon.svg`, `ui/public/favicon.svg`) draws
  Pupin-type zircon crystals projected from the STL models of
  mineral-habit-matrix (https://github.com/duserzym/mineral-habit-matrix),
  MIT licence, by Yiming Zhang. `tauri/icon/zircons.py` regenerates it from
  those models.

## Fonts

- DejaVu Sans (`crates/grainatlas-core/assets/fonts/DejaVuSans.ttf`), embedded
  in the core so printed PDFs have the same text natively and in the browser.
  Bitstream Vera Fonts copyright Bitstream, Inc.; DejaVu changes are in the
  public domain; licence in `crates/grainatlas-core/assets/fonts/LICENSE_DEJAVU`.

## Algorithms implemented here from the literature

- Phase correlation: Kuglin and Hines (1975); sub-pixel peak estimator after
  Foroosh, Zerubia and Berthod, IEEE TIP 2002.
- Stage prior, pairwise registration and global least squares: after
  Preibisch, Saalfeld and Tomancak, Bioinformatics 2009.
- Translation refinement: Lucas and Kanade (1981), Gauss-Newton form.
- Euclidean distance transform: Felzenszwalb and Huttenlocher, Theory of
  Computing 2012.
- Grayscale reconstruction (h-maxima): Vincent, IEEE TIP 1993.
- CLAHE: Zuiderveld, Graphics Gems IV, 1994.
- Contour tracing: Moore neighbourhood with Jacob's stopping criterion;
  polygon simplification: Douglas and Peucker (1973).
- Exact date conversion in the CLI: Hinnant's civil-from-days algorithm.

## Data

- Example SEM data in `tests/fixtures/` were collected by Yiming Zhang at the
  Boise State Isotope Geology Laboratory on a Hitachi TM4000 (2026) and are
  provided under the repository license for testing.
