//! Native UI shell and user-facing state.

mod command;
mod components;
mod device;
mod diagnostics;
mod locale;
mod pages;
mod player;
mod snapshot;
mod theme;

pub use command::UserCommand;
pub use locale::Locale;
pub use snapshot::{
    AppSnapshot, DialRole, DiscoveredPeer, DiscoveryState, MediaState, PeerDiscoveryState,
    PeerSnapshot, RemoteAudioPhase, RemoteAudioSnapshot, RemoteScreenSnapshot, ScreenAvailability,
    StateError, TransportState,
};

use eframe::egui::{self, Color32, Frame, Margin, Stroke};
use moqcast_ui::{
    COLORS, NavItemSpec, PageWidth, Size, Spacing, TypographyRole, app_bar_content_rect, nav_item,
    page_content_rect, typography,
};

use crate::runtime::{PlaybackFrameIdentity, RuntimeHandle, RuntimeStartError};

const STORAGE_LOCALE: &str = "moqcast.locale";
const STORAGE_SYSTEM_AUDIO: &str = "moqcast.system-audio";
const STORAGE_DETAILED_DIAGNOSTICS: &str = "moqcast.detailed-diagnostics";
const STORAGE_DEVELOPER_MODE: &str = "moqcast.developer-mode";
#[cfg(test)]
const CONTENT_MAX_WIDTH: f32 = Size::PAGE_WIDE_MAX;
#[cfg(test)]
const DEVICE_WORKSPACE_SPLIT_WIDTH: f32 = Size::SPLIT_BREAKPOINT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeviceWorkspaceLayout {
    Split,
    Single,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShellLayout {
    content_width: f32,
    page_padding: f32,
    compact_app_bar: bool,
    device_workspace: DeviceWorkspaceLayout,
}

#[cfg(test)]
fn shell_layout(window_width: f32) -> ShellLayout {
    let compact_app_bar = window_width < Size::SPLIT_BREAKPOINT;
    let page_padding = moqcast_ui::page_horizontal_inset(window_width);
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

fn navigation_height(window_width: f32) -> f32 {
    if window_width < Size::SPLIT_BREAKPOINT {
        Size::APP_BAR_COMPACT
    } else {
        Size::APP_BAR
    }
}

fn device_workspace_layout(content_width: f32) -> DeviceWorkspaceLayout {
    if content_width >= Size::SPLIT_BREAKPOINT {
        DeviceWorkspaceLayout::Split
    } else {
        DeviceWorkspaceLayout::Single
    }
}

fn parse_stored_bool(value: Option<String>) -> bool {
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Page {
    /// Nearby peer discovery and connection.
    #[default]
    Nearby,
    /// Screen source and publish controls.
    ScreenShare,
    /// Remote screen playback.
    Watch,
    /// Local application settings.
    Settings,
}

impl Page {
    fn content_width(self) -> PageWidth {
        match self {
            Self::Nearby => PageWidth::Wide,
            Self::ScreenShare | Self::Watch => PageWidth::Medium,
            Self::Settings => PageWidth::Narrow,
        }
    }

    fn scroll_id(self) -> egui::Id {
        egui::Id::new(("linux-page-scroll", self))
    }
}

/// The native MoQCast desktop application.
pub struct MoqCastApp {
    page: Page,
    selected_peer: Option<String>,
    locale: Locale,
    system_audio: bool,
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

        Ok(Self {
            page: Page::default(),
            selected_peer: None,
            locale,
            system_audio,
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

    fn app_bar(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &AppSnapshot,
        compact: bool,
        _controls: ShellControls,
    ) {
        egui::Panel::top("app-bar")
            .exact_size(if compact {
                Size::APP_BAR_COMPACT
            } else {
                Size::APP_BAR
            })
            .frame(Frame::new().fill(COLORS.chrome.into()))
            .show(ui, |ui| {
                let content =
                    app_bar_content_rect(ui.max_rect()).shrink2(egui::vec2(0.0, Spacing::SM));
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(content)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                    |ui| {
                        if compact {
                            let top_height = (content.height() - Spacing::XS - Size::NAV).max(1.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(content.width(), top_height),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    app_brand(ui);
                                    app_status(ui, self.locale, snapshot);
                                },
                            );
                            ui.add_space(Spacing::XS);
                            view_switcher(ui, &mut self.page, self.locale);
                        } else {
                            ui.allocate_ui_with_layout(
                                content.size(),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    app_brand(ui);
                                    ui.add_space(Spacing::XL);
                                    view_switcher(ui, &mut self.page, self.locale);
                                    app_status(ui, self.locale, snapshot);
                                },
                            );
                        }
                    },
                );
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
                    .fill(COLORS.surface.into())
                    .stroke(Stroke::new(Size::BORDER, COLORS.border))
                    .inner_margin(Margin::symmetric(24, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    components::status_line(ui, status, tone);
                    ui.label(typography(
                        self.locale.media_keeps_mesh(),
                        TypographyRole::Meta,
                        COLORS.muted.into(),
                    ));
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
                    if let Some(pages::watch::WatchAction::Command(command)) = pages::watch::show(
                        ui,
                        self.locale,
                        &snapshot,
                        self.playback_texture.as_ref(),
                        &mut self.player,
                    ) {
                        self.send(command);
                    }
                });
            context.request_repaint_after(std::time::Duration::from_millis(33));
            return;
        }
        let compact = navigation_height(ui.available_width()) == Size::APP_BAR_COMPACT;
        let controls = shell_controls(&snapshot.media);
        self.app_bar(ui, &snapshot, compact, controls);
        self.active_media_bar(ui, &snapshot, controls);

        egui::CentralPanel::default()
            .frame(Frame::new().fill(COLORS.surface.into()))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(self.page.scroll_id())
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let content = page_content_rect(
                            ui.available_rect_before_wrap(),
                            self.page.content_width(),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                            ui.set_width(content.width());
                            let (title, description) = match self.page {
                                Page::Nearby => {
                                    (self.locale.nearby(), self.locale.nearby_description())
                                }
                                Page::ScreenShare => {
                                    (self.locale.screen_share(), self.locale.share_description())
                                }
                                Page::Watch => {
                                    (self.locale.watch(), self.locale.watch_description())
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
                            let mut command = if error.is_some() {
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
                                    Page::ScreenShare | Page::Watch | Page::Settings => None,
                                };
                                let clicked = components::error_banner(
                                    ui,
                                    self.locale.attention_required(),
                                    self.locale.operation_failed(),
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
                                    &self.local_device_name,
                                    &mut self.selected_peer,
                                    device_workspace_layout(content.width()),
                                ),
                                Page::ScreenShare => pages::screen_share::show(
                                    ui,
                                    self.locale,
                                    &snapshot,
                                    &mut self.system_audio,
                                ),
                                Page::Watch => match pages::watch::show(
                                    ui,
                                    self.locale,
                                    &snapshot,
                                    self.playback_texture.as_ref(),
                                    &mut self.player,
                                ) {
                                    Some(pages::watch::WatchAction::Command(command)) => {
                                        Some(command)
                                    }
                                    Some(pages::watch::WatchAction::OpenNearby) => {
                                        self.page = Page::Nearby;
                                        None
                                    }
                                    None => None,
                                },
                                Page::Settings => {
                                    if let Some(locale) = pages::settings::show(
                                        ui,
                                        self.locale,
                                        &mut self.developer_mode,
                                        &mut self.diagnostics,
                                    ) {
                                        self.locale = locale;
                                    }
                                    None
                                }
                            };
                            command = command.or(page_command);

                            if let Some(command) = command {
                                self.page = match command {
                                    UserCommand::StartWatching { .. } => Page::Watch,
                                    UserCommand::StartScreenShare { .. } => Page::ScreenShare,
                                    _ => self.page,
                                };
                                self.send(command);
                            }
                        });
                    });
            });

        if self.developer_mode {
            self.diagnostics.show_window(&context, self.locale);
        }

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

fn view_switcher(ui: &mut egui::Ui, page: &mut Page, locale: Locale) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = Spacing::XS;
        for (index, (target, label)) in [
            (Page::Nearby, locale.nearby()),
            (Page::ScreenShare, locale.screen_share()),
            (Page::Watch, locale.watch()),
            (Page::Settings, locale.settings()),
        ]
        .into_iter()
        .enumerate()
        {
            if nav_item(
                ui,
                NavItemSpec::new(egui::Id::new(("linux-nav", index)), label)
                    .selected(*page == target),
            )
            .clicked()
            {
                *page = target;
            }
        }
    });
}

