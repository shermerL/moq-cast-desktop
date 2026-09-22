write_build_info() {
    local output_file=$1

    {
        printf 'app_version=%s\n' "$VERSION"
        printf 'build_identity=%s\n' "$PACKAGE_VARIANT"
        printf 'source_identity=%s\n' "$SOURCE_COMMIT"
        printf 'dependency_identity=%s\n' "$DEPENDENCY_IDENTITY"
        printf 'source_commit=%s\n' "$SOURCE_COMMIT"
        printf 'moq_revision=%s\n' "$MOQ_REVISION"
        printf 'moq_video_source=vendored\n'
        printf 'moq_video_revision=%s\n' "$MOQ_VIDEO_REVISION"
        printf 'libspa_source=vendored-0.10.1\n'
        printf 'cargo_features=moq-tokio:aws-lc-rs,mdns,noq;moq-audio:playback;moq-video:capture,nvidia,openh264,pipewire\n'
        printf 'system_audio=pipewire\n'
        printf 'remote_audio_output=cpal-alsa\n'
        printf 'build_date=%s\n' "$BUILD_DATE"
        printf 'target=x86_64-unknown-linux-gnu\n'
        printf 'package_variant=%s\n' "$PACKAGE_VARIANT"
        printf 'build_distribution=%s-%s\n' "$BUILD_DISTRO_ID" "$BUILD_DISTRO_VERSION"
        printf 'glibc_version=%s\n' "$GLIBC_VERSION"
        printf 'pipewire_build_version=%s\n' "$PIPEWIRE_VERSION"
        printf 'alsa_build_version=%s\n' "$ALSA_VERSION"
        printf 'intended_targets=%s\n' "$INTENDED_TARGETS"
    } >"$output_file"
}
