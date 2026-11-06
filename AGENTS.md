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
2. **No real data in the repository.** `tests/fixtures/` holds the one small
   TM4000 area; never copy files from `~/My Drive…` or other repositories.
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
6. Other repositories (`../upbstudio`, `../laicpmsdb`, `../UPbRPy`) are
   read-only unless the brief says otherwise.
7. Tests must pass before you stop; never skip, delete or loosen a test to
   make it pass. If a test is wrong, say so in your final message.

## Commands

```bash
make bridge                 # builds ../upbstudio/bridge/dist once (@lab/bridge)
make wasm                   # wasm core into ui/src/lib/core/pkg
cd ui && pnpm check         # svelte-check
make unit                   # node --test ui/unit/*.test.ts
cd ui && pnpm build         # static build in ui/dist
cargo check -p grainatlas-desktop   # the Tauri shell compiles
```

Playwright (`make e2e`, `ui/tests/*.spec.ts`) needs browsers and the preview
server; write or update the spec files when the brief asks, and run them if
`pnpm exec playwright test <file>` works on this machine. You cannot run the
desktop app; the supervisor does.

## Style

- TypeScript strict, Svelte 5 runes, small modules under `ui/src/lib/`.
- Rust: `cargo fmt`, clippy-clean.
- Prose in docs: plain sentences, no marketing, no emoji, numbers with units.
- Say in your final message what you did not finish or could not verify.
