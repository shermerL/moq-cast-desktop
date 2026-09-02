//! Business-neutral egui visual primitives for MoQCast desktop applications.

#![warn(missing_docs)]

mod components;
mod interaction;
mod theme;
mod tokens;
mod typography;

pub use components::{
    BadgeTone, ButtonSpec, CheckboxSpec, DetailRowSpec, DeviceBadgeSpec, DeviceListItemSpec,
    DeviceListSpec, DeviceRowSpec, DialogClosePolicy, DialogResponse, DialogSpec, IconButtonSpec,
    NavItemSpec, PageWidth, PlayerRects, PlayerSurfaceResponse, SelectError, SelectSpec,
    SettingRowSpec, StatePanelKind, StatePanelSpec, SwitchSpec, app_bar_content_rect, checkbox,
    control_button, danger_button, detail_row, device_list, device_row, dialog, icon_button,
    major_section_break, nav_item, page_content_rect, page_header, page_horizontal_inset,
    page_shell, player_button, player_icon_button, player_rects, player_stage, player_stage_at,
    player_surface, player_toolbar, player_toolbar_at, primary_button, secondary_button,
    section_header, select, setting_row, state_panel, status_badge, status_strip, switch,
};
pub use interaction::{ControlRole, Interaction, ResolvedVisual, resolve_control_visual};
pub use theme::{Theme, install_ui_font};
pub use tokens::{COLORS, Color, Colors, Radius, Size, Spacing};
pub use typography::{TypographyRole, TypographySpec, typography, typography_spec};

/// The embedded variable UI font used by the catalog and available to platform callers.
pub const NOTO_SANS_SC: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-VF.otf");
