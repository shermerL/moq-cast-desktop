#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
LINUX_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
TEST_DIRECTORY=$(mktemp -d "${TMPDIR:-/tmp}/moqcast-build-provenance.XXXXXX")
TEST_BINARY="$TEST_DIRECTORY/provenance-tests"
BUILD_SCRIPT_BINARY="$TEST_DIRECTORY/build-script"
BUILD_OUTPUT="$TEST_DIRECTORY/build-output.txt"
BUILD_INFO_FILE="$TEST_DIRECTORY/build-info.txt"

cleanup() {
    rm -f "$TEST_BINARY" "$BUILD_SCRIPT_BINARY" "$BUILD_OUTPUT" "$BUILD_INFO_FILE"
    rmdir "$TEST_DIRECTORY"
}
trap cleanup EXIT

rustc --edition=2024 -D warnings --test "$LINUX_DIR/build.rs" -o "$TEST_BINARY"
"$TEST_BINARY"

VERSION=0.5.0-dev.1
PACKAGE_VARIANT=linux-x86_64-ubuntu24.04-glibc2.39
SOURCE_COMMIT=0123456789ab
MOQ_REVISION=615d166d246b04cde8d0449c80a556f22356f719
MOQ_VIDEO_REVISION=615d166d246b04cde8d0449c80a556f22356f719
DEPENDENCY_IDENTITY="moq-dev/moq@$MOQ_REVISION;vendored/moq-video@$MOQ_VIDEO_REVISION"
BUILD_DATE=2026-09-13T00:00:00Z
BUILD_DISTRO_ID=ubuntu
BUILD_DISTRO_VERSION=24.04
GLIBC_VERSION=2.39
PIPEWIRE_VERSION=1.0.5
ALSA_VERSION=1.2.11
INTENDED_TARGETS=ubuntu-24.04,mint-22
source "$SCRIPT_DIR/build-info.sh"
write_build_info "$BUILD_INFO_FILE"

rustc --edition=2024 -D warnings "$LINUX_DIR/build.rs" -o "$BUILD_SCRIPT_BINARY"
CARGO_PKG_VERSION="$VERSION" \
MOQCAST_PROVENANCE_FILE="$BUILD_INFO_FILE" \
    "$BUILD_SCRIPT_BINARY" >"$BUILD_OUTPUT"

require_script_text() {
    local expected=$1
    if ! grep -Fq -- "$expected" "$LINUX_DIR/scripts/build-appimage.sh"; then
        echo "Linux packaging is missing provenance contract: $expected" >&2
        exit 1
    fi
}

reject_script_text() {
    local forbidden=$1
    if grep -Fq -- "$forbidden" "$LINUX_DIR/scripts/build-appimage.sh"; then
        echo "Linux packaging bypasses provenance file: $forbidden" >&2
        exit 1
    fi
}

require_output_text() {
    local expected=$1
    if ! grep -Fq -- "$expected" "$BUILD_OUTPUT"; then
        echo "Linux build script did not embed: $expected" >&2
        exit 1
    fi
}

require_output_text "cargo:rustc-env=MOQCAST_EMBEDDED_BUILD_IDENTITY=$PACKAGE_VARIANT"
require_output_text "cargo:rustc-env=MOQCAST_EMBEDDED_SOURCE_IDENTITY=$SOURCE_COMMIT"
require_output_text "cargo:rustc-env=MOQCAST_EMBEDDED_DEPENDENCY_IDENTITY=$DEPENDENCY_IDENTITY"
require_script_text 'source "$SCRIPT_DIR/build-info.sh"'
require_script_text 'if [[ $MOQ_VIDEO_REVISION != "$MOQ_REVISION" ]]; then'
require_script_text '"$LINUX_DIR/vendor/moq-video/Cargo.toml"'
require_script_text 'MOQCAST_PROVENANCE_FILE="$BUILD_INFO_FILE" \'
reject_script_text 'MOQCAST_BUILD_IDENTITY="$PACKAGE_VARIANT" \'
reject_script_text 'MOQCAST_SOURCE_COMMIT="$SOURCE_COMMIT" \'
