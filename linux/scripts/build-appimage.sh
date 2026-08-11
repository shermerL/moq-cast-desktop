#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
LINUX_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
REPO_DIR=$(CDPATH= cd -- "$LINUX_DIR/.." && pwd)
OUTPUT_ROOT=${MOQCAST_PACKAGE_DIR:-"$LINUX_DIR/target/package"}
LINUXDEPLOY=${LINUXDEPLOY:-linuxdeploy}
APPIMAGETOOL=${APPIMAGETOOL:-appimagetool}

if [[ $(uname -s) != Linux ]]; then
    echo "AppImage must be built on Linux." >&2
    exit 1
fi

for tool in cargo git pkg-config ldd install sha256sum "$LINUXDEPLOY" "$APPIMAGETOOL"; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Required build tool is missing: $tool" >&2
        exit 1
    fi
done

if ! pkg-config --exists libpipewire-0.3; then
    echo "libpipewire-0.3 development files are required." >&2
    exit 1
fi

VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$LINUX_DIR/Cargo.toml" | head -n 1)
SOURCE_COMMIT=$(git -C "$REPO_DIR" rev-parse --short=12 HEAD)
MOQ_REVISION=$(sed -n 's/.*moq-native.*rev = "\([^"]*\)".*/\1/p' "$LINUX_DIR/Cargo.toml")
BUILD_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
PACKAGE_ID="MoQCast-${VERSION}-${SOURCE_COMMIT}-x86_64"
APPDIR="$OUTPUT_ROOT/${PACKAGE_ID}.AppDir"
APPIMAGE="$OUTPUT_ROOT/${PACKAGE_ID}.AppImage"

mkdir -p "$OUTPUT_ROOT"
if [[ -e "$APPDIR" || -e "$APPIMAGE" ]]; then
    echo "Package output already exists: $PACKAGE_ID" >&2
    echo "Choose an empty MOQCAST_PACKAGE_DIR instead of deleting existing artifacts." >&2
    exit 1
fi
mkdir "$APPDIR"

cd "$LINUX_DIR"
cargo build --locked --release

install -Dm755 target/release/moq-cast-desktop "$APPDIR/usr/bin/moq-cast-desktop"
install -Dm755 packaging/appimage/AppRun "$APPDIR/AppRun"
install -Dm644 packaging/appimage/dev.moq.moqcast.desktop.desktop "$APPDIR/dev.moq.moqcast.desktop.desktop"
install -Dm644 packaging/appimage/moqcast.svg "$APPDIR/moqcast.svg"
install -Dm644 packaging/appimage/dev.moq.moqcast.desktop.desktop \
    "$APPDIR/usr/share/applications/dev.moq.moqcast.desktop.desktop"
install -Dm644 packaging/appimage/moqcast.svg \
    "$APPDIR/usr/share/icons/hicolor/scalable/apps/moqcast.svg"

mkdir -p "$APPDIR/usr/share/doc/moqcast"
{
    echo "source_commit=$SOURCE_COMMIT"
    echo "moq_revision=$MOQ_REVISION"
    echo "cargo_features=moq-native:aws-lc-rs,mdns,quinn;moq-video:nvenc,pipewire"
    echo "build_date=$BUILD_DATE"
    echo "target=x86_64-unknown-linux-gnu"
    echo "compatibility_baseline=ubuntu-24.04,glibc-2.39,pipewire-1.0"
} >"$APPDIR/usr/share/doc/moqcast/build-info.txt"

APPIMAGE_EXTRACT_AND_RUN=1 "$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/moq-cast-desktop" \
    --desktop-file "$APPDIR/dev.moq.moqcast.desktop.desktop" \
    --icon-file "$APPDIR/moqcast.svg"

ldd "$APPDIR/usr/bin/moq-cast-desktop" >"$APPDIR/usr/share/doc/moqcast/linked-libraries.txt"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" "$APPDIR" "$APPIMAGE"
chmod +x "$APPIMAGE"
APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGE" --version
sha256sum "$APPIMAGE" >"$APPIMAGE.sha256"

echo "Created $APPIMAGE"
