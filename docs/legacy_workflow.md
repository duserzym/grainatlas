# Importing a legacy lab visit

Most legacy sessions have SEM tile packages taken days before the laser
session and no laser log, coordinate export or spot list. The spot positions
survive only as hand annotations on printouts (often photographed) or as pits
in post-laser images. This workflow keeps the original files, builds a project
per package and turns annotations into spots with UUIDv7 ids. Spot names must
equal the session's instrument labels so the laicpmsdb join
(`acquisition.instrument_label` = spot name) is exact.

Keep visit data outside this repository. Build the CLI with
`cargo build --release -p grainatlas-cli`; the commands below assume
`target/release/grainatlas` is on `PATH`.

## 1. Copy and verify the visit

Start with [scripts/legacy/rules.example.json](../scripts/legacy/rules.example.json).
It uses fake paths; edit a separate rules file for the archive being imported.
The destination is `Lab_visits/<year>_<lab>/`:

```text
Lab_visits/<year>_<lab>/
  SEM/                         original packages, one direct folder per package
  SEM_grainatlas/              generated .grainatlas projects
  SEM_print/annotated_photos/  photographs of annotated printouts
  mount_photos/               optical and navigation images
  TIMS_targets/               target images and picking records
  LAICPMS_data/               canonical session exports and per-spot folders
  MANIFEST.csv
```

The copy tool creates destination folders that contain copied files. Create
empty categories yourself if needed; `grainatlas batch` creates the projects.
Run the plan from a scratch directory outside the archive and this repository:

```bash
python3 /path/to/grainatlas/scripts/legacy/copy_visit.py /path/to/visit_rules.json
python3 /path/to/grainatlas/scripts/legacy/copy_visit.py /path/to/visit_rules.json --execute
```

Without `--execute`, it writes only `plan_<visit>.csv` in the current directory.
Review the plan before copying. With `--execute`, it copies through temporary
files, checks SHA-256 before publishing each copy, writes `MANIFEST.csv` in each
visit and verifies every destination. A re-run checks matching files without
copying them or adding an “already present” note to the manifest. Different
existing bytes are refused; other files continue and the command exits non-zero.
`--visit KEY` selects one configured visit. `--manifest-only --execute` verifies
existing destination files and writes manifests without copying data. Without
`--execute`, `--manifest-only` still only writes a plan.

Rules are JSON. Relative root paths are resolved against the rules file:

| Field | Meaning |
| --- | --- |
| `source_root` | Read-only archive root. |
| `visits` | Map of arbitrary visit keys to destination folders. |
| `forbidden_write_roots` | Additional protected roots, including through symlinks. |
| `skip_names` | Exact file names ignored everywhere, such as `Thumbs.db`. |
| `known_samples` | Sample tokens inferred from matched file names when a rule has no samples. |
| `coverage_roots` | Source directories checked recursively; every non-skipped file must be claimed by a rule, even when selecting one visit. |
| `rules` | Ordered rules; the first rule claiming a source path wins. |

Each rule requires `visit`, `src` relative to `source_root`, and `dst` relative
to the visit destination. A file rule copies one file to `dst`. A rule with
`match` selects files directly inside `src` using a case-sensitive filename
glob and appends each filename to `dst`. A rule with `package: true` copies a
whole tree, preserving relative paths. Its optional `exclude_dirs` excludes
only named direct subfolders; give nested packages their own earlier rule.
Directory symlinks in package trees are refused. Source files resolving outside
`source_root`, escaping destinations, protected writes and destination
collisions are refused.

Optional `kind`, `mount`, `samples` (semicolon-separated) and `notes` are copied
to manifest columns. Use `dst: null` to record an intentional omission; optional
`superseded_by_dir` identifies a replacement by filename. Non-package files with
the same bytes as an earlier destination are listed with `duplicate_of` and no
second copy. Package files stay together even when their bytes repeat.

