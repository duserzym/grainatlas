# Test tiers

Two checks. `make quick` on every change; `make full` once before an install
or build, and for any change to sync, storage, saved-file formats, print layout
or anything touching registered data. `make test` (CI) is full without the
Svelte and desktop checks, so CI needs no Tauri system libraries.

## Quick check

`make quick` compares the working tree (staged, unstaged and untracked files)
with `HEAD`; `BASE=HEAD~1 make quick` adds the last commit. It prints what it
chose and why. `scripts/quick-check.sh --files-from <list> --dry-run` shows
the selection for a list of paths without running anything.

| Changed files | Quick check runs |
| --- | --- |
| `docs/`, `site/`, Markdown only | nothing |
| `ui/src/`, `ui/unit/` | `pnpm check` and unit tests, then the web build and the browser specs mapped in `ui/tests/affected.mjs`, plus `smoke.spec.ts` |
| `ui/tests/` specs or helpers | the changed specs and the specs that import them, plus smoke |
| `crates/grainatlas-core/` | `cargo test -p grainatlas-core` (wasm is rebuilt first if browser specs run too) |
| `crates/grainatlas-cli/` | the CLI crate's tests |
| `crates/grainatlas-wasm/` | wasm build and native/wasm parity |
| `tauri/` | `make desktop-check` |
| Makefile, Cargo manifest or lock, UI package or lock, Vite or Playwright config, the spec map, `scripts/`, any other path | `make full` |

The map in `ui/tests/affected.mjs` is a sample of the specs that exercise each
module or screen. A UI file with no entry still runs smoke and says so. Quick
is a sample; the full check keeps every test.

## Full check

`make full` runs the Svelte check, unit tests, core and CLI tests, parity and
`desktop-check` as four parallel Make jobs, then every Playwright test on its
own: with cargo running beside it, CPU load makes timing-sensitive specs
flaky, and the parallel lanes saved no time. Wasm, the release CLI, the
atlas fixture and the web build are each made once. `make unit` builds the
atlas fixture it reads, and `make desktop-check` generates the icons the
desktop crate embeds, so both work in a fresh checkout.

Playwright runs tests inside a file in parallel on 10 workers, against one
`vite preview` server on port 4173. Five files run their tests in order
(`atlas`, `results`, `open-server`, `presentation`, `shell-focus`): each one
resets and reads one fake-server namespace, and a reset from a parallel test
would erase another test's state.

Three fixed waits stay, because each asserts that something does not happen
within a time window: hover-info's 100 ms detour and 300 ms window around the
250 ms dismissal timer, and shell-open-files' 7 s check that the storage
warning outlives a normal notice.

`node scripts/playwright-times.mjs <report.json>` prints the wall time and the ten
slowest spec files of a Playwright JSON report (`--reporter=json`).

## Install builds: shared Cargo target

On the owner's request (2026-11-14) local macOS install builds keep one shared
Cargo target, `~/Library/Caches/lab-builds/grainatlas-target`, set by
`RELEASE_TARGET` in the Makefile. Installs build in a fresh checkout, so
without it each install build is cold. Measured for the whole `make
desktop-build` (wasm, web build, desktop release, `.app` and `.dmg`): 149 s
cold, 116 s with the shared target after a one-file core change; less when
only the UI changed. It holds 2.6 GiB between installs.

- `make desktop-build` and the wasm it builds use the shared target. The
  `.app` lands in `~/Library/Caches/lab-builds/grainatlas-target/release/bundle/macos/`;
  copy it to `/Applications` and delete it there, so Spotlight finds one copy.
- Tests, checks, `make wasm`, `make ui-build` and the release CLI keep using
  `target/`.
- `make clean-build` (after a major update) removes `target/`, keeps the
  shared target and prints its size; `make clean-release-cache` removes it,
  only when the owner asks.
- `make desktop-build RELEASE_TARGET=` builds into `target/` as before.
  GitHub Actions always does.
- Cargo locks the target, so two install builds at once wait for each other.
  The release profile (thin LTO) is unchanged.
