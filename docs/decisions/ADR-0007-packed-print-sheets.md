# ADR-0007: Packed print sheets

Date: 2026-11-17. Status: proposed (plan, not built).

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

## Decision

Add a packing mode to print sheets. Cutting stays exactly as it is, and only
placement changes.

1. **Parameter.** `SheetParams.packing: "lines" | "packed"`, serde default
   `"lines"`, so saved `sheet_params.json` files and the CLI keep today's output.
   The CLI gets `--packing lines|packed`. The Layout screen gets a select under Scale:
   "Arrangement: In lines, mount order" / "Packed, fewer sheets".
2. **Two stages in `sheets()`.** Split the current loop into
   (a) cut: clusters → pieces → runs, unchanged, still using the line width and body
   height as the cut limits; and (b) place: give every run a sheet and an `(x, y)`
   origin. `lines` places runs as the code does now. Rendering then draws each run
   at its origin instead of following a y cursor. In lines mode the SVG and the
   registration must stay byte-identical to the current output.
3. **Packed placement.** One block per run: its label, its pieces side by side
   with their `//` gap marks, and the note line. Width is `run.end - run.x`
   (label included), and height is `LABEL_MM + max piece height + NOTE_MM`.
   MaxRects, bottom-left rule (lowest top edge, then leftmost), 2.5 mm gap, no
   rotation and no scaling. Page 1 and later pages use their own body heights as
   now. Try four orders and keep the one with the fewest sheets, ties going to the
   earlier order: mount order filling sheets in turn (no back-fill); mount order
   with back-fill; tallest first; widest first. The placement is deterministic.
   No dashed separators between lines in packed mode.
4. **Why the layout rules still hold.** Packing moves whole runs and never
   changes what is in them, so properties 1 to 11 of "Layout rule: context
   first" (`docs/print_sheets.md`) are untouched: the bands, cuts, repeated
   strips, owners and strays are the same as in lines mode. A run's pieces stay
   together with their gap marks. The rule now applies to every arrangement of
   the same cut. Paper still ranks last; packing only uses paper that lines mode
   left empty.
5. **Registration and stats.** Same schema. `layout` becomes `"packed"` or
   `"lines"`, and `sheet_rect_mm` is the packed position, so the mapping is unchanged.
   `fill.line_width` is reported for lines mode only. `height_budget` gets a
   `blank_mm2` per sheet for packed mode. The audit adds `bands_overlapping`,
   which must be 0, next to `bands_outside_body`.
6. **Header and map.** They are unchanged in this round. "On this sheet", the mount map
   (dark areas, band boxes) and the spot key are already computed per sheet from
   the runs placed there.

## Order of work

1. **Restore the layout property tests first.** The synthetic tests for
   properties 1 to 11 (`crates/grainatlas-core/tests/sheets.rs`, 59 tests, no
   images) left the repository with the sample suite on 2026-11-15; a copy is in
   `~/Github/lab-builds/grainatlas-private/grainatlas-tests-15f362c.tar.gz`. Bring
   back the ones that need no sample files, with every sample, area and place
   name neutralised (AGENTS.md rule 2). Keep the invented
   `mount-inset-areas.json` fixture only if it holds no real names. They must pass
   on the current code before step 2.
2. Refactor `sheets()` into cut and place without changing output. A golden test
   on a synthetic mount compares the registration and a hash of each SVG page
   before and after.
3. Add packed placement, the parameter, the CLI flag and the audit field. New tests:
   packed and lines give the same bands, grains, `mosaic_rect_um`, repeated
   strips, context and strays, and differ only in `sheet` and `sheet_rect_mm`;
   packed sheets ≤ lines sheets; no overlaps; every band inside its body;
   identical output on a second run.
4. UI select, types in `ui/src/lib/core/types.ts`, the stats line naming the
   arrangement. Check at 1440 x 900 and 1280 x 800.
5. Docs: a "Layout rule: packing" section in `docs/print_sheets.md` and a line in
   the README's print step.
6. `make full`. The supervisor renders both modes from the two November mounts
   outside the repository, compares sheet counts with the table above and
   rasterises every page.

## Not in this round

- Cuts that know about packing, for example cutting a piece to fill a leftover
  rectangle instead of the line end.
- A shorter header on sheets 2 and later (38 mm now; the prototype used 23 mm).
- Making `packed` the default. That decision waits until packed sheets have
  been used at the laser.
