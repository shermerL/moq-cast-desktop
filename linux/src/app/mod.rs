//! Native UI shell and user-facing state.
mod command;
mod desktop;
mod device;
mod diagnostics;
mod locale;
mod pages;
mod player;
mod snapshot;
mod theme;
pub use crate::publish::source::{CaptureSource, SourceCatalog};
use crate::runtime::{PlaybackFrameIdentity, RuntimeHandle, RuntimeStartError};
pub use command::UserCommand;
use eframe::egui::{self, Color32, Frame};
pub use locale::Locale;
use moqcast_ui::PlayerFrameSample;
pub use snapshot::{
    AppSnapshot, DialRole, DiscoveredPeer, DiscoveryState, MediaState, PeerDiscoveryState,
    PeerSnapshot, RemoteAudioPhase, RemoteAudioSnapshot, RemoteScreenSnapshot, RemoteVideoSnapshot,
    ScreenAvailability, StateError, TransportState,
};
const STORAGE_LOCALE: &str = "moqcast.locale";
const STORAGE_THEME: &str = "moqcast.theme";
const STORAGE_SYSTEM_AUDIO: &str = "moqcast.system-audio";
const STORAGE_DETAILED_DIAGNOSTICS: &str = "moqcast.detailed-diagnostics";
const STORAGE_DEVELOPER_MODE: &str = "moqcast.developer-mode";
fn parse_stored_bool(value: Option<String>) -> bool {
    value.as_deref() == Some("true")
}
/// A workspace within the desktop window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    /// Nearby devices and the local sharing entry.
    #[default]
    Nearby,
    /// Capture selection and publication controls.
    ScreenShare,
    /// Independent playback canvas.
    Watch,
}
/// The native MoQCast desktop application.
pub struct MoqCastApp {
    page: Page,
    selected_peer: Option<String>,
    settings_open: bool,
    confirm_nearby_off: bool,
    theme_preference: egui::ThemePreference,
    locale: Locale,
    system_audio: bool,
    selected_source: Option<CaptureSource>,
    developer_mode: bool,
    local_device_name: String,
    diagnostics: diagnostics::DiagnosticsUi,
    runtime: RuntimeHandle,
    command_error: Option<String>,
    playback_texture: Option<egui::TextureHandle>,
    playback_identity: Option<PlaybackFrameIdentity>,
    player: player::LivePlayer,
}
impl MoqCastApp {
    /// Create the UI and its owned background runtime.
    pub fn new(
        creation_context: &eframe::CreationContext<'_>,
        diagnostics: moqcast_diagnostics::Handle,
    ) -> Result<Self, RuntimeStartError> {
        theme::configure(&creation_context.egui_ctx);
        let locale = creation_context
            .storage
            .and_then(|storage| storage.get_string(STORAGE_LOCALE))
            .and_then(|value| Locale::parse(&value))
            .unwrap_or_default();
        let system_audio = parse_stored_bool(
            creation_context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_SYSTEM_AUDIO)),
        );
        let detailed_diagnostics = parse_stored_bool(
            creation_context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_DETAILED_DIAGNOSTICS)),
        );
        let developer_mode = parse_stored_bool(
            creation_context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_DEVELOPER_MODE)),
        );

        let theme_preference = creation_context
            .storage
            .and_then(|storage| storage.get_string(STORAGE_THEME))
            .map(|value| match value.as_str() {
                "light" => egui::ThemePreference::Light,
                "dark" => egui::ThemePreference::Dark,
                _ => egui::ThemePreference::System,
            })
            .unwrap_or(egui::ThemePreference::System);
        creation_context.egui_ctx.set_theme(theme_preference);
        Ok(Self {
            theme_preference,
            settings_open: false,
            confirm_nearby_off: false,
            page: Page::default(),
            selected_peer: None,
            locale,
            system_audio,
            selected_source: None,
            developer_mode,
            local_device_name: device::name(),
            diagnostics: diagnostics::DiagnosticsUi::new(diagnostics, detailed_diagnostics),
            runtime: RuntimeHandle::start()?,
            command_error: None,
            playback_texture: None,
            playback_identity: None,
            player: player::LivePlayer::default(),
        })
    }

    fn send(&mut self, command: UserCommand) {
        self.command_error = self.runtime.try_send(command).err().map(|error| {
            tracing::warn!(stage = "ui-command", %error, "user command could not be queued");
            error.to_string()
        });
    }

    fn text<'a>(&self, zh: &'a str, en: &'a str) -> &'a str {
        self.locale.text(zh, en)
    }
    fn watch(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        match pages::watch::show(
            ui,
            self.locale,
            snapshot,
            playback_frame_sample(self.playback_identity),
            self.playback_texture.as_ref(),
            &mut self.player,
        ) {
            Some(pages::watch::WatchAction::Command(command)) => self.send(command),
            Some(pages::watch::WatchAction::OpenNearby) => self.page = Page::Nearby,
            None => {}
        }
    }
}
impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        let snapshot = self.runtime.snapshot();
        if let Some(frame) = self.runtime.playback_frame()
            && Some(frame.identity) != self.playback_identity
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
            self.playback_identity = Some(frame.identity);
        }
        if !matches!(
            snapshot.media,
            MediaState::Viewing { .. } | MediaState::PreparingView { .. }
        ) {
            self.playback_texture = None;
            self.playback_identity = None;
        }

        let viewing = matches!(snapshot.media, MediaState::Viewing { .. });
        let fullscreen = self.player.reconcile_fullscreen(&context, viewing);
        if fullscreen {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(Color32::BLACK))
                .show(ui, |ui| self.watch(ui, &snapshot));
        } else {
            self.desktop_shell(ui, &snapshot);
        }
        if self.developer_mode {
            let visible = self.diagnostics.visible();
            self.diagnostics.show_window(&context, self.locale);
            if visible && !self.diagnostics.visible() {
                self.settings_open = true;
            }
        }
        context.request_repaint_after(std::time::Duration::from_millis(if viewing {
            33
        } else {
            250
        }));
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(
            STORAGE_THEME,
            match self.theme_preference {
                egui::ThemePreference::Light => "light",
                egui::ThemePreference::Dark => "dark",
                egui::ThemePreference::System => "system",
            }
            .to_owned(),
        );
        storage.set_string(STORAGE_LOCALE, self.locale.as_str().to_owned());
        storage.set_string(
            STORAGE_SYSTEM_AUDIO,
            if self.system_audio { "true" } else { "false" }.to_owned(),
        );
        storage.set_string(
            STORAGE_DETAILED_DIAGNOSTICS,
            if self.diagnostics.detailed() {
                "true"
            } else {
                "false"
            }
            .to_owned(),
        );
        storage.set_string(
            STORAGE_DEVELOPER_MODE,
            if self.developer_mode { "true" } else { "false" }.to_owned(),
        );
    }
}

fn playback_frame_sample(identity: Option<PlaybackFrameIdentity>) -> Option<PlayerFrameSample> {
    identity.map(|identity| PlayerFrameSample {
        view_generation: identity.view_generation,
        decoder_generation: identity.decoder_generation,
        sequence: identity.sequence,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_audio_is_off_unless_explicitly_enabled() {
        assert!(!parse_stored_bool(None));
        assert!(!parse_stored_bool(Some("false".to_owned())));
        assert!(!parse_stored_bool(Some("invalid".to_owned())));
        assert!(parse_stored_bool(Some("true".to_owned())));
    }

    #[test]
    fn configured_fonts_cover_simplified_chinese() {
        let context = egui::Context::default();
        theme::configure(&context);

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

#[cfg(test)]
mod layout_tests;
