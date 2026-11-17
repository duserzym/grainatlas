# ADR-0007: Packed print sheets and a strip editor

Date: 2026-11-17. Status: accepted, built (2026-11-17; see "As built" at the
end). Revised the same day to
add the strip editor (part B), at the operator's request: "add flexibility to
have users to edit, crop, and rearrange the position of the zircon strips
before printing".

## Context

`layout::sheets::sheets()` cuts each context cluster into pieces and places them
in one pass, line by line: a piece goes on the current line if it fits the
remaining width, otherwise a new line starts, and a new sheet starts when the
line does not fit the remaining height. A line is as tall as its tallest piece.
When a band takes about two thirds of the body width, the other third of every
line stays empty, and the bottom of a sheet stays empty whenever the next band
is taller than what is left.

Measured on two zircon CL mounts from November 2026 (tabloid landscape):

| Mount | Auto scale (app default) | Fixed scale, lines | Same bands packed in 2-D |
|---|---|---|---|
| 7 areas, 315 grains, median 124 µm | 10 sheets (8 µm/mm) | 4 sheets (16 µm/mm) | 2 sheets |
| 4 areas, 524 grains, median 80 µm | 37 sheets (5 µm/mm) | 5 sheets (12.5 µm/mm) | 4 sheets |

At 12.5 µm/mm the bands of the first mount cover 1.8 sheets of paper but print on
5. The "packed" column came from a prototype outside the repository. It clipped
each band, with its label, scale bar and repeated-strip hatching, 1:1 from the
grainatlas PDF and placed it with a MaxRects packer. Nothing was scaled, rotated
or cut again, and the registration kept its mapping with new `sheet_rect_mm`.
The operator asked for this layout because the app's output was not compact
enough to carry to the laser.

No automatic arrangement knows what the operator wants next to what at the
laser, or which empty epoxy is worth paper. So the operator also gets to edit
the result by hand: move strips, crop them, and change a few things about
them before printing.

Words used below: a **strip** is what the editor moves, one block on the
paper: a label, one or more band images side by side with `//` gap marks
between them, and a note line (in the code, a run). A **band** is one image in
a strip (a piece, one `BandRecord`).

## Decision

### A. Packed placement

Add a packing mode to print sheets. Cutting stays exactly as it is, and only
placement changes.

1. **Parameter.** `SheetParams.packing: "lines" | "packed"`, serde default
   `"lines"`, so saved `sheet_params.json` files and the CLI keep today's output.
   The CLI gets `--packing lines|packed`. The Layout screen gets a select under Scale:
   "Arrangement: In lines, mount order" / "Packed, fewer sheets".
2. **Plan, then render.** Split `sheets()` into three stages:
   (a) cut: clusters → pieces → runs, unchanged, still using the line width and
   body height as the cut limits;
   (b) place: give every strip a sheet and an `(x, y)` origin, giving a
   `SheetPlan`;
   (c) render: draw pages, registration, stats and audit from a `SheetPlan`.
   `lines` places strips as the code does now; in lines mode the SVG and the
   registration must stay byte-identical to the current output. Rendering draws
   each strip at its origin instead of following a y cursor. The editor (part B)
   changes a plan between (b) and (c), so there is one renderer for automatic
   and edited sheets.
3. **Packed placement.** One block per strip. Width is `run.end - run.x`
   (label included), and height is `LABEL_MM + max band height + NOTE_MM`.
   MaxRects, bottom-left rule (lowest top edge, then leftmost), 2.5 mm gap, no
   rotation and no scaling. Page 1 and later pages use their own body heights as
   now. Try four orders and keep the one with the fewest sheets, ties going to the
   earlier order: mount order filling sheets in turn (no back-fill); mount order
   with back-fill; tallest first; widest first. The placement is deterministic.
   The packer accepts rectangles that are already taken, which the editor's
   "Re-pack the others" uses. No dashed separators between lines in packed mode.
4. **Why the layout rules still hold.** Packing moves whole strips and never
   changes what is in them, so properties 1 to 11 of "Layout rule: context
   first" (`docs/print_sheets.md`) are untouched: the bands, cuts, repeated
   strips, owners and strays are the same as in lines mode. A strip's bands stay
   together with their gap marks. The rule now applies to every arrangement of
   the same cut. Paper still ranks last; packing only uses paper that lines mode
   left empty.
5. **Registration and stats.** Same schema. `layout` becomes `"packed"` or
   `"lines"` (`"edited"` after part B edits), and `sheet_rect_mm` is the placed
   position, so the mapping is unchanged. `fill.line_width` is reported for
   lines mode only. `height_budget` gets a `blank_mm2` per sheet for the other
   arrangements. The audit adds `bands_overlapping`, which must be 0, next to
   `bands_outside_body`.
6. **Header and map.** They are unchanged in this round. "On this sheet", the
   mount map (dark areas, band boxes) and the spot key are already computed per
   sheet from the strips placed there, so they follow moves and edits.

