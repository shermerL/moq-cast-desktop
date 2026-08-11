# Vendored libspa

source_repository = `https://gitlab.freedesktop.org/pipewire/pipewire-rs`
source_version = `0.10.0`
source_package = `https://crates.io/crates/libspa/0.10.0`

The local copy keeps the public crate version and dependencies unchanged. It
implements two SPA metadata macros in Rust, initializes `spa_video_info_raw`
without assuming fields added in PipeWire 0.3.65, gates the matching flags API,
and accepts the signed modifier type used by PipeWire 0.3.48. This allows the
Ubuntu 22.04 build to use its native PipeWire development headers.
