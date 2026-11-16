# Documentation screenshots

Real images are allowed only in deliberately committed documentation screenshots.
The source package is copied only into the ignored `target/screenshot-package/`
folder and is never committed. Neutralise every name and piece of metadata:
package, area and sample names, people, places, sessions, dates, paths and
instrument serials. Inspect sidecars and navigation images as well as tile names.

Use the package copy script with this argument order:

```bash
python3 scripts/neutralise_package.py SRC OUT_PARENT NEW_NAME OLD_FOLDER=NEW_AREA ...
```

For the screenshot package, use `target/screenshot-package` as `OUT_PARENT`,
`Example_mount` as `NEW_NAME`, and map the two original area folders to
`Sample-A` and `Sample-B`. Substitute the original folder names locally; do not
record them in tracked files. The resulting package belongs at
`target/screenshot-package/Example_mount/`, with area folders `001_Sample-A`
and `002_Sample-B`, and area and sample names `Sample-A` and `Sample-B`.
Sample-A has 10 × 2 tiles and Sample-B has 9 × 2 tiles. Tiles are 1280 × 960 px
at about 1.65 µm/px. Do not copy the package anywhere else in the repository.

The capture uses the static web build in `ui/dist`, served by `vite preview`
on port 4173. It requires the UI dependencies, Playwright Chromium and `cwebp`
on `PATH`. If Chromium is missing, install it with
`cd ui && pnpm exec playwright install chromium`. Run from the repository root:

```bash
make full
make ui-build
scripts/screenshots.sh
```

The build must pass the repository's `make full` checks before it runs. Follow
AGENTS.md if building manually. Stop any unrelated server on port 4173 first.
Playwright reuses a server already listening there, so it must serve this build.
The capture script does not build the UI or change the source package.

`ui/playwright.screenshots.config.ts` is a separate configuration for documentation
capture. It runs Chromium with one worker, no retries and a device scale factor
of 1. Each theme opens and processes the package once. Each shot is captured at
1440 × 900 and 1280 × 800 CSS px, in light and dark themes. Image views are fitted
after resizing; detail views then zoom to a dense group found from the app's
drawn grain outlines. The script waits for busy indicators and live rendering,
two animation frames and a short settling interval before capturing the viewport.

The shot names and order are fixed:

| Name | View |
| --- | --- |
| `open` | Package read, areas listed and tile overview visible, before stitching. The app navigates from Open to Area when reading finishes. |
| `stitch` | Sample-A after Stitch all areas and Auto tone, with histogram, tone controls and residuals. |
| `grains` | A dense part of Sample-A at about 40 % zoom with numbered grain outlines and the Grains table. Detection uses CL and brighter or darker polarity. |
| `outline` | Three nearby grains, a selected grain card and an unfinished Outline lasso. Escape cancels the lasso after each capture. |
| `spots` | Eight spots inside grains, named `Sample-A_1` through `Sample-A_8`, with the Spots table and one selected spot. |
| `print` | Layout with a generated grain print sheet preview and its statistics. |
| `settings` | A separate empty Settings page showing Server and Sign in…. A minimal Tauri startup stub exposes the desktop sign-in control in the web build. No package is opened on this page. |
| `sync` | The open project linked through a mocked server to Example mount at revision 3, with Database has more and the details panel open. |

The PNG file names are
`target/screenshots/<name>-<width>x<height>-<theme>.png`. For example,
`target/screenshots/spots-1440x900-light.png`. There are 32 captures in a complete
run. Every written file is logged. PNGs and Playwright output remain under ignored
`target/`. The script converts every PNG directly in `target/screenshots/` using
`cwebp -q 82 -m 6 -quiet`. The matching documentation files are
`docs/images/<name>-<width>x<height>-<theme>.webp`, for example
`docs/images/spots-1440x900-light.webp`. Only those WebP screenshots and the
existing blank-app image are exceptions to the repository's image ignore rule.

The sync example intercepts `https://upbdb.example.org/**` with local responses.
It offers a fake mount, uses the app's Link action, and returns one additional
annotation, Example note. The account is Example user. Mount and annotation IDs
are fixed fake UUIDs; the project identity comes from the newly
opened local project. The mock accepts no element writes or image uploads and
fails on unexpected requests. The token is an example string set through Settings.
The separate Settings page also uses `https://upbdb.example.org` and does not sign in.

For a direct capture invocation:

```bash
cd ui
pnpm exec playwright test -c playwright.screenshots.config.ts
```

The spec accepts `DEMO_PACKAGE` as an absolute path or a path relative to the
command's working directory. Its default is the package location above, resolved
relative to the spec. Use only a neutralised package in ignored
`target/screenshot-package/`. The shell wrapper requires the standard package
location. To check capture discovery and TypeScript transpilation without
starting the preview server:

```bash
cd ui
pnpm exec playwright test -c playwright.screenshots.config.ts --list
```

Inspect every PNG and WebP before committing. Check names, paths and server names,
including the print header, grain and spot cards, navigation image, Settings and
sync details. Check that text is readable and content fits both viewport sizes.
Browser folder uploads use relative names; no absolute package path is intended
to be visible. The capture rejects common local absolute path prefixes, but that
check does not replace inspection. Never commit a source package, a trace or a
capture that still contains identifying information.
