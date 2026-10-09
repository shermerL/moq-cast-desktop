# Vendored moq-video

source_repository = `https://github.com/moq-dev/moq`
source_revision = `f8215bc47199b48512d805ae9fc710cc6586de51`
source_path = `rs/moq-video`

The baseline includes the accepted Portal selection API (PR #5089), including
per-selection restore tokens, cancellation-safe grant storage and older Portal
source-type compatibility, and the X11 MIT-SHM/event-update backend (PR #4749).
Those implementations now come directly from upstream, rather than backports.

The retained Linux product patches are:

- `encode::Options::max_size` resizes display capture before probing and
  encoding, keeping the catalog and encoded output within MoQCast's 1080p
  ceiling. It is applied through the upstream `encode::Capture` driver.
- X11 backend selection prioritizes `XDG_SESSION_TYPE`, so stale display
  environment variables do not redirect an X11 session or expose XWayland
  windows as native Wayland sources. The upstream SHM implementation remains
  intact, including the GetImage fallback and event-driven geometry handling.
- `capture::cleanup::Owner` retains portal acquisition and close tasks outside
  a cancellable capture future. The application stops capture, joins its
  PipeWire thread, and awaits session close acknowledgement before completing
  publication teardown. Close failures remain visible. A recoverable stream
  end or demand idle may reopen after cleanup; an upstream source-loss error is
  terminal and clears the restore token. A sticky cleanup error prevents a
  demand-idle race from reopening after source loss. The vendor's cleanup and
  producer tests cover this decision; real portal/compositor behavior still
  requires a Wayland environment.

Upstream now creates its ScreenCast `SessionGuard` immediately after
`CreateSession` (PR #4492). The local `Owner`/`Handle` contract already owns the
session before negotiation, including an in-flight `CreateSession`, and waits
for close acknowledgement. It remains the sole cleanup owner here; no second
upstream guard is layered onto the same session.

The upstream revision also includes native camera mode selection, capture-clock
fixtures, capture-clock reanchoring, V4L2/PipeWire camera deduplication, encoder
flush timing, and the NVDEC CUDA-context lifetime fix. The
local cleanup scope is retained across the shared screen/camera capture loop;
the application exposes screen and single-window sharing through the portal.

The upstream revision already provides primary-display XRandR selection,
XFixes cursor blending, Frame conversion, stable PipeWire transfer constants,
and the PipeWire loop quit/join ordering. Those are not separate local patches.
The old `src/encode/rate.rs` remains as an unreferenced file from the previous
vendor copy; the synchronized upstream crate does not include it, and this
update intentionally deletes no files.

`Cargo.toml` replaces workspace dependencies with the same pinned moq-dev
revision used by the desktop application, including moq-nvenc and moq-v4l.
PipeWire tests retain fixed negotiated format fixtures and stable transfer
constants for the supported distribution headers. `LICENSE-APACHE` and `LICENSE-MIT`
come from the source repository root.
