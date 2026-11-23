# Shared UI Icon Assets

All desktop v2 icons come from [Lucide Icons](https://lucide.dev/) (ISC License; see `LICENSE.txt`), using a uniform 24×24 outline grid with 2px rounded strokes (`stroke-linecap="round"`, `stroke-linejoin="round"`).

Each SVG is rasterized offline at 72×72 (3× HiDPI) into a pure-white (`RGB=255,255,255`) RGBA alpha mask (`*.rgba`) so `egui` can tint icons dynamically with `v2::Palette` colors without adding runtime SVG dependencies.

| Local file | Upstream Lucide icon |
| --- | --- |
| `chevron-left` / `chevron-right` / `chevron-down` | `chevron-left` / `chevron-right` / `chevron-down` |
| `x` | `x` |
| `settings` | `settings` |
| `info-circle` | `info` |
| `maximize` / `minimize` | `maximize` / `minimize` |
| `volume-high` / `volume-low` / `volume-muted` / `volume-off` | `volume-2` / `volume-1` / `volume-x` / `volume-off` |
| `alert-triangle` / `alert-circle` | `triangle-alert` / `circle-alert` |
| `device-desktop` | `monitor` |
