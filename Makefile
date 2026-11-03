# grainmap tasks. `make setup && make test` from a clean clone.
SHELL := /bin/bash
WASM_BINDGEN_VERSION := $(shell grep -A1 'name = "wasm-bindgen"$$' Cargo.lock 2>/dev/null | sed -n 's/version = "\(.*\)"/\1/p' | head -1)
WASM_OUT := ui/src/lib/core/pkg

.PHONY: setup core-test wasm ui-dev ui-build icons desktop-dev desktop-build test parity e2e

setup:
	rustup update stable
	rustup target add wasm32-unknown-unknown
	@if [ -n "$(WASM_BINDGEN_VERSION)" ] && ! wasm-bindgen --version 2>/dev/null | grep -q "$(WASM_BINDGEN_VERSION)"; then \
		cargo install --locked wasm-bindgen-cli --version $(WASM_BINDGEN_VERSION); fi
	@if [ -d ui ]; then cd ui && pnpm install --frozen-lockfile && pnpm exec playwright install chromium; fi

core-test:
	cargo test

wasm:
	cargo build -p grainmap-wasm --target wasm32-unknown-unknown --release
	wasm-bindgen --target web --out-dir $(WASM_OUT) --out-name grainmap target/wasm32-unknown-unknown/release/grainmap_wasm.wasm

ui-dev: wasm
	cd ui && pnpm dev

ui-build: wasm
	cd ui && pnpm build

# Icons are generated from tauri/assets-icon.svg (no binary images in git).
icons:
	@test -f tauri/src-tauri/icons/icon.png || (cd tauri && cargo tauri icon assets-icon.svg -o src-tauri/icons && rm -rf src-tauri/icons/android src-tauri/icons/ios)

desktop-dev: ui-build icons
	cd tauri && cargo tauri dev

desktop-build: ui-build icons
	cd tauri && cargo tauri build

# Native and wasm builds must hash the fixture mosaic identically.
parity: wasm
	cargo run --release -q -p grainmap-cli -- hash tests/fixtures/tm4000/Sweden_XS_150X > target/native.hash
	node ui/scripts/wasm-hash.mjs tests/fixtures/tm4000/Sweden_XS_150X > target/wasm.hash
	diff target/native.hash target/wasm.hash && echo "parity ok: $$(cat target/native.hash)"

e2e: ui-build
	cd ui && pnpm exec playwright test

test: core-test parity e2e
