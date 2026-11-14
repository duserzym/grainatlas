# grainatlas tasks. `make setup && make full` from a clean clone.
SHELL := /bin/bash
WASM_BINDGEN_VERSION := $(shell grep -A1 'name = "wasm-bindgen"$$' Cargo.lock 2>/dev/null | sed -n 's/version = "\(.*\)"/\1/p' | head -1)
WASM_OUT := ui/src/lib/core/pkg

.PHONY: setup bridge core-test unit wasm ui-dev ui-build icons desktop-dev desktop-build test parity e2e release-web atlas-fixture screenshots site
.PHONY: updater-setup check-version
.PHONY: quick full check desktop-check cli-release scripts-test

# @lab/bridge (UPb Studio's module API) is a sibling-checkout dependency until it
# is published: ui/package.json has "file:../../upblab/bridge".
UPBSTUDIO ?= ../upblab
VERSION := $(shell node -p "require('./ui/package.json').version" 2>/dev/null)
RELEASE_DIR := target/release-web
RELEASE_ASSET := grainatlas-web-$(VERSION).tar.gz
SHA256 := $(shell command -v sha256sum >/dev/null 2>&1 && echo sha256sum || echo "shasum -a 256")

setup:
	rustup update stable
	rustup target add wasm32-unknown-unknown
	@if [ -n "$(WASM_BINDGEN_VERSION)" ] && ! wasm-bindgen --version 2>/dev/null | grep -q "$(WASM_BINDGEN_VERSION)"; then \
		cargo install --locked wasm-bindgen-cli --version $(WASM_BINDGEN_VERSION); fi
	$(MAKE) bridge
	@if [ -d ui ]; then cd ui && pnpm install --frozen-lockfile && pnpm exec playwright install chromium; fi

# Builds $(UPBSTUDIO)/bridge/dist once (writes only build outputs there).
bridge:
	@if [ -f $(UPBSTUDIO)/bridge/dist/index.js ]; then echo "@lab/bridge: $(UPBSTUDIO)/bridge/dist is built"; \
	elif [ -d $(UPBSTUDIO)/bridge ]; then cd $(UPBSTUDIO) && pnpm install --frozen-lockfile --filter @lab/bridge && pnpm --filter @lab/bridge build; \
	else echo "@lab/bridge: no $(UPBSTUDIO) checkout next to grainatlas; see README.md (Dependencies) for the tarball fallback"; exit 1; fi

core-test:
	cargo test

wasm:
	cargo build -p grainatlas-wasm --target wasm32-unknown-unknown --release
	wasm-bindgen --target web --out-dir $(WASM_OUT) --out-name grainatlas target/wasm32-unknown-unknown/release/grainatlas_wasm.wasm

ui-dev: wasm
	cd ui && pnpm dev

ui-build: wasm
	cd ui && pnpm build

# Icons are generated from tauri/assets-icon.svg (no binary images in git), again whenever the
# SVG is newer than the generated set. tauri/icon/zircons.py regenerates the SVG itself.
icons: tauri/src-tauri/icons/icon.png

tauri/src-tauri/icons/icon.png: tauri/assets-icon.svg
	cd tauri && cargo tauri icon assets-icon.svg -o src-tauri/icons && rm -rf src-tauri/icons/android src-tauri/icons/ios
	@# The icons are embedded at compile time; Cargo does not watch them, so rebuild the app.
	touch tauri/src-tauri/build.rs

desktop-dev: ui-build icons
	cd tauri && cargo tauri dev

desktop-build: ui-build icons
	cd tauri && cargo tauri build

desktop-check: icons
	cargo check -p grainatlas-desktop

check: wasm
	cd ui && pnpm check

updater-setup:
	node scripts/setup-updater.mjs

check-version:
	@node scripts/check-version.mjs

# Native and wasm builds must hash the fixture mosaic identically.
cli-release:
	cargo build --release -q -p grainatlas-cli