fn app_brand(ui: &mut egui::Ui) {
    ui.label(typography(
        "MoQCast Desktop",
        TypographyRole::Row,
        COLORS.text.into(),
    ));
}

fn app_status(ui: &mut egui::Ui, locale: Locale, snapshot: &AppSnapshot) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        components::mesh_summary(ui, locale, snapshot);
    });
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
    fn pages_use_shared_role_widths_and_independent_scroll_ids() {
        assert_eq!(Page::Nearby.content_width(), moqcast_ui::PageWidth::Wide);
        assert_eq!(
            Page::ScreenShare.content_width(),
            moqcast_ui::PageWidth::Medium
        );
        assert_eq!(Page::Watch.content_width(), moqcast_ui::PageWidth::Medium);
        assert_eq!(
            Page::Settings.content_width(),
            moqcast_ui::PageWidth::Narrow
        );

        let ids = [
            Page::Nearby.scroll_id(),
            Page::ScreenShare.scroll_id(),
            Page::Watch.scroll_id(),
            Page::Settings.scroll_id(),
        ];
        for left in 0..ids.len() {
            for right in left + 1..ids.len() {
                assert_ne!(ids[left], ids[right]);
            }
        }
        assert_eq!(ids[0], Page::Nearby.scroll_id());
    }

    #[test]
    fn ordinary_ui_copy_keeps_internal_network_identity_private() {
        let source = [
            include_str!("pages/nearby.rs"),
            include_str!("pages/screen_share.rs"),
            include_str!("pages/settings.rs"),
            include_str!("player.rs"),
        ]
        .join("\n");
        for forbidden in [
            ["peer_", "identifier()"].concat(),
            ["network_", "endpoints()"].concat(),
            ["fingerprint_", "pinning()"].concat(),
            ["lan_", "session()"].concat(),
            ["inbound_", "sessions()"].concat(),
            ["outbound_", "sessions()"].concat(),
            ["source_", "commit"].concat(),
            ["dependency_", "revision"].concat(),
            ["ffi_", "version"].concat(),
            ["last_", "error"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "ordinary UI still renders {forbidden}"
            );
        }
    }

    #[test]
    fn shared_breakpoint_selects_compact_navigation_and_nearby_layout() {
        assert_eq!(navigation_height(1024.0), Size::APP_BAR);
        assert_eq!(device_workspace_layout(944.0), DeviceWorkspaceLayout::Split);
        assert_eq!(navigation_height(680.0), Size::APP_BAR_COMPACT);
        assert_eq!(
            device_workspace_layout(632.0),
            DeviceWorkspaceLayout::Single
        );
    }

    #[test]
    fn wide_shell_centers_content_at_the_maximum_width() {
        let layout = shell_layout(1440.0);

        assert_eq!(layout.content_width, 1120.0);
        assert_eq!(layout.page_padding, 40.0);
        assert!(!layout.compact_app_bar);
        assert_eq!(layout.device_workspace, DeviceWorkspaceLayout::Split);
    }

    #[test]
    fn standard_window_keeps_the_device_workspace_split() {
        let layout = shell_layout(1024.0);

        assert_eq!(layout.content_width, 944.0);
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
