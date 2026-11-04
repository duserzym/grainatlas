# Stitch notes: Sweden_XS_150X (TM4000, 150x, SE)

Measured 2026-11-03 on an Apple M4 Pro (14 cores), release build,
`grainatlas stitch <package> --out <project>` (phase 1 pipeline: stage prior,
phase correlation, Lucas-Kanade refinement, global solve, seams, multiband,
pyramid). Photometric correction is not applied in these numbers (phase 2).

| Area | Tiles | Mosaic (px) | Pairs measured | RMS residual (px) | Max residual (px) | Total (ms) | stitch / blend / pyramid (ms) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 001_HJ-GC1 | 3 x 2 | 3327 x 1796 | 6 / 7 | 0.056 | 0.066 | 109 | 40 / 54 / 12 |
| 002_Skin_felsic | 3 x 5 | 3376 x 4066 | 20 / 22 | 0.168 | 0.320 | 330 | 163 / 137 / 28 |
| 003_AH2-2 | 3 x 5 | 3374 x 4059 | 21 / 22 | 0.266 | 0.563 | 285 | 136 / 120 / 27 |
| 004_Skin_mafic | 3 x 5 | 3383 x 4059 | 21 / 22 | 0.157 | 0.279 | 308 | 142 / 138 / 27 |
| 005_AH1-GC1 | 4 x 8 | 4442 x 6341 | 49 / 52 | 0.377 | 1.205 | 583-691 | 230 / 291 / 57 |

The 4 x 8 area stitches in 0.6 s natively (requirement: under 10 s); peak
resident memory of the CLI for it is 2.5 GB with all cores busy (rayon builds
several multiband bands at once; the wasm build builds one at a time).

## What the data showed

- **Stage axes are not the scan axes.** Every left-right neighbour is about
  24-28 px lower than the stage says (dy ≈ +25 px over a 1022 px step, a
  1.4° skew between the stage x axis and the raster), and about 6 px short in
  x. The tiles themselves are not rotated relative to each other, so a
  translation-only model still fits to well under a pixel.
- **Row changes have backlash.** Up-down neighbours are 13-19 px left of the
  prior and alternate between about +10 to +16 px and -21 to -28 px in y from
  row to row, consistent with the stage approaching each row from alternating
  sides. Zigzag's own `*_stitch.jpg` ignores this, which is where its visible
  seams come from.
- **Empty pairs fall back.** 2-3 pairs per area (only epoxy in the overlap)
  fail the peak-to-sidelobe gate (PSR < 7) and are tied to the stage prior
  with a weight of 1e-3, so they follow their measured neighbours.

## Choices made while tuning (within plan section 6.2)

- Coarse phase correlation at half resolution, then full resolution around
  the coarse estimate, then a Lucas-Kanade (Gauss-Newton) translation
  refinement on the overlap with both sides normalised to zero mean and unit
  variance. Without the LK step the loop closures left 0.4-0.7 px RMS; with it
  0.06-0.38 px.
- Measured pairs are weighted by `clamp(PSR / 40, 0.1, 1)`.
- Outlier rejection drops the single worst pair above 3 px and re-solves,
  repeating while any pair is above 3 px. Dropping every pair above 3 px at
  once (one literal reading of the plan) discarded all pairs of the fixture,
  because one bad pair (4-5, PSR 13) spread its error over its loop.
- The worst remaining residual (005, pair 2-3 / 3-7 around the top-right
  corner tile, PSR 11-22, mostly epoxy) is 1.2 px; all other pairs are under
  1 px.
- Seams are found on a grid 8 times coarser than the mosaic
  (`seam_labels` needs one full-canvas plane per tile) and upsampled to
  binary weights for `multiband`.

Synthetic test (`crates/grainatlas-core/tests/stitch.rs`): a 3 x 3 grid with
random sub-pixel drifts up to ±8 px is recovered within 0.25 px.

## Phase 2: photometric correction

Same machine, full pipeline including photometric correction and tone
(read, stitch, photometric, blend, tone, pyramid), ms:

| Area | Total | photometric | blend |
| --- | --- | --- | --- |
| 001_HJ-GC1 | 180 | 39 | 54 |
| 002_Skin_felsic | 482 | 92 | 166 |
| 003_AH2-2 | 430 | 95 | 139 |
| 004_Skin_mafic | 441 | 89 | 135 |
| 005_AH1-GC1 | 1040 | 219 | 434 |

What the overlaps showed and what is now done about it:

- **Beam-exposure steps.** In every tile the band that the beam had already
  scanned while a neighbour was captured is 2-3 grey levels (of 255) darker,
  with a sharp edge 150-320 px from the tile border (wider than the 192 px
  geometric overlap). A smooth background surface cannot fit a step, and this
  was the main source of the visible tile rectangles in Zigzag's stitch and in
  our first runs. `background::detect_edge_steps` fits
  `a + b·i + h·[i < s]` to the background row and column profiles near each
  edge (strongest end first, then the other end on the corrected profile),
  keeps steps with t > 12 and |h| > 1/255, and adds them back with a 4 px ramp.
- **Gain and offset.** Our affine least squares over the overlap pixels
  (three rounds of residual trimming, gentle regularisation relative to the
  data weight). Gains on this package are within 0.94-1.04 and offsets within
  ±0.025, so the SE detector was stable; the background leveling removes most
  of the offset differences anyway.
- **Background.** 2nd-order polynomial per tile fitted to 64 px cell medians
  of the background (Otsu grain mask dilated by 6 px), subtracted, mosaic
  median added back.
- Remaining: in a few places a 1 grey-level contour follows a tile border in
  the near-black epoxy (8-bit JPEG quantisation of the source at about 10/255;
  invisible at normal contrast, visible only under extreme stretch).

## Loop-free pairs (2026-11-03)

A single row of loose images closes no loop, so the solve's residual check
cannot see a false peak. In the fixture's bottom row as a loose 3 × 1 grid,
the 4-5 overlap (mostly empty resin) correlated at PSR 13.6 to an offset
about 41 px from where the stage and the package's loops put it, and the row
came out that much short. `mosaic::consensus` now takes, per direction, the
PSR-weighted median deviation of the measured pairs from the prior; a pair
that closes no loop, lies more than `consensus_px` (20 px) from it and has a
PSR below 3 × `min_psr` is dropped, and every pair's prior in that direction
is corrected by the consensus. The dropped step then lands 7 px from the
package's placement, within the spread of the stage's own left-right steps
(up to ~10 px here). Pairs inside loops are untouched (the fixture mosaic
hash did not change).