### B. Strip editor

After Make print sheets, the page preview gets an **Edit strips** mode. The
operator works on the plan, and the core renders the pages from the edited
plan, so the header, mount map, registration and audit always match what is
printed.

7. **What can be done to a strip.**
   - **Move:** drag within a page, or drop on another page's thumbnail (it goes
     in the first free place there, top left first). Arrow keys nudge 0.5 mm,
     Shift+arrow 5 mm. Moves snap to the body edges and to a 2.5 mm gap from
     other strips; holding Alt turns snapping off. Pages can be added, emptied
     pages removed, and pages reordered.
   - **Crop:** handles on each band's four edges. A crop changes the band's
     `mosaic_rect_um` and can shrink or grow it, up to the area image. An edge
     never stops inside a grain: while dragging, it snaps to the nearest position
     that crosses no printed grain's box, and the grains that would leave the
     band turn red. Grains that leave are listed as left off by the operator
     (point 9). Grains that come in and are owned by another band are drawn and
     listed as context, as in repeated strips. Repeated strips are recomputed
     from the new rectangles.
   - **Edit:** change the label text; hide a strip (its grains are left off by
     the operator) and show it again; split a band at a line the operator places,
     which snaps to a gap between grains and goes through the existing cut code
     with its repeated strips; join two bands that came from a split; separate a
     strip into one strip per band, each with its own label and a "continues on
     sheet N" note in place of the `//` mark.
   - **Re-pack the others:** strips the operator has touched are pinned; the
     rest are packed again around them (point 3). **Back to automatic** drops
     every edit, after a confirmation that says how many edits will go.
   - Undo and redo over the whole edit list (Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z).
     "Reset layout" already names the window layout action (UX rule 7), so the
     editor never uses that name.
8. **What cannot be done, and why.** No rotation and no scaling of single
   strips: the registration maps every band with one `um_per_mm` and no
   rotation, and the scale bar and spot key hold for the whole sheet. Grains are
   never cut: no crop edge or split line may cross a printed grain. Rotation is
   left for a later decision, as it needs a rotation in the mapping.
9. **Rules the editor keeps.** The core checks every edited plan before it
   renders it. Export PDF, Export SVG, Print… and Registration stay disabled
   while any strip overlaps another, sticks out of the sheet body, or any
   printed grain is cut by a band edge or printed twice. The panel lists each
   problem with a button that selects the strip. Grains that leave the sheets
   through a crop or a hidden strip are not errors. They are listed in the
   stats line ("12 grains left off by crop, 30 in hidden strips") and in the
   registration as `left_off` (the `StrayGrain` fields plus `reason`:
   `"cropped"` or `"hidden"`), next to the existing `strays`, so every grain of
   the mount is still accounted for. The registration gets an `edits` summary
   (counts of moves, crops, splits, joins, label changes, hidden strips) and
   `layout: "edited"`. The mapping per band is unchanged.
10. **Where edits are kept.** The project keeps the edit list in `project.json`
    under `print_sheets` (an `extra` key today, typed when built): the sheet
    parameters it was made with and an ordered list of edits. Each edit names its
    strip or band by a stable key: the area id plus a hash of the sorted ids of
    the grains it owns, plus a counter for bands made by a split. When the
    operator makes print sheets again, the stored edits are replayed if the
    parameters match. Edits whose strip no longer exists (the scale, the grains
    or the context distance changed) are not applied, and the panel says how
    many were dropped and why, before anything is saved. The edits are saved
    with the project in the usual way; they are not a separate file.
11. **Editor view.** The page view follows the gesture module (UX rule 2):
    wheel zoom, trackpad pan, fit on double-click on empty space. In edit mode a
    drag on a strip moves it, and a drag on empty paper pans. Band images come
    from the plan (the PNGs the renderer already makes). While cropping, the old
    image is clipped or shown on grey for the new area, and the core
    renders the band again when the drag ends. The header area is drawn but
    cannot take strips. Page thumbnails run down the left of the page view and
    accept drops. The controls column folds as the other panels do. Checked at
    1440 x 900 and 1280 x 800 (UX rule 7, `ui/tests/sizes.spec.ts`).
12. **Core API.** `print_sheets` returns the plan with the pages. Two calls
    are added on both hosts (Tauri commands and the wasm worker):
    `print_plan_render(plan, edits) -> SheetResult` (pages, registration,
    stats, audit, problems) and `print_band_image(area, mosaic_rect_um)`. The
    CLI gets `grainatlas sheets --edits FILE` so an edited set can be rendered
    again without the app.

## Order of work

Each step ends green and is committed and pushed before the next starts.

