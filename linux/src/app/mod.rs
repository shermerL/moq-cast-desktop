//! Native UI shell and user-facing state.

mod command;
mod locale;
mod pages;
mod snapshot;

pub use command::UserCommand;
pub use locale::Locale;
pub use snapshot::{
    AppSnapshot, DiscoveredPeer, DiscoveryState, MediaState, PeerDiscoveryState, PeerSnapshot,
    RemoteScreenSnapshot, ScreenAvailability, StateError, TransportState,
};

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke};

use crate::runtime::{RuntimeHandle, RuntimeStartError};

const STORAGE_LOCALE: &str = "moqcast.locale";
const BACKGROUND: Color32 = Color32::from_rgb(246, 249, 248);
const SURFACE: Color32 = Color32::from_rgb(255, 255, 255);
const BORDER: Color32 = Color32::from_rgb(222, 230, 227);
const TEAL: Color32 = Color32::from_rgb(0, 126, 115);
const TEAL_SOFT: Color32 = Color32::from_rgb(218, 242, 238);
const TEXT: Color32 = Color32::from_rgb(28, 39, 37);
const MUTED: Color32 = Color32::from_rgb(99, 116, 112);

/// A top-level page in the desktop application.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    /// Nearby peer discovery and connection.
    #[default]
    Nearby,
    /// Screen source and publish controls.
    ScreenShare,
    /// Local application settings.
    Settings,
}

/// The native MoQCast desktop application.
pub struct MoqCastApp {
    page: Page,
    locale: Locale,
    runtime: RuntimeHandle,
    command_error: Option<String>,
    playback_texture: Option<egui::TextureHandle>,
    playback_sequence: u64,
}

impl MoqCastApp {
    /// Create the UI and its owned background runtime.
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Result<Self, RuntimeStartError> {
        configure_fonts(&creation_context.egui_ctx);
        configure_style(&creation_context.egui_ctx);
        let locale = creation_context
            .storage
            .and_then(|storage| storage.get_string(STORAGE_LOCALE))
            .and_then(|value| Locale::parse(&value))
            .unwrap_or_default();

        Ok(Self {
            page: Page::default(),
            locale,
            runtime: RuntimeHandle::start()?,
            command_error: None,
            playback_texture: None,
            playback_sequence: 0,
        })
    }

    fn send(&mut self, command: UserCommand) {
        self.command_error = self
            .runtime
            .try_send(command)
            .err()
            .map(|error| error.to_string());
    }

    fn navigation(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        egui::Panel::left("navigation")
            .exact_size(224.0)
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::same(20)),
            )
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.label(RichText::new("MoQCast").size(26.0).strong().color(TEAL));
                ui.label(RichText::new(self.locale.desktop()).size(12.0).color(MUTED));
                ui.add_space(34.0);

                nav_button(ui, &mut self.page, Page::Nearby, self.locale.nearby(), true);
                nav_button(
                    ui,
                    &mut self.page,
                    Page::ScreenShare,
                    self.locale.screen_share(),
                    snapshot.has_mesh_session(),
                );
                nav_button(
                    ui,
                    &mut self.page,
                    Page::Settings,
                    self.locale.settings(),
                    true,
                );

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(RichText::new("MoQ / QUIC").size(11.0).color(MUTED));
                });
            });
    }
}

impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        let snapshot = self.runtime.snapshot();
        if let Some(frame) = self.runtime.playback_frame()
            && frame.sequence != self.playback_sequence
        {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
            if let Some(texture) = self.playback_texture.as_mut() {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.playback_texture = Some(context.load_texture(
                    "remote-screen",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            self.playback_sequence = frame.sequence;
        }
        if !matches!(
            snapshot.media,
            MediaState::Viewing { .. } | MediaState::PreparingView { .. }
        ) {
            self.playback_texture = None;
        }
        self.navigation(ui, &snapshot);

        egui::CentralPanel::default()
            .frame(Frame::new().fill(BACKGROUND).inner_margin(Margin::same(32)))
            .show(ui, |ui| {
                let command = match self.page {
                    Page::Nearby => pages::nearby::show(ui, self.locale, &snapshot),
                    Page::ScreenShare => pages::screen_share::show(
                        ui,
                        self.locale,
                        &snapshot,
                        self.playback_texture.as_ref(),
                    ),
                    Page::Settings => {
                        if let Some(locale) = pages::settings::show(ui, self.locale) {
                            self.locale = locale;
                        }
                        None
                    }
                };

                if let Some(command) = command {
                    if matches!(command, UserCommand::StartWatching { .. }) {
                        self.page = Page::ScreenShare;
                    }
                    self.send(command);
                }

                if let Some(error) = self
                    .command_error
                    .as_deref()
                    .or(snapshot.last_error.as_deref())
                {
                    ui.add_space(16.0);
                    ui.colored_label(Color32::from_rgb(176, 50, 50), error);
                }
            });

        let repaint = if matches!(snapshot.media, MediaState::Viewing { .. }) {
            33
        } else {
            250
        };
        context.request_repaint_after(std::time::Duration::from_millis(repaint));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_LOCALE, self.locale.as_str().to_owned());
    }
}

fn nav_button(ui: &mut egui::Ui, page: &mut Page, target: Page, label: &str, enabled: bool) {
    let active = *page == target;
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(label)
                .size(15.0)
                .color(if active { TEAL } else { TEXT }),
        )
        .fill(if active {
            TEAL_SOFT
        } else {
            Color32::TRANSPARENT
        })
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(7))
        .min_size(egui::vec2(184.0, 42.0)),
    );
    if response.clicked() {
        *page = target;
    }
    ui.add_space(4.0);
}

fn configure_fonts(context: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};

    context.add_font(FontInsert::new(
        "Noto Sans SC",
        egui::FontData::from_static(include_bytes!("../../assets/fonts/NotoSansSC-Regular.otf")),
        vec![
            InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: FontPriority::Lowest,
            },
            InsertFontFamily {
                family: egui::FontFamily::Monospace,
                priority: FontPriority::Lowest,
            },
        ],
    ));
}

fn configure_style(context: &egui::Context) {
    let mut style = (*context.style_of(egui::Theme::Light)).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = BACKGROUND;
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(7);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(7);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(7);
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    context.set_style_of(egui::Theme::Light, style);
}

pub(super) fn section_frame() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(22))
}

pub(super) fn heading(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).size(28.0).strong().color(TEXT));
    if !description.is_empty() {
        ui.label(RichText::new(description).size(14.0).color(MUTED));
    }
    ui.add_space(22.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_fonts_cover_simplified_chinese() {
        let context = egui::Context::default();
        configure_fonts(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            ui.fonts_mut(|fonts| {
                assert!(
                    fonts.has_glyphs(&egui::FontId::proportional(14.0), "附近设备屏幕共享设置")
                );
            });
        });
        output.textures_delta.clear();
    }
}
