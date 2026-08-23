# Vendored moq-video

source_repository = `https://github.com/moq-dev/moq`
source_revision = `81d39f7bf04c82aae324a9ee4251b7f8aa08fb53`
source_path = `rs/moq-video`

The local copy carries two Linux product patches:

- `encode::Options::max_size` resizes display capture before probing and
  encoding, keeping the catalog and encoded output within MoQCast's 1080p
  ceiling.
- Native X11 display capture uses XRandR for monitor selection, MIT-SHM with an
  XGetImage fallback for pixels, and XFixes for the cursor. Wayland continues to
  use the upstream portal and PipeWire backend.

`Cargo.toml` replaces workspace dependencies with the same pinned moq-dev
revision used by the desktop application. `LICENSE-APACHE` and `LICENSE-MIT`
come from the source repository root.
