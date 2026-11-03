# Stitch notes: Sweden_XS_150X (TM4000, 150x, SE)

Measured 2026-11-03 on an Apple M4 Pro (14 cores), release build,
`grainmap stitch <package> --out <project>` (phase 1 pipeline: stage prior,
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

Synthetic test (`crates/grainmap-core/tests/stitch.rs`): a 3 x 3 grid with
random sub-pixel drifts up to ±8 px is recovered within 0.25 px.
