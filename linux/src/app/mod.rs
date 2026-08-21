//! Native UI shell and user-facing state.

mod command;
mod components;
mod locale;
mod pages;
mod player;
mod snapshot;
mod theme;

pub use command::UserCommand;
pub use locale::Locale;
pub use snapshot::{
    AppSnapshot, DialRole, DiscoveredPeer, DiscoveryState, MediaState, PeerDiscoveryState,
    PeerSnapshot, RemoteScreenSnapshot, ScreenAvailability, StateError, TransportState,
};

use eframe::egui::{self, Color32, Frame, Margin, RichText, Stroke};

use crate::runtime::{PlaybackFrameIdentity, RuntimeHandle, RuntimeStartError};

const STORAGE_LOCALE: &str = "moqcast.locale";
const STORAGE_SYSTEM_AUDIO: &str = "moqcast.system-audio";
const CONTENT_MAX_WIDTH: f32 = 1040.0;
const DEVICE_WORKSPACE_SPLIT_WIDTH: f32 = 900.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeviceWorkspaceLayout {
    Split,
    Single,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ShellLayout {
    content_width: f32,
    page_padding: f32,
    compact_app_bar: bool,
    device_workspace: DeviceWorkspaceLayout,
}

fn shell_layout(window_width: f32) -> ShellLayout {
    let compact_app_bar = window_width < 820.0;
    let page_padding = if compact_app_bar { 24.0 } else { 32.0 };
    ShellLayout {
        content_width: (window_width - page_padding * 2.0).clamp(1.0, CONTENT_MAX_WIDTH),
        page_padding,
        compact_app_bar,
        device_workspace: if window_width >= DEVICE_WORKSPACE_SPLIT_WIDTH {
            DeviceWorkspaceLayout::Split
        } else {
            DeviceWorkspaceLayout::Single
        },
    }
}

fn parse_system_audio(value: Option<String>) -> bool {
    value.as_deref() == Some("true")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaStopAction {
    Publish,
    View,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShellControls {
    navigation_visible: bool,
    stop: Option<MediaStopAction>,
    stop_enabled: bool,
}

fn shell_controls(media: &MediaState) -> ShellControls {
    let (stop, stop_enabled) = match media {
        MediaState::Idle => (None, false),
        MediaState::PreparingPublish => (Some(MediaStopAction::Publish), false),
        MediaState::Publishing => (Some(MediaStopAction::Publish), true),
        MediaState::StoppingPublish => (Some(MediaStopAction::Publish), false),
        MediaState::PreparingView { .. } | MediaState::Viewing { .. } => {
            (Some(MediaStopAction::View), true)
        }
        MediaState::StoppingView { .. } => (Some(MediaStopAction::View), false),
    };
    ShellControls {
        navigation_visible: true,
        stop,
        stop_enabled,
    }
}

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
    selected_peer: Option<String>,
    locale: Locale,
    system_audio: bool,
    runtime: RuntimeHandle,
    command_error: Option<String>,
    playback_texture: Option<egui::TextureHandle>,
    playback_identity: Option<PlaybackFrameIdentity>,
    player: player::LivePlayer,
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
        let system_audio = parse_system_audio(
            creation_context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_SYSTEM_AUDIO)),
        );

        Ok(Self {
            page: Page::default(),
            selected_peer: None,
            locale,
            system_audio,
            runtime: RuntimeHandle::start()?,
            command_error: None,
            playback_texture: None,
            playback_identity: None,
            player: player::LivePlayer::default(),
        })
    }

    fn send(&mut self, command: UserCommand) {
        self.command_error = self
            .runtime
            .try_send(command)
            .err()
            .map(|error| error.to_string());
    }

    fn app_bar(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &AppSnapshot,
        compact: bool,
        controls: ShellControls,
    ) {
        egui::Panel::top("app-bar")
            .exact_size(if compact { 104.0 } else { 64.0 })
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(Margin::symmetric(24, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("MoQCast")
                            .size(20.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                    if !compact && controls.navigation_visible {
                        ui.add_space(18.0);
                        view_switcher(ui, &mut self.page, self.locale);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        components::mesh_summary(ui, self.locale, snapshot);
                    });
                });
                if compact && controls.navigation_visible {
                    ui.add_space(6.0);
                    view_switcher(ui, &mut self.page, self.locale);
                }
            });
    }

    fn active_media_bar(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &AppSnapshot,
        controls: ShellControls,
    ) {
        let Some(stop) = controls.stop else {
            return;
        };
        let (status, tone, stop_label, command) = match stop {
            MediaStopAction::Publish => (
                match snapshot.media {
                    MediaState::PreparingPublish => self.locale.preparing_share(),
                    MediaState::StoppingPublish => self.locale.stopping_share(),
                    _ => self.locale.sharing_screen(),
                },
                if snapshot.media == MediaState::Publishing {
                    components::BadgeTone::Success
                } else {
                    components::BadgeTone::Info
                },
                self.locale.stop_sharing(),
                UserCommand::StopScreenShare,
            ),
            MediaStopAction::View => (
                match snapshot.media {
                    MediaState::PreparingView { .. } => self.locale.preparing_view(),
                    MediaState::StoppingView { .. } => self.locale.stopping_view(),
                    _ => self.locale.viewing_screen(),
                },
                if matches!(snapshot.media, MediaState::Viewing { .. }) {
                    components::BadgeTone::Success
                } else {
                    components::BadgeTone::Info
                },
                self.locale.stop_watching(),
                UserCommand::StopWatching,
            ),
        };

        egui::Panel::top("active-media")
            .exact_size(52.0)
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(Margin::symmetric(24, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    components::status_line(ui, status, tone);
                    ui.label(
                        RichText::new(self.locale.media_keeps_mesh())
                            .size(11.0)
                            .color(theme::MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if components::danger_button(ui, stop_label, controls.stop_enabled)
                            .clicked()
                        {
                            self.send(command);
                        }
                    });
                });
            });
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
                .show(ui, |ui| {
                    if matches!(
                        snapshot.media,
                        MediaState::Viewing { .. } | MediaState::StoppingView { .. }
                    ) && let Some(command) = pages::screen_share::show(
                        ui,
                        self.locale,
                        &snapshot,
                        self.playback_texture.as_ref(),
                        &mut self.player,
                        &mut self.system_audio,
                    ) {
                        self.send(command);
                    }
                });
            context.request_repaint_after(std::time::Duration::from_millis(33));
            return;
        }
        let layout = shell_layout(ui.available_width());
        let controls = shell_controls(&snapshot.media);
        self.app_bar(ui, &snapshot, layout.compact_app_bar, controls);
        self.active_media_bar(ui, &snapshot, controls);

        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::PAGE))
            .show(ui, |ui| {
                let available = ui.available_rect_before_wrap();
                let content = egui::Rect::from_min_size(
                    egui::pos2(
                        available.center().x - layout.content_width / 2.0,
                        available.top() + layout.page_padding,
                    ),
                    egui::vec2(
                        layout.content_width,
                        (available.height() - layout.page_padding * 2.0).max(1.0),
                    ),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                    ui.set_width(layout.content_width);
                    let (title, description) = match self.page {
                        Page::Nearby => (self.locale.nearby(), self.locale.nearby_description()),
                        Page::ScreenShare => {
                            (self.locale.screen_share(), self.locale.share_description())
                        }
                        Page::Settings => {
                            (self.locale.settings(), self.locale.settings_description())
                        }
                    };
                    components::page_header(ui, title, description);

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
                                Some((
                                    self.locale.retry(),
                                    UserCommand::StartScreenShare {
                                        system_audio: self.system_audio,
                                    },
                                ))
                            }
                            Page::ScreenShare | Page::Settings => None,
                        };
                        let clicked = components::error_banner(
                            ui,
                            error,
                            recovery.as_ref().map(|(label, _)| *label),
                        );
                        ui.add_space(12.0);
                        clicked.then(|| recovery.expect("a recovery action was rendered").1)
                    } else {
                        None
                    };

                    let page_command = match self.page {
                        Page::Nearby => pages::nearby::show(
                            ui,
                            self.locale,
                            &snapshot,
                            &mut self.selected_peer,
                            layout.device_workspace,
                            self.system_audio,
                        ),
                        Page::ScreenShare => pages::screen_share::show(
                            ui,
                            self.locale,
                            &snapshot,
                            self.playback_texture.as_ref(),
                            &mut self.player,
                            &mut self.system_audio,
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
                        if matches!(
                            command,
                            UserCommand::StartWatching { .. }
                                | UserCommand::StartScreenShare { .. }
                        ) {
                            self.page = Page::ScreenShare;
                        }
                        self.send(command);
                    }
                });
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
        storage.set_string(
            STORAGE_SYSTEM_AUDIO,
            if self.system_audio { "true" } else { "false" }.to_owned(),
        );
    }
}

