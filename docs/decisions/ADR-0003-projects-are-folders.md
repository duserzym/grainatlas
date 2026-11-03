# ADR-0003: a project is a folder with a versioned JSON manifest

Date: 2026-11-03. Status: accepted.

Context: image data must stay out of git and off any server; users work from
laptops and the lab NAS; results must be reproducible from the source tiles.

Decision: `<name>.grainatlas/` holds `project.json` (schema versioned), pyramid
tiles, masks, spots and layouts. Source tiles are referenced by path and
SHA-256, never copied. Every automatic step stores its parameters; manual
edits are stored as parameters, not baked pixels.

Consequences: a project can be re-linked to moved tiles by hash, re-run
deterministically, and later ingested by laicpmsdb as attachments without
transformation.