The manifest columns are `source_path`, `destination_path`, `sha256`, `kind`,
`mount`, `samples`, `date`, `notes`, and `duplicate_of`. Dates come first from
TM4000 sidecars, then the package's earliest sidecar date, then EXIF (2020 or
later), then file time with a note. Empty sidecar values stay empty. CL evidence
from `.zag` and sidecar signal/magnification are retained as notes. The tool
uses plain Python 3 and the standard library; optional Pillow enables EXIF dates.
Its synthetic self-test is `python3 scripts/legacy/test_copy_visit.py`.

## 2. Build a project per package

```bash
grainatlas batch "Lab_visits/<year>_<lab>/SEM" \
  --out "Lab_visits/<year>_<lab>/SEM_grainatlas" --segment \
  --sample Mount_A=SAMPLE_A --sample Child_package=SAMPLE_B \
  --atlas-dir "/path/to/atlas_bundles" > /path/to/batch_report.json
```

Every direct subfolder that opens as a package uses the same stitching path as
`grainatlas stitch`: stage prior, phase correlation, global solve, photometric
correction and multiband blend. `--segment` finds and numbers grains and writes
masks and grain CSVs. Each result is `SEM_grainatlas/<folder>.grainatlas/`, with
`project.json` (including grains), the pyramid and masks. `--sample PACKAGE=SAMPLE`
sets `{sample}` for every area of that package when a mount holds one sample.
Mounts with several samples keep the area name and the sample is typed per spot.
`--polarity bright|dark|both` controls segmentation; bright is the default.

The command prints one JSON array to stdout; progress goes to stderr. Each row
has `package` and `status`: `ok`, `failed`, `skipped` (cannot open), or `existing`
(with `--skip-existing`). Successful rows include the project path, UUID and
`areas`, whose entries include `area`, `tiles`, `pairs`, `pairs_measured`,
`rms_px`, `max_px`, and `grains` (null without segmentation). Skipped or failed
rows include an error. A package failure does not stop later packages and makes
the final exit code non-zero. Folders that do not open are reported and skipped.
A failed package can leave partial files; fix its input and re-run without
`--skip-existing` to retry it.

`--skip-existing` leaves an existing project folder and its atlas untouched.
Without it, batch follows stitch's existing-project behaviour. `--atlas-dir`
writes `<folder>.zip` using the same defaults, segmentation, Auto tone and
per-grain normalisation as `grainatlas atlas`, with its project id pinned to
the saved project's UUID. This atlas is regenerated from the package; it does
not export later manual edits in a saved project.

For a single area or a printable PDF, the existing command remains available:

```bash
grainatlas stitch "<package>/<area>" --out "<dir>/<package>.grainatlas" --segment --print tabloid --sample <SAMPLE>
```

This also writes a printable PDF of the grains. Keep original annotated
printouts separate from these generated PDFs.

## 3. Check the mosaics by eye

```bash
grainatlas render "Lab_visits/<year>_<lab>/SEM_grainatlas/Mount_A.grainatlas" \
  --out /path/to/Mount_A_overview.jpg --max-width 1800
```

Render stacks every saved area, using the widest pyramid level no wider than
the requested bound. If even the smallest level is too wide, it is reduced
further. Orange outlines come from the 16-bit labels in
`masks/<area>/grains.png`. Each header gives the area id, tile count, grain
count, full mosaic size in px and um/px. Unsegmented areas show `?` for grains.
PNG and JPEG outputs are supported; the default width bound is 1800 px.
Inspect joins, missing tiles and grain outlines before placing spots.

## 4. Open the project in the desktop app

Drop the `<package>.grainatlas` folder on the window (or choose it). The app
re-opens the package from the path recorded in `project.json` and restores
parameters and grains.

## 5. Register the annotated printout or post-laser image

To list a printout photo and initialise an overlay before opening the app:

```bash
grainatlas overlay "Lab_visits/<year>_<lab>/SEM_grainatlas/Mount_A.grainatlas" \
  --area 001_Area --image "Lab_visits/<year>_<lab>/SEM_print/annotated_photos/photo.jpg" \
  --note "Upper half; title matches Mount A"
```