1. **Restore the layout property tests first.** The synthetic tests for
   properties 1 to 11 (`crates/grainatlas-core/tests/sheets.rs`, 59 tests, no
   images) left the repository with the sample suite on 2026-11-15; a copy is in
   `~/Github/lab-builds/grainatlas-private/grainatlas-tests-15f362c.tar.gz`. Bring
   back the ones that need no sample files, with every sample, area and place
   name neutralised (AGENTS.md rule 2). Keep the invented
   `mount-inset-areas.json` fixture only if it holds no real names. They must pass
   on the current code before step 2.
2. Refactor `sheets()` into cut, place and render without changing output. A
   golden test on a synthetic mount compares the registration and a hash of each
   SVG page before and after.
3. Add packed placement, the parameter, the CLI flag and the audit field. New tests:
   packed and lines give the same bands, grains, `mosaic_rect_um`, repeated
   strips, context and strays, and differ only in `sheet` and `sheet_rect_mm`;
   packed sheets ≤ lines sheets; no overlaps; every band inside its body;
   identical output on a second run.
4. Packed in the UI: the Arrangement select, types in `ui/src/lib/core/types.ts`,
   the stats line naming the arrangement.
5. Edits in the core: the edit types, validation, `left_off`, the `edits`
   summary, replay by stable key, `--edits` on the CLI, the two new host calls.
   Tests: a move changes only `sheet` and `sheet_rect_mm`; a crop that would cut
   a grain is refused; a shrinking crop lists exactly the grains it drops; a
   growing crop adds context and never a second owner; split then join gives
   back the original band; hide then show gives back the original plan; an
   overlap or an out-of-body strip makes `problems` non-empty; replay after a
   scale change drops the edits and reports them; the same edits give
   byte-identical output.
6. The editor UI (part B points 7, 9 and 11), with Playwright tests for drag,
   nudge, drop on another page, crop with a grain turning red, export disabled
   while strips overlap, undo and redo, and the sizes spec at 1440 x 900 and
   1280 x 800. Codex cannot bind ports: the supervisor runs Playwright.
7. Saving edits in the project (point 10), with a test that a saved and
   reopened project replays its edits.
8. Docs: a "Layout rule: packing" and an "Editing strips" section in
   `docs/print_sheets.md`, and a line in the README's print step.
9. `make full`. The supervisor renders both modes from the two November mounts
   outside the repository, compares sheet counts with the table above,
   rasterises every page, then makes a few edits by hand in the app (a move to
   another sheet, a crop, a split) and checks the printed rectangles against the
   registration.

## Not in this round

- Cuts that know about packing, for example cutting a piece to fill a leftover
  rectangle instead of the line end. (The operator can do this by hand with
  split.)
- Rotating or scaling single strips (point 8).
- Making `packed` the default. That decision waits until packed sheets have
  been used at the laser.

## As built

Built in steps 1 to 8 as ordered above (commits cb2467e to 8331bef). Decisions
taken while building, within this plan:

- Lines output stays byte-identical, so a lines registration keeps `layout`
  empty (omitted); packed writes `"packed"`, an edited plan `"edited"`. Packed
  and edited registrations also list `strip_blocks`.
- Every strip in a packed or edited plan carries its sample and area label. A
  lines-mode run that shared the label of the run before it would otherwise
  print unnamed on another sheet.
- Ownership after edits: a band keeps the owners still wholly inside it; a grain
  that loses its band is adopted by another shown band of its area that holds it
  whole, otherwise it is listed in `left_off` (cropped or hidden). Split
  children stay side by side in their strip; Separate makes one strip each.
- `print_sheets` in project.json also stores the resolved scale, so a changed
  auto scale drops the stored edits. A stored list this version cannot read is
  kept verbatim.

Measured on the two November mounts at the scales in the table: lines 4 and 5
sheets (registrations identical to version 0.1.2), packed 3 and 5. The
prototype's 2 and 4 come from its shorter headers (about 39 and 23 mm against
50 and 36 mm) and from leaving out the 4.2 mm note line: the app's own strips
re-packed with both changes give 2 and 4. The compact-header follow-up is
recorded below. A hand edit in the app (a crop, a split separated into two
strips, a piece moved to another sheet) gave a registration whose rectangles
match the printed pages.

## Follow-up (2026-11-17)

Built compact headers for packed and edited plans: 40 mm on sheet 1 and
25 mm on later sheets, with additional height when wrapped titles, acquisition
values or the spot key need it. Sheet 1 keeps its acquisition content. Later sheets
keep the title, sheet number, print line, samples and areas, spot key and mount
map. Compact map labels are abbreviated and limited to the space available,
with text at least 1.8 mm high. Packed and edited strips reserve the 4.2 mm
note line only when they have a continuation note; Separate restores that
space and the editor reports any resulting overlap. Cutting retains its
original header and note limits. Lines output remains byte-identical. The
synthetic golden mount uses 2 packed sheets before and after this follow-up.
The two November mounts at the scales in the table now pack into 2 and 4
sheets (3 and 5 before), the same as the prototype; their lines output is
unchanged apart from the print time.
