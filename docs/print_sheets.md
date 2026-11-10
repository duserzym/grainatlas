# Print sheets, grain ownership and CL-dark grains

First used on the Boise State SEM run of 2026-11-03 (Hitachi TM4000, zircon
CL, three zigzag packages, 22 tabloid sheets) for spotting by hand next to the
digital spots in grainatlas.

## Layout rule: context first

At the laser a grain is found on the printout by the grains around it: the
operator matches the pattern of neighbours on paper to the pattern in the
camera view. A grain cropped alone, or a row cut away from the row above it,
cannot be found that way. So every automatic print layout in grainatlas
(print sheets here, and any later layout) keeps continuous pieces of the
mosaic together, and gives up paper before it gives up context. Cutting,
rearranging and removing empty stretches are allowed only where they do not
strip a grain of its neighbours.

Terms: `r` is the area's reference grain size (`segment::filter::reference_size`
of its printed grains, see below); the **context distance** `c` is
`context_um`, or when that is 0 (the default) `max(3 r, gap_break_um)`, which
is 420 µm on the Boise zircon mounts and 525 µm on mount L. Two grains are
**neighbours** when the gap between their boxes is at most `c`. A **cluster**
is a set of grains connected through neighbours.

The properties every layout must keep, each checked by a test in
`crates/grainatlas-core/tests/sheets.rs`:

1. **A band is a translated mosaic rectangle.** Nothing inside a band is moved,
   scaled, rotated or removed (background stripping greys pixels, it does not
   move them). Why: distances and directions between grains on paper are the
   ones under the laser camera, so the pattern matches.
2. **Neighbours share a band.** Grains of a cluster are printed in one band
   whenever the cluster's rectangle fits a sheet line (`body width` x
   `body height`). Why: the neighbours a grain is found by are the ones within
   a few grain sizes.
3. **2-D clusters stay 2-D.** A cluster that spans several rows of grains is
   printed as one tall band, not as one thin band per row. Why: rows printed
   apart put a grain's vertical neighbours on another line or sheet; the old
   row chaining did this on the apatite mounts (three rows of HJ17 on five
   lines of sheet 1).
4. **Bands of one area do not overlap**, except in repeated strips (6). Clusters
   whose rectangles would overlap are printed as one band. Why: a grain that
   appears whole in two bands can be spotted twice.
5. **Cuts go through empty gaps near the line end.** A cluster wider than a
   line (or taller than a page body) is cut. The cut is placed in an empty
   column (row) within the last 30 % of the line: the one nearest the line end
   among those at least `min(r, widest gap there)` wide, cut in its middle. A
   cut crosses a grain only when no empty column exists there; then the grain
   belongs to the side holding its centre and that band holds it whole. Each
   piece is only as tall as the grains it owns. Why: an empty gap is a place
   where the operator expects the pattern to stop; a gap near the line end and
   pieces fitted to their own grains keep the paper cost of a cut small, and
   neither removes a neighbour.
6. **Repeated strips at cuts.** Each band at a cut repeats a strip of
   `overlap_um`, by default `max(r, c / 3)` (140 µm on S, 175 µm on L; 17
   and 22 mm of paper at 8 µm/mm), and more where a crossing grain it owns
   needs it, from the other side, under a thin black and white diagonal hatch
   with "repeated" written along it. Its registration lists the grains in the strip as
   `context`, not as its own. The hatch never covers a grain the band owns, and the
   strip keeps its own tone (a white veil read as an odd bright panel). Why:
   the grains at the cut keep their neighbours on both lines, and no grain is
   counted or spotted twice. A wider strip (`c / 2`) showed whole large grains
   twice and cost 16 % of every line on mount L.
7. **No small orphans.** A break (a removed empty stretch, marked "//") never
   leaves a group of fewer than `min_group` grains (default 3: a grain and two
   neighbours) on its own when merging the group into the nearest band of the
   same area still fits a line, or when that band is cut anyway and the merge
   adds at most one line of width. Why: a lone grain on paper has nothing to be
   found by; the 2026-11 Grenville S sheets had 17 of 325 grains alone in
   their band.
8. **Breaks only at long gaps.** An empty stretch is removed only where it is
   longer than `c`. Why: shorter gaps hold the next grain inside the view the
   operator matches.
9. **Paper ranks last.** When a rule above costs paper (taller bands, a
   half-empty line before a cluster that fits a full line), the paper is spent.
   Sheet count is reported, not optimised against the rules.
