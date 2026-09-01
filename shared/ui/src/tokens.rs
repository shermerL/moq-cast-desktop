use egui::Color32;

/// A stable RGB design color independent of egui's runtime visuals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color {
    red: u8,
    green: u8,
    blue: u8,
}

impl Color {
    /// Creates an opaque color from red, green, and blue channels.
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// Returns the canonical uppercase hexadecimal representation.
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }
}

impl From<Color> for Color32 {
    fn from(value: Color) -> Self {
        Self::from_rgb(value.red, value.green, value.blue)
    }
}

/// The shared light-theme color contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Colors {
    /// Application canvas.
    pub canvas: Color,
    /// Window chrome and top navigation.
    pub chrome: Color,
    /// Primary raised surface.
    pub surface: Color,
    /// Muted surface.
    pub surface_muted: Color,
    /// Primary text.
    pub text: Color,
    /// Secondary text.
    pub muted: Color,
    /// Default border.
    pub border: Color,
    /// Strong border.
    pub border_strong: Color,
    /// Brand action color.
    pub brand: Color,
    /// Hovered brand color.
    pub brand_hover: Color,
    /// Pressed brand color.
    pub brand_pressed: Color,
    /// Selected background.
    pub brand_soft: Color,
    /// Pressed secondary-control background.
    pub secondary_pressed: Color,
    /// Destructive action color.
    pub danger: Color,
    /// Destructive state background.
    pub danger_soft: Color,
    /// Destructive hover background.
    pub danger_hover: Color,
    /// Destructive pressed background.
    pub danger_pressed: Color,
    /// Warning text.
    pub warning: Color,
    /// Warning background.
    pub warning_soft: Color,
    /// Informational text.
    pub info: Color,
    /// Informational background.
    pub info_soft: Color,
    /// Keyboard focus ring.
    pub focus: Color,
    /// Video-stage background.
    pub player: Color,
    /// Video-toolbar background.
    pub player_bar: Color,
    /// Primary video-toolbar text.
    pub player_text: Color,
    /// Secondary video-toolbar text.
    pub player_muted: Color,
}

/// The frozen shared light-theme colors.
pub const COLORS: Colors = Colors {
    canvas: Color::rgb(0xEC, 0xEE, 0xEF),
    chrome: Color::rgb(0xF6, 0xF7, 0xF8),
    surface: Color::rgb(0xFF, 0xFF, 0xFF),
    surface_muted: Color::rgb(0xF5, 0xF6, 0xF7),
    text: Color::rgb(0x1D, 0x22, 0x28),
    muted: Color::rgb(0x62, 0x6B, 0x75),
    border: Color::rgb(0xD9, 0xDD, 0xE1),
    border_strong: Color::rgb(0xB8, 0xC0, 0xC8),
    brand: Color::rgb(0x08, 0x7C, 0x82),
    brand_hover: Color::rgb(0x06, 0x6B, 0x70),
    brand_pressed: Color::rgb(0x05, 0x5B, 0x60),
    brand_soft: Color::rgb(0xE4, 0xF3, 0xF3),
    secondary_pressed: Color::rgb(0xE8, 0xEB, 0xED),
    danger: Color::rgb(0xB3, 0x26, 0x1E),
    danger_soft: Color::rgb(0xFC, 0xE8, 0xE6),
    danger_hover: Color::rgb(0xF9, 0xDA, 0xD7),
    danger_pressed: Color::rgb(0xF5, 0xC8, 0xC4),
    warning: Color::rgb(0x8A, 0x58, 0x00),
    warning_soft: Color::rgb(0xFF, 0xF4, 0xD6),
    info: Color::rgb(0x17, 0x4F, 0x7A),
    info_soft: Color::rgb(0xED, 0xF5, 0xFC),
    focus: Color::rgb(0x00, 0x67, 0xC0),
    player: Color::rgb(0x05, 0x06, 0x07),
    player_bar: Color::rgb(0x17, 0x19, 0x1C),
    player_text: Color::rgb(0xF7, 0xF8, 0xF9),
    player_muted: Color::rgb(0xB8, 0xBE, 0xC4),
};

/// Shared spacing tokens in logical points.
pub struct Spacing;

impl Spacing {
    /// The complete spacing scale.
    pub const ALL: [f32; 9] = [0.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 40.0, 48.0];
    /// No spacing.
    pub const NONE: f32 = 0.0;
    /// Extra-small spacing.
    pub const XS: f32 = 4.0;
    /// Small spacing.
    pub const SM: f32 = 8.0;
    /// Compact spacing.
    pub const MD: f32 = 12.0;
    /// Standard spacing.
    pub const LG: f32 = 16.0;
    /// Section spacing.
    pub const XL: f32 = 24.0;
    /// Large section spacing.
    pub const XXL: f32 = 32.0;
    /// Extra-large spacing.
    pub const XXXL: f32 = 40.0;
    /// Maximum layout spacing.
    pub const MAX: f32 = 48.0;
}

