# Vendored moq-video

source_repository = `https://github.com/moq-dev/moq`
source_revision = `81d39f7bf04c82aae324a9ee4251b7f8aa08fb53`
source_path = `rs/moq-video`

The local copy carries four Linux product patches:

- `encode::Options::max_size` resizes display capture before probing and
  encoding, keeping the catalog and encoded output within MoQCast's 1080p
  ceiling.
- Native X11 display capture uses XRandR for monitor selection, MIT-SHM with an
  XGetImage fallback for pixels, and XFixes for the cursor. Wayland uses the
  portal and PipeWire backend with the local lifecycle patch below.
- PipeWire transfer-function validation uses the stable SPA enum values for
  names missing from older distribution headers.
- `capture::cleanup::Owner` retains portal acquisition and close tasks outside
  a cancellable capture future. The application stops capture, joins its
  PipeWire thread, and awaits session close acknowledgement before completing
  publication teardown. Close failures remain visible. Demand idle may reopen
  capture after cleanup; source loss is terminal and clears the restore token
  instead of automatically opening another picker. Production ownership and
  exit-policy tests run through `linux/tests/portal_cleanup.rs`; real portal
  and compositor behavior still requires a Wayland environment.

`Cargo.toml` replaces workspace dependencies with the same pinned moq-dev
revision used by the desktop application. `LICENSE-APACHE` and `LICENSE-MIT`
come from the source repository root.
