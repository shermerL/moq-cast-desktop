# Vendored moq-video

source_repository = `https://github.com/moq-dev/moq`
source_revision = `615d166d246b04cde8d0449c80a556f22356f719`
source_path = `rs/moq-video`

The local copy carries these Linux product patches on top of that revision:

- `encode::Options::max_size` resizes display capture before probing and
  encoding, keeping the catalog and encoded output within MoQCast's 1080p
  ceiling.
- X11 backend selection prioritizes `XDG_SESSION_TYPE`; X11 pixel capture
  uses MIT-SHM with an XGetImage fallback. XRandR monitor selection and XFixes
  cursor handling come from upstream.
- `capture::cleanup::Owner` retains portal acquisition and close tasks outside
  a cancellable capture future. The application stops capture, joins its
  PipeWire thread, and awaits session close acknowledgement before completing
  publication teardown. Close failures remain visible. A recoverable stream
  end or demand idle may reopen after cleanup; an upstream source-loss error is
  terminal and clears the restore token. A sticky cleanup error prevents a
  demand-idle race from reopening after source loss. The vendor's cleanup and
  producer tests cover this decision; real portal/compositor behavior still
  requires a Wayland environment.

The upstream revision already provides primary-display XRandR selection,
XFixes cursor blending, Frame conversion, stable PipeWire transfer constants,
and the PipeWire loop quit/join ordering. Those are not separate local patches.
The old `src/encode/rate.rs` remains as an unreferenced file from the previous
vendor copy; the synchronized upstream crate does not include it, and this
update intentionally deletes no files.

`Cargo.toml` replaces workspace dependencies with the same pinned moq-dev
revision used by the desktop application. `LICENSE-APACHE` and `LICENSE-MIT`
come from the source repository root.
