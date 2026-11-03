# ADR-0002: reuse photocraft-algo for blending, HDR and segmentation

Date: 2026-11-03. Status: accepted.

Context: panorama blending, Laplacian pyramids, Poisson editing, HDR merge and
GrabCut are classical, well-documented algorithms that storytold/photocraft
implements in pure, safe Rust that builds for wasm, under MIT OR Apache-2.0.

Decision: grainatlas depends on `photocraft-algo` (git, pinned rev). Implement here only
what is specific to SEM mounts: readers, stage-prior translation stitching with
phase correlation, background leveling, dark-background grain segmentation,
scale bars, spots and print layout.

Consequences: Rust 1.95 or newer; attribution in `ATTRIBUTION.md`; if the
upstream API moves, copy the needed function with attribution rather than
tracking an unstable branch.
