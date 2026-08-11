#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
LINUX_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
REPO_DIR=$(CDPATH= cd -- "$LINUX_DIR/.." && pwd)
OUTPUT_ROOT=${MOQCAST_PACKAGE_DIR:-"$LINUX_DIR/target/package"}
LINUXDEPLOY=${LINUXDEPLOY:-linuxdeploy}
APPIMAGETOOL=${APPIMAGETOOL:-appimagetool}
PACKAGE_VARIANT=${MOQCAST_PACKAGE_VARIANT:-linux-x86_64}
INTENDED_TARGETS=${MOQCAST_INTENDED_TARGETS:-unspecified}

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

if [[ ! $PACKAGE_VARIANT =~ ^[a-z0-9][a-z0-9._-]*$ ]]; then
    echo "Invalid package variant: $PACKAGE_VARIANT" >&2
    exit 1
fi

VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$LINUX_DIR/Cargo.toml" | head -n 1)
SOURCE_COMMIT=$(git -C "$REPO_DIR" rev-parse --short=12 HEAD)
MOQ_REVISION=$(sed -n 's/.*moq-native.*rev = "\([^"]*\)".*/\1/p' "$LINUX_DIR/Cargo.toml")
MOQ_VIDEO_REVISION=$(sed -n 's/^source_revision = `\([^`]*\)`/\1/p' "$LINUX_DIR/vendor/moq-video/VENDORED.md")
BUILD_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD_DISTRO_ID=$(sed -n 's/^ID="\{0,1\}\([^" ]*\)"\{0,1\}$/\1/p' /etc/os-release | head -n 1)
BUILD_DISTRO_VERSION=$(sed -n 's/^VERSION_ID="\{0,1\}\([^" ]*\)"\{0,1\}$/\1/p' /etc/os-release | head -n 1)
GLIBC_VERSION=$(ldd --version | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')
PIPEWIRE_VERSION=$(pkg-config --modversion libpipewire-0.3)
PACKAGE_ID="MoQCast-${VERSION}-${PACKAGE_VARIANT}"
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
install -Dm644 assets/fonts/LICENSE-NOTO \
    "$APPDIR/usr/share/licenses/moqcast/Noto-Sans-CJK-OFL.txt"
install -Dm644 vendor/moq-video/LICENSE-APACHE \
    "$APPDIR/usr/share/licenses/moqcast/moq-video-LICENSE-APACHE.txt"
install -Dm644 vendor/moq-video/LICENSE-MIT \
    "$APPDIR/usr/share/licenses/moqcast/moq-video-LICENSE-MIT.txt"

mkdir -p "$APPDIR/usr/share/doc/moqcast"
{
    echo "source_commit=$SOURCE_COMMIT"
    echo "moq_revision=$MOQ_REVISION"
    echo "moq_video_source=vendored"
    echo "moq_video_revision=$MOQ_VIDEO_REVISION"
    echo "cargo_features=moq-native:aws-lc-rs,mdns,quinn;moq-video:nvenc,pipewire"
    echo "build_date=$BUILD_DATE"
    echo "target=x86_64-unknown-linux-gnu"
    echo "package_variant=$PACKAGE_VARIANT"
    echo "build_distribution=$BUILD_DISTRO_ID-$BUILD_DISTRO_VERSION"
    echo "glibc_version=$GLIBC_VERSION"
    echo "pipewire_build_version=$PIPEWIRE_VERSION"
    echo "intended_targets=$INTENDED_TARGETS"
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
