# grainatlas tasks. `make setup && make full` from a clean clone (with upblab checked out beside it).
SHELL := /bin/bash
WASM_BINDGEN_VERSION := $(shell grep -A1 'name = "wasm-bindgen"$$' Cargo.lock 2>/dev/null | sed -n 's/version = "\(.*\)"/\1/p' | head -1)
WASM_OUT := ui/src/lib/core/pkg

.PHONY: setup bridge core-test wasm ui-dev ui-build icons desktop-dev desktop-build test release-web site
.PHONY: updater-setup check-version
.PHONY: full check desktop-check screenshots

# @lab/bridge (UPb Studio's module API) is a sibling-checkout dependency until it
# is published: ui/package.json has "file:../../upblab/bridge".
UPBSTUDIO ?= ../upblab
VERSION := $(shell node -p "require('./ui/package.json').version" 2>/dev/null)
RELEASE_DIR := target/release-web
RELEASE_ASSET := grainatlas-web-$(VERSION).tar.gz
SHA256 := $(shell command -v sha256sum >/dev/null 2>&1 && echo sha256sum || echo "shasum -a 256")

# Local macOS install builds (`make desktop-build` and the wasm it builds) share one Cargo
# target outside the checkout, so a fresh build checkout reuses compiled dependencies (149 s
# cold, 116 s after a one-file core change; 2.6 GiB of disk). `make clean-build`
# keeps it; `make clean-release-cache` removes it; `make desktop-build RELEASE_TARGET=`
# builds into target/ as before. Tests, checks and GitHub Actions always use target/.
ifeq ($(shell uname -s),Darwin)
ifndef GITHUB_ACTIONS
RELEASE_TARGET ?= $(HOME)/Library/Caches/lab-builds/grainatlas-target
endif
endif
# Cargo target for the wasm build: target/ normally, the shared target under desktop-build.
WASM_TARGET_DIR = $(CURDIR)/target

setup:
	rustup update stable
	rustup target add wasm32-unknown-unknown
	@if [ -n "$(WASM_BINDGEN_VERSION)" ] && ! wasm-bindgen --version 2>/dev/null | grep -q "$(WASM_BINDGEN_VERSION)"; then \
		cargo install --locked wasm-bindgen-cli --version $(WASM_BINDGEN_VERSION); fi
	$(MAKE) bridge
	@if [ -d ui ]; then cd ui && pnpm install --frozen-lockfile; fi

# Builds $(UPBSTUDIO)/bridge/dist once (writes only build outputs there).
bridge:
	@if [ -f $(UPBSTUDIO)/bridge/dist/index.js ]; then echo "@lab/bridge: $(UPBSTUDIO)/bridge/dist is built"; \
	elif [ -d $(UPBSTUDIO)/bridge ]; then cd $(UPBSTUDIO) && pnpm install --frozen-lockfile --filter @lab/bridge && pnpm --filter @lab/bridge build; \
	else echo "@lab/bridge: no $(UPBSTUDIO) checkout next to grainatlas; see README.md (Dependencies) for the tarball fallback"; exit 1; fi

core-test:
	cargo test

wasm:
	CARGO_TARGET_DIR="$(WASM_TARGET_DIR)" cargo build -p grainatlas-wasm --target wasm32-unknown-unknown --release
	wasm-bindgen --target web --out-dir $(WASM_OUT) --out-name grainatlas "$(WASM_TARGET_DIR)/wasm32-unknown-unknown/release/grainatlas_wasm.wasm"

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

# Sign macOS builds with the lab's code-signing certificate when this Mac has it
# ("UPb Lab Code Signing" in the login keychain; ~/Github/lab-builds/SIGNING.md), so
# keychain permissions survive rebuilds and updates. Elsewhere builds stay ad hoc.
LAB_SIGNING_IDENTITY ?= $(shell security find-identity -p codesigning 2>/dev/null | grep -q '"UPb Lab Code Signing"' && echo 'UPb Lab Code Signing')
SIGN_ENV = $(if $(LAB_SIGNING_IDENTITY),APPLE_SIGNING_IDENTITY="$(LAB_SIGNING_IDENTITY)")

# Target-specific, so the wasm built as a prerequisite here also uses the shared target.
desktop-build: WASM_TARGET_DIR = $(if $(RELEASE_TARGET),$(RELEASE_TARGET),$(CURDIR)/target)
desktop-build: ui-build icons
	cd tauri && $(SIGN_ENV) CARGO_TARGET_DIR="$(WASM_TARGET_DIR)" cargo tauri build
	@echo "App: $(WASM_TARGET_DIR)/release/bundle/ (copy the .app out, then delete it there)"

desktop-check: icons
	cargo check -p grainatlas-desktop

check: wasm
	cd ui && pnpm check

updater-setup:
	node scripts/setup-updater.mjs

check-version:
	@node scripts/check-version.mjs

# Checks: Rust unit tests, svelte-check and the desktop crate.
full:
	+$(MAKE) -j3 check core-test desktop-check

# CI entry point: like full without svelte-check and desktop-check (no Tauri system libraries).
test: core-test ui-build

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
	@$(if $(RELEASE_TARGET),test -d "$(RELEASE_TARGET)" && du -sh "$(RELEASE_TARGET)" | awk '{print "Kept shared release target: " $$1 " in " $$2}' || true,true)

# Removes the shared install-build target (only when the owner asks; the next install is cold).
.PHONY: clean-release-cache
clean-release-cache:
	@test -n "$(RELEASE_TARGET)" || { echo "No shared release target on this machine."; exit 0; }
	@if pgrep -f "cargo.*$(RELEASE_TARGET)|$(RELEASE_TARGET)/" >/dev/null; then echo "A build is writing to $(RELEASE_TARGET); not cleaning."; exit 1; fi
	@df -h / | awk 'NR==2 {print "Free on /: " $$4 " (before)"}'
	rm -rf "$(RELEASE_TARGET)"
	@df -h / | awk 'NR==2 {print "Free on /: " $$4 " (after)"}'

# Docs screenshots from a neutralised package in target/screenshot-package (docs/screenshots.md).
screenshots:
	scripts/screenshots.sh

# The project website, assembled in target/site.
# Preview: `python3 -m http.server -d target/site 8000`. Publishing is by hand (pages.yml).
site:
	mkdir -p target/site/images
	cp -R site/. target/site/
	@if compgen -G 'docs/images/*.webp' > /dev/null; then cp docs/images/*.webp target/site/images/; fi
	cp docs/app-blank.png target/site/
	cp tauri/assets-icon.svg target/site/icon.svg
	cp ui/public/favicon.svg target/site/
