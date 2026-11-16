#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [[ ! -d target/screenshot-package/Example_mount ]]; then
  echo 'Place a neutralised package in target/screenshot-package/Example_mount; see docs/screenshots.md.' >&2
  exit 1
fi
if ! command -v cwebp >/dev/null 2>&1; then
  echo 'cwebp is required to convert the screenshots; see docs/screenshots.md.' >&2
  exit 1
fi

(cd ui && pnpm exec playwright test -c playwright.screenshots.config.ts)

mkdir -p docs/images
shopt -s nullglob
pngs=(target/screenshots/*.png)
if (( ${#pngs[@]} == 0 )); then
  echo 'No PNG screenshots were written to target/screenshots/.' >&2
  exit 1
fi
for png in "${pngs[@]}"; do
  name="$(basename "${png%.png}")"
  webp="docs/images/${name}.webp"
  cwebp -q 82 -m 6 -quiet "$png" -o "$webp"
  echo "Wrote $webp"
done
