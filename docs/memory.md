# Session memory

The base mosaic and pyramid level 0 share one `Arc<Vec<u8>>`. Tone changes
replace both references together. Other pyramid levels remain resident, so
viewing, PNG export, print layout and atlas export need no package reads.

`Session::set_memory_budget(bytes)` sets one budget for linear pixels,
coverage, segmentation work images and downsampled preview inputs. Defaults
are 512 MiB on native targets and 192 MiB on wasm. Areas are evicted in order
of their last heavy-buffer, work-image or preview use. Eviction drops image
allocations, retaining mosaic dimensions, base, pyramid, placement and
mosaic metadata. Coverage remains `f32`.

The current operation is protected. Its temporary allocations can exceed
the budget. A completed linear mosaic larger than the budget is immediately
evicted. A current segmentation work image larger than the budget is retained
for editing, with other areas and its own linear/preview buffers evicted.
Changing the budget can also evict that work image. Downsampled inputs retain
only one preview factor per area and share the same area LRU.

On desktop, the window also shares one 512 MiB budget across all tab sessions.
After a command that can retain evictable buffers finishes, including a failed
command, the shell counts linear, coverage, work and preview inputs across
tabs. It trims other tabs in order of their session's last heavy/work/preview
use, using the same area eviction path as the per-session budget. The caller's
tab is protected for that pass; each session's own budget is unchanged.
`Session::evictable_bytes()`, `evict_to(bytes)` and `last_heavy_use()` expose
this accounting, eviction and a monotonic stamp comparable across sessions.
Eviction does not change the stamp or the configured per-session budget.

Eviction passes release the caller's session lock first and serialize only
with other eviction passes. They use `try_lock` for tab sessions and never
wait for a running command. A busy tab contributes its last completed
allocation count; its temporary allocations are unknown until completion.
An allocating command publishes its new count and triggers another pass on
completion; other busy tabs are retried on the next such pass. Busy tabs and
an oversized protected tab can temporarily leave the window over budget.
Base images, pyramids and metadata remain resident in every tab.

The shared window budget is desktop-only. Browser tabs keep the existing
192 MiB per-session budget: each worker owns a separate wasm memory, and
freeing Rust buffers does not shrink that memory's high-water allocation.
The wasm worker and its budget policy are unchanged.

A heavy-buffer miss reads the package again and runs HDR grouping,
photometric correction and blending with the stored placement and parameters.
It does not re-stitch, re-tone, change the resident base, or mutate the saved
project. This trades repeat processing for bounded residency. On the small
fixture, a heavy rebuild took 82 to 86 ms, and rebuild plus CLAHE re-tone took
112 ms in a local optimized native test run. This is not a desktop-app or
large-project benchmark. Decoded source tiles are dropped before correction
and blending during a rebuild to reduce its temporary allocations.

`Session::memory_report()` returns `areas`, `total` and `budget`. Each area's
`base`, `pyramid`, `linear`, `coverage`, `work`, `small` and `total` are bytes
of resident image allocations, using vector capacities. `pyramid` excludes
level 0, which is counted once as `base`. Source files, metadata, allocator
fragmentation and operation scratch buffers are outside this report.
The Tauri command and wasm worker operation are named `memory_report`;
`Core.memoryReport()` wraps both. The worker adds `wasmBytes`, read from the
current wasm memory buffer. It is a high-water allocation and does not shrink
when Rust frees buffers.

Canvas caches have separate LRU budgets: 96 MiB for decoded bitmaps
(`width * height * 4`) and 32 MiB for raw grey pixels (`width * height`).
LUT changes release decoded bitmaps and remap retained grey pixels. Coarser
fallbacks inspect cached bitmaps without requesting more levels.

All wasm workers receive the same page-level compiled module. They still
have separate wasm memories and sessions. Cancellation terminates and
replaces the worker using the same module. Closing a tab calls `dispose()`,
which terminates its worker. Unused initialization promises have rejection handlers
so a cancelled module or wasm fetch during a hosted-page reload does not raise
an unhandled page error. Callers that await initialization still receive the
original failure. Browser regressions abort each fetch independently.