The area id must exist in the project. The command reads EXIF-oriented image
dimensions and places the whole image over the mosaic at the initial scale,
with opacity 0.6. It writes `project.json` atomically and preserves all other
JSON fields. A different photo already attached to that area stays in place;
reloading the same photo and dimensions keeps its registration. Every call
appends `{photo, file, area, note}` to `annotated_photos.json` under `photos`,
keeping the image path exactly as supplied. Existing plain-list listings are
also supported. Image bytes are not embedded; the overlay URL is null, so
choose the file again in the app to display it. This records a reference; it
does not register features or place spots.

Canvas → Overlay → Load image, then Add point pairs: click a feature on the
image, then the same feature on the mosaic; repeat for 4–6 well-spread grains.
One pair moves the image, two scale and rotate it, three or more fit an affine
transform (least squares; RMS and per-pair residuals shown). Photos of paper
printouts are close to affine when taken square-on; take pairs from all
corners. For a second listed photo of an area, load it in the Overlay tab when
needed.

## 6. Place the spots under the annotations

Spots tab: set the pattern to the session's label pattern, for example
`{sample}_{n}` for Qtegra run numbers (`WS2_17`), and the diameter from the
printout legend. Before each click set "Next n" to the run number written next
to the circle (it then counts up, so consecutive runs need no typing). Spots
link to the grain under them automatically. Drag to adjust, double-click the
name to correct it.

## 7. Export

Spots CSV (ids, names, grain, um, px, stage nm) and GeoJSON (grains and spots)
come from the Spots tab; Save project writes the `.grainatlas` folder. The CSV
carries the project, area and (if set) mount ids. Export an updated atlas from
the app after manual grain or spot edits, then publish that version.

## 8. Register and ingest in laicpmsdb

Run these on the laicpmsdb side, using its configured storage and database.
Consult [laicpmsdb docs/workflow_api.md](../../laicpmsdb/docs/workflow_api.md)
for API details and that checkout's command help for arguments:

- `manage.py register_mounts`: register the visit's mounts and retain their ids; see [workflow_api.md](../../laicpmsdb/docs/workflow_api.md).
- `manage.py publish_atlas`: publish each atlas bundle against its mount; see [workflow_api.md](../../laicpmsdb/docs/workflow_api.md).

`grainatlas atlas` re-stitches a package with defaults and finds new grains, so its bundle
does not match a project that was edited in the app. For a saved project use
`grainatlas atlas-project <name>.grainatlas --out <name>.zip [--mount-id <mount>]`: it renders
every area from the saved placement, parameters and tone and carries the project's own ids,
grains, spots and labels, as the app's Publish does. It only reads the project folder, so the
project's `laicpmsdb.published` list is not updated; publishing once from the app records it.
- `manage.py attach_files`: attach source images, annotated printouts and other visit records; see [workflow_api.md](../../laicpmsdb/docs/workflow_api.md).
- `manage.py ingest_session`: ingest the canonical Qtegra session export or per-spot folder, with labels matching the placed spots; see [workflow_api.md](../../laicpmsdb/docs/workflow_api.md).

## Reader notes from the legacy data

- On TM4000 CL areas the tile `.txt` says `SignalName=SE`; CL is recorded only
  as `SignalType1=UVD` in the `.zag` (kept in the area metadata).
- Some areas have no `.zag` covering them; the reader works from the tile
  sidecars alone.
- A nested folder with its own `.zag` is a separate package unless the parent
  `.zag` names that folder as an area.
- An empty or missing `AreaName` falls back to the folder name without its
  numeric prefix. If that is empty too, the reader uses the package name,
  adding ` 2`, ` 3` for repeated fallbacks.
- Single captures (`NAME(xMAG).jpg`) have a data bar burned into the bottom
  strip; use them as overlays, not as tiles.
- Stage frames from different holder loads differ by up to a few hundred
  micrometres: register across loads with point pairs, not raw stage XY.
