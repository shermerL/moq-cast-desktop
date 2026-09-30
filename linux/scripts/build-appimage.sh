#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
LINUX_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
source "$SCRIPT_DIR/build-info.sh"
OUTPUT_ROOT=${MOQCAST_PACKAGE_DIR:-"$LINUX_DIR/target/package"}
LINUXDEPLOY=${LINUXDEPLOY:-linuxdeploy}
PACKAGE_VARIANT=${MOQCAST_PACKAGE_VARIANT:-linux-x86_64}
INTENDED_TARGETS=${MOQCAST_INTENDED_TARGETS:-unspecified}

if [[ $(uname -s) != Linux ]]; then
    echo "AppImage must be built on Linux." >&2
    exit 1
fi

for tool in cargo file git pkg-config ldd install sha256sum "$LINUXDEPLOY"; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Required build tool is missing: $tool" >&2
        exit 1
    fi
done

if ! pkg-config --exists libpipewire-0.3; then
    echo "libpipewire-0.3 development files are required." >&2
    exit 1
fi

if ! pkg-config --exists alsa; then
    echo "ALSA development files are required for remote audio playback." >&2
    exit 1
fi

if [[ ! $PACKAGE_VARIANT =~ ^[a-z0-9][a-z0-9._-]*$ ]]; then
    echo "Invalid package variant: $PACKAGE_VARIANT" >&2
    exit 1
fi

mkdir -p "$OUTPUT_ROOT"
OUTPUT_ROOT=$(CDPATH= cd -- "$OUTPUT_ROOT" && pwd)
cd "$LINUX_DIR"
GENERATED_INFO="$LINUX_DIR/target/release/moqcast-build-info.txt"
MOQCAST_BUILD_IDENTITY="$PACKAGE_VARIANT" \
MOQCAST_PROVENANCE_OUTPUT="$GENERATED_INFO" \
cargo build --locked --release
VERSION=$(sed -n 's/^app_version=//p' "$GENERATED_INFO")
if [[ -z $VERSION ]]; then
    echo "Generated provenance is missing app_version." >&2
    exit 1
fi
BUILD_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD_DISTRO_ID=$(sed -n 's/^ID="\{0,1\}\([^" ]*\)"\{0,1\}$/\1/p' /etc/os-release | head -n 1)
BUILD_DISTRO_VERSION=$(sed -n 's/^VERSION_ID="\{0,1\}\([^" ]*\)"\{0,1\}$/\1/p' /etc/os-release | head -n 1)
GLIBC_VERSION=$(ldd --version | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')
PIPEWIRE_VERSION=$(pkg-config --modversion libpipewire-0.3)
ALSA_VERSION=$(pkg-config --modversion alsa)
PACKAGE_ID="MoQCast-${VERSION}-${PACKAGE_VARIANT}"
APPDIR="$OUTPUT_ROOT/${PACKAGE_ID}.AppDir"
APPIMAGE="$OUTPUT_ROOT/${PACKAGE_ID}.AppImage"
BUILD_INFO_FILE="$APPDIR/usr/share/doc/moqcast/build-info.txt"

if [[ -e "$APPDIR" || -e "$APPIMAGE" ]]; then
    echo "Package output already exists: $PACKAGE_ID" >&2
    echo "Choose an empty MOQCAST_PACKAGE_DIR instead of deleting existing artifacts." >&2
    exit 1
fi
mkdir "$APPDIR"
mkdir -p "$(dirname -- "$BUILD_INFO_FILE")"
write_build_info "$BUILD_INFO_FILE" "$GENERATED_INFO"

install -Dm755 target/release/moq-cast-desktop "$APPDIR/usr/bin/moq-cast-desktop"
install -Dm755 packaging/appimage/AppRun "$APPDIR/AppRun"
install -Dm644 packaging/appimage/dev.moq.moqcast.desktop.desktop "$APPDIR/dev.moq.moqcast.desktop.desktop"
install -Dm644 assets/icons/hicolor/512x512/apps/moqcast.png "$APPDIR/moqcast.png"
install -Dm644 packaging/appimage/dev.moq.moqcast.desktop.desktop \
    "$APPDIR/usr/share/applications/dev.moq.moqcast.desktop.desktop"
for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "assets/icons/hicolor/${size}x${size}/apps/moqcast.png" \
        "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps/moqcast.png"
done
install -Dm644 assets/fonts/LICENSE-NOTO \
    "$APPDIR/usr/share/licenses/moqcast/Noto-Sans-CJK-OFL.txt"
install -Dm644 vendor/moq-video/LICENSE-APACHE \
    "$APPDIR/usr/share/licenses/moqcast/moq-video-LICENSE-APACHE.txt"
install -Dm644 vendor/moq-video/LICENSE-MIT \
    "$APPDIR/usr/share/licenses/moqcast/moq-video-LICENSE-MIT.txt"
install -Dm644 vendor/libspa/LICENSE \
    "$APPDIR/usr/share/licenses/moqcast/libspa-LICENSE.txt"

ldd "$APPDIR/usr/bin/moq-cast-desktop" >"$APPDIR/usr/share/doc/moqcast/linked-libraries.txt"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 LDAI_OUTPUT="$APPIMAGE" "$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/moq-cast-desktop" \
    --desktop-file "$APPDIR/dev.moq.moqcast.desktop.desktop" \
    --icon-file "$APPDIR/moqcast.png" \
    --output appimage

chmod +x "$APPIMAGE"
APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGE" --version
sha256sum "$APPIMAGE" >"$APPIMAGE.sha256"

echo "Created $APPIMAGE"
