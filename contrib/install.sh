#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
prefix=${PREFIX:-/usr}
dest=${DESTDIR:-}$prefix
build=${IRA_BUILD_DIR:-$root/target/release}

for binary in ira ira-input ira-overlay-ui; do
    install -Dm755 "$build/$binary" "$dest/lib/ira/$binary"
done
for library in libira_overlay_shim.so libira_overlay_vk.so; do
    install -Dm755 "$build/$library" "$dest/lib/ira/$library"
done
mkdir -p "$dest/bin"
ln -sfn ../lib/ira/ira "$dest/bin/ira"
install -Dm644 "$root/contrib/com.github.ira.desktop" "$dest/share/applications/com.github.ira.desktop"
install -Dm644 "$root/contrib/com.github.ira.svg" "$dest/share/icons/hicolor/scalable/apps/com.github.ira.svg"
install -Dm644 "$root/contrib/com.github.ira.png" "$dest/share/icons/hicolor/256x256/apps/com.github.ira.png"
install -Dm644 "$root/contrib/com.github.ira.metainfo.xml" "$dest/share/metainfo/com.github.ira.metainfo.xml"
install -Dm644 "$root/LICENSE" "$dest/share/licenses/ira/LICENSE"
for rule in "$root"/contrib/*.rules; do
    install -Dm644 "$rule" "$dest/lib/udev/rules.d/$(basename "$rule")"
done
for po in "$root"/po/*.po; do
    [[ -f $po ]] || continue
    lang=$(basename "$po" .po)
    mkdir -p "$dest/share/locale/$lang/LC_MESSAGES"
    msgfmt --check "$po" -o "$dest/share/locale/$lang/LC_MESSAGES/ira.mo"
done