10. **Areas are boundaries.** A band never mixes areas (each is its own
    mosaic); bands of different areas and samples may share a line, each with
    its label. Grains near an area edge owned by the neighbouring area keep only
    the neighbours in that area's mosaic.

The layout reports `isolated`: printed grains with fewer than two other grains
of their area visible in the band that owns them. It is shown in the sheet
statistics so a regression is seen before printing.

Any change to a print layout is held to these properties. Isolated single-grain
crops remain available only where the user asks for them by name (Layout →
"Grains or areas" → Items → "single grains, cropped alone (no neighbours)").

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
5. **Fused bubbles** (`flag_bubbles`): a round dark region inside a grain (12 µm to
   1.2 r) is a bubble when a circle 20 µm beyond it runs mostly (≥ 45 %) through the
   epoxy, because the bubble sticks out of the grain. A CL-dark core is ringed by its own
   grain and stays. The bubble and its halo come off the grain, which keeps its id. Bubbles
   are also found in a second, lower-threshold dark mask (closed and filled), so a
   large bubble with a grey textured middle and a faint, one-sided halo is caught
   (circularity > 0.75, solidity > 0.92).
6. **Bridged grains** (`flag_slivers`): a grain's own outline opened at 1.65 × the seam
   radius (6.6 µm on S) that falls apart into two or more compact parts (solidity ≥ 0.75,
   over 0.32 r) becomes those grains. Each part grows back over the outline by up to 8 µm,
   and the largest keeps the id. The test is geometric, so a CL-dark part splits off as
   well as a bright one.
7. **Classification** of every grain, after the splits, so halo pieces left beside a
   detached bubble are flagged:
   - `bubble`: round and dark with a bright halo ring, up to 1.5 r.
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

### Signal profiles: CL and BSE (2026-11-09)

The rules above were tuned on zircon CL. On BSE (backscattered electrons:
apatite, titanite and rutile mounts of the same run) grains are bright and
fairly uniform on dark epoxy, with dark charging shadows at their edges and no
bright bubble halos. There the bubble rules found 12 to 135 "bubbles" per area
(mostly the shadows), cut notches into grain outlines through the halo zone, and
the oversized-blob step split real large grains (a 775 µm chip, a 280 µm grain)
into numbered fragments.

`SegmentParams.signal` selects the profile: `null` (Auto, the default and what
older projects load as) reads the area metadata, `"cl"` or `"bse"` overrides it.
Auto uses `zag.ABCCHistory.SignalType1` (`BSE…` is BSE, `UVD` or `…CL…` is CL);
without it, a tile `SignalName` starting with `BSE` counts as BSE. A tile
`SignalName=SE` alone is ignored, because TM4000 CL tiles carry it
(`docs/legacy_workflow.md`). Unknown signals get the CL rules, so CL results
are unchanged (checked on Grenville_S/L_150X, Sweden_XS_150X and
Grenville_M_100X: the same outlines before and after).

One pipeline; with BSE these steps are gated:

- No bubble detection, halo zone, fused-bubble detachment or `bubble` / `halo`
  verdicts, whatever `flag_bubbles` says (the Grains tab disables it), and the
  split tool does not flag a cut-off part as a bubble.
- An oversized blob (over 3 r) is split and flagged `block` only when it
  follows tile edges: 28 % or more of its outline lies in the overlap bands of
  neighbouring tiles (placed tile footprints, tolerance max(6 µm, 2 work px)), or
  it is weakly contrasted against the epoxy around it, or box-shaped. Measured:
  exposure-step tiles 0.31 to 0.54, real large grains 0.24 to 0.25. The margin
  is small: a large grain lying along a tile seam can still be split; merge it
  back with Merge. Without tile footprints (loose images) BSE falls back to the
  CL rule.
- The `block` verdict measures a non-boxy BSE grain's contrast against the
  epoxy outside it, not only against the running background, which a large
  uniform grain can fill.
- Polarity stays as set; Both (CL-dark grains) is only meaningful for CL.

The CLI reports per area the grains, the artifacts by rule and the signal with
its source, and warns when an area finds nothing or flags 80 % or more of its
detections. `grainatlas segment <project> [--missing] [--area ID] [--signal
auto|cl|bse]` segments a saved project as the app does (saved placement and
parameters, previous grains and decisions kept, spots re-linked, every other
part of `project.json` untouched). `grainatlas check` lists areas that have
never been segmented. Print registrations record `project_uuid` and
`project_path`, so a printed sheet can be traced to the project whose grains it
shows: the Grenville_apatite_50X sheets of 2026-11-07 were printed from a
temporary copy whose grains never reached the project.