fn view_switcher(ui: &mut egui::Ui, page: &mut Page, locale: Locale) {
    ui.horizontal(|ui| {
        view_button(ui, page, Page::Nearby, locale.nearby());
        view_button(ui, page, Page::ScreenShare, locale.screen_share());
        view_button(ui, page, Page::Settings, locale.settings());
    });
}

fn view_button(ui: &mut egui::Ui, page: &mut Page, target: Page, label: &str) {
    let active = *page == target;
    let response = ui.add(
        egui::Button::new(RichText::new(label).size(13.0).strong().color(if active {
            theme::BRAND_DARK
        } else {
            theme::TEXT
        }))
        .fill(if active {
            theme::BRAND_SOFT
        } else {
            Color32::TRANSPARENT
        })
        .stroke(Stroke::NONE)
        .corner_radius(theme::RADIUS)
        .min_size(egui::vec2(104.0, 36.0)),
    );
    if response.clicked() {
        *page = target;
    }
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
    fn system_audio_is_off_unless_explicitly_enabled() {
        assert!(!parse_system_audio(None));
        assert!(!parse_system_audio(Some("false".to_owned())));
        assert!(!parse_system_audio(Some("invalid".to_owned())));
        assert!(parse_system_audio(Some("true".to_owned())));
    }

    #[test]
    fn wide_shell_centers_content_at_the_maximum_width() {
        let layout = shell_layout(1440.0);

        assert_eq!(layout.content_width, 1040.0);
        assert_eq!(layout.page_padding, 32.0);
        assert!(!layout.compact_app_bar);
        assert_eq!(layout.device_workspace, DeviceWorkspaceLayout::Split);
    }

    #[test]
    fn standard_window_keeps_the_device_workspace_split() {
        let layout = shell_layout(1024.0);

        assert_eq!(layout.content_width, 960.0);
        assert_eq!(layout.device_workspace, DeviceWorkspaceLayout::Split);
    }

    #[test]
    fn minimum_window_keeps_compact_padding_and_available_content() {
        let layout = shell_layout(680.0);

        assert_eq!(layout.content_width, 632.0);
        assert_eq!(layout.page_padding, 24.0);
        assert!(layout.compact_app_bar);
        assert_eq!(layout.device_workspace, DeviceWorkspaceLayout::Single);
    }

    #[test]
    fn active_media_keeps_navigation_and_stop_available() {
        let publishing = shell_controls(&MediaState::Publishing);
        assert!(publishing.navigation_visible);
        assert_eq!(publishing.stop, Some(MediaStopAction::Publish));
        assert!(publishing.stop_enabled);

        let viewing = shell_controls(&MediaState::Viewing {
            path: "moqcast.screen/peer-a".to_owned(),
        });
        assert!(viewing.navigation_visible);
        assert_eq!(viewing.stop, Some(MediaStopAction::View));
        assert!(viewing.stop_enabled);

        let stopping = shell_controls(&MediaState::StoppingView {
            path: "moqcast.screen/peer-a".to_owned(),
        });
        assert!(stopping.navigation_visible);
        assert_eq!(stopping.stop, Some(MediaStopAction::View));
        assert!(!stopping.stop_enabled);

        let preparing_view = shell_controls(&MediaState::PreparingView {
            path: "moqcast.screen/peer-a".to_owned(),
        });
        assert_eq!(preparing_view.stop, Some(MediaStopAction::View));
        assert!(preparing_view.stop_enabled);

        let preparing_publish = shell_controls(&MediaState::PreparingPublish);
        assert_eq!(preparing_publish.stop, Some(MediaStopAction::Publish));
        assert!(!preparing_publish.stop_enabled);
    }

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
