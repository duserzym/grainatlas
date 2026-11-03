# ADR-0001: one Rust core compiled to native and wasm, one Svelte UI

Date: 2026-11-03. Status: accepted.

Context: the tool must be fast on 40-megapixel mosaics, run as a web page
without a server, and ship as a desktop app on macOS and Windows, with
identical results everywhere.

Decision: all image algorithms live in `grainmap-core` (Rust) compiled to
`wasm32` for the browser and natively for Tauri and the CLI. The Svelte UI
talks to a `Core` interface with a wasm-worker and a Tauri implementation.
No Python in the product; Python may be used for one-off analysis scripts.

Consequences: a single algorithm implementation and test suite; browser builds
are single-threaded until wasm threads are enabled; Tauri builds get rayon.
