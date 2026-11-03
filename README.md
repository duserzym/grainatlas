# grainmap

Open-source stitching, correction, segmentation, spot-placement and print-layout
tool for SEM grain-mount images (CL, BSE, SE) used to plan LA-ICP-MS sessions.
One Rust core, delivered as a static web page and as a desktop app (macOS,
Windows) from the same Svelte interface. Companion to
[laicpmsdb](../laicpmsdb) and [UPbRPy](../UPbRPy).

Status: planning (see `ARCHITECTURE_PLAN.md`). Nothing runs yet.

What it will do, in order of the roadmap:

1. read a Hitachi TM4000 Zigzag package (and plain image folders) with scale
   and stage metadata;
2. stitch tiles by stage prior plus phase-correlation drift refinement, with
   gain, offset and background leveling and multiband blending, including HDR
   merges and manual overrides;
3. segment grains, number them and register micrometre scale bars;
4. place and name laser spots on an infinite canvas with a floating metadata
   panel;
5. pack grains onto Letter, Tabloid and A-series pages for printing and export
   PDF, SVG, CSV and GeoJSON;
6. link spots to `laicpmsdb` mounts and sessions by UUID.

Code only lives here. Image data stays on the lab NAS or your laptop; the only
images in this repository are the small test fixture under `tests/fixtures/`.

License: MIT. Image algorithms reused from
[storytold/photocraft](https://github.com/storytold/photocraft) (MIT OR
Apache-2.0); see `ATTRIBUTION.md`.
