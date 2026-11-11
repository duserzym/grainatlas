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
rounding (all four counts must be 0). An individual grain that cannot fit at the
requested fixed scale remains whole and triggers this audit; change scale or
paper before printing such a result. Samples are ordered top of the
mount first; areas retain their sample's order. Bands of different areas and
samples may share a line, each with its label.

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
to fill a partly used one. Areas and pieces retain their reading order. Empty stretches between clusters become
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
of the same area in their band), `strays` and the four audit counts.
`fill.band_area` is band image area divided by the available body area across
all sheets; `fill.line_width` is the mean sum of band image widths on a line
divided by body width. Labels, line gaps and inter-band gaps are excluded from
the numerator. The denominator sums the real body area of every sheet.
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
`other_line_mm2` (gaps and space below shorter images). Together with
`blank_below_mm` times body width, these partition the paper area. The last
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
Sheet 1 retains the full acquisition block in a 50 mm header. Subsequent sheets
use a 36 mm header with title, sheet number, counts, scale and processing,
"acquisition details on sheet 1", the samples on that sheet, the laser key
and the mount map. Text sizes are unchanged. The map occupies a lower panel
below the title and counts; its labels stay inside it. For mounts with many
sample labels, either header can grow to `19.1 + 2.7 × labels` mm, capped at
66 mm; both heights are recorded. The measured Sandbox and Grenville XL mounts,
and the letter-size Sandbox render, use 50 mm and 36 mm. No margins, footer
height or print scale change.

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

`grainatlas sheets <project.grainatlas> --out <dir> [--paper tabloid] [--scale auto|8] [--params sheet_params.json] [--svg] [--title "…"] [--lab "…"]` re-renders saved mosaics and grains without stitching or segmenting. It writes the PDF, print registration and sheet stats JSON to `--out`, plus `<package>_sheet<N>.svg` with `--svg`. JSON parameters override defaults; flags override JSON. The output directory must be outside the project folder.

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
