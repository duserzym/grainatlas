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

## Segmentation and the artifact classifier (`segment::filter`)

The primary threshold segmenter is unchanged. On top of it, artifacts are
**flagged, never deleted**: a detection gets `artifact` = `bubble`, `halo`,
`sliver`, `block`, `speck` or `manual`. Artifacts stay in the area's grain list,
unnumbered, drawn dimmed and dotted with their reason, and are left off print
sheets, spot linking, ownership, per-grain tone and the atlas bundle. In the
Grains tab, "Not grains" lists them, and **Keep as grain** restores one with one
click (`kept: true`); **Not a grain** marks selected grains (`manual`). Re-running
Find grains keeps both decisions (matched by position, ids kept).

Sizes scale with the area's **reference grain size** `r` (module `rel`), tuned
on the Boise mounts at r ≈ 110 µm (S) and 175 µm (L):

| rule | S (r = 110) | L (r = 175) | relative |
|---|---|---|---|
| largest bubble | 75 µm | 115 µm | 0.68 r |
| seam opening radius | 4 µm | 6.6 µm | 0.037 r |
| oversized blob, split | 330 µm | 500 µm | 3 r |
| speck | < 22 µm | < 35 µm | 0.21 r |
| speck off the row | > 200 µm | > 315 µm | 1.8 r |
| halo crescent | < 50 µm | < 79 µm | 0.45 r |
| fragment of one grain | < 35 µm | < 56 µm | 0.32 r |

`r` is `reference_size_um` when set, else the median eq. diameter of the area's
grains over 35 µm, weighted by eq. diameter (fragments count less), leaving out
blobs over three times the upper quartile (tile blocks). The first estimate comes
from raw detections, which split zoned grains; when the whole grains found with
it differ by more than 25 %, the classifier runs again with theirs.

The steps (`segment()`, then `second_pass`):

1. **Bubbles** (`flag_bubbles`; not in transmitted light): round dark blobs
   (circularity > 0.72, solidity > 0.88, aspect < 1.35) from 12 µm to 0.68 r,
   or up to 1.2 r when a bright halo rings them. The cores, grown by 9 µm (the halo
   zone), are taken out of the bright mask before labelling, so a halo can neither
   become a crescent "grain" nor fuse with a zircon next to it. Bubbles are listed
   as artifacts, except dark blobs inside a grain (inclusions).
2. **Seam lines** (`flag_slivers`): an opening of 0.037 r cuts seam lines that
   bridge grains.
3. **Oversized blobs** (`flag_blocks`): over 3 r, the blob is flagged `block` and
   its raw parts (pixels that stand out from the local epoxy level, opened at twice
   the seam radius) are added back, because real grains sit inside exposure-step
   blocks.
4. **Whole-grain footprints** (`polarity: "both"`): the two-polarity mask joined
   with the grain outlines, closed by 5 µm, holes filled, opened like the seams,
   bubbles removed, split by the watershed. The pieces in one footprint become one
   grain (keeping the largest piece's id) unless two large pieces each fill over a
   quarter of it; a footprint with one grain replaces it when the detection covers
   under two thirds (the bright part of a CL-dark grain); a footprint with none is
   a new grain. Detections nested inside a larger grain are dropped.
5. **Classification** of every grain:
   - `halo`: centred on a bubble and not much larger; a crescent under 0.45 r
     touching a bubble; an arc on a circle (radius ≤ 0.8 r, solidity < 0.7); a round
     blob whose middle is at the epoxy level; or a crescent (solidity < 0.8,
     aspect > 1.8) whose concave side is at the epoxy level (a bubble too grey to be
     found).
   - `sliver`: minor axis < 15 µm, or aspect > 5, minor < 30 µm, hugging its box and
     either weak (contrast < 0.6) or very long (aspect > 8). Bright zircon needles
     along an axis stay.
   - `block`: under 15 % of its pixels stand out from the local epoxy level; or
     under 50 % and larger than 2.4 r or box-shaped; or a dark rectangle filling its
     box; or larger than `max_size_um` when set.
   - `speck` (`flag_specks`): under 0.21 r and weakly contrasted, or farther than
     1.8 r from every grain.

`GRAINATLAS_DEBUG_SEG=1` prints every detection's measures, verdict and `r`.

Validation: galleries of every flagged detection on the four Boise packages
(Grenville S and L, Sweden S and XS) were checked by eye. On Grenville S, the
prototype's extra detections were noise patches on the epoxy and fragments of
grains this pass outlines whole. Known misses on S HH11 and L H11: a few rims of
large grey-centred bubbles and a step region stay grains, and on S a few
fragments of real grains next to bubbles are flagged `halo` (one click restores
them).

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

Tabloid landscape at a fixed scale (1 mm = 8 µm by default). Only grains are
printed: artifacts and duplicates are left off, and the stats count both. A print
check (`audit`) counts printed grains that no band holds whole (always 0 so far)
and grains printed twice (a neighbouring row's band holds them too). Samples are
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
