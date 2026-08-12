//! Native UI shell and user-facing state.

mod command;
mod components;
mod locale;
mod pages;
mod snapshot;
mod theme;

pub use command::UserCommand;
pub use locale::Locale;
pub use snapshot::{
    AppSnapshot, DialRole, DiscoveredPeer, DiscoveryState, MediaState, PeerDiscoveryState,
    PeerSnapshot, RemoteScreenSnapshot, ScreenAvailability, StateError, TransportState,
};

use eframe::egui::{self, Color32, Frame, Margin, RichText, Stroke};

use crate::runtime::{RuntimeHandle, RuntimeStartError};

const STORAGE_LOCALE: &str = "moqcast.locale";

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
        theme::configure(&creation_context.egui_ctx);
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
        let compact = ui.available_width() < 760.0;
        let width = if compact { 156.0 } else { 220.0 };
        egui::Panel::left("navigation")
            .exact_size(width)
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(Margin::same(if compact { 12 } else { 20 })),
            )
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("MoQCast")
                        .size(if compact { 21.0 } else { 26.0 })
                        .strong()
                        .color(theme::BRAND),
                );
                if !compact {
                    ui.label(
                        RichText::new(self.locale.desktop())
                            .size(12.0)
                            .color(theme::MUTED),
                    );
                }
                ui.add_space(if compact { 24.0 } else { 34.0 });

                nav_button(
                    ui,
                    &mut self.page,
                    Page::Nearby,
                    self.locale.nearby(),
                    true,
                    width - if compact { 24.0 } else { 40.0 },
                );
                nav_button(
                    ui,
                    &mut self.page,
                    Page::ScreenShare,
                    self.locale.screen_share(),
                    snapshot.has_mesh_session() || snapshot.media != MediaState::Idle,
                    width - if compact { 24.0 } else { 40.0 },
                );
                nav_button(
                    ui,
                    &mut self.page,
                    Page::Settings,
                    self.locale.settings(),
                    true,
                    width - if compact { 24.0 } else { 40.0 },
                );

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(RichText::new("MoQ / QUIC").size(11.0).color(theme::MUTED));
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
            .frame(Frame::new().fill(theme::PAGE).inner_margin(Margin::same(
                if ui.available_width() < 760.0 { 20 } else { 32 },
            )))
            .show(ui, |ui| {
                let (title, description) = match self.page {
                    Page::Nearby => (self.locale.nearby(), self.locale.nearby_description()),
                    Page::ScreenShare => {
                        (self.locale.screen_share(), self.locale.share_description())
                    }
                    Page::Settings => (self.locale.settings(), self.locale.settings_description()),
                };
                components::page_header(ui, title, description, self.locale, &snapshot);

                let error = self
                    .command_error
                    .as_deref()
                    .or(snapshot.last_error.as_deref());
                let mut command = if let Some(error) = error {
                    let recovery = match self.page {
                        Page::Nearby if snapshot.discovery == DiscoveryState::Error => {
                            Some((self.locale.retry(), UserCommand::RetryDiscovery))
                        }
                        Page::Nearby => None,
                        Page::ScreenShare
                            if snapshot.has_mesh_session()
                                && snapshot.discovery != DiscoveryState::Error
                                && matches!(snapshot.media, MediaState::Idle) =>
                        {
                            Some((self.locale.retry(), UserCommand::StartScreenShare))
                        }
                        Page::ScreenShare | Page::Settings => None,
                    };
                    let clicked = components::error_banner(
                        ui,
                        error,
                        recovery.as_ref().map(|(label, _)| *label),
                    );
                    ui.add_space(16.0);
                    clicked.then(|| recovery.expect("a recovery action was rendered").1)
                } else {
                    None
                };

                let page_command = match self.page {
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
                command = command.or(page_command);

                if let Some(command) = command {
                    if matches!(command, UserCommand::StartWatching { .. }) {
                        self.page = Page::ScreenShare;
                    }
                    self.send(command);
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

fn nav_button(
    ui: &mut egui::Ui,
    page: &mut Page,
    target: Page,
    label: &str,
    enabled: bool,
    width: f32,
) {
    let active = *page == target;
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).size(15.0).color(if active {
            theme::BRAND_DARK
        } else {
            theme::TEXT
        }))
        .fill(if active {
            theme::BRAND_SOFT
        } else {
            Color32::TRANSPARENT
        })
        .stroke(if active {
            Stroke::new(1.0, theme::BRAND_SOFT)
        } else {
            Stroke::NONE
        })
        .corner_radius(theme::RADIUS)
        .min_size(egui::vec2(width, 42.0)),
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
