#!/bin/bash

set -euo pipefail

script_directory=$(cd "$(dirname "$0")" && pwd)
workflow=${1:-"$script_directory/../../.github/workflows/macos.yml"}

if [[ ! -f "$workflow" ]]; then
    echo "missing macOS workflow: $workflow" >&2
    exit 1
fi

require_text() {
    local expected=$1
    if ! grep -Fq -- "$expected" "$workflow"; then
        echo "macOS workflow is missing: $expected" >&2
        exit 1
    fi
}

reject_pattern() {
    local pattern=$1
    if grep -Eq -- "$pattern" "$workflow"; then
        echo "macOS workflow contains forbidden pattern: $pattern" >&2
        exit 1
    fi
}

require_text "verify:"
require_text "build-release:"
require_text "package:"
require_text "needs: [verify, build-release]"
require_text "permissions:"
require_text "contents: read"
require_text 'group: ${{ github.workflow }}-${{ github.ref }}'
require_text "cancel-in-progress: true"
require_text "name: Verify shared diagnostics"
require_text "working-directory: shared/diagnostics"
require_text "name: Verify build provenance generator"
require_text "working-directory: shared/build-provenance"
require_text "name: Check formatting"
require_text "run: cargo fmt --all --check"
require_text "name: Run default-feature tests"
require_text "cargo test --locked --all-targets"
require_text "name: Run no-default-features contract tests"
require_text "cargo test --locked --no-default-features --lib"
require_text "name: Run default-feature Clippy"
require_text "cargo clippy --locked --all-targets -- -D warnings"
require_text "aarch64-apple-darwin"
require_text "x86_64-apple-darwin"
require_text 'cargo build --locked --release --target "${{ matrix.target }}"'
require_text "MOQCAST_BUILD_IDENTITY: macos-universal2-adhoc"
require_text 'MOQCAST_PROVENANCE_OUTPUT: target/${{ matrix.target }}/release/moqcast-build-info.txt'
require_text 'MoQCast-macOS-binary-${{ matrix.target }}'
require_text "MoQCast-macOS-binary-aarch64-apple-darwin"
require_text "MoQCast-macOS-binary-x86_64-apple-darwin"
require_text 'path: ${{ runner.temp }}/moqcast-binary/aarch64-apple-darwin'
require_text 'path: ${{ runner.temp }}/moqcast-binary/x86_64-apple-darwin'
require_text "uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093"
require_text "uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02"
require_text '"$RUNNER_TEMP/moqcast-binary/aarch64-apple-darwin/moqcast-macos"'
require_text '"$RUNNER_TEMP/moqcast-binary/aarch64-apple-darwin/moqcast-build-info.txt"'
require_text '"$RUNNER_TEMP/moqcast-binary/x86_64-apple-darwin/moqcast-macos"'
require_text '"$RUNNER_TEMP/moqcast-binary/x86_64-apple-darwin/moqcast-build-info.txt"'
require_text "./scripts/package-app.sh"
require_text "iconutil -c iconset"
require_text 'lipo "$binary" -verify_arch arm64 x86_64'
require_text 'codesign --verify --deep --strict "$app"'
require_text 'shasum -a 256 -c "$(basename "$archive").sha256"'
require_text 'grep -Fxq "dependency_identity=moq-dev/moq@${moq_revision}" "$build_info"'
reject_pattern 'MoQCast-macOS-binary-\*'
reject_pattern '^[[:space:]]*uses: [^ ]+@v[0-9]'
reject_pattern 'moq-dev/moq@[0-9a-f]{40}'

if ! grep -Eq '^[[:space:]]+name: MoQCast-macOS$' "$workflow"; then
    echo "macOS workflow is missing the final app artifact" >&2
    exit 1
fi