In the app, the Grains tab says "Grain finding has not been run on this area"
for an area without a grain list (an empty list means it ran and found
nothing, with the likely reason), marks such areas with a hollow dot in the
area lists, and its all-areas button finds grains in the areas without grains
only; when every area has grains it re-runs all of them after a second click,
since a re-run replaces hand-edited outlines that are not drawn, kept or marked.

Split tool: a part that a cut separates from a grain is flagged as a bubble when it is
round and below 0.7 × the epoxy level around the grain. One cut through the neck
therefore detaches a fused bubble; "Keep as grain" undoes the flag.

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
check (`audit`) counts grains missing from an owner band or not whole in it,
grains owned by more than one band, and whole visible grains absent from both
ownership and context (all three must be 0). Samples are ordered top of the
mount first; areas retain their sample's order. Bands of different areas and
samples may share a line, each with its label.

The layout follows [context first](#layout-rule-context-first): printed grains
form 2-D clusters through box gaps of at most `context_um` (0 uses the larger
of three reference grain sizes and `gap_break_um`). Intersecting padded cluster
rectangles merge until stable. Groups smaller than `min_group` (default 3)
join the nearest cluster when their union fits a full line. Clusters follow the
mosaic's long axis and stay whole whenever they fit a full line, even when that
spends a new line. Empty stretches between clusters become breaks marked "//"
with their length. Bands of the same area on one line share one sample/area
label, with no grain numbers.

Oversized clusters are cut vertically first, then horizontally, in the nearest
empty gap at least `min(r, widest gap)` wide in the final 30 % before the
available line ends. When no empty gap is available, a cut crosses the fewest
boxes and assigns each grain by its centroid, extending its owner's rectangle
to hold the whole box. A cluster with at least `2 * min_group` grains keeps at
least `min_group` owned grains in every piece. A small head spends a new line;
a small tail moves the cut earlier, searching backwards from the position that
leaves `min_group` grains, within 30 % of a full line. Smaller clusters retain
their usual cuts.

Each side repeats `overlap_um` beyond the cut (0 uses `max(r, c / 3)`), included
in the space needed on paper. If that strip holds no part of a grain box, it
widens to the nearest box plus `r / 2`, capped at `c / 2`. If no box lies within
`c / 2`, the default width stays. A thin black and white diagonal hatch marks the repeated
strip (the image keeps its tone), with a dashed grey cut line and "repeated" along it; owned boxes plus
padding are clear of the hatch. Registration distinguishes owned `grains` from
other bands' visible `context`, and records mosaic strip rectangles in
`repeated_um`. `row` is the area's 1-based cluster number in reading order;
all bands cut from that cluster keep that number. Stats include `clusters`,
`cuts` and `isolated` (owned grains with fewer than two other printed box centres
of the same area in their band), alongside the three audit counts.

Every band carries a 100 µm (or 50 µm) scale bar inside its owned part, left of
any right-hand repeated strip, and corner ticks. The header has the instrument
metadata from the sidecars (instrument and serial, detector, kV, emission, WD,
scan, magnification, tiles and pixel
size, capture times), the sample legend, laser spot circles at print scale and
the mount map: area boxes from stage coordinates in the SEM view, this sheet's
samples dark with their grains as white dots and its bands outlined.
Everything is black, white and grey. Corner marks sit in the sheet margins.

The registration sidecar (`<package>_print_registration.json`) is the
`print_sheet` frame, separate from the laser stage registration. In this example,
`row: 1` identifies the area's first cluster; further bands cut from that cluster
also use `row: 1`. `context` and `repeated_um` are empty because this band has no
cuts:

```json
{ "frame": "print_sheet", "um_per_mm": 8.0, "paper_mm": [431.8, 279.4],
  "sheet_origin": "sheet top-left corner, mm, y down",
  "mapping": "mosaic_um = mosaic_rect_um[0:2] + (sheet_mm - sheet_rect_mm[0:2]) * um_per_mm",
  "bands": [{ "sheet": 1, "sample": "HH3", "area": "007_HH3", "row": 1,
              "grains": [{ "id": "…", "number": 4 }],
              "context": [], "repeated_um": [],
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

`grainatlas sheets <project.grainatlas> --out <dir> [--paper tabloid] [--scale 8] [--params sheet_params.json] [--svg] [--title "…"] [--lab "…"]` re-renders saved mosaics and grains without stitching or segmenting. It writes the PDF, print registration and sheet stats JSON to `--out`, plus `<package>_sheet<N>.svg` with `--svg`. JSON parameters override defaults; flags override JSON. The output directory must be outside the project folder.
