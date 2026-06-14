#!/usr/bin/env bash
#
# Download the full Lucide icon set into assets/icons/lucide/ (gitignored) so any
# icon is usable by name during development. Debug builds serve these from disk
# (see the asset source in src/main.rs); release builds ship only the embedded
# set plus the icons compiled in under assets/icons/. Run once; re-run to update.
#
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
dest="$repo/assets/icons/lucide"
mkdir -p "$dest"

echo "resolving latest lucide-static…"
tarball=$(curl -fsSL https://registry.npmjs.org/lucide-static/latest \
  | python3 -c 'import sys, json; print(json.load(sys.stdin)["dist"]["tarball"])')

echo "downloading $tarball"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$tarball" | tar xz -C "$tmp"

# npm tarballs unpack under package/; the icons live in package/icons/*.svg.
cp "$tmp"/package/icons/*.svg "$dest"/

echo "done: $(ls "$dest" | wc -l | tr -d ' ') icons in assets/icons/lucide/"
