#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

tmpdir=$(mktemp --directory)
trap 'rm -rf "$tmpdir"' EXIT

sources=()
while IFS= read -r source; do
    destination="$tmpdir/$source"
    mkdir -p "$(dirname "$destination")"
    perl -pe 's/\b(?:crate|ira_overlay)::tr!/tr!/g' "$source" > "$destination"
    sources+=("$destination")
done < <(rg --files crates/ira/src crates/overlay/src -g '*.rs')

xgettext \
    --language=Rust \
    --keyword='tr!' \
    --from-code=UTF-8 \
    --package-name=Ira \
    --output=po/ira.pot \
    "${sources[@]}"

perl -pi -e "s|\Q$tmpdir/\E||g" po/ira.pot
