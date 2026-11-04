# Print sheets, grain ownership and CL-dark grains

First used on the Boise State SEM run of 2026-11-03 (Hitachi TM4000, zircon
CL, three zigzag packages, 22 tabloid sheets) for spotting by hand next to the
digital spots in grainatlas.

## Cross-area grain ownership (`crate::ownership`)

In a zigzag package neighbouring areas overlap (100 to 850 µm on that run), so
one grain is detected in several areas. Each detection is mapped mosaic → stage
through its nearest tile (as `AreaRecord::stage_of`) and from the stage into
every other area. A detection in another area within half the grain's size plus
`match_um` (20 µm) is the same grain; linked detections form one grain. The
owner is the area where the grain's box lies farthest inside the covered mosaic
(a signed distance field of the tile rectangles); a detection cut by an area's
edge always loses to a whole one. The others are duplicates of the owner's
detection. A grain that maps into another area where nothing was detected stays
with its own area, so a missed detection never loses a grain.

Areas whose names differ only by a numeric `_N` (`KH2-30.7`, `KH2-30.7_2`) are
one sample, when the base name is another area's name.

In the app the Grains tab shows duplicates dashed, with "→ HH4 #12" in the
grain table; print sheets and their counts leave them out.

## Segmentation (`segment::filter`)

The primary segmenter is unchanged. On top of it:

- `polarity: "both"` adds grains brighter or darker than the local epoxy level
  (a running median of 330 µm on the mosaic normalised to its p1..p99.5): a
  pixel is dark when it is below that level by the noise margin (5.5 σ, at
  least 18/255) or by `dark_rel` (0.45) of the level, whichever is smaller. A
  supplement grain is added when it overlaps no primary grain, and replaces the
  primary grains it covers when it is at least 1.5 times their area (the
  primary pass found only the bright part of a mostly CL-dark grain).
- `reject_bubbles`: round dark blobs of 12 to 75 µm (circularity > 0.72,
  solidity > 0.88, aspect < 1.35) are bubbles, and bright grains centred on one
  are its halo; bright arcs and rings on a circle of 12 to 120 µm whose middle
  is at the epoxy level are bubble rims. Off for `polarity: "dark"`.
- `reject_slivers`: minor axis < 15 µm, or aspect > 5, minor axis < 30 µm and
  straight along a tile axis (the outline hugs its box).
- `reject_blocks`: exposure-step tile blocks: regions where fewer than 15 % of
  the pixels stand out from the local epoxy level, or under 50 % and larger
  than 260 µm or box-shaped; and anything larger than `max_size_um` when set.

`GRAINATLAS_DEBUG_SEG=1` prints every primary grain's measures and verdict.

Known limits on the Boise run: bright rims of large bubbles (80 to 100 µm,
grey middles) next to CL-dark grains are still found as small grains in HH11,
two step regions in HH11 survive, and mid-grey grains close to the epoxy level
need click-grow.

## Export enhancement (`photometric::enhance`)

Non-local means (5 x 5 patches, 13 x 13 search, h = 0.7 σ with σ from the
Immerkær estimate) and an unsharp mask (radius 1.6 px, amount 0.55), on the
printed bands only. "Strip background" makes everything outside the dilated
(6 µm) convex hulls of every grain in the band one grey, with a soft edge. The
hull covers a grain whose outline caught only its bright rim. A band keeps its
background when a grain reaching into it has no closed outline, or when a blob
of 30 µm or more outside every hull departs from the background by 6 robust σ
(an undetected grain would vanish).

## Sheets (`layout::sheets`)

Tabloid landscape at a fixed scale (1 mm = 8 µm by default). Samples are
ordered top of the mount first. The owned grains of each area are chained into
rows along x; rows flow across the lines of a sheet and break only between
grains. A row whose grains step up or down is cut into bands of their own
height; an empty stretch over 420 µm becomes a short break marked "//" with its
length. Rows of the same area on one line share one label (sample and area,
no grain numbers). Every band carries a 100 µm (or 50 µm) scale bar and corner
ticks. The header has the instrument metadata from the sidecars (instrument
and serial, detector, kV, emission, WD, scan, magnification, tiles and pixel
size, capture times), the sample legend, laser spot circles at print scale and
the mount map: area boxes from stage coordinates in the SEM view, this sheet's
samples dark with their grains as white dots and its bands outlined.
Everything is black, white and grey. Corner marks sit in the sheet margins.

The registration sidecar (`<package>_print_registration.json`) is the
`print_sheet` frame, separate from the laser stage registration:

```json
{ "frame": "print_sheet", "um_per_mm": 8.0, "paper_mm": [431.8, 279.4],
  "sheet_origin": "sheet top-left corner, mm, y down",
  "mapping": "mosaic_um = mosaic_rect_um[0:2] + (sheet_mm - sheet_rect_mm[0:2]) * um_per_mm",
  "bands": [{ "sheet": 1, "sample": "HH3", "area": "007_HH3", "row": 1,
              "grains": [{ "id": "…", "number": 4 }],
              "sheet_rect_mm": [8.0, 74.6, 120.4, 101.9],
              "mosaic_rect_um": [432.6, 366.4, 1331.6, 584.8],
              "background": "kept" }] }
```

Command line:

```bash
grainatlas stitch <package> --out <package>.grainatlas --auto --polarity both \
  --print tabloid [--scale 8] [--strip-bg 22] [--no-denoise] [--no-sharpen] \
  [--title "Grenville · mount S · zircon CL"] [--lab "…"]
```

writes `exports/<package>_grain_sheets_tabloid.pdf`,
`exports/<package>_print_registration.json` and `exports/ownership.json`.
In the app: Layout → Print sheets.
