# Laser spot lists and adapters

grainatlas registers each area's mosaic to the laser stage from fiducials and
writes the area's spots in the laser's own frame and units (roadmap phase 11).

## Registration

1. Place spots as usual on the Canvas. Pick three or more of them spread over
   the area as fiducials (corners of the area work best; not on one line).
2. On the laser, drive to each fiducial and read the stage position the laser
   software shows.
3. Canvas → Laser tab: choose the stage units (mm or µm), add one row per
   fiducial, pick its spot and type the two stage values.

The core fits an affine map from mosaic micrometres to the stage by least
squares (`crates/grainatlas-core/src/laser/mod.rs`, `laser::fit`). With three
fiducials the map passes through them exactly, so a fourth is the only check
on the readings; the residual of each fiducial is shown in micrometres, and
any residual above 5 µm is flagged. The fit also reports the scale, the
rotation and whether an axis is flipped, for a sanity check against the laser.

The registration is stored per area in `project.json` under
`laser.areas.<area id>` (`units`, `adapter`, `fiducials`, `fit`).

## Generic CSV

Always available; one row per spot, line or raster of the area:

```text
name,x_stage,y_stage,x_um,y_um,diameter_um,kind,grain,sample,x2_stage,y2_stage
HJ-GC1-1-1,13.0000,40.0000,500.00,250.00,25,spot,1,HJ-GC1,,
HJ-GC1-2-2,13.2000,39.9500,700.00,300.00,15,line,,HJ-GC1,13.2600,39.9500
```

Stage values are in the laser's units (4 decimals in mm, 2 in µm); `x_um`,
`y_um` are mosaic micrometres (origin top-left, y down); `x2_stage`,
`y2_stage` are the end of a line or the opposite corner of a raster (empty for
spots). The spot name is the label the operator types into Qtegra.

## Adding the lab's laser software

When the lab names its laser control software, send an example spot or
position list exported from that software (a few points are enough), with
the software name and version. The adapter is then:

1. a unit struct implementing `LaserAdapter` in
   `crates/grainatlas-core/src/laser/` (`id`, `label`, `extension`, `write`),
   written to reproduce the example file exactly: column order, header lines,
   decimal separator, line endings, units and axis directions;
2. one line in `laser::adapters()`;
3. a test that writes known rows and compares them with the example file
   (kept under `tests/fixtures/laser/`, text only).

The UI lists every adapter in the Laser tab's "Laser software" menu, and the
choice is stored with the area's registration.
