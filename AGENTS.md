# AGENTS.md: rules for coding agents in grainatlas

grainatlas is the SEM grain-mount module of an open-source LA-ICP-MS U-Pb
workflow: a Rust core (`crates/`, native and wasm), a Svelte 5 + TypeScript
page in `ui/` (static web build, also hosted by UPb Studio through
`@lab/bridge`), and a Tauri 2 desktop shell in `tauri/src-tauri/`. The owner
is Yiming Zhang (they/them). A supervising Claude session writes the briefs,
audits every diff, runs the browser and app checks and makes the commits.

## Hard rules

1. **Never commit, tag, push, or create remotes, releases or secrets.** Leave
   the work in the working tree. `git status` and `git diff` are fine.
2. **No sample data in the repository; it is public.** No SEM packages,
   tiles, projects or test fixtures are committed. The sample-based test
   suite and fixtures were removed when the repository went public
   (2026-11-15); a copy is kept outside git in
   `~/Github/lab-builds/grainatlas-private/`. Real images appear only in
   deliberately committed docs screenshots (`docs/images/*.webp`, plus
   `docs/app-blank.png`), with every name and piece of metadata neutralised:
   package, project, area and sample names, people, places, sessions, dates,
   paths, emails, server names and instrument serials (`docs/screenshots.md`).
   A source package may be copied only into an ignored `target/` folder to
   make screenshots (`scripts/neutralise_package.py`), never anywhere else in
   the repository, and is never committed.
3. **CI stays quiet.** Workflows run only on `v*` tags and
   `workflow_dispatch`. Never add push or pull-request triggers.
4. **Light theme by default; every screen fits 1440 x 900 and 1280 x 800**
   CSS px (`docs/ux-conventions.md`). Use the design tokens in
   `ui/src/app.css` and the existing classes (`glass`, `panel`, `field`,
   `check`, `icon`, `faint small`) instead of new ones. Plain, short English in
   the UI; no exclamation marks, no emoji.
5. **Hosted in UPb Studio** (`isHosted()` from `@lab/bridge`) the page must
   not do what the shell does: no own window chrome, no own updater, the
   shell's theme wins.
6. Other repositories (`../upbstudio`, `../upbdb`, `../UPbRPy`) are
   read-only unless the brief says otherwise.
7. Tests must pass before you stop; never skip, delete or loosen a test to
   make it pass. If a test is wrong, say so in your final message.

## Commands

```bash
make bridge                 # builds ../upbstudio/bridge/dist once (@lab/bridge)
make wasm                   # wasm core into ui/src/lib/core/pkg
make full                   # Rust unit tests, svelte-check and desktop compilation
cd ui && pnpm check         # svelte-check
cd ui && pnpm test          # UI unit tests (node --test, part of make full)
cd ui && npx playwright test # e2e with a synthetic bundle and a mocked server (needs a port)
cd ui && pnpm build         # static build in ui/dist
make desktop-check          # generates icons and checks the Tauri shell
```

You cannot run the desktop app; the supervisor does.

## Checks

Run `make full` before an install or build, and for any change to sync,
storage, saved-file formats, print layout or anything touching registered data.
It runs the Rust unit tests, svelte-check and desktop-check. Docs-only changes
need no checks.

## Style

- TypeScript strict, Svelte 5 runes, small modules under `ui/src/lib/`.
- Rust: `cargo fmt`, clippy-clean.
- Prose in docs: plain sentences, no marketing, no emoji, numbers with units.
- Say in your final message what you did not finish or could not verify.

## Print layout rule

Every automatic print layout keeps neighbouring grains together: a grain on
paper is found by the grains around it. Read "Layout rule: context first" in
`docs/print_sheets.md` before changing `crates/grainatlas-core/src/layout/`;
its properties are tested and must not be loosened.

## After a major update

Rust and PyInstaller builds fill the disk quickly. Once the work is pushed and
any app built from it is installed in `/Applications` and smoke-tested:

1. Note free space (`df -h /`).
2. Run `make clean-build`. It removes `target/` (native, wasm and Tauri builds, bundles, `target/release-web`). It keeps `ui/dist` (UPb Lab bundles it from here), `ui/src/lib/core/pkg` and `ui/node_modules`. It stops if a build
   is still writing there, and prints free space before and after. It also
   keeps the shared install-build target
   `~/Library/Caches/lab-builds/grainatlas-target` (2.6 GiB, kept on the
   owner's request so installs are faster). Delete the
   `.app` left in its `release/bundle/macos/` after copying it out; remove the
   whole cache (`make clean-release-cache`) only when the owner asks.
3. Remove the worktrees and build checkouts you created for this work
   (`git worktree remove`, then `git branch -d` for merged branches).
4. Keep installed apps and review artifacts (screenshots, previews, print
   files, COMMIT notes in `~/Github/lab-builds`). Never touch another
   session's worktree, scratchpad or running build, project data or
   databases.
5. Report free space before and after.
