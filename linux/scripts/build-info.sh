# Extend the compiler-generated record with package environment facts.
write_build_info() {
    local output_file=$1
    local generated_file=$2
    if [[ $(grep -Fxc "build_identity=$PACKAGE_VARIANT" "$generated_file") -ne 1 ]] ||
       [[ $(grep -Fxc 'target=x86_64-unknown-linux-gnu' "$generated_file") -ne 1 ]]; then
        echo "Generated provenance does not match this Linux package variant/target." >&2
        return 1
    fi
    {
        cat "$generated_file"
        printf 'system_audio=pipewire\n'
        printf 'remote_audio_output=cpal-alsa\n'
        printf 'build_date=%s\n' "$BUILD_DATE"
        printf 'package_variant=%s\n' "$PACKAGE_VARIANT"
        printf 'build_distribution=%s-%s\n' "$BUILD_DISTRO_ID" "$BUILD_DISTRO_VERSION"
        printf 'glibc_version=%s\n' "$GLIBC_VERSION"
        printf 'pipewire_build_version=%s\n' "$PIPEWIRE_VERSION"
        printf 'alsa_build_version=%s\n' "$ALSA_VERSION"
        printf 'intended_targets=%s\n' "$INTENDED_TARGETS"
    } >"$output_file"
}
