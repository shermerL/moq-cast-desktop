# Shared UI Icon Assets

## First-Party MoQCast Icons

All 15 desktop v2 UI icons are first-party MoQCast outline icons drawn from a blank `24×24` coordinate grid with `stroke-width="2"`, `stroke-linecap="round"`, `stroke-linejoin="round"`, `fill="none"`, and `stroke="currentColor"`.

Each SVG is rasterized offline at `72×72` (3× HiDPI) into a pure-white (`RGB=255,255,255` for `a > 0`, `RGB=0,0,0` for `a == 0`) RGBA alpha mask (`*.rgba`, 20,736 bytes) so `egui` can tint icons dynamically with `v2::Palette` colors without adding runtime SVG dependencies.

All bundled third-party Lucide/Feather icon assets and their exclusive `LICENSE.txt` notice have been retired from Desktop. Font notices (Noto Sans SC SIL OFL) and crate/library notices remain unchanged in their respective locations.

| `v2::Icon` variant | Local file | Semantic role | Origin |
| --- | --- | --- | --- |
| `Icon::Back` | `back` | Navigate back | First-party MoQCast Sample C (Stage Return U-Turn Hook) |
| `Icon::Close` | `x` | Close dialog / modal | First-party MoQCast Icon Set v2 |
| `Icon::ChevronRight` | `chevron-right` | Collapsed disclosure | First-party MoQCast Icon Set v2 |
| `Icon::ChevronDown` | `chevron-down` | Expanded disclosure / select | First-party MoQCast Icon Set v2 |
| `Icon::Settings` | `settings` | Open settings | First-party MoQCast Icon Set v2 |
| `Icon::Information` | `info-circle` | Device details / playback info | First-party MoQCast Icon Set v2 |
| `Icon::Fullscreen` | `maximize` | Enter fullscreen | First-party MoQCast Icon Set v2 |
| `Icon::ExitFullscreen` | `minimize` | Exit fullscreen | First-party MoQCast Icon Set v2 |
| `Icon::VolumeHigh` | `volume-high` | Volume > 50% | First-party MoQCast Icon Set v2 |
| `Icon::VolumeLow` | `volume-low` | Volume 1–50% | First-party MoQCast Icon Set v2 |
| `Icon::VolumeMuted` | `volume-muted` | Active audio locally muted (0%) | First-party MoQCast Icon Set v2 |
| `Icon::VolumeOff` | `volume-off` | No playable audio track | First-party MoQCast Icon Set v2 (V-B) |
| `Icon::AlertTriangle` | `alert-triangle` | Warning status indicator | First-party MoQCast Icon Set v2 |
| `Icon::AlertCircle` | `alert-circle` | Error status indicator | First-party MoQCast Icon Set v2 |
| `Icon::Monitor` | `device-desktop` | Desktop display / peer identity | First-party MoQCast Icon Set v2 |
