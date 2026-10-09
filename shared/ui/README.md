# MoQCast shared UI foundation

`moqcast-ui` contains business-neutral egui 0.36.1 presentation primitives shared by the full Windows, Linux, and macOS desktop applications.

It owns design tokens, semantic typography, interaction-state resolution, centered role-based page shells, caller-data-only components, and the visual catalog. Platform crates continue to own windows, page composition, locale, discovery, media, permissions, diagnostics, and product lifecycle.

Share pages reserve `page_actions` inside the finite page viewport before rendering their scrollable source and status content. Keep the page itself outside any outer scroll area so long lists cannot move the start/stop action off screen.

Run the catalog locally:

```sh
cargo run --locked --example catalog
```

The catalog provides 1440 x 900 Windows, 1024 x 768 Linux, 720 x 900 macOS, and 680 x 640 minimum viewport presets. It includes Chinese and English long copy, static platform capability fixtures, and production-component interaction states. It is a design and component verification surface, not platform page code.

The dedicated Shared UI workflow owns formatting, tests, Clippy, catalog compilation, and rustdoc. Platform workflows trigger on `shared/ui/**` changes and compile their applications as consumer-integration gates without repeating the shared crate's full suite.
