# Shared UI Icon Assets

## First-Party MoQCast Icons

| Local file | Origin |
| --- | --- |
| `back` | First-party MoQCast back control icon (Sample C: Stage Return U-Turn Hook), drawn from a blank 24×24 coordinate canvas (`stroke-width="2"`, round caps/joins). |

## Bundled Third-Party Icons (Lucide)

The remaining bundled [Lucide icons](https://lucide.dev/) use ISC and, for Feather-derived icons, MIT. All three desktop apps embed the [full notices](LICENSE.txt) and display them under Settings → Icon licenses. The icons use a 24×24 outline grid with 2px rounded strokes (`stroke-linecap="round"`, `stroke-linejoin="round"`).

Each SVG is rasterized offline at 72×72 (3× HiDPI) into a pure-white (`RGB=255,255,255`) RGBA alpha mask (`*.rgba`) so `egui` can tint icons dynamically with `v2::Palette` colors without adding runtime SVG dependencies.

| Local file | Upstream Lucide icon |
| --- | --- |
| `chevron-right` / `chevron-down` | `chevron-right` / `chevron-down` |
| `x` | `x` |
| `settings` | `settings` |
| `info-circle` | `info` |
| `maximize` / `minimize` | `maximize` / `minimize` |
| `volume-high` / `volume-low` / `volume-muted` / `volume-off` | `volume-2` / `volume-1` / `volume-x` / `volume-off` |
| `alert-triangle` / `alert-circle` | `triangle-alert` / `circle-alert` |
| `device-desktop` | `monitor` |