## Idle work

The timer audit covered `ui/src` and the Tauri shell source. Viewer rendering
queues a single animation frame in response to changes or tile arrivals; its
callback clears the pending frame and does not re-arm it. Canvas contrast
sampling behaves the same way. The live-render debounce and ownership timer
run after edits. `liveIdle` waits only while an explicitly requested render
is pending. Flash, download URL cleanup, print and version-chip timers are
one-shot. The updater's initial check and six-hour interval are intentional.
No recurring application render or polling loop runs at idle. This is a
source audit; desktop CPU and browser idle behavior still need runtime checks.

## Regression checks

`crates/grainatlas-core/tests/memory.rs` uses a 1 KiB budget on the fixture,
compares it with an unlimited run, and checks the golden hash, all pyramid
pixels, PNGs, histograms, previews, Auto, per-grain tone, segmentation geometry,
project files including masks, and the atlas zip. Rebuilt linear and coverage
values are also compared bit-for-bit before and after a correction change.
Synthetic areas exercise heavy-buffer LRU order. Explicit session eviction
checks all four evictable buffer classes, partial eviction, the last-use stamp,
base/pyramid identity, hash, PNG, histogram, placement, project JSON and re-tone.
Session unit tests cover the shared work/preview LRU. Desktop Rust tests use
three fixture sessions to check window LRU order, caller protection, unchanged
session budgets, failed operations, busy-tab accounting and concurrent command
completions.
`ui/unit/tiles.test.ts` checks byte budgets, LRU order, LUT remapping, generation
guards and cache-only fallback.

`ui/tests/memory.spec.ts` opens the fixture in the actual UI worker, stitches
all areas and visits each Canvas area at several zooms. Its comments derive
bounds for resident images and the wasm high-water allocation. A second test
checks shared module identity across tabs and cancellation, one wasm request,
and worker termination on cancellation and tab close. The built worker, run
under Node with a supplied compiled module, reported 55,739,068 resident image
bytes and a 263,127,040-byte wasm buffer on the fixture.

The completed audit passed `make test`: native tests, 23 UI unit tests,
native/wasm parity and 59 Chromium browser tests. Eleven focused WebKit tests
covering memory, window sizes and shell focus also passed. The macOS release
app bundle built successfully. The native real-project measurement below uses
the supported project path and `--stitch` startup arguments with isolated settings.

## Large project measurement

On 2026-11-06, an isolated Chromium browser opened a scratch copy of
Grenville_S_150X, stitched all 13 areas (224 Mpx), visited every area and settled
for 30 seconds. The measured total process-tree physical footprint was
1,999 MB after opening, 2,048 MB after visiting the areas and 1,996 MB settled.
The worker reported 436,001,752 resident image bytes and a 1,853,423,616-byte
wasm buffer. The 192 MiB budget applies to evictable buffers; base images,
pyramids, operation scratch space and the wasm high-water allocation remain
outside it.

The earlier measurement was 3,990 MB for the renderer after Stitch all.
The new measurement includes the browser process tree, so this is evidence
of a substantial reduction rather than a controlled same-process comparison.
The measured open-and-stitch time rose from the earlier 61.6 seconds to
91.0 seconds. The bounded caches trade repeated processing for lower resident allocations.

The rebuilt native app opened the same scratch project with all areas stitched.
Physical footprint for the app and its new WebKit helpers was 1,651.9 MiB at
61.5 seconds and 1,289.0 MiB at 141.7 seconds. Both 20- and 30-second idle CPU
samples measured 0.0 percent. The earlier native settled baseline was about
2,990 MiB, making this a substantial reduction on the same project. This was
a short idle measurement, not the originally proposed ten-minute soak.

## Desktop window budget measurement, 2026-11-10

Release builds on an M4 Pro (24 GB), scratch copies of the projects, isolated
settings. "Before" is cb0dd7f, "after" adds the shared window budget.
Footprint is macOS physical footprint (decimal MB) of the app and the WebKit
helpers it started, settled after `--stitch` finished.

Where the memory goes, from a native probe that opens saved projects through
the same path as `open_project`, stitches every area with its saved
parameters and reads `Session::memory_report()` (before the change):

