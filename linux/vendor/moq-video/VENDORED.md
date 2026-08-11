# Vendored moq-video

source_repository = `https://github.com/moq-dev/moq`
source_revision = `68cf64603f067f64bc02fe800464aa59710a258c`
source_path = `rs/moq-video`

The local copy adds `encode::Options::max_size` so MoQCast can resize display
capture before encoding. This keeps the OpenH264 fallback within a 1080p output
limit without requiring an unpublished moq-dev revision.

`Cargo.toml` replaces workspace dependencies with the same pinned moq-dev
revision used by the desktop application. `LICENSE-APACHE` and `LICENSE-MIT`
come from the source repository root.
