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
Strips follow each grain's own sample when it has one, falling back to its area's sample.
Grains of different samples form separate strips even within one area; context and stray rules apply within each sample there.

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
   column (row) within the last 10 % of the line: the one nearest the line end
   among those at least `min(r, widest gap there)` wide, cut in its middle.
   Wide clusters are cut along x first, then each piece along y if necessary;
   height follows the local grains instead of making a sparse full-width slice.
   A cut keeps `min_group` owners on each side where possible; a short tail
   moves the search window earlier. On a full line, if the window cannot yield
   a fitting cut, search the full line before allowing fewer owners on a side.
   Both children must be worth a panel: each owns at least `min_cut_grains`
   grains (0 uses `min_cluster_grains`, default 5), and its repeated strips
   cover at most `max_repeated_fraction` (default one third) of its image area.
   Strip intersections count once, including inherited seams and clipped
   context fragments. The existing `min_group` bound also applies where
   possible. An optional cut to fill a partly used line or sheet is rejected
   when either child fails; the piece moves whole to the next line or sheet,
   without searching for a different spare-paper slice.
   For a cut needed to fit an empty body, try other positions in the window,
   then the whole line, then the other direction before forcing a cut. Among
   fitting candidates, a forced cut minimises the worse child's sum of missing
   owner fraction and excess repeated fraction; ties keep the gap/crossing
   preference and primary direction. Stats count these as `forced_cuts`.
   Whole owners and marked repeated strips are never relaxed. These defaults
   reject Sandbox M's 6-owner, 41 % repeated tail and S's 3-owner remainder,
   spending paper to keep their rows together.
   Empty target pieces are never emitted. A cut crosses a grain only when no
   empty column exists there; then the grain
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
   strip keeps its own tone (a white veil read as an odd bright panel). When
   fitting a piece to its owners clips a context grain at an adjacent edge,
   that visible fragment is also marked and registered as repeated. Why:
   the grains at the cut keep their neighbours on both lines, and no grain is
   counted or spotted twice. A wider strip (`c / 2`) showed whole large grains
   twice and cost 16 % of every line on mount L.
7. **Only cheap orphan joins.** Before rectangle merging, a disconnected
   group of fewer than `min_group` grains (default 3) joins its nearest
   affordable group of the same area. The closest pair of grain boxes must
   have a gap at most `1.25 c`, and the union of the pixel-snapped, padded
   rectangles must add at most 15 % to the host's rectangle area. The union
   must still fit a line, or add at most one line of extent to a host that
   already needs cuts. Both cost limits apply, regardless of paper size.
   Otherwise the group stays separate and the stray rule (11) decides whether
   it prints. Why: a far detection must not buy a broad strip of bare epoxy
   merely because the enlarged rectangle fits the paper. The extra quarter
   of `c` admits modest breaks near a long row; the area bound stops an
   off-row singleton from making the whole row tall.
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
11. **Strays stay off the paper.** After the cheap orphan joins (7), and before
    rectangle merging, a connected component owning fewer than `min_cluster_grains` grains (default 5)
    gets no band, unless it is tied for the largest cluster in its area. Setting
    this parameter to 0 disables the rule. Why: a panel for a lone off-row
    detection costs a sheet line and is not a dating target. The grain stays in
    the project and can be spotted in the app. Registration lists each omitted
    grain under `strays`, with its area, id, number, diameter, centroid and box
    in mosaic µm. Image content inside another band's rectangle is preserved;
    a stray is never registered as a target. The header and stats count strays
    separately, and per-area grain counts include only printed targets. Keeping
    the largest clusters, including ties, protects areas with only a few grains.
    The threshold 5 removes the separate 1 to 3 detection groups on both Sandbox
    mounts and S's four-detection off-row group, without applying an increasing
    cutoff to densely populated areas. Testing before rectangle merging is
    necessary: a long row's global bounding box can enclose a disconnected
    off-row detection which stretches a local cut piece. After omission,
    intersecting rectangles of retained components merge until stable (4).

The layout reports `isolated`: printed grains with fewer than two other grains
of their area visible in the band that owns them. It is shown in the sheet
statistics so a regression is seen before printing.

Any change to a print layout is held to these properties. Isolated single-grain
crops remain available only where the user asks for them by name (Layout →
"Grains or areas" → Items → "single grains, cropped alone (no neighbours)").

## Layout rule: print scale

A grain on paper needs to be large enough to recognise by its shape and zoning,
to find among its neighbours, and to take a hand-drawn spot circle and its
number. It does not need to be larger. At 8 µm/mm the Boise zircon S and L
sheets printed their median grains at 15 and 24 mm, and those sheets worked at
the laser; the same scale printed the XL mount's (median 287 µm) at 36 mm on 13
sheets, where 16 µm/mm prints them at 18 mm on 6.

So the default scale (`um_per_mm: "auto"`, `--scale auto`, an empty Scale field
in Layout) is chosen per mount from its printed grains
(`layout::sheets::auto_scale`):

1. **Target.** The coarsest round scale (R10 series: … 5, 6.3, 8, 10, 12.5, 16,
   20, 25 … µm/mm) at which the median printed grain is at least
   `target_grain_mm` across: 15 mm for CL (the zoning must be read to place a
   spot; the median then prints at 15 to 19 mm), 10 mm when every printed area is
   BSE (apatite, titanite, rutile: uniform grains; the BSE sheets chosen by hand
   in November printed their medians at 7 to 12 mm). A number in
   `target_grain_mm` overrides both.
2. **Lower limits.** The scale is no coarser than any of: the 10th percentile
   of the printed grains at 3 mm (room for a spot circle and its number on the
   small grains); CL zones about 10 µm wide at 0.4 mm (25 µm/mm); the third
   largest laser spot size at 1 mm across (so the spot key keeps three circles).
   When one of these binds, the registration says which (`auto_scale.limit`).
3. **Spot key.** At any scale, spot sizes whose circle is under 1 mm, or within
   0.3 mm of a larger circle, are left off the key; the header names them and
   the registration lists them (`spot_sizes_left_off_um`).

