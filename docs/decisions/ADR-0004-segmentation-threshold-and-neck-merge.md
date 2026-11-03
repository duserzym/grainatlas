# ADR-0004: background-relative threshold and neck test for grain segmentation

Date: 2026-11-03. Status: accepted. Amends plan section 6.5, steps 1-2.

Context: on the Sweden_XS_150X SE mosaics, an Otsu threshold over the covered
pixels falls between the bright and the dark zones inside the zircons (the
grain intensities are themselves bimodal), so grey grains were missed and
outlines followed internal zoning. The h-maxima watershed alone cut long
prisms across wherever their width varied.

Decision: the default threshold is background-relative: the epoxy level is the
histogram mode, its noise σ the half width below the mode, and the threshold is
`bg + max(6σ, 0.12 · (p99.5 − bg))` on a mosaic smoothed with σ = 1 work px.
Otsu stays available (`method: "otsu"`). After the marker-controlled watershed,
neighbouring regions are merged back unless the distance at their shared
boundary is below 0.75 times the smaller region's largest distance (a real
neck between touching grains). Both are parameters in `project.json`.

Consequences: on the five example areas the automatic result is one outline
per grain for nearly all grains (149 of about 150 in the 4 x 8 area), with
outlines about 1 work pixel (1.3 um) generous. Dark grains with bright rims
are found because holes are filled. Manual add, split and merge cover the rest.
