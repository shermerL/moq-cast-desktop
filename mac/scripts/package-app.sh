#!/bin/bash

set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "usage: $0 <arm64-binary> <x86_64-binary> <output-directory>" >&2
    exit 2
fi

arm64_binary=$1
x86_64_binary=$2
output_directory=$3
script_directory=$(cd "$(dirname "$0")" && pwd)
mac_directory=$(cd "$script_directory/.." && pwd)
manifest_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$mac_directory/Cargo.toml" | head -n 1)
if [[ -z "$manifest_version" ]]; then
    echo "missing package version in $mac_directory/Cargo.toml" >&2
    exit 1
fi
package_version=${MOQCAST_PACKAGE_VERSION:-$manifest_version}
marketing_version=${MOQCAST_MARKETING_VERSION:-${package_version%%-*}}
build_version=${MOQCAST_BUILD_VERSION:-1}
source_commit=${MOQCAST_SOURCE_COMMIT:-unknown}
archive_name="MoQCast-macOS-${package_version}.zip"
app_directory="$output_directory/MoQCast.app"
archive_path="$output_directory/$archive_name"

for binary in "$arm64_binary" "$x86_64_binary"; do
    if [[ ! -f "$binary" ]]; then
        echo "missing input binary: $binary" >&2
        exit 1
    fi
done

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

cat > "$app_directory/Contents/Resources/build-info.txt" <<EOF
package_version=$package_version
build_identity=macos-universal2-adhoc
source_commit=$source_commit
minimum_macos=14.2
moq_dependency=moq-dev/moq@81d39f7bf04c82aae324a9ee4251b7f8aa08fb53
EOF

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
