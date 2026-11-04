# ADR-0006: Open anything from a package down to loose files

Date: 2026-11-03. Status: accepted.

## Context

Users have whole TM4000 packages, but also a single area folder copied off the
instrument, or a handful of JPEGs from somewhere else with no sidecars. The
readers needed a `.zag` (or `positions.csv`), and a top-level area folder got
an empty area id.

## Decision

- **Three kinds**, recorded as `package.kind` in `project.json`:
  `package` (instrument folder), `area` (one `001_<name>/` folder alone, no
  `.zag`: a one-area package named after the folder, scale and stage positions
  from the `.txt` sidecars), `loose` (images without calibration).
- **Two-step open.** `Session::open_source` returns a probe: `opened` with the
  PackageInfo, or `needs_scale` with the images found. The UI then asks for one
  scale for the whole set (µm per pixel, or the field width in µm) and calls
  `Session::open_loose(LooseSpec)`. Each image is its own single-tile area, or,
  when the user says so, the tiles of one grid (columns, rows, overlap %,
  order); the stitcher refines the grid prior as for any package.
- **The spec is stored** in `package.loose` (and, on desktop, the absolute
  paths in `package.files`), so a reopened project rebuilds the same areas. The
  web build reopens the same file set with the saved scale without asking;
  "Change…" on the Area screen asks again. The last scale typed is the
  dialog's default for the rest of the session (`sessionStorage`).
- Desktop opens paths: one folder (`DirSource`) or chosen files
  (`FileListSource`); the web build hands files over as before.

## Consequences

`project.json` gains `package.kind` (default `package`, so older projects read
as before), `package.loose` and `package.files`; `schema_version` stays 1.
Loose areas have no stage positions, so stage columns in exports are empty.
