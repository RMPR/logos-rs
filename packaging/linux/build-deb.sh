#!/usr/bin/env bash
# Build a self-contained LeVerbe .deb from a release build.
#
#   packaging/linux/build-deb.sh <version> [out-dir]
#
# Expects target/release/logos-rs and data/bible.db (cargo build --release;
# target/release/import). The prebuilt database ships in the package, so the
# app starts immediately and never writes outside the user's config directory.
set -euo pipefail

version="${1:?usage: build-deb.sh <version> [out-dir]}"
out="${2:-dist}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
here="$root/packaging/linux"
arch="$(dpkg --print-architecture)"
pkg="leverbe"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
dest="$stage/$pkg"

install -Dm755 "$root/target/release/logos-rs" "$dest/usr/bin/leverbe"
strip --strip-unneeded "$dest/usr/bin/leverbe"
install -Dm644 "$root/data/bible.db" "$dest/usr/share/leverbe/bible.db"
install -Dm644 "$here/leverbe.desktop" "$dest/usr/share/applications/leverbe.desktop"
install -Dm644 "$here/leverbe.svg" "$dest/usr/share/icons/hicolor/scalable/apps/leverbe.svg"
install -Dm644 "$here/copyright" "$dest/usr/share/doc/$pkg/copyright"

# Shared-library dependencies of the binary (fontconfig, freetype, libc, …).
mkdir -p "$stage/shlibs/debian"
printf 'Source: %s\n\nPackage: %s\nArchitecture: any\n' "$pkg" "$pkg" > "$stage/shlibs/debian/control"
shlibs="$(cd "$stage/shlibs" && dpkg-shlibdeps -O "$dest/usr/bin/leverbe" | sed -n 's/^shlibs:Depends=//p')"
# Loaded at runtime with dlopen by winit/glutin, so invisible to dpkg-shlibdeps:
# Wayland and X11 windowing, keyboard handling, and OpenGL/EGL. Fonts that
# cover Hebrew and Greek with their diacritics.
runtime="libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1, libx11-6, libx11-xcb1, libxcursor1, libxi6, libxrandr2, libegl1, libgl1, fonts-noto-core | fonts-dejavu-core"

# The import card's file picker asks the desktop's portal over D-Bus; every
# desktop install has one, hence only a recommendation.
installed_size="$(du -sk "$dest" | cut -f1)"
mkdir -p "$dest/DEBIAN"
cat > "$dest/DEBIAN/control" <<CONTROL
Package: $pkg
Version: $version
Section: text
Priority: optional
Architecture: $arch
Maintainer: LeVerbe <noreply@github.com>
Installed-Size: $installed_size
Depends: ${shlibs:+$shlibs, }$runtime
Recommends: xdg-desktop-portal
Homepage: https://github.com/rmpr/logos-rs
Description: Bible study in the original languages
 LeVerbe is a desktop Bible study app: parallel translations, full-text and
 morphological search over the Hebrew Old Testament and Greek New Testament,
 a reader's edition with glosses for rare words, word study with Strong's
 lexicons and a reverse interlinear for KJV, ULT and Louis Segond. Bibles
 you own can be imported from EPUB, PDF or DOCX and are linked to the Hebrew
 and Greek the same way.
 .
 The complete Bible database is included; no network access is needed.
CONTROL

mkdir -p "$out"
deb="$out/${pkg}_${version}_${arch}.deb"
dpkg-deb --root-owner-group -Zxz --build "$dest" "$deb"
echo "$deb"
