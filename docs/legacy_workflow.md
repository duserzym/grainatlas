# Registering legacy imaging with past LA-ICP-MS sessions

Most legacy sessions have SEM tile packages taken days before the laser
session and no laser log, coordinate export or spot list. The spot positions
survive only as hand annotations on printouts (often photographed) or as pits
in post-laser images. The workflow below turns that into spots with UUIDv7 ids
whose names equal the instrument labels of the session, so the laicpmsdb join
(`acquisition.instrument_label` = spot name) is exact.

## 1. Build a project per mount (batch)

```bash
grainatlas stitch "<package>/<area>" --out "<dir>/<Session>/<package>.grainatlas" --segment --print tabloid --sample <SAMPLE>
```

This stitches the area (stage prior, phase correlation, global solve,
photometric correction, multiband blend), finds and numbers the grains, and
writes `project.json` (with the grains), the pyramid, masks, a grain CSV and a
printable PDF of the grains. `--sample` sets `{sample}` for spot names when a
mount holds one sample; mounts with several samples keep the area name and the
sample is typed per spot.

## 2. Open it in the desktop app

Drop the `<package>.grainatlas` folder on the window (or choose it). The app
re-opens the package from the path recorded in `project.json` and restores
parameters and grains.

## 3. Register the annotated printout or post-laser image

Canvas → Overlay → Load image, then Add point pairs: click a feature on the
image, then the same feature on the mosaic; repeat for 4–6 well-spread grains.
One pair moves the image, two scale and rotate it, three or more fit an affine
transform (least squares; RMS and per-pair residuals shown). Photos of paper
printouts are close to affine when taken square-on; take pairs from all
corners.

## 4. Place the spots under the annotations

Spots tab: set the pattern to the session's label pattern, for example
`{sample}_{n}` for Qtegra run numbers (`WS2_17`), and the diameter from the
printout legend. Before each click set "Next n" to the run number written next
to the circle (it then counts up, so consecutive runs need no typing). Spots
link to the grain under them automatically. Drag to adjust, double-click the
name to correct it.

## 5. Export

Spots CSV (ids, names, grain, um, px, stage nm) and GeoJSON (grains and spots)
from the Spots tab; Save project writes the `.grainatlas` folder. Linking to
laicpmsdb is phase 8 of the plan (`POST /v1/mounts/{id}/spots` of the spot
export); until then the CSV carries the project, area and (if set) mount ids.

## Reader notes from the legacy data

- On TM4000 CL areas the tile `.txt` says `SignalName=SE`; CL is recorded only
  as `SignalType1=UVD` in the `.zag` (kept in the area metadata).
- Some areas have no `.zag` covering them, or an empty `AreaName`; the reader
  works from the tile sidecars alone and falls back to the folder name.
- Single captures (`NAME(xMAG).jpg`) have a data bar burned into the bottom
  strip; use them as overlays, not as tiles.
- Stage frames from different holder loads differ by up to a few hundred
  micrometres: register across loads with point pairs, not raw stage XY.