/// Shared corner-radius tokens in logical points.
pub struct Radius;

impl Radius {
    /// The complete corner-radius scale.
    pub const ALL: [f32; 4] = [0.0, 4.0, 6.0, 8.0];
    /// Square corners.
    pub const NONE: f32 = 0.0;
    /// Small corners.
    pub const SM: f32 = 4.0;
    /// Standard control corners.
    pub const MD: f32 = 6.0;
    /// Maximum shared corners.
    pub const LG: f32 = 8.0;
}

/// Shared component and layout sizes in logical points.
pub struct Size;

impl Size {
    /// App bar height.
    pub const APP_BAR: f32 = 68.0;
    /// Compact two-row app bar height.
    pub const APP_BAR_COMPACT: f32 = 108.0;
    /// Navigation item height.
    pub const NAV: f32 = 48.0;
    /// Control and icon hit-target edge.
    pub const CONTROL: f32 = 40.0;
    /// Switch visual width and height.
    pub const SWITCH: [f32; 2] = [44.0, 26.0];
    /// Badge height.
    pub const BADGE: f32 = 24.0;
    /// Settings row minimum height.
    pub const SETTING_ROW: f32 = 68.0;
    /// Device row minimum height.
    pub const DEVICE_ROW: f32 = 72.0;
    /// General page maximum width.
    pub const PAGE_MAX: f32 = 1180.0;
    /// Wide-window horizontal page padding.
    pub const PAGE_HORIZONTAL_WIDE: f32 = 40.0;
    /// Narrow-window horizontal page padding.
    pub const PAGE_HORIZONTAL_NARROW: f32 = 24.0;
    /// Wide-window top page padding.
    pub const PAGE_TOP_WIDE: f32 = 32.0;
    /// Narrow-window top page padding.
    pub const PAGE_TOP_NARROW: f32 = 24.0;
    /// Bottom page padding.
    pub const PAGE_BOTTOM: f32 = 48.0;
    /// Page-header minimum height.
    pub const PAGE_HEADER_MIN: f32 = 74.0;
    /// Gap below the page header.
    pub const PAGE_HEADER_SPACING: f32 = 32.0;
    /// Settings page maximum width.
    pub const SETTINGS_MAX: f32 = 860.0;
    /// Gap between settings groups.
    pub const SETTINGS_GROUP_SPACING: f32 = 32.0;
    /// Width below which a settings row stacks its trailing control.
    pub const SETTINGS_BREAKPOINT: f32 = 720.0;
    /// Watch page maximum width.
    pub const WATCH_MAX: f32 = 960.0;
    /// Gap between the player stage and toolbar.
    pub const PLAYER_SPACING: f32 = 0.0;
    /// Horizontal player-toolbar item and action gap.
    pub const PLAYER_TOOLBAR_ITEM_SPACING: f32 = 8.0;
    /// Player stage width and height ratio.
    pub const PLAYER_ASPECT: [f32; 2] = [16.0, 9.0];
    /// State-panel minimum height.
    pub const STATE_PANEL_MIN: f32 = 240.0;
    /// Dialog maximum width.
    pub const DIALOG_MAX: f32 = 440.0;
    /// Dialog inner padding.
    pub const DIALOG_PADDING: f32 = 24.0;
    /// Gap between dialog actions.
    pub const DIALOG_ACTION_SPACING: f32 = 8.0;
    /// Player-toolbar height.
    pub const PLAYER_TOOLBAR: f32 = 52.0;
    /// Minimum supported viewport width and height.
    pub const MIN_VIEWPORT: [f32; 2] = [680.0, 640.0];
    /// Windows catalog viewport preset.
    pub const VIEWPORT_WINDOWS: [f32; 2] = [1440.0, 900.0];
    /// Linux catalog viewport preset.
    pub const VIEWPORT_LINUX: [f32; 2] = [1024.0, 768.0];
    /// macOS catalog viewport preset.
    pub const VIEWPORT_MACOS: [f32; 2] = [720.0, 900.0];
    /// Nearby device-list width.
    pub const NEARBY_LIST: f32 = 360.0;
    /// Nearby workspace minimum width.
    pub const WORKSPACE_MIN: f32 = 360.0;
    /// Breakpoint for switching Nearby from split to stacked layout.
    pub const SPLIT_BREAKPOINT: f32 = 920.0;
    /// Default border width.
    pub const BORDER: f32 = 1.0;
    /// Focus-ring width.
    pub const FOCUS: f32 = 2.0;
    /// Focus-ring outer offset.
    pub const FOCUS_OUTSET: f32 = 2.0;
    /// Selected navigation underline height.
    pub const NAV_UNDERLINE: f32 = 3.0;
    /// Disabled content alpha.
    pub const DISABLED_ALPHA: f32 = 0.55;
}