| projects open (one session each) | report total | heavy (linear + coverage) | process footprint |
| --- | --- | --- | --- |
| Grenville_S_150X (13 areas, 224 Mpx) | 712 MiB | 426 MiB | 1,215–1,429 MB |
| Grenville_apatite_50X, Grenville_Sweden_apatite_60X, Sliderock_apatite_60X, Sweden_apatite_80X (28 areas, 428 Mpx) | 2,108 MiB | 1,632 MiB | 2,668–3,066 MB |

About 600 MB of each footprint is freed stitch scratch that macOS malloc keeps
as "Malloc Large (empty)" and "Malloc Small (empty)" (`vmmap --summary`);
`malloc_zone_pressure_relief` returns 0 bytes there. This change leaves the
allocator alone.

The desktop app (`grainatlas-desktop <projects…> --stitch`):

| workload | 2026-11-06 | before | after |
| --- | --- | --- | --- |
| first window, no project | 259–776 ms | 224–265 ms (712 ms cold) | 209–221 ms |
| idle, no project | 210 MB | 88–89 MB | 89 MB |
| Grenville_S_150X stitched | 2,990 MB | 1,624–1,660 MB | 1,652 MB |
| four apatite tabs stitched | n/a | 3,109–3,560 MB (main process 2,763–3,173) | 2,221–2,613 MB (main 1,870–2,258) |
| four apatite tabs, 10 minutes later | n/a | app process gone in both runs (no crash report; unexplained) | 2,588 MB, idle CPU 0.03 % |
| CPU time to open and stitch the four tabs | n/a | 59.6–62.0 s | 58.5–60.2 s |

Four tabs settle about 900 MB lower (−29 %); one project is unchanged, as
expected. Peak during the four-tab stitch was about 4.3 GB before the change
(main process, sampled every 5 s).

CLI workloads, unchanged by this change and identical in output:
`grainatlas segment` (all areas, saved decisions) on Grenville_apatite_50X
(BSE) 4.9 s and 1.58–1.61 GB peak, on Grenville_S_150X (CL) 10.5–10.9 s and
2.1–2.2 GB; outlines identical before and after (only newly minted UUIDv7
grain ids differ). `grainatlas sheets`: Grenville_apatite_50X 6.8–6.9 s,
0.76–0.78 GB, 25.5 MB PDF; Grenville_S_150X 9.3–9.6 s, 0.99–1.0 GB,
27.9 MB PDF (the 2026-11-06 PDF of that project was 21 MB). Registration and
stats JSON are byte-identical before and after; the PDF bytes differ between
two runs of the same build, so they are not compared. The sheet PDFs embed
31 lossless grey bands at 307 ppi (Flate, about 60 % of raw); a lossy
encoding would shrink them several-fold but changes the printed pixels, so it
is left as a decision.

## Changed files

- Core storage and eviction: `crates/grainatlas-core/src/mosaic/pyramid.rs`,
  `crates/grainatlas-core/src/project/pipeline.rs`,
  `crates/grainatlas-core/src/project/session.rs`.
- Core readers and regression tests: `crates/grainatlas-cli/src/main.rs`,
  `crates/grainatlas-core/tests/auto_tone.rs`,
  `crates/grainatlas-core/tests/memory.rs`.
- Memory report bridges: `crates/grainatlas-wasm/src/lib.rs`,
  `tauri/src-tauri/src/lib.rs`, `ui/src/lib/core/core.ts`,
  `ui/src/lib/core/types.ts`, `ui/src/lib/core/tauri.ts`.
- Worker compilation and initialization: `ui/src/lib/core/wasm.ts`,
  `ui/src/lib/core/worker.ts`.
- Canvas cache and fallback: `ui/src/lib/canvas/tiles.ts`,
  `ui/src/lib/canvas/Viewer.svelte`.
- UI regression tests: `ui/unit/tiles.test.ts`, `ui/tests/memory.spec.ts`.
- Desktop window eviction policy and tests: `tauri/src-tauri/src/memory.rs`.
- This report: `docs/memory.md`.
