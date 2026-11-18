# Export segmented grains

In the grain panel, choose **Export grains...**, or use the desktop File menu.
Choose selected grains, the current area or the whole mount; original 16-bit or
displayed 8-bit values; PNG or TIFF; and a crop margin in µm (default 10 µm).
A mount opened from the server first loads its published images (with the
usual progress). A raw package needs every area with grains stitched, so mount
reference values include their full populations. Additional layers also need loaded
pixels. Turn off both crops and masks for a table-only export. Grain CSV remains
available for the earlier geometry-only table.

The desktop app writes a new `grain-export-<date>-<time>/` subfolder in the
chosen folder. Cancel removes partial output; if removal fails, `INCOMPLETE.txt`
marks it. The browser writes batches into a new folder where directory access
is supported, with `INCOMPLETE.txt` kept until success. Otherwise it downloads a
STORE ZIP assembled in encoded chunks through the core ZIP writer. The fallback
keeps browser Blob parts for the archive, so very large exports should use a
folder. Hosted exports follow the page's existing browser save path.

`grains.csv` has one row per grain per layer, crop and mask paths, geometry in
µm, intensity statistics on the 0..1 scale, texture and relative measures.
`outlines.geojson` contains stage/mount coordinates in µm, with null geometry
where stage information is absent. `manifest.json` contains pixel and polygon
hashes, corrections, parameters, provenance and the ordered column dictionary.
Image names use `<source area name>-<grain label>-<last 8 grain UUID hex>`
with sanitised names. Additional layers append the sanitised name of the
sampled area. Collisions append a counter starting at 2, including names differing
only in case. TIFF crops keep `.tiff`; masks use `.png`. The table keeps full
grain, source area and sampled area UUIDs.
The included `README.md` gives every definition and unit. CSV loads directly in
pandas, polars and R; Parquet and grayscale float TIFF are not included.

Original values are corrected, blended f32 mosaic intensities before tone,
including configured gain/offset, background correction (smooth charging
surfaces and beam-overlap steps when enabled), tile overrides and any
HDR merge, clamped to 0..1. They are encoded as round(value * 65535); statistics
retain the f32 precision. Displayed values include the current display LUT.
Published bundles contain only 8-bit pixels and are marked `published-8bit` in
every row and area. SEM tile identifiers stay empty for published mosaics.
Additional layers require consistent stage transforms across real tiles within
0.1 pixel and complete mask coverage. Unsupported mappings are not guessed.

The reference sets use every non-artifact grain, independent of export scope.
For mean and median separately, z = (x - reference mean) / sample sd;
robust z = (x - reference median) / (1.4826 * MAD); percentile rank uses
100 * (count below + half the count equal) / n. Area and mount measures need
at least 2 reference values, tile measures at least 3. Zero sd or MAD leaves
the corresponding cell empty. Mount sets group by metadata signal when all
areas contributing native reference grains have known signals; otherwise they
pool areas, while always
separating pixel-source kinds. Relative measures are comparable within these
reference sets only.

When modelling brightness, contrast or texture, use `area_id` as an image
random intercept and `tile_index` as a tile intercept nested within image;
include signal, acquisition date and session as covariates when available.
Use `grain_id` to account for repeated layers of a grain. These effects account
for image and tile differences without treating relative brightness as a
calibrated measurement across sessions.

A deterministic synthetic mount (two areas, four tiles each, 36 grains per
area) exercises constant, gradient, checkerboard and seeded Gaussian pixels:

```sh
cargo run -p grainatlas-core --example synthetic_grain_export -- /tmp/grain-export-example
```

The destination must not already exist. All pixels and metadata are invented.

For a scale check with 1,500 grains per area (3,000 native grains and 6,000
exported rows including additional layers):

```sh
cargo run -p grainatlas-core --example synthetic_grain_export -- /tmp/grain-export-large --large
# Override the number per area:
cargo run -p grainatlas-core --example synthetic_grain_export -- /tmp/grain-export-large-100 --large 100
```

This uses seed 42. `synthetic::large_mount(seed, grains_per_area)` accepts other
seeds. Each area is 6,000 × 6,000 px at 0.5 µm/px, with a 4 × 4 tile grid and
small tile gain/offset differences. Grain polygons have 30–60 vertices, random
orientations and lengths of 30–150 µm. Disjoint bounding boxes leave epoxy
between irregular rounded outlines. Grain pixels mix bright and dark
populations with zoning and noise; area b is 0.55 times area a plus 0.06.
The generator holds both invented f32 mosaics in memory because it has no
source package from which to rebuild them after eviction. Excessive requested
densities fail after bounded placement retries. The example reports generation,
begin/measurement, crop rasterisation/encoding/writing and metadata finish
times separately. Large-mount and raster timing tests are ignored by default:

```sh
cargo test -p grainatlas-core grain_export::tests::large_synthetic_mount_export -- --ignored
cargo test -p grainatlas-core grain_export::tests::scanline_raster_timing -- --ignored --nocapture
```
