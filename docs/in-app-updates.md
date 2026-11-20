# In-app updates

Every app of the lab workflow updates itself the way repohub and CodexBar do:
a quiet check in the background, a download when allowed, and one "Restart
to update" action. The toolbar shows the current version and a circular-arrows
button that opens the update popover.

## Mechanism (Tauri apps)

- `tauri-plugin-updater` 2 reads a `latest.json` from GitHub Releases, compares
  the version with the running build, downloads the signed update bundle and
  verifies its minisign signature against the public key compiled into
  `tauri.conf.json` (`plugins.updater.pubkey`). An update that does not verify
  is refused.
- The app's own Rust commands (`update_check`, `update_download`,
  `update_install`, `update_status`, `update_supported`) wrap the plugin, so
  the web page never needs the plugin's JavaScript permissions. `update_install` installs the
  downloaded bytes and restarts the app.
- The page checks 8 seconds after launch and every 6 hours. With
  "Download updates automatically" on (the default), an available update is
  downloaded quietly and the popover offers "Restart to update". With it
  off, the popover and Settings panel offer Download.
- Only the latest stable release is used, through
  `https://github.com/duserzym/grainatlas/releases/latest/download/latest.json`.
  Older Beta preferences migrate silently to the stable feed.
- Hosted inside UPb Studio (the `@lab/bridge` transport is `tauri` or
  `postMessage`), a module never runs its own updater: the shell updates
  itself and the modules it pins in `modules.lock.json`. The chip is hidden
  there; the shell's own Updates panel lists the module versions.
- The static web build (no Tauri) shows the version chip but cannot update
  itself; the panel says so and links to the releases page.
- On Windows, only installed copies can download or install updates. The
  executable's folder must contain the NSIS `uninstall.exe` or the MSI
  `Uninstall grainatlas.lnk` shortcut. These names come from Tauri's
  [NSIS template](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi)
  and [MSI template](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/msi/main.wxs).
  A portable `grainatlas.exe` can check for updates and links to GitHub;
  run the release's setup file once, then use the installed app for in-app
  updates. macOS behaviour is unchanged.

## What the user sees

- **Toolbar**: a version chip `v1.2.3` and a circular-arrows check button,
  with a badge for available updates and rotation while busy.
- **Popover and Settings > Updates**: the canonical update status, automatic
  download preference, and Download or Restart to update when applicable.
  Available and ready states show plain-text notes clamped to four lines and
  a Release notes link to the exact version on GitHub in the system browser.
- **What's new**: after an upgrade or a manual reinstall with a newer version,
  a small card beside the check button shows the running version's bundled
  notes once. The app saves manifest notes just before installing as a fallback.
  With neither source it says "Updated to v<version>." First installs show
  nothing. Dismiss with the close button or Esc; focus returns to the check
  button. The popover's What's new link reopens it.
- Checks request the stable `latest.json`; there is no releases-list lookup
  and no telemetry.

## Release flow

1. Bump the version (the three copies must agree; see the repository section),
   commit and push.
2. Tag the stable version `vX.Y.Z` and push the tag.
3. The release workflow (`v*` tags and manual dispatch only, never on push)
   builds the installers, signs the update bundles with the private key from
   the `TAURI_SIGNING_PRIVATE_KEY` secret, and creates a draft GitHub Release
   with the installers, the signed bundles (`.app.tar.gz` + `.sig` on macOS,
   `.exe`/`.msi` + `.sig` on Windows) and `latest.json`.
4. Publish the draft after the workflows pass. Installed copies see the new
   version at their next check.

A manual run (`gh workflow run <name>`) builds the same artifacts and keeps
them as workflow artifacts without publishing a release; it is the dry run.

### Windows release from a Mac

From v0.1.5, Windows releases include the current-user NSIS installer
`grainatlas_<version>_x64-setup.exe`. Updates run it in passive mode.
Install `cargo-xwin`, the `x86_64-pc-windows-msvc` Rust target and Homebrew
NSIS (`makensis`) first. Run `make full`, then from the repository root:

```bash
make windows-installer
```

The target builds the web page and icons, reads the private key from
`TAURI_SIGNING_PRIVATE_KEY_PATH` (default `~/.tauri/grainatlas-updater.key`)
into `TAURI_SIGNING_PRIVATE_KEY` inside the recipe without printing it, and
runs this command from `tauri/`:

```bash
LC_ALL=en_US.UTF-8 LANG=en_US.UTF-8 TAURI_SIGNING_PRIVATE_KEY=... cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis --config src-tauri/tauri.release.conf.json
```

