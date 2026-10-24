#!/usr/bin/env bash
set -euo pipefail

APPIMAGE=$(realpath -- "${1:?Usage: check-appimage-checksum.sh package.AppImage}")
PACKAGE_NAME=$(basename -- "$APPIMAGE")
# Checking from the build directory can conceal absolute paths in the sidecar.
EXPECTED=$(cd -- "$(dirname -- "$APPIMAGE")" && sha256sum "$PACKAGE_NAME")
if [[ $(cat "$APPIMAGE.sha256") != "$EXPECTED" ]]; then
    echo "Checksum must contain the package basename and its SHA-256." >&2
    exit 1
fi
CHECK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/moqcast-checksum.XXXXXX")
cleanup() {
    rm -f -- "$CHECK_DIR/$PACKAGE_NAME" "$CHECK_DIR/$PACKAGE_NAME.sha256"
    rmdir -- "$CHECK_DIR"
}
trap cleanup EXIT
cp -- "$APPIMAGE" "$APPIMAGE.sha256" "$CHECK_DIR/"
(cd -- "$CHECK_DIR" && sha256sum -c "$PACKAGE_NAME.sha256")