The header gives the scale and how it was chosen ("1 mm = 16 µm (62×) · auto:
median grain 287 µm prints 17.9 mm"); the registration records `um_per_mm`,
`scale` ("auto" or "fixed") and `auto_scale` (median, 10th percentile, limit).
An explicit scale (a number in the Scale field, `--scale 8`, or a saved
`sheet_params.json` with a number) is used as given. The rule ranks below
context first: a coarser scale puts more of the mosaic on a line, so 2-D
clusters (the two rows of an area) more often print whole as one tall band;
the context distance and the cut rules are unchanged, and paper is spent
before a lower limit is given up. Tested in `crates/grainatlas-core/tests/sheets.rs`
on synthetic size distributions of the 2026 mounts (zircon XS, S, L, XL and
apatite).

## Layout rule: packing

The `packing` parameter chooses placement after the bands have been cut.
`"lines"` places strips in mount order, filling a line before starting the next
line or sheet. `"packed"` places whole strips in the free rectangles of the
sheet body. Packing never changes the cut: properties 1 to 11 of
[context first](#layout-rule-context-first) hold for every automatic arrangement
of the same cut. Bands, repeated strips, context, owners and strays stay the
same. Paper still ranks last: packing uses space left empty by lines without
trading away neighbours or making extra cuts to fill a hole.

A **strip** is one block on paper (a run in the code): its label, one or more
band images side by side, their `//` gap marks, and a note line when needed.
Its width covers the images, label and gap marks, including trailing marks;
its height is the 4.6 mm label allowance plus the tallest image. A strip with
a continuation note also reserves 4.2 mm below the images. Strips without a
note reserve no note space. Lines mode keeps its 4.2 mm allowance on every line.
Every packed strip carries its sample and area label, including
continuation text, even when its run shared a label in lines mode. There are
no dashed line separators in packed mode.

The packer uses MaxRects with the bottom-left rule: choose the free rectangle
with the lowest top-edge coordinate, then the leftmost. It leaves a 2.5 mm gap
between blocks, with no rotation or scaling. Sheet 1 and later sheets keep
their own body heights. Packed and edited placement use a 40 mm first header
and a 25 mm later header. The first header keeps the acquisition grid, Print,
Package, counts, sheet samples and areas, spot key and mount map. Values wrap
within their columns; long titles, acquisition values or a larger spot key can grow
the header enough to keep all its content inside. Later headers keep the title,
sheet number, one print line, up to two lines of samples and areas, the spot
key and a smaller mount map. Long print text ends with an ellipsis; a list that
does not fit ends with "+ N more". Compact map labels are at least 1.8 mm high,
abbreviated to their column width, and limited to samples on the sheet with
"+ N more" for labels that do not fit. The map darkens areas on this sheet and
outlines its bands. Labels do not grow compact headers with the sample count.
Cutting still uses the original 50 mm and 36 mm headers, including their
existing growth for map labels, and the 4.2 mm note allowance. Only placement
uses the extra body space. `stats.header_mm` reports the first and later
placement header heights; `sheet_bodies` records each drawn header and body.
Sheets added in the editor use these same compact heights. Four orders are tried:

1. Mount order, filling sheets in turn without returning to earlier sheets.
2. Mount order with back-fill into earlier sheets.
3. Tallest strip first, with back-fill.
4. Widest strip first, with back-fill.

The fewest sheets wins; ties keep the earlier order. Equal heights or widths
keep mount order. Placement is deterministic for the same input. The packer
can also reserve occupied rectangles, as used by **Re-pack the others**.

Lines remains the default, including when older parameter JSON has no packing
parameter; other values are rejected. In Layout, Arrangement offers **In lines,
mount order** and **Packed, fewer sheets**. The `sheets` and `stitch` commands
accept `--packing lines|packed`. Making packed the default waits until it has
been used at the laser.

For grain-sheet registration, absent or empty `layout` means lines; packed
writes `"packed"` and an edited plan writes `"edited"`. Each band's
`sheet_rect_mm` follows its placement without changing the mosaic-to-paper
mapping. The optional `strip_blocks` list records packed and edited blocks
with a 1-based `sheet` and `rect_mm`, covering labels, images, gap marks and
notes. Explicit blocks also contribute to `bands_outside_body`, so an
oversized label is reported even when its image fits. The audit counts
intersecting image pairs and intersecting block pairs as `bands_overlapping`,
with an epsilon of 1e-7 mm; a pair may contribute to both counts. A block
covering a header also contributes. All audit counts must be 0. Legacy lines
registrations reconstruct same-area line groups from standard labels and
band order; their internal gap marks and dividers share horizontal space at
different heights. Zero overlap counts are omitted from stats JSON to preserve
lines output; missing counts default to 0.

In packed and edited plans, each sheet's stats budget uses `blank_mm2` for
body area minus strip blocks; `blank_below_mm` and `blank_right_mm2` stay 0.
Labels and notes use each block's width, and `other_line_mm2` is the space
inside blocks that is neither labels/notes nor images. Missing blank area
defaults to 0; zero is omitted so lines JSON stays unchanged. The header,
mount map and per-sheet counts follow the strips placed on each sheet.

## Editing strips

After **Make print sheets**, turn on **Edit strips** in the preview. Drag a
strip within the page. Moves snap to body edges and to a 2.5 mm gap from other
strips; hold Alt to turn snapping off. Arrow keys nudge the selected strip by
0.5 mm, or 5 mm with Shift. Drop a strip on another page's thumbnail to put it
in the first free place there, top edge first, then leftmost; if it cannot fit,
the move is refused. Refused edits show a reason but do not count as edits,
enter undo or redo history, or get saved with the project. Pages can be added,
empty pages removed, and pages moved earlier or later. The header cannot take
strips.

Drag a band's four crop handles to shrink or grow its mosaic rectangle, up to
the area image bounds. An edge snaps to the nearest position that crosses no
printed grain's box, so a crop never cuts a printed grain. Grains that would
leave turn red during the drag. The preview clips the old image or shows grey
where the crop grows; the band image is rendered again when the drag ends.
Repeated strips are recomputed from the new rectangles.

Change a strip's label, hide it, or select a hidden strip to show it again.
**Split** places a vertical or horizontal line that snaps to a gap between
grains and uses the existing cut code with its repeated strips. Both sides
must own a grain. **Join** restores two direct siblings from a split.
**Separate** makes one strip per band, each with its own label and a
"continues on sheet N" note in place of the `//` mark. The note follows the
neighbour's sheet when it moves. Separate restores the 4.2 mm note allowance
and grows the strip block. An overlap or body overflow caused by this growth
is reported by the same checks as any other edit.

**Re-pack the others** pins the strips already touched and packs the remaining
shown strips around them. **Back to automatic** drops every edit after a
confirmation that gives the edit count. Undo and redo apply to the whole edit
list, using the buttons or Cmd/Ctrl+Z and Shift+Cmd/Ctrl+Z.

The legacy Edit strips controls keep a shared `um_per_mm` without rotation.
Arrange mode below adds per-piece scale and rotation. Crop edges and split
lines in the legacy editor may not cross printed grains.

After each edit, a shown band keeps its existing owners wherever it still
holds their whole boxes. A grain that loses its owner can be adopted by
another shown band of the same area that holds it whole. Otherwise it is
listed in registration under `left_off`, with reason `"cropped"` or
`"hidden"` and the same grain fields as a stray. The stats count both reasons;
these omissions are not errors. A growing crop never takes a grain from an
existing whole owner: it shows and registers that grain as context. Automatic
strays remain separate, so every target is accounted for without a second owner.

The core checks the edited plan before rendering. Overlapping images or strip
blocks, a strip outside the body or covering the header, a target without a
whole owner unless listed as left off, and a target owned twice are problems.
**Export PDF**, **Export SVG**, **Print…** and **Registration** stay disabled
until the problems are fixed and the preview has finished rendering.
**Fix before export** lists the problems, with **Show strip** buttons. The
registration also carries an `edits` summary of the applied operations.

Edits are kept with the project in `project.json` under `print_sheets`: the
sheet parameters, resolved scale, and ordered edits naming stable strip or
band keys. A key contains the area id and a hash of the sorted owned grain
ids, with a counter for bands made by a split. The usual **Save project** saves
the list; it is not a separate file. Making sheets again replays it on the
new automatic plan. If cutting or placement parameters or the resolved scale
differ, replay drops the edits and says why. Display and header changes do
not invalidate them. An edit whose strip no longer exists is dropped with its
reason. Replay reports dropped edits without rewriting the stored list; the
next strip edit stores the surviving list with the new edit. An unreadable
stored list is kept verbatim when loading and saving, with a notice; a new
strip edit replaces it.

For command-line replay, `grainatlas sheets --edits FILE` reads a JSON object
with the same parameters, resolved scale and edit list. It reports dropped
edits and refuses export while problems remain. Both the desktop and browser
hosts use `print_plan_render` to apply edits to the automatic plan and render
pages, registration, stats, audit and problems, and `print_band_image` to
request a band's image for the editor.

A lines-mode plan can also be edited. Its dashed line separators disappear
after the first applied edit. Unlabelled runs keep their short divider until
moved; moving one gives it its own sample and area label. Packing the other
runs also gives them their own labels.

## Cross-area grain ownership (`crate::ownership`)

In a zigzag package neighbouring areas overlap (100 to 850 µm on that run), so
one grain is detected in several areas. Each detection is mapped mosaic → stage
through its nearest tile (as `AreaRecord::stage_of`) and from the stage into
every other area. A detection in another area within half the grain's size plus
`match_um` (20 µm), or whose outline holds the mapped centre, is the same grain;
linked detections form one grain. The outline test catches a piece of a grain
that the other area found whole (a CL-dark half, or a grain cut by this
mosaic's edge): on Grenville_XL_80X such pieces in HH3 lay 230 µm from the
centre of HH5's whole grain and would have been printed under both samples.

The owner is the area whose operator-drawn rectangle holds the grain's stage
position (`ownership::drawn_area_nm`). The TM4000 `.zag` gives each area's
rectangle as `CenterX/Y` and `AreaWidth/Height` in display pixels of the
navigation image: its `ImageInfoPixelSize` times `ImageInfoWidth /
ImageDispInfoWidth` (91.67 µm on the 2026 Boise runs), counted from its top-left
stage corner. The instrument covers the rectangle with whole tiles, so the tile
grid is up to 3 mm taller than the drawn row and a neighbour's grid can hold a
row more completely than the row's own area does. On Grenville_apatite_50X the
coverage rule alone gave HH11's row to HH5, WS3's row to HH11 and every grain of
TH3 to UM2 (57 grains against their drawn area); the drawn rectangles fix all
but grains whose own area did not detect them. Where rectangles nest (HH11
drawn inside HH5) the smallest one decides. A detection cut by its mosaic edge
never owns a grain another area has whole. A grain in no drawn rectangle, or an
area without that metadata, falls back to the coverage rule: the area where the
grain's box lies farthest inside the covered mosaic (a signed distance field of
the tile rectangles), a cut detection losing to a whole one. Such grains carry
`outside_drawn` and the app says "outside every drawn area" for them. The other
detections are duplicates of the owner's detection. A grain that maps into another area where nothing was detected stays
with its own area, so a missed detection never loses a grain.

Areas whose names differ only by a numeric `_N` (`KH2-30.7`, `KH2-30.7_2`) are
one sample, when the base name is another area's name.

In the app the canvas draws duplicates dashed in a colour per owning area, as
visibly as the area's own grains, with a legend ("32 grains held by HH5") and a
switch to hide them; the grain table shows "→ HH4 #12". A spot placed on a
grain another area holds is named for the current area as before, and the app
says which area holds the grain, with a button to go there. Print sheets and
their counts leave duplicates out.

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
never been segmented. `grainatlas import-grains <project> --from <copy> --missing` copies
the grains of another copy of the same project (for example the copy a print was
made from) into areas without grains, only when project uuid, area uuid, mosaic
hash and size, pixel size and tile placement all match; ids and numbers stay as
printed, spots in those areas are linked to the grain under them, nothing else in
`project.json` changes, and it never replaces existing grains. Print registrations record `project_uuid` and
`project_path`, so a printed sheet can be traced to the project whose grains it
shows: the Grenville_apatite_50X sheets of 2026-11-07 were printed from a
temporary copy whose grains never reached the project.

In the app, the Grains tab says "Grain finding has not been run on this area"
for an area without a grain list (an empty list means it ran and found
nothing, with the likely reason), marks such areas with a hollow dot in the
area lists, and its all-areas button finds grains in the areas without grains
only; when every area has grains it re-runs all of them after a second click,
since a re-run replaces hand-edited outlines that are not drawn, kept or marked.

Find missed grains in the Grains tab runs the same area segmentation and adds
only detections that no existing grain or artifact covers. Existing outlines,
ids, numbers and spot links stay unchanged. New grains continue after the largest
number; new artifacts remain flagged and unnumbered. Only unlinked spots inside
new grains receive links. Dashed accent outlines mark the additions: show only
new filters the table, Accept or Accept all clears their review flag, and one
undo restores the whole action.

`grainatlas segment <project> --add-missing [--area ID]... [--signal auto|cl|bse]
[--dry-run]` performs the additive pass on saved areas with grain lists. Areas
never segmented are skipped with a message to use `--missing`; the two flags
cannot be combined. It appends new JSON entries, writes masks and grain CSVs,
then saves the project atomically. It reports existing grains, added grains and
added artifacts by reason on stderr and as JSON on stdout. `--dry-run` reports
without writing any files.

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

Tabloid landscape at one scale per mount, by default chosen from the grain
sizes ([print scale](#layout-rule-print-scale)). Only grains are
printed: artifacts and duplicates are left off, and the stats count both. A print
check (`audit`) counts grains missing from an owner band or not whole in it,
grains owned by more than one band, and whole visible grains absent from both
ownership and context. It also counts `bands_outside_body`, using the recorded
`sheet_bodies[].body_rect_mm` in sheet coordinates, including repeated strips and pixel
rounding (all audit counts must be 0). An individual grain that cannot fit at the
requested fixed scale remains whole and triggers this audit; change scale or
paper before printing such a result. Cutting starts with samples at the top of
the mount; areas retain their sample's order. In lines mode, bands of different
areas and samples may share a line, each with its label.

The layout follows [context first](#layout-rule-context-first): printed grains
form 2-D clusters through box gaps of at most `context_um` (0 uses the larger
of three reference grain sizes and `gap_break_um`). Groups smaller than
`min_group` (default 3) join their nearest affordable group only across a
pairwise grain-box gap of at most `1.25 c` and with at most 15 % growth of the
host's padded rectangle area, subject to the existing line-size bound (7).
Remaining components smaller than `min_cluster_grains` (default 5) are strays,
except those tied for largest in their area. Only then do intersecting retained
padded rectangles merge until stable. This order prevents a far detection
inside a long row's global rectangle from stretching its local cut pieces. Clusters follow the mosaic's long axis and stay whole whenever they fit a full line, even when that
spends a new line. Short remainders can share a line with the next cluster or
cut piece, including the next area; a height cut does not force a new line when
its final piece leaves enough width. Lines use the actual remaining height of
the current sheet. A cut cluster may also take a height cut near that remaining
line end, only when both panels pass the owner-count and repeated-area limits
(5, 6). Otherwise it moves whole to the next sheet. A
whole cluster which fits an empty body spends a new sheet instead of being cut
to fill a partly used one. Lines mode keeps areas and pieces in reading order. Empty stretches between clusters become
breaks marked "//" with their length. Bands of the same area on one line share one sample/area
label, with no grain numbers.

Oversized clusters are cut along x first, then along y only where each width
piece still needs it, in the nearest empty gap at least `min(r, widest gap)`
wide in the final 10 % before the available line ends. When no empty gap is
available, a cut crosses the fewest boxes and assigns each grain by its centroid, extending its owner's rectangle
to hold the whole box. A cluster with at least `2 * min_group` grains keeps at
least `min_group` owned grains in every piece whenever a fitting cut permits it.
A small head spends a new line; a small tail moves the cut earlier, searching backwards from the position that
leaves `min_group` grains, within 10 % of a full line. If a full line still
cannot be cut, search the whole line, then try the other direction before
forcing the least costly fitting cut rather than sending the band beyond the
paper. Small clusters also pass the
panel-worth check; when a cut is unavoidable the forced-cut rule applies.
Height cuts use the same empty-gap preference as width cuts, favouring gaps
between grain rows before positions through a row.

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
of the same area in their band), `strays` and the audit counts.
`fill.band_area` is band image area divided by the available body area across
all sheets; in lines mode, `fill.line_width` is the mean sum of band image
widths on a line divided by body width. Labels, line gaps and inter-band gaps are excluded from
the numerator. Packed mode keeps `fill.line_width` at 0 in JSON. The denominator
sums the real body area of every sheet.
Registration records `sheet_bodies`, each with `sheet` (1-based), `header_mm`
and `body_rect_mm` (`x0, y0, x1, y1`). Bodies start 1 mm below their header and
end before the unchanged 5 mm footer. The legacy `body_rect_mm` is the envelope
of all bodies (exact for a single sheet); it is useful for older readers but
cannot audit individual sheets. The audit uses the per-sheet record, falling
back to the legacy rectangle, or paper bounds for older registrations.
A failing bounds audit can inflate fill, so check the audit before interpreting it.

Stats record the full and compact `header_mm` heights and `height_budget` for
every sheet. Area fields are in mm²: `header_mm2`, `footer_margins_mm2` (also
includes the 1 mm header clearance), `labels_notes_mm2`, `band_mm2`,
`blank_right_mm2` (to the right of the last image on each line), and
`other_line_mm2` (gaps and space below shorter images). In lines mode, together
with `blank_below_mm` times body width, these partition the paper area. The last
line includes its 4.2 mm notes allowance. `empty_epoxy_mm2` is a subset of band
area: full-width rows farther than `pad_um` (default 22 µm) from every visible
printed grain box in that band. It excludes strays and artifacts, and includes
owned and repeated context targets. This row measure identifies tall empty
strips; it does not claim to measure every background pixel between grains.
`fill.empty_epoxy` divides that area by total band image area.
`fill.repeated` divides the sum of each band's `repeated_um` rectangle union
by the sum of band areas; `worst_repeated_fraction` is the largest individual
band fraction. These use the registered strips (before the drawing mask
clears owned grain boxes), with intersections counted once. `cut_sources` in
each band lists its inherited cuts in order: `width`, `height_line` (an empty
body), or `height_sheet` (the remaining body). A forced cut is included in
`cuts` and also counted in `forced_cuts`. Older JSON defaults the new stats to 0
and cut provenance to an empty list.

Every band carries a 100 µm (or 50 µm) scale bar inside its owned part, left of
any right-hand repeated strip, and corner ticks. The header has the instrument
metadata from the sidecars (instrument and serial, detector, kV, emission, WD,
scan, magnification, tiles and pixel
size, capture times), the sample legend, laser spot circles at print scale and
the mount map: area boxes from stage coordinates in the SEM view, this sheet's
samples dark with their grains as white dots and its bands outlined.
Everything is black, white and grey. Corner marks sit in the sheet margins.
In lines mode, sheet 1 retains the full acquisition block in a 50 mm header.
Subsequent sheets
use a 36 mm header with title, sheet number, counts, scale and processing,
"acquisition details on sheet 1", the samples on that sheet, the laser key
and the mount map. Text sizes are unchanged. The map occupies a lower panel
below the title and counts; its labels stay inside it. For mounts with many
sample labels, either header can grow to `19.1 + 2.7 × labels` mm, capped at
66 mm; both heights are recorded. The measured Sandbox and Grenville XL mounts,
and the letter-size Sandbox render, use 50 mm and 36 mm. No margins, footer
height or print scale change.

See [Layout rule: packing](#layout-rule-packing) for packed placement, block registration and budgets.

The registration sidecar (`<package>_print_registration.json`) is the
`print_sheet` frame, separate from the laser stage registration. Its
`registration_file` gives the exact export filename, including case, and is
used both by the footer and the UI download. Exports preserve the saved project
folder's spelling when it differs from the native package name only by case
(`Sandbox_S.grainatlas` versus `sandbox_s.zag`); other package names stay as
saved. The CLI also honours an explicit `registration_file` parameter. Renaming
a delivered sidecar after rendering cannot update a printed footer; choose the
export name before rendering. In this example,
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
  --print tabloid [--scale auto|8] [--strip-bg 22] [--no-denoise] [--no-sharpen] \
  [--title "Grenville · mount S · zircon CL"] [--lab "…"]
```

writes `exports/<package>_grain_sheets_tabloid.pdf`,
`exports/<package>_print_registration.json` and `exports/ownership.json`.
In the app: Layout → Print sheets.
Print… opens the system print dialog in the desktop apps and in browsers,
without a pop-up window.

`grainatlas sheets <project.grainatlas> --out <dir> [--paper tabloid] [--scale auto|8] [--params sheet_params.json] [--packing lines|packed] [--edits FILE] [--svg] [--title "…"] [--lab "…"]` re-renders saved mosaics and grains without stitching or segmenting. It writes the PDF, print registration and sheet stats JSON to `--out`, plus `<package>_sheet<N>.svg` with `--svg`. JSON parameters override defaults; flags override JSON. The output directory must be outside the project folder.

## Whole area on one sheet (`layout::whole`)

Use this mode for a thin section, slab, or any area that must be printed whole.
It needs a stitched base mosaic and its tone, with no segmentation or grains.
It is exempt from the context-first band rules because nothing is cut, moved
or omitted: property 1 holds trivially. Grain-sheet layouts are unchanged.

The image is the continuous, toned grey mosaic, embedded once. Its dimensions
on paper are exactly its dimensions in mosaic um divided by `um_per_mm`.
There is no rotation of the image. `orientation: "auto"` compares landscape
and portrait paper and chooses the larger possible print scale, accounting
for the 30 mm header, the grain sheets' 8 mm default margin and 5 mm footer,
the 1 mm header clearance, and 12 mm tick/label gutters on every image edge.
Auto rounds the required um/mm upward (coarser) to the first fitting value of
`1, 1.25, 1.6, 2, 2.5, 3.2, 4, 5, 6.3, 8` times a power of ten. A fixed
scale is used exactly; if it cannot fit, the error gives the fitting round
scale. Paper presets are tabloid, letter, A4 and A3.

The TM4000 example (19698 x 12657 px at 1.984375 um/px, approximately
39.09 x 25.12 mm) fits one landscape tabloid at 125 um/mm. A scale bar is
20 to 50 mm on paper. The header includes the title, sample, area, lab,
acquisition metadata, mosaic size in px and mm, and the explicit printed
ratio. The smallest printed detail is estimated as two printer dots:
`2 * 25.4 / printer_dpi * um_per_mm` um of sample. At 125 um/mm this is
about 21 um at 300 dpi or 11 um at 600 dpi. The source pixel size is shown
separately; printing cannot recover detail missing from the source.
The embedded image targets 600 dpi by default, using a fractional box-area
filter after tone, rounded to integer pixel dimensions and capped at the
source width and height independently. It is never upsampled. Stats include
its pixel dimensions and actual dpi, the source pixel size, both detail
estimates, the chosen orientation and scale, and image/gutter rectangles.

The mosaic top-left is the coordinate origin, with x right and y down. All
four edges have minor ticks every 1 mm of sample and labelled major ticks
every 5 mm (labels are sample mm from the top-left). With `grid: true`, a
light grey grid with a lighter halo overlays the mosaic. Its adaptive major
step is the largest 1-2-5 sample distance no greater than 40 paper mm,
giving full cells 16 to 40 mm across. On the example it coincides with the
5 mm major ticks. Cell columns run A, B, ..., Z, AA, ... left to right;
rows run 1, 2, ... top to bottom. Labels appear in the gutters on both
opposite edges. Edge cells may be partial. Disabling the grid keeps the
edge ticks and stage marks.

Two arrows labelled `+X stage` and `+Y stage` show increasing stage
coordinates: `stage_sign: [-1, -1]` means left and up. Corner stage X/Y
coordinates in mm, to three decimals, come directly from
`AreaRecord::stage_of`, the same nearest-tile-centre mosaic-to-stage mapping
used by stage/laser export. If stage data or placement is absent the corner
says that stage X/Y is unavailable; no calibration is invented.

The registration is the existing `PrintRegistration` schema, frame
`print_sheet`, with one band on sheet 1, the area and sample, empty `grains`,
and the full `[0, 0, width_um, height_um]` mosaic rectangle. Its
`sheet_rect_mm` is exactly the image rectangle, excluding gutters.
`sheet_bodies` records the single body; `scale` is `"auto"` or `"fixed"`.
The additional optional fields are `layout: "whole_area"`, `grid_step_um`
and `grid_origin_um: [0, 0]`. Grain registrations omit these fields;
older registrations deserialize with empty layout and no grid.

For each axis, the existing mapping remains:

```text
mosaic_um = mosaic_rect_um[0:2]
            + (sheet_mm - sheet_rect_mm[0:2]) * um_per_mm
sheet_mm  = sheet_rect_mm[0:2]
            + (mosaic_um - mosaic_rect_um[0:2]) / um_per_mm
```

For a grid label, convert the column to a zero-based index `c` (A = 0,
B = 1, ..., AA = 26) and the row to `r` (1 = 0). Its top-left mosaic
coordinate is `grid_origin_um + [c, r] * grid_step_um`. Add one step for
the bottom-right and clip to the full mosaic rectangle. For example, with
a 5000 um step, C4 is `[10000, 15000, 15000, 20000]` um. The core helper
`layout::whole::grid_cell_rect` implements this mapping. There are no grid
fields when the grid is off.

```bash
grainatlas sheets section.grainatlas --out /tmp/section-print \
  --layout whole --area 001_section --paper tabloid --orientation auto \
  --scale auto --dpi 600 --name section-plan --title "Section plan" \
  --lab "Lab name" --svg
```

This writes `section-plan.pdf`, `section-plan_print_registration.json`,
`section-plan_stats.json`, and optionally `section-plan.svg`. Without `--name`
the stem is `<package>_whole_<paper>`. The footer names the exact registration
filename and gives the printed date and page 1/1. The output directory must
be outside the saved project, and the stem must have no directory. Use
`--no-grid` to omit the grid; `--scale 125` fixes the scale. `--params` accepts
WholeParams JSON, and explicit flags override it. `--area` defaults to the
only project area; multiple areas require a selection even when only one
has a saved mosaic. The default `--layout` remains grain sheets.

To stitch and print a single area directly:

```bash
grainatlas stitch <single-area-folder> --out section.grainatlas \
  --print tabloid --print-style whole
```

This writes the whole-area PDF, registration and stats under `exports/`.
This style does not imply segmentation; an explicit `--segment` still runs
it. A package with several areas must use an area folder, or stitch first
and then select an area with `sheets --layout whole --area`.

Shell/UI integration note: expose
`print_whole(params_json: String, tone_json: String) -> Result<String>`
through the shell, calling `Session::print_whole(&params_json, &tone_json)`.
The tone JSON is `{area_id: ToneParams}`, as for `print_sheets`; missing
areas use their saved tone. The returned JSON is `{pages, registration,
stats}`; use `layout::pdf::pdf_from_svgs(&pages)` for PDF. Hosts can override
`printed` (the current UTC date by default), and should choose `name` or
`registration_file` before rendering. The call borrows only
resident base pixels, so it works after heavy-buffer eviction. It reports
an error if the selected area has no stitched mosaic.

In the app, Layout → Print sheets offers Grain sheets (default) and Whole
area, one sheet. Whole-area printing selects a stitched area without requiring
grains, with auto, landscape or portrait orientation, automatic fit or a fixed
µm/mm scale, and a grid checkbox. PDF and registration use the same export
controls as grain sheets. Both the desktop blocking pool and browser worker
call `Session::print_whole`; the base mosaic remains available after eviction.

## Arrange mode

Layout → Print sheets offers Automatic and Arrange. Arrange makes each image
band in the packed layout an object on paper,
including its sample label, scale bar and spot-size references. Runs with
several bands become separate objects with continuation notes. It uses the
existing strip edit replay and the same MaxRects packing algorithm as Packed.
No registered grain, spot, name or link
moves in mount or stage coordinates, and source pixels are never changed.

Manual arrangement is an explicit departure from automatic context rules:
whole pieces may rotate on paper and each piece may have its own uniform
scale. Grains within a piece retain their neighbours and their aspect ratio.
The orientation arrow marks mount +y and the paper rotation angle. Automatic
cutting, ownership, repeated context and stray rules are unchanged. Auto-arrange
restores the packed automatic scale and orientation for all, selected or
unselected pieces, reserving space occupied by the others. Back to automatic
asks before discarding the edit list.

Click selects a piece; Shift-click adds or removes it. Dragging on empty paper
selects a box. Drag moves the selection and corner handles scale the whole
selection proportionally about the opposite corner. Snap guides follow body
margins and other pieces' edges and centres; Alt temporarily disables Snap.
Arrows nudge 1 mm, or 10 mm with Shift. Align and Distribute use paper mm.
The side panel also edits position, dimensions, scale and rotation, and moves
pieces to another sheet. A drop on a page thumbnail moves the selection there.

The header is fixed. A selection is clamped below it as a group. Overlapping
drops are refused atomically, with a message; they never enter saved edits or
undo history. Collision checks conservatively use the full rotated bounding
box, including labels and references, so rotated pieces also keep clearance
at their corners. An oversized group is refused instead of distorted.

Each piece's spot circles use its exact physical scale, and each band has a
1–2–5 scale bar labelled in µm. Grain and reference labels scale with the piece
with a 6 pt minimum. A reserved reference row keeps spot circles off neighbouring
pieces. When scales differ, the fixed header says “Spot sizes are shown with
each piece”; otherwise it shows the shared scale key.

New arrangements use version 2 of `project.print_sheets`; version 1 and older
unversioned strip edit lists still replay. The field travels in the existing
complete project records used for save, recovery, mirror mode and atlas bundle
publishing, and participates in the print cache fingerprint. Element sync
preserves the local recipe when refreshing core records. The draft also retains
the recipe independently of pending elements, so reopening an older server
bundle with a clean outbox cannot discard it. An explicit reset is retained too.
UPbDB’s current
element-sync schema has no project recipe field, so recipe-only edits need
atlas publishing to reach another computer; automatic recipe-only outbox sync
needs the contract extension specified in [sync-client.md](sync-client.md#print-recipes-and-local-recovery).
Version 1 strip edits use these same paths. Undo and redo store atomic group operations.
Unknown edits or versions are retained as raw project data rather than dropped
on save.

For manual bands, registration includes `transform.crop_um`, `um_per_mm`,
`rotation_deg`, `mount_to_paper` and `paper_to_mount`. The two affine matrices
use SVG order `[a,b,c,d,e,f]`: `x' = a*x + c*y + e`,
`y' = b*x + d*y + f`. Mount positions are the area's mosaic µm, and paper
positions are sheet mm from the top left, y down. `sheet_rect_mm` is the rotated
image's bounding box; use the matrices for registration, not that box's edges.
The matrices include Canvas rotation and flips before the piece's paper
transform. `transform.rotation_deg` is the additional paper rotation;
`transform.crop_um` and `mosaic_rect_um` are in the image display frame, while
`image_crop` retains the original mount crop. The top-level `paper_to_mount`
matches `transform.paper_to_mount` on arranged bands.
The same SVG geometry drives preview, full-quality print and PDF. PDF embeds
the registration and uses the transformed PNGs without resampling their pixels.

Canvas crop and mask filtering also apply to arranged pieces. Each band uses
the image recipe before its paper transform; the registered source geometry
stays in mount µm.

## Canvas image recipes

Canvas → Edit image saves a version 1 `image_edits` recipe per area. Rectangle
and polygon crops and mask strokes use mount µm. A grain is left off print
sheets when its centroid is outside the crop or inside a mask stroke, including
the round ends and joins. Its geometry, name, links and registered spots remain
in the project. Reset restores eligibility. The Canvas grain table dims these
grains and reports how many have registered spots.

The crop is defined before rotation and flips. Rotation is about the crop
centre (the image centre without a crop); flips use the mount axes. Image
levels follow the display LUT. Cropped pixels print as the sheet background.
Masks replace the whole brush path with light grey and a hatched border, in
both the view and print bands, after enhancement too. The hatch stays aligned
with the mount when the view is rotated or flipped. Automatic packing runs on a temporary
oriented copy of eligible grains; the stored geometry never changes. The
context-first rules above still govern automatic cuts in that view.

Each registration band includes `paper_to_mount`, a six-number SVG affine
`[a,b,c,d,e,f]`: `mount_x = a*paper_x + c*paper_y + e`,
`mount_y = b*paper_x + d*paper_y + f`. Paper inputs are mm; mount outputs are
µm. This composes image rotation and flips with band scale and translation.
`mosaic_rect_um` is the band's rectangle in the oriented display frame;
`image_crop` remains in original mount µm. Consumers must use the affine when
present; the older rectangle-only mapping still applies to older registrations.
Whole-area sheets carry the same affine.

Manual arrangement may rotate pieces on paper and give pieces different scales.
Those choices depart from automatic layout's shared scale and mount orientation;
the user explicitly chooses them. Auto-arrange restores the packed automatic
layout. Canvas image orientation is part of the area's view and remains in
that automatic layout. The saved `paper_to_mount` already includes the piece's
paper transform; consumers must not apply it a second time.

Recipes survive Save, local recovery, undo/redo, tab changes and published
project JSON. Mirror copies retain the full project record and the publication
outbox tracks recipe changes using the existing project publication route.
The annotation sync endpoint carries spots, grains and labels; recipes travel
in the versioned project publication, as print recipes do. No per-grain server
request is made by image editing.
The histogram uses resident pixels, or at most four tiles from the published
overview when full-resolution pixels are not loaded. Levels Auto uses that
sample; opening the editor never downloads the full mosaic for a histogram.

## Spots and labels on print sheets

**Print sheets draws no spots in any mode, including Arrange.**
This changes the earlier Arrange pieces, which drew spots with fixed labels.
**Grains with spots draws them in every mode:** Lines, Packed, Edit strips,
Arrange and Whole area. Choose it in Style to use the usual sheet controls
with registered laser spots.
Switching between this style and Print sheets changes display only. It keeps
cuts, saved strip edits, grain ownership, scale and arrangement. Display options
are on the render plan, outside the cutting parameters in `project.print_sheets`.
Export PDF, Registration and Print use the same current render and print-quality
job as the preview. Print uses the hidden print document in the main window.
On Windows, the desktop print dialog shows the layout's paper size and sends
it to the chosen printer at 100% scale. If the printer does not report that
paper size, choose another printer or change the paper in Layout.

Every spot whose registered position is inside a printed image is drawn, including
lines and rasters. Circles have their registered diameter, lines have their
registered width, and rasters use their registered corners. Image edits and the
band or piece transform map these shapes to paper mm, including piece scale and
rotation. Markers are clipped to the image and have dark and light strokes.
Labels stay upright. A spot in a repeated strip is also labelled there, in grey;
unique anchor counts exclude these repeated occurrences. The report separately
lists occurrences so context copies can be inspected.

Names are exactly the project spot names, with XML escaping when writing SVG.
Placement never renames, moves or edits spots, grains or links. It changes neither
sync elements nor their hashes. Regression tests compare adding, moving and
clearing pins through the real AppState save path with the same sync JSON and
hashes. A SHA-256 regression also compares the spots-off SVG bytes with the
existing synthetic print editor fixture rendered at revision cc57ab0.

Label size is the SVG em size in paper mm. **Auto** is the default; a fixed
number from 1.6 to 4.0 mm retains fixed-size placement. Spot and rail labels use
IBM Plex Sans Condensed Medium (SIL OFL 1.1). Header, strip and other print text
retain DejaVu Sans, and spots-off SVG bytes remain unchanged. Glyph advance
widths, ascender and descender come from the bundled Plex TTF with ttf-parser;
kerning and ligatures are disabled. Glyphs missing from Plex, including Ω, use
the bundled DejaVu Sans advance and face in both SVG and PDF. Boxes add 0.8 mm
horizontally and 0.5 mm vertically. Every SVG page with labels embeds Plex in
one `@font-face` with a base64 TTF data URI; pages needing fallback glyphs also
embed DejaVu Sans. Image/blob previews therefore use the same fonts on macOS
and Windows. PDF export registers, embeds and subsets both faces.
The browser width check keeps its 3 % bound. Its former 3.75 % error came from
Ω falling back to a system face while Rust reserved a full em for that missing
Plex glyph; the 0.8 mm plate padding was correct.

Auto chooses one size for the whole page. Its descending list is 3.2, 2.9, 2.6
and 2.4 mm. Auto never goes below 2.4 mm (about 7 pt); a smaller fixed size can
still be entered. Sizes above the paper-scale cap are omitted: the padded box
height should be at most 1.5 times the median equivalent-area grain diameter
on paper. The paper-space polygons already include print scale and rotation.
The cap has a 2.4 mm floor because readability takes priority. Without grain
outlines, the upper bound is 3.2 mm.

Trials reuse the marker, grain, decoration and image grids. They try adjacent
slots and short straight connections in eight directions at three distances,
and stop at the largest size with zero conflicts, zero ambiguous associations
and at most 15 % in-image leaders among the labels placed on the page. A leader
counts against this limit only when its label box touches an image. Straight
connector ticks shorter than 3 mm do not count against the limit. Labels on white-paper rails do not
count: when paper is available, larger text on rails is preferable to smaller
text inside the image.
If no trial qualifies, use 2.4 mm and run its full bounded rail and repair search.
The fallback result is compared with its trial by conflict count, ambiguous
count, omitted context count, in-image leader count, total tick and leader count, then total leader length; keep
it when that score improves or stays equal. A qualifying trial with omitted
context also runs full repair at its chosen size before settling the omissions.
Expensive fallback is run only once. There is no clock-based early exit.
The report gives `size_mm` per page; Print shows “Labels 2.6 mm on this page” in
Auto. Pin offsets remain paper mm from the anchor to the box's top left at every
chosen size; a box moved beyond the body is brought back inside for printing.

On images, labels have a white plate at 0.9 opacity, corner radius 0.35 times
box height, a 0.08 mm mid-grey outline and dark `#111` text. Repeated labels
use `#777`. Labels on white paper have no plate or outline; each horizontal
rail tier shares a baseline. Leaders are 0.14 mm `#333`, with a white casing
clipped to the image. Their visible ends leave 0.25 mm gaps at the spot and
label; the stored conservative collision paths include those gaps. Every label
whose box is more than 0.8 mm from its physical marker edge has a straight
0.1 mm connector tick to the nearest point on the box, unless it already needs
a routed leader. Ticks have the same white image casing and reach both edges.
They participate in marker, label and leader crossings and the 0.8 mm leader
clearance checks. `connector_tick` identifies them in the report. Rail leaders
approach the middle of the near label edge. Marker geometry keeps registered
size with a thinner dark casing; circles under 0.8 mm also get a 0.25 mm centre
dot. Legacy Arrange callers without explicit display options keep their previous
minimum text-size allowance.

The placer works once per sheet, after all its geometry is known. It reserves
all markers and the existing header, footer, strip text, notes, repeated-strip
text, gap marks, bars, ticks and grid labels. The faint whole-area grid strokes
are background references: labels may cover them and leaders may cross them.
Treating those strokes as hard walls fenced nearby markers into grid cells and
caused four false placement conflicts in the twelve-spot whole-area regression.
Grain polygons and image borders have soft costs. A label can cover its own image or free paper in the body but
cannot cover another image. Rotated image polygons are checked in paper space.

Candidates start with four corner positions and four positions centred on the
spot's horizontal or vertical axis. The centred ring-0 positions have the same
base costs as the corners, including for wide boxes (over three times their
height). Their gap is capped at 2.9 mm to keep a short connector while leaving
room for a small manual move. Corners retain one box height of clearance.
Further candidates use five distance rings, outward square rings and slots
along image edges and adjacent paper. Costs include
distance, nearby direction consistency, zoning and grain-edge coverage, image
border straddling, leader length and bends. Hard conflicts dominate these costs.
An 8 mm uniform grid restricts marker, decoration, grain and placed-label queries
to local neighbours. For a candidate without a connector, measure the exact
shortest distance from its box to its own physical marker and query nearby
markers within 1.2 times that distance. Another marker at or below that ratio
makes the unconnected label ambiguous. These boxes lie within 0.8 mm of their
own marker. A drawn tick or leader identifies its marker explicitly and carries
no ambiguity count or penalty. Its collision checks still apply. Ambiguity
costs 10000, above grain coverage and below the 1000000 cost of a hard conflict.
Greedy placement, annealing, local repair, farther slots and group matching use
this cost.
A straight local connection of at least 3 mm adds 24, so an adjacent axial
plate can be preferred to a distant corner that covers less grain. Wide boxes
prefer 2.5 mm of marker clearance (48 per nearby marker); small boundary repairs
include clear decoration-edge slots for wide boxes before spending route budget
on other labels. Their first assignment pass requires that clearance where a
complete placement permits it. This preserves room for a small manual pin drag.

Anchors sort by spot id and stable band occurrence. Pins are placed first, then
anchors with the fewest free local candidates. Greedy placement tries nearby
labels with their connector ticks before longer leaders. Centred adjacent
plates use at most 2.9 mm clearance from the marker bounds. Rail slots reserve
one box height around markers,
leaving room for a small manual label move without covering a fixed marker.

After greedy placement and a short first annealing pass, remaining hard conflicts
seed local groups. Conflicting anchors join when their centre distance is at most
`3 × max(label heights) + max(label widths)`. Each group's hull is the axis-aligned
bounds of its anchor centres. Any current label box wholly inside that hull,
expanded by the group's maximum label height, joins too. Growth and merging repeat
until stable. An automatic plate outside the body also brings nearby plates
into its group, so a clear neighbour cannot fence it outside the page. This
expansion is measured from the conflicting anchor. Groups are processed by
their smallest spot id and occurrence.

Clear adjacent plates stay fixed outside local repair. Small dense components
of up to 16 anchors can move together to clear connector crossings. They use
at most 32 candidates per anchor and 4096 forward-search nodes, sharing the
page's six-million pair-check budget. A component grows through markers within
two label heights plus the collision gap. Straight candidates are tried first;
a blocked interior anchor can use the existing bounded two-bend last resort.
Grain-corner waypoints expose narrow epoxy gaps missed by the fixed grid.
Blocked plates try at most two successful routes, with reflected alternatives
checked against all current geometry. Different paths to the same box remain
separate candidates, allowing the assignment to use either side of a corridor.
A hard conflict, an existing leader, or a
plate covering the middle of another grain sends that anchor to an image boundary
rail. Rails are rows of slots on white paper above and below wide images, or
left and right of tall images. The first tier starts at the outermost nearby
image decoration (grid letters, tick numbers and stage corner text), plus
1.5 mm. Further tiers advance outward by one label height plus 1.5 mm.
Tiers start beside the image, regardless of unused space at the body edges.
Plates keep the 1.5 mm decoration gutter; leader strokes keep the normal
collision margin around decoration. A rail must fit inside the body and cannot
cross a different band. Thus open white paper beside a line can be used, while
most 2.5 mm packed gaps cannot.

Each tier preserves anchor order along the image's long axis. Dynamic
programming minimises the sum of squared label-centre displacement along that
axis over 0.5 mm offsets and exact anchor-centred offsets, with at least 0.8 mm
between plates. It checks adjacent leader geometry and verifies all pairs in
the resulting assignment. An anchor tries the innermost tier on both sides
before a further tier. Blocked vertical exits are rejected once, rather than
searched again at every tier. After joint boundary repair, rails move inward
where a collision-free position exists, and each occupied tier is assigned
again to reduce displacement. This preserves short local leaders and fixed
adjacent labels while freeing blocked exits.

An automatic label inside its image has a single straight leader of at most
two label heights (7.24 mm at a fixed 2.4 mm font size). The segment must
clear every grain except the grain containing its anchor. A label needing a
longer connection or a bend goes to a boundary rail. Saved pins keep their
explicit positions.

Rail leaders run perpendicular to the image edge from the spot to 0.2 mm
beyond the image, then straight to the centre of the label edge facing the
image. For horizontal rails the first segment is vertical. The outside segment
is at least 45 degrees to the rail, except that a first-tier label may use a
flatter segment when its horizontal displacement is under 3 mm. A failed angle
or collision check sends the label to a further tier or the other side.
Rail leaders never use A*. Sorted order is verified with segment and clearance
checks, including leaders on other rails, pins and fixed adjacent labels.
A leader approach closer than 0.8 mm centre to centre is a hard conflict; only
the first 2 mm around anchors is exempt from this clearance. Crossings remain
conflicts there too.

If a boundary rail is unavailable, local groups search epoxy and nearby paper.
Groups use Hungarian slot matching with six padding steps
(12, 20, 32, 48, 72 and 100 mm).
Dense components also include nearby blocking plates and leaders in a hull
expanded by three label heights. They try four deterministic route orders, and
can move clear members to other clear slots to open a corridor. Outside labels
and leaders remain fixed obstacles. Only improvements that do not create
outside conflicts are retained. Groups larger than 80 anchors split
canonically at their median along their longer axis. Route attempts are capped
at 512 per group and 1024 per page; route results are cached for unchanged
obstacles and rechecked before reuse after a placement changes.
Failed parts search further slots and refine only their unpinned labels.
Small boundary repairs with fewer than 32 straight candidates also try the
bounded routed fallback. A blocked nearest-point connection can try a facing
corner or edge midpoint, with the same length and collision checks. Route trials
keep the nearest 16 distinct slots and choose up to 16 more by spatial spread,
within the existing 32-trial limit. Nearest-only trials could all use one blocked
exit after a pin and leave two automatic labels colliding.
Small boundary repairs (at most 20 labels) also try exact decoration-edge slots
and a bounded set of detours for blocked plates. Their compact candidate pools
use a deterministic forward-checking search with 20000 nodes per pass before
the existing refinement. The first pass prefers marker clearance of one label
height for wide plates, so a small pin drag has room; the second pass permits
tighter placements. All ordinary collision, routing and page budgets still apply.

Refinement uses a fixed-seed xorshift generator, with 8 moves per conflicting
unpinned label before grouping and up to 36 after a failed placement, capped at
1024 moves per pass. Resolved labels skip these moves. Local routes are straight.
Only when rails and straight routes fail may a bounded 0.4 mm A* search run. Its
shortcut path is accepted only with at most two bends. A longer zig-zag is
rejected and the unresolved label remains counted as a conflict. Collision
checks use the rendered polyline or flattened curve. Fixed iteration limits,
canonical ordering and a fixed random seed make placement deterministic.
The last resort first tries a bounded visibility graph, then at most 512 A*
expansions per route, with a shared page budget of 32000 search steps. The report marks
these routes with `last_resort`. A failed route remains in the conflict count.
Boundary repair keeps at most 128 candidates per label (64 local, 32 on each
rail side across four nearby tiers) and tries at most 8192 moves. A page permits
at most two repairs and six million candidate geometry checks after bounding-box
rejection. Candidate bounds are cached before the pair checks.
It never stops on a wall-clock deadline. Candidate route scores and nearest-slot
distances are cached. Short routes use available room for marker clearance,
while keeping their straight segment and length limit.

The report includes every placed label box, marker centre, occurrence, leader
and pin, and counts anchors, nearby labels, leaders, pins and labels involved in
remaining hard conflicts, plus `counts.ambiguous` for remaining uncertain
associations without a connector. `still_overlapping` counts automatic labels;
`pinned_overlapping` counts pinned labels with hard conflicts, once per label.
A conflict between an automatic label and a pin still counts against the
automatic label. Pins remain obstacles for every automatic label and leader. `with_ticks` counts straight marker-to-box
connectors shorter than 3 mm. `with_leaders` counts connectors of at least 3 mm
and all rail or routed leaders. These two counts do not overlap.
`next_to_spot` counts unconnected labels and short ticks, so it overlaps
`with_ticks`. Print shows the tick and leader counts separately.
A conflict count includes labels covering markers or
decorations, leader crossings and close approaches, leader/label intersections
and violations of the in-image leader rule. Label boxes and leaders stay inside
the page body. Impossible owned placements remain visible inside the body and
report their remaining conflicts; owned labels are never omitted.

Labels in repeated (hatched) context strips are optional because the spot is
labelled at full strength in its owning band. When a context label has no
conflict-free slot, omit its text and leader but keep its marker. The per-page
`counts.context_omitted` records these omissions; `occurrences` still includes
their markers. Omitted context does not count as overlap, and conflicts are
recounted after removal so another label is not penalised for touching a removed
box. Print shows overlap and ambiguous-label counts together, can jump to the
first page affected by either, and reports any omitted context labels. Registration and stats include `spot_labels`
only when spots are displayed.

Turn on **Move labels** to drag a label box in the preview. The overlay is inside
the usual PanZoom content and follows pan, zoom and the view zoom keys. A drag
pins the box's top-left offset from its marker centre in paper mm. This offset
follows a strip when it moves; the name remains upright. Double-click a label or
press Delete while it is focused to unpin it. Reset labels asks for confirmation
and gives the number of pins being removed. Pins can deliberately create a
conflict: the user's chosen position takes priority. The report counts it under
`pinned_overlapping`, and Print shows “1 pinned label overlaps” (or the plural).
Every other label and leader must avoid the pinned box and leader. A pin covering
a marker or decoration therefore leaves `still_overlapping` at 0 when all
automatic labels are clear; it does not hide conflicts on automatic labels.

Pins and size (`auto` or a number) are saved only under
`grainatlas_ui.print_label_pins` and `grainatlas_ui.print_label_size_mm`.
Occurrence keys use the band identity rather than its page number or position;
Whole area uses its area identity. The print fingerprint includes display style,
size, spot revision and offsets. These fields never enter `project.print_sheets`
or element sync. Server-bound atlas bundles omit both fields; local saves and
local atlas exports retain them. Editable database copies use their existing
local project/draft storage. A read-only view keeps label preferences only in that tab's memory,
consistent with the rule that read-only views write no local storage. The
existing database image workflow requires Edit a copy before print pixels are
available; this change does not download an entire database mount in a view.

The command line uses the same placer and the exact saved `grainatlas_ui.spots`:

```bash
grainatlas sheets mount-a.grainatlas --out /tmp/mount-a-print --spots --svg
grainatlas sheets mount-a.grainatlas --out /tmp/mount-a-packed --packing packed --spots --label-size 3
grainatlas sheets mount-a.grainatlas --out /tmp/mount-a-edited --edits edits.json --spots
grainatlas sheets mount-a.grainatlas --out /tmp/mount-a-whole --layout whole --spots
```

The saved pins and label size apply; `--label-size MM` overrides only the size
and requires `--spots`. The project is opened read-only, all output stays under
`--out`, and `spot_labels` appears in the registration and stats JSON. Without
`--spots` the existing output is unchanged.

Synthetic tests cover 200 sparse anchors, a crowded 60-anchor cluster, touching
grains, image edges, pins, impossible placement, escaped names, determinism,
40 isolated anchors beside a group of 12, and 300 isolated anchors beside three
groups of 15. An eight-grain row at 8 µm/mm, with `AB12_S_211`-style names,
reproduces the old corner ambiguity at 3.2 mm and requires zero ambiguous labels
and zero conflicts. Every box more than 0.8 mm from its marker must have a
connector. Other cases check exact circle, line and raster distances, hairline
SVG styling, exhaustive ambiguity and tick/leader counts, ambiguity penalties
for unconnected boxes, and Auto keeping 3.2 mm for a tick-connected pin that
formerly forced 2.9 mm through a false ambiguity. The supervisor's 12-spot,
8 mm grid fixture at 3 mm label size,
including the fitted-preview (+15 CSS px, +12 CSS px) pin drag, is checked
through `print_plan_render` with actual Lines and Packed sheet geometry and
decorations. At 1280 × 800 CSS px, the 21 rem sidebar and 24 px of layout spacing
leave a 920 px preview. Tabloid landscape is 431.8 mm wide; the 0.94 fit factor
gives 2.00278 CSS px/mm, so the drag is (+7.48959 mm, +5.99167 mm).
The pin regression also requires zero automatic conflicts after a deliberate
pin over its own marker, with exactly one `pinned_overlapping` warning.
The same synthetic `print_edit_bundle` input, all twelve server spot records,
Auto label size, Auto whole-area scale and grid are also checked through
`print_whole`.

The realistic timing page has 600 anchors: 420 spaced one grain apart (22 by
12 mm), and 180 in groups of 4 to 12 on adjacent grains. Names have 10 to 14
characters and diameters are 20 to 35 µm at 8 µm/mm. The body is 594 by 420 mm.
The explicit release test requires placement under 300 ms; the debug bound is
8 s. The original sparse 600-anchor timing still requires under 150 ms.
On the development Mac, round 9 release timings with Auto on were 147.041 ms
for 600 realistic anchors (533 ticks, 67 longer leaders, zero conflicts and zero
ambiguous labels), 15.890 ms for 600 sparse anchors, and 0.657 ms for decoration
collection with 10 MiB of base64 images. The extreme 60-anchor fallback took
135.884 ms, with one tick, 59 longer leaders, 18 reported conflicts and zero
ambiguous labels. These impossible owned placements remain visible inside the
body; the test independently reconstructs their conflict and ambiguity counts.
Round 8 timings were 265.803 ms, 21.023 ms, 0.731 ms and 325.567 ms respectively.
Round 10 release timings are 143.185 ms for 600 realistic anchors (533 ticks,
67 leaders, zero conflicts and zero ambiguity), 14.905 ms for 600 sparse anchors,
0.808 ms for decoration collection and 131.981 ms for the 60-anchor fallback
(one tick, 59 leaders, 18 conflicts and zero ambiguity). The boundary whole-area
fixture takes 274.635 ms (3 ticks, 42 leaders, zero conflicts and zero ambiguity),
and the dense sheet takes 1.577 ms (72 ticks, zero leaders or conflicts).
All existing limits and timing tests remain unchanged.
Straight trials avoid route-cache work, and impossible plates
are rejected before checking their full connector geometry.
The 300 ms realistic, 150 ms sparse and 500 ms fallback limits remain enforced.
Timings exclude image rendering and PDF export.
Image data is stripped from a temporary parsing copy before decoration
collection; the output SVG and its images are unchanged. Run the timing tests with:

```bash
cargo test -p grainatlas-core --release layout::labels::tests --lib -- --ignored --nocapture
cargo test -p grainatlas-core --release --test boundary_labels -- --ignored --nocapture
```

Browser timing, Playwright and the 10 % Print performance comparison must be run
by the supervisor because this worktree cannot bind a browser-test port.

The boundary fixture is a 280 by 26 mm synthetic image on tabloid paper at
25 µm/mm, with 45 anchors, 2 to 6 mm spacing, full names, grid text on all four
edges and stage coordinates. Tests check zero conflicts, order and spacing in
each tier, the first-tier decoration distance, and an independent search for
free positions on every closer tier. Mean centre displacement must be under
4 mm and the maximum under 15 mm. They also check vertical image exits, the
45-degree outside rule, the short in-image rule, 0.8 mm leader clearance,
decoration avoidance and tall-image side rails. Inner-tier probes include the
same 0.8 mm spacing between plates as the final rail validation.
The review helper writes standalone SVGs, PDFs and placement reports with
synthetic names only, without opening a port. Auto still chooses 2.4 mm on the thin
whole-area page (3 short ticks, 42 longer or rail leaders, zero conflicts and
zero ambiguous labels), and 3.2 mm on the grain sheet (72 adjacent labels with
short ticks, zero longer leaders, zero conflicts and zero ambiguous labels).
The whole-area page still uses the readability floor because its larger
short-slot trials do not qualify. Placement release tests took 385.498 ms and
1.672 ms respectively; both keep the 500 ms limit. The review helper including
SVG drawing took 420.234 ms and 1.886 ms. A third demo puts a repeated context
marker at the body's left edge with no clear text slot: at 3.2 mm it keeps both
markers, omits one context label and retains the owned label with one short tick,
zero conflicts and zero ambiguity (1.399 ms including drawing). The eight-grain
association row keeps 3.2 mm, all eight ticks, zero conflicts and zero ambiguity
(0.290 ms). Round 8 placement tests took 344.820 ms and 1.758 ms; its review
helper took 336.986 ms, 1.998 ms, 2.826 ms and 0.298 ms respectively.
The round 8 PDFs were rendered with Poppler and visually inspected.
Plex's PostScript name (`IBMPlexSansCond-Medium`) is checked in the exported PDF.
Every label test page asserts that label boxes and leaders stay in the body,
including the Lines, Packed and whole-area print pipelines, deliberate conflicts
and pins. The impossible-name fixture uses a body wide enough for its full name
and a hard obstacle filling that body; the former 2 mm body could not physically
contain the text. Context omission and shuffled-anchor determinism are checked
separately. Run:

```bash
cargo run --release -p grainatlas-core --example spot_label_demo -- OUT_DIR
```
