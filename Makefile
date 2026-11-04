# grainatlas tasks. `make setup && make test` from a clean clone.
SHELL := /bin/bash
WASM_BINDGEN_VERSION := $(shell grep -A1 'name = "wasm-bindgen"$$' Cargo.lock 2>/dev/null | sed -n 's/version = "\(.*\)"/\1/p' | head -1)
WASM_OUT := ui/src/lib/core/pkg

.PHONY: setup bridge core-test wasm ui-dev ui-build icons desktop-dev desktop-build test parity e2e release-web

# @lab/bridge (UPb Studio's module API) is a sibling-checkout dependency until it
# is published: ui/package.json has "file:../../upbstudio/bridge".
UPBSTUDIO ?= ../upbstudio
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

# Native and wasm builds must hash the fixture mosaic identically.
parity: wasm
	cargo run --release -q -p grainatlas-cli -- hash tests/fixtures/tm4000/Sweden_XS_150X > target/native.hash
	node ui/scripts/wasm-hash.mjs tests/fixtures/tm4000/Sweden_XS_150X > target/wasm.hash
	diff target/native.hash target/wasm.hash && echo "parity ok: $$(cat target/native.hash)"

e2e: ui-build
	cd ui && pnpm exec playwright test

test: core-test parity e2e

# Static web build for UPb Studio's modules.lock.json, index.html at the top of
# the archive. COPYFILE_DISABLE keeps macOS metadata files (._*) out of it.
release-web: ui-build
	@test -n "$(VERSION)" || (echo "no version in ui/package.json"; exit 1)
	mkdir -p $(RELEASE_DIR)
	cd ui/dist && COPYFILE_DISABLE=1 tar -czf ../../$(RELEASE_DIR)/$(RELEASE_ASSET) $$(ls -A)
	cd $(RELEASE_DIR) && $(SHA256) $(RELEASE_ASSET) | tee $(RELEASE_ASSET).sha256