Here `...` means the key's contents supplied through the environment; keep
them out of command history. Both locale variables are needed: Homebrew
`makensis` aborts with `std::bad_alloc` under the C locale.
The release config enables updater artifacts. The build writes
`target/x86_64-pc-windows-msvc/release/bundle/nsis/grainatlas_<version>_x64-setup.exe`
and the adjacent `grainatlas_<version>_x64-setup.exe.sig`.

Include the setup exe with `--windows-x86_64` when generating the feed below.

## Releasing

1. Write `release-notes/<version>.md`: plain text or at most five short `- `
   bullets for lab users, no headings, nonempty and at most 600 characters.
   The web build bundles these files for the running version's What's new card.
2. Run `make full`, then build the web page and icons. From `tauri/`, build
   with `TAURI_SIGNING_PRIVATE_KEY` supplied securely in the environment:

   ```bash
   cargo tauri build --config src-tauri/tauri.release.conf.json
   ```

3. From the repository root, generate the feed, replacing the paths below:

   ```bash
   node scripts/release-feed.mjs --darwin-aarch64 path/to/grainatlas.app.tar.gz --windows-x86_64 path/to/grainatlas_<version>_x64-setup.exe --out latest.json
   ```

   Omit `--windows-x86_64` for a macOS-only release. `--version` defaults to
   `tauri/src-tauri/tauri.conf.json`. Each artifact needs its adjacent `.sig`.
   The script verifies the artifact and global minisign signatures against
   the configured public key before writing anything. It refuses missing,
   empty or oversized notes and invalid signatures. This replaces writing
   `latest.json` by hand; the release workflows are unchanged.
4. Upload the dmg, `.app.tar.gz`, adjacent `.sig`, and `latest.json` to the
   matching stable GitHub Release. For Windows, also upload the setup exe
   and its `.sig`. Publish the draft after checking the assets.

## One-time setup per repository (the owner does this)

1. Generate the signing key and write the public key into `tauri.conf.json`:

   ```bash
   node scripts/setup-updater.mjs
   ```

   The private key lands in `~/.tauri/<app>-updater.key` (back it up: losing
   it means installed copies cannot verify future updates). The script writes
   the public key into `tauri/src-tauri/tauri.conf.json` in grainatlas; commit
   that change.
2. Store the private key as the repository secret `TAURI_SIGNING_PRIVATE_KEY`
   (the script does it with `gh secret set` when `gh` is signed in; otherwise
   paste the file's contents in GitHub > Settings > Secrets and variables >
   Actions). The key has no password, so `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
   stays unset.
3. Nothing else: GitHub Releases is the only hosting the updater needs, and
   the default `GITHUB_TOKEN` can publish releases in the same repository.

## Checks in the repository

- `pnpm --dir ui test` runs unit tests for stable preference migration,
  version tracking, notes, update states and release-feed signature verification.
- `ui/e2e/updates.spec.ts` checks both update surfaces and the What's new card
  at 1440 x 900 and 1280 x 800, in light and dark themes. Preview states use
  `?update-preview=ready` (also `idle`, `checking`, `none`, `available`,
  `downloading`, `installing`, `error` and `unsupported`) outside Tauri.

## grainatlas specifics

- Tauri project: `tauri/src-tauri/` (the Cargo workspace member
  `grainatlas-desktop`). Updater code: `tauri/src-tauri/src/updater.rs`;
  page side `ui/src/lib/updates.ts` (pure state machine and version logic,
  unit-tested in `ui/src/lib/updates.test.mjs`), `ui/src/lib/updater.svelte.ts`
  (scheduler and Tauri calls), `ui/src/components/VersionChip.svelte` and the
  Updates panel in `ui/src/components/UpdatesPanel.svelte`, mounted by
  `ui/src/screens/Settings.svelte`.
- Version copies that must agree: `Cargo.toml` (`[workspace.package] version`),
  `tauri/src-tauri/tauri.conf.json` (`version`) and `ui/package.json`
  (`version`); `make check-version` compares them and the `desktop` workflow
  refuses a tag that does not match.
- Signing key: `~/.tauri/grainatlas-updater.key`; setup script
  `scripts/setup-updater.mjs` (run from the repository root with Node 22).
- Release workflow: `.github/workflows/desktop.yml` (macOS arm64 and Windows
  x86-64; the installers, the signed update bundles and `latest.json` go to
  the GitHub Release of the tag; the separate `release` workflow still attaches
  the web archive that UPb Studio pins). Both are draft releases: publish the
  draft on GitHub once both workflows are green, and the updater endpoint
  starts serving it.
- Update endpoint: `https://github.com/duserzym/grainatlas/releases/latest/download/latest.json`.
- Settings: `localStorage['grainatlas.updates']` holds the
  automatic-download switch, the last check, `lastSeenVersion` and the saved
  manifest notes (per computer, never in a project file).
