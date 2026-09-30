#!/bin/bash

set -euo pipefail

if [[ $# -ne 5 ]]; then
    echo "usage: $0 <arm64-binary> <arm64-provenance> <x86_64-binary> <x86_64-provenance> <output-directory>" >&2
    exit 2
fi

arm64_binary=$1
arm64_provenance=$2
x86_64_binary=$3
x86_64_provenance=$4
output_directory=$5
script_directory=$(cd "$(dirname "$0")" && pwd)
mac_directory=$(cd "$script_directory/.." && pwd)
manifest_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$mac_directory/Cargo.toml" | head -n 1)
if [[ -z "$manifest_version" ]]; then
    echo "missing package version in $mac_directory/Cargo.toml" >&2
    exit 1
fi
for binary in "$arm64_binary" "$x86_64_binary"; do
    if [[ ! -f "$binary" ]]; then
        echo "missing input binary: $binary" >&2
        exit 1
    fi
done

for provenance in "$arm64_provenance" "$x86_64_provenance"; do
    if [[ ! -f "$provenance" ]]; then
        echo "missing input provenance: $provenance" >&2
        exit 1
    fi
done

if ! cmp -s \
    <(grep -v '^target=' "$arm64_provenance") \
    <(grep -v '^target=' "$x86_64_provenance"); then
    echo "architecture provenance records disagree on common build facts" >&2
    exit 1
fi

fact() {
    local key=$1
    local file=$2
    local count
    local value
    count=$(grep -c "^${key}=" "$file" || true)
    if [[ "$count" -ne 1 ]]; then
        echo "expected exactly one $key field in $file" >&2
        exit 1
    fi
    value=$(sed -n "s/^${key}=//p" "$file")
    if [[ -z "$value" ]]; then
        echo "empty $key field in $file" >&2
        exit 1
    fi
    printf '%s' "$value"
}

package_version=$(fact app_version "$arm64_provenance")
build_identity=$(fact build_identity "$arm64_provenance")
arm64_target=$(fact target "$arm64_provenance")
x86_64_target=$(fact target "$x86_64_provenance")
fact source_identity "$arm64_provenance" >/dev/null
fact dependency_identity "$arm64_provenance" >/dev/null
fact source_commit "$arm64_provenance" >/dev/null
fact source_state "$arm64_provenance" >/dev/null
fact moq_revision "$arm64_provenance" >/dev/null

if [[ "$package_version" != "$manifest_version" ]]; then
    echo "provenance app version $package_version does not match manifest $manifest_version" >&2
    exit 1
fi
if [[ "$build_identity" != "macos-universal2-adhoc" ]]; then
    echo "unexpected build identity: $build_identity" >&2
    exit 1
fi
if [[ "$arm64_target" != "aarch64-apple-darwin" ]]; then
    echo "unexpected arm64 build target: $arm64_target" >&2
    exit 1
fi
if [[ "$x86_64_target" != "x86_64-apple-darwin" ]]; then
    echo "unexpected x86_64 build target: $x86_64_target" >&2
    exit 1
fi

marketing_version=${MOQCAST_MARKETING_VERSION:-${package_version%%-*}}
build_version=${MOQCAST_BUILD_VERSION:-1}
archive_name="MoQCast-macOS-${package_version}.zip"
app_directory="$output_directory/MoQCast.app"
archive_path="$output_directory/$archive_name"

if [[ -e "$app_directory" || -e "$archive_path" || -e "$archive_path.sha256" ]]; then
    echo "package output already exists in $output_directory" >&2
    exit 1
fi

mkdir -p "$app_directory/Contents/MacOS" "$app_directory/Contents/Resources"
lipo -create "$arm64_binary" "$x86_64_binary" -output "$app_directory/Contents/MacOS/moqcast-macos"
chmod 755 "$app_directory/Contents/MacOS/moqcast-macos"

sed \
    -e "s/__MARKETING_VERSION__/$marketing_version/g" \
    -e "s/__BUILD_VERSION__/$build_version/g" \
    "$mac_directory/packaging/Info.plist.in" > "$app_directory/Contents/Info.plist"
cp "$mac_directory/assets/icons/MoQCast.icns" "$app_directory/Contents/Resources/MoQCast.icns"
cp "$mac_directory/packaging/entitlements.plist" "$app_directory/Contents/Resources/entitlements.plist"

build_info="$app_directory/Contents/Resources/build-info.txt"
grep -v '^target=' "$arm64_provenance" > "$build_info"
printf 'target=universal2-apple-darwin\nminimum_macos=14.2\n' >> "$build_info"

plutil -lint "$app_directory/Contents/Info.plist"
lipo "$app_directory/Contents/MacOS/moqcast-macos" -verify_arch arm64 x86_64
codesign --force --sign - \
    --entitlements "$mac_directory/packaging/entitlements.plist" \
    "$app_directory/Contents/MacOS/moqcast-macos"
codesign --force --sign - \
    --entitlements "$mac_directory/packaging/entitlements.plist" \
    "$app_directory"
codesign --verify --deep --strict "$app_directory"
ditto -c -k --sequesterRsrc --keepParent "$app_directory" "$archive_path"
(
    cd "$output_directory"
    shasum -a 256 "$archive_name" > "$archive_name.sha256"
)