parity: wasm cli-release
	target/release/grainatlas hash tests/fixtures/tm4000/Sweden_XS_150X > target/native.hash
	node ui/scripts/wasm-hash.mjs tests/fixtures/tm4000/Sweden_XS_150X > target/wasm.hash
	diff target/native.hash target/wasm.hash && echo "parity ok: $$(cat target/native.hash)"

# The fixture's atlas bundle (fixed ids, three spots, linked to a mount), served by the fake
# laicpmsdb of `pnpm preview` for the open-from-server and results tests.
ATLAS_FIXTURE_ID := 01972c4e-0000-7000-8000-00000000a7a5
atlas-fixture: cli-release
	target/release/grainatlas atlas tests/fixtures/tm4000/Sweden_XS_150X --out target/atlas-fixture \
		--project-id $(ATLAS_FIXTURE_ID) --mount-id 01972c4e-0000-7000-8000-0000000000b1 --demo-spots 3 --exported 2026-11-03T00:00:00Z

e2e: ui-build atlas-fixture
	cd ui && pnpm exec playwright test

# Browser-free UI unit tests (Node's runner strips the TypeScript types).
unit: atlas-fixture
	cd ui && node --test 'unit/*.test.ts'

# Non-browser checks run as parallel Make jobs, sharing one wasm, release CLI, atlas fixture
# and web build; Playwright then runs alone (its 10 workers use the cores, and CPU load from
# cargo running beside it makes timing-sensitive specs flaky).
scripts-test:
	node --test scripts/quick-check.test.mjs

full:
	+$(MAKE) -j4 check unit core-test scripts-test parity desktop-check ui-build atlas-fixture
	cd ui && pnpm exec playwright test

# CI entry point: like full without svelte-check and desktop-check (no Tauri system libraries).
test:
	+$(MAKE) -j4 unit core-test scripts-test parity ui-build atlas-fixture
	cd ui && pnpm exec playwright test

quick:
	@bash scripts/quick-check.sh

# Static web build for UPb Studio's modules.lock.json, index.html at the top of
# the archive. COPYFILE_DISABLE keeps macOS metadata files (._*) out of it.
release-web: ui-build
	@test -n "$(VERSION)" || (echo "no version in ui/package.json"; exit 1)
	mkdir -p $(RELEASE_DIR)
	cd ui/dist && COPYFILE_DISABLE=1 tar -czf ../../$(RELEASE_DIR)/$(RELEASE_ASSET) $$(ls -A)
	cd $(RELEASE_DIR) && $(SHA256) $(RELEASE_ASSET) | tee $(RELEASE_ASSET).sha256

# After a major update (AGENTS.md): drop compilation output once the app is
# installed. Keeps ui/dist (UPb Lab bundles it), the wasm pkg and node_modules.
.PHONY: clean-build
clean-build:
	@if pgrep -f "$(CURDIR)/target/" >/dev/null; then echo "A build is writing to $(CURDIR)/target; not cleaning."; exit 1; fi
	@df -h / | awk 'NR==2 {print "Free on /: " $$4 " (before)"}'
	rm -rf ./target
	@df -h / | awk 'NR==2 {print "Free on /: " $$4 " (after)"}'

# README and site screenshots: one pass through the workflow on the test fixture (Playwright).
# PNGs land in target/screenshots, WebP copies (cwebp; `brew install webp`) in docs/images.
screenshots: ui-build
	mkdir -p target/screenshots docs/images
	cd ui && pnpm exec playwright test -c playwright.screenshots.config.ts
	for f in target/screenshots/*.png; do cwebp -quiet -q 86 -m 6 "$$f" -o docs/images/$$(basename "$${f%.png}").webp; done

# The project website, assembled in target/site (site/ plus the docs/images screenshots).
# Preview: `python3 -m http.server -d target/site 8000`. Publishing is by hand (pages.yml).
site:
	mkdir -p target/site/images
	cp -R site/. target/site/
	cp docs/images/*.webp target/site/images/
	cp ui/public/favicon.svg target/site/
