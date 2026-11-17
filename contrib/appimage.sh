#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
linuxdeploy=${LINUXDEPLOY:-linuxdeploy}
command -v "$linuxdeploy" >/dev/null || { echo 'Set LINUXDEPLOY to a linuxdeploy executable.' >&2; exit 1; }
for tool in file glib-compile-schemas readelf secret-tool python3 Xvfb dbus-run-session gdbus; do
    command -v "$tool" >/dev/null || { echo "Missing packaging tool: $tool" >&2; exit 1; }
done
if [[ ${SKIP_BUILD:-0} != 1 ]]; then
    cargo build --locked --release -p ira -p ira-input -p ira-overlay-ui -p ira-overlay-shim -p ira-overlay-vk --bins --lib
fi
mkdir -p "$root/target/appimage"
appdir=$(mktemp -d "$root/target/appimage/Ira.XXXXXXXX.AppDir")
chmod 755 "$appdir"
DESTDIR="$appdir" PREFIX=/usr bash "$root/contrib/install.sh"
install -Dm755 "$(command -v secret-tool)" "$appdir/usr/bin/secret-tool"
if [[ -d /usr/share/licenses ]]; then
    cp -r /usr/share/licenses/. "$appdir/usr/share/licenses/"
fi
mkdir -p "$appdir/usr/share/ira"
getconf GNU_LIBC_VERSION > "$appdir/usr/share/ira/build-platform.txt"
rustc --version >> "$appdir/usr/share/ira/build-platform.txt"

# Data files are loaded at runtime, so ELF dependency discovery cannot find them.
mkdir -p "$appdir/usr/share/glib-2.0/schemas"
cp /usr/share/glib-2.0/schemas/org.gtk.gtk4.*.xml "$appdir/usr/share/glib-2.0/schemas/"
glib-compile-schemas "$appdir/usr/share/glib-2.0/schemas"
for theme in Adwaita hicolor; do
    if [[ -d /usr/share/icons/$theme ]]; then
        mkdir -p "$appdir/usr/share/icons/$theme"
        cp -r /usr/share/icons/"$theme"/. "$appdir/usr/share/icons/$theme/"
    fi
done
if [[ -d /usr/share/mime ]]; then
    cp -r /usr/share/mime "$appdir/usr/share/"
fi
if [[ -d /usr/share/glycin-loaders ]]; then
    command -v bwrap >/dev/null || { echo 'Glycin requires bubblewrap.' >&2; exit 1; }
    install -Dm755 "$(command -v bwrap)" "$appdir/usr/bin/bwrap"
    cp -r /usr/share/glycin-loaders "$appdir/usr/share/"
    mkdir -p "$appdir/usr/libexec"
    cp -r /usr/libexec/glycin-loaders "$appdir/usr/libexec/"
fi

# RPATH is local to the bundled ELF files. LD_LIBRARY_PATH would also affect
# Wine, emulators, and native games spawned by Ira.
export APPIMAGE_EXTRACT_AND_RUN=1
deps=(--deploy-deps-only "$appdir/usr/bin" --deploy-deps-only "$appdir/usr/lib/ira")
if [[ -d $appdir/usr/libexec ]]; then
    while IFS= read -r -d '' loader; do
        deps+=(--deploy-deps-only "$loader")
    done < <(find "$appdir/usr/libexec" -type f -executable -print0)
fi
"$linuxdeploy" --appdir "$appdir" "${deps[@]}" \
    --desktop-file "$root/contrib/com.github.ira.desktop" \
    --icon-file "$root/contrib/com.github.ira.png" --custom-apprun "$root/contrib/AppRun"
install -m644 "$root/contrib/com.github.ira.png" "$appdir/com.github.ira.png"
ln -sfn com.github.ira.png "$appdir/.DirIcon"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/ira/Cargo.toml | head -1)
output=${LDAI_OUTPUT:-"$root/target/appimage/Ira-$version-$(uname -m).AppImage"}
export LDAI_OUTPUT="$output"
"$linuxdeploy" --appdir "$appdir" --output appimage
python3 "$root/contrib/check-desktop-protocol.py" "$output"
(
    cd -- "$(dirname -- "$output")"
    sha256sum "$(basename -- "$output")"
) > "$output.sha256"
printf 'AppImage: %s\nAppDir: %s\n' "$output" "$appdir"
