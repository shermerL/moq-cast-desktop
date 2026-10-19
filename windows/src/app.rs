mod desktop;
// Native state and platform commands; visual composition lives in desktop.

use eframe::egui::{self, Align, Color32, Frame, Layout};
use moqcast_ui::install_ui_font;

use crate::{
    audio::AudioPhase,
    diagnostics::DiagnosticsUi,
    media::{
        CaptureSourceCatalogPhase, CaptureSourceChoice, CaptureSourceKind, MediaPhase,
        VideoEncodingPolicy,
    },
    playback::{PlaybackFrameIdentity, ViewPhase},
    player::{LivePlayer, PlayerAction},
    remote::ScreenAvailability,
    runtime::{
        DiscoveryState, PeerView, RuntimeCommand, RuntimeOwner, RuntimeSnapshot,
        TransportDirectionView, TransportPhaseView,
    },
};

const STORAGE_DETAILED_DIAGNOSTICS: &str = "moqcast.detailed-diagnostics";
const STORAGE_DEVELOPER_MODE: &str = "moqcast.developer-mode";
const STORAGE_LOCALE: &str = "moqcast.locale";

fn parse_stored_bool(value: Option<String>) -> bool {
    value.as_deref() == Some("true")
}

fn parse_stored_locale(value: Option<String>) -> Locale {
    match value.as_deref() {
        Some("en") => Locale::English,
        _ => Locale::Chinese,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Nearby,
    ScreenShare,
    Watch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchProjection {
    Empty,
    Failed,
    Player,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Locale {
    #[default]
    Chinese,
    English,
}

impl Locale {
    fn nearby(self) -> &'static str {
        match self {
            Self::Chinese => "附近设备",
            Self::English => "Nearby",
        }
    }

    fn settings(self) -> &'static str {
        match self {
            Self::Chinese => "设置",
            Self::English => "Settings",
        }
    }

    fn watch(self) -> &'static str {
        match self {
            Self::Chinese => "观看",
            Self::English => "Watch",
        }
    }
}

pub(crate) struct MoqCastApp {
    page: Page,
    locale: Locale,
    developer_mode: bool,
    settings_open: bool,
    theme_preference: egui::ThemePreference,
    capture_kind: CaptureSourceKind,
    selected_peer: Option<String>,
    confirm_turn_off_nearby: bool,
    nearby_turn_off_pending: bool,
    diagnostics: DiagnosticsUi,
    runtime: RuntimeOwner,
    snapshot: RuntimeSnapshot,
    command_error: Option<String>,
    playback_texture: Option<egui::TextureHandle>,
    playback_identity: Option<PlaybackFrameIdentity>,
    playback_timestamp_us: Option<u128>,
    playback_high_water_timestamp_us: Option<u128>,
    viewport_fullscreen: bool,
    player: LivePlayer,
}

impl MoqCastApp {
    pub(crate) fn new(
        context: &eframe::CreationContext<'_>,
        mut runtime: RuntimeOwner,
        diagnostics: moqcast_diagnostics::Handle,
    ) -> Self {
        install_ui_font(
            &context.egui_ctx,
            std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC),
        );
        moqcast_ui::v2::install(&context.egui_ctx);
        let theme_preference = match context
            .storage
            .and_then(|storage| storage.get_string("moqcast.theme"))
            .as_deref()
        {
            Some("light") => egui::ThemePreference::Light,
            Some("dark") => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        };
        context.egui_ctx.set_theme(theme_preference);
        let detailed_diagnostics = parse_stored_bool(
            context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_DETAILED_DIAGNOSTICS)),
        );
        let developer_mode = parse_stored_bool(
            context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_DEVELOPER_MODE)),
        );
        let locale = parse_stored_locale(
            context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_LOCALE)),
        );
        let snapshot = runtime.snapshot();
        Self {
            page: Page::Nearby,
            locale,
            developer_mode,
            settings_open: false,
            theme_preference,
            capture_kind: snapshot
                .media
                .capture_sources
                .selected
                .as_ref()
                .map_or(CaptureSourceKind::Display, |c| c.kind),
            selected_peer: None,
            confirm_turn_off_nearby: false,
            nearby_turn_off_pending: false,
            diagnostics: DiagnosticsUi::new(diagnostics, detailed_diagnostics),
            runtime,
            snapshot,
            command_error: None,
            playback_texture: None,
            playback_identity: None,
            playback_timestamp_us: None,
            playback_high_water_timestamp_us: None,
            viewport_fullscreen: false,
            player: LivePlayer::default(),
        }
    }

    fn send(&mut self, command: RuntimeCommand) -> bool {
        match self.runtime.try_send(command) {
            Ok(()) => {
                self.command_error = None;
                true
            }
            Err(error) => {
                self.command_error = Some(match self.locale {
                    Locale::Chinese => format!("命令未发送：{error}"),
                    Locale::English => format!("Command was not sent: {error}"),
                });
                false
            }
        }
    }

    fn handle_player_action(&mut self, action: Option<PlayerAction>) {
        match action {
            Some(PlayerAction::Stop) => {
                self.send(RuntimeCommand::StopWatching);
            }
            Some(PlayerAction::SetVolume {
                generation,
                percent,
            }) => {
                self.send(RuntimeCommand::SetPlaybackVolume {
                    generation,
                    percent,
                });
            }
            None => {}
        }
    }

    fn request_turn_off_nearby(&mut self) {
        if self.nearby_turn_off_pending || self.snapshot.discovery == DiscoveryState::Stopping {
            return;
        }
        if self.send(RuntimeCommand::StopScan) {
            self.nearby_turn_off_pending = true;
        }
    }

    fn update_playback_texture(&mut self, context: &egui::Context) {
        if let Some(frame) = self.runtime.playback_frame() {
            if should_commit_playback_frame(
                self.snapshot.view.generation,
                self.playback_identity,
                frame.identity,
            ) {
                let same_view = self.playback_identity.is_some_and(|identity| {
                    identity.view_generation == frame.identity.view_generation
                });
                if !same_view {
                    self.playback_timestamp_us = None;
                    self.playback_high_water_timestamp_us = None;
                }
                let previous_timestamp_us = self.playback_timestamp_us;
                let high_water_timestamp_us = self.playback_high_water_timestamp_us;
                let regressed =
                    previous_timestamp_us.is_some_and(|previous| frame.timestamp_us < previous);
                let behind_high_water_us = high_water_timestamp_us
                    .map(|high_water| high_water.saturating_sub(frame.timestamp_us))
                    .unwrap_or(0);
                if same_view
                    && behind_high_water_us > 0
                    && (regressed
                        || frame.identity.sequence == 1
                        || frame.identity.sequence.is_multiple_of(30))
                {
                    tracing::warn!(
                        view_generation = frame.identity.view_generation,
                        decoder_generation = frame.identity.decoder_generation,
                        sequence = frame.identity.sequence,
                        previous_identity = ?self.playback_identity,
                        previous_pts_us = %previous_timestamp_us.unwrap_or_default(),
                        frame_pts_us = %frame.timestamp_us,
                        texture_high_water_pts_us = %high_water_timestamp_us.unwrap_or_default(),
                        behind_high_water_us = %behind_high_water_us,
                        fullscreen = self.viewport_fullscreen,
                        "remote video texture commit is behind the displayed PTS high-water mark"
                    );
                }
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width, frame.height],
                    &frame.rgba,
                );
                if let Some(texture) = &mut self.playback_texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.playback_texture = Some(context.load_texture(
                        "remote-screen",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.playback_identity = Some(frame.identity);
                self.playback_timestamp_us = Some(frame.timestamp_us);
                self.playback_high_water_timestamp_us = Some(
                    high_water_timestamp_us.map_or(frame.timestamp_us, |high_water| {
                        high_water.max(frame.timestamp_us)
                    }),
                );
                if frame.identity.sequence == 1 || frame.identity.sequence.is_multiple_of(300) {
                    tracing::info!(
                        view_generation = frame.identity.view_generation,
                        decoder_generation = frame.identity.decoder_generation,
                        sequence = frame.identity.sequence,
                        frame_pts_us = %frame.timestamp_us,
                        fullscreen = self.viewport_fullscreen,
                        "remote video texture committed"
                    );
                }
            } else if Some(frame.identity) != self.playback_identity {
                tracing::warn!(
                    current_view_generation = self.snapshot.view.generation,
                    incoming_view_generation = frame.identity.view_generation,
                    incoming_decoder_generation = frame.identity.decoder_generation,
                    incoming_sequence = frame.identity.sequence,
                    previous_identity = ?self.playback_identity,
                    frame_pts_us = %frame.timestamp_us,
                    "stale remote video frame rejected before texture commit"
                );
            }
        }
        if matches!(
            self.snapshot.view.phase,
            ViewPhase::Idle | ViewPhase::Failed
        ) {
            self.playback_texture = None;
            self.playback_identity = None;
            self.playback_timestamp_us = None;
            self.playback_high_water_timestamp_us = None;
        }
    }
}

impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.snapshot = self.runtime.snapshot();
        match self.snapshot.discovery {
            DiscoveryState::Stopping => self.nearby_turn_off_pending = true,
            DiscoveryState::Stopped | DiscoveryState::Failed => {
                self.nearby_turn_off_pending = false;
                self.confirm_turn_off_nearby = false;
            }
            DiscoveryState::Starting | DiscoveryState::Ready | DiscoveryState::Empty => {}
        }
        let context = ui.ctx().clone();
        self.update_playback_texture(&context);
        let viewing = matches!(
            self.snapshot.view.phase,
            ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
        );
        let viewport_fullscreen = self
            .player
            .reconcile_fullscreen(&context, self.snapshot.view.phase == ViewPhase::Viewing);
        if viewport_fullscreen != self.viewport_fullscreen {
            tracing::info!(
                fullscreen = viewport_fullscreen,
                view_generation = self.snapshot.view.generation,
                decoder_generation = self
                    .playback_identity
                    .map(|identity| identity.decoder_generation),
                sequence = self.playback_identity.map(|identity| identity.sequence),
                "playback fullscreen changed"
            );
            self.viewport_fullscreen = viewport_fullscreen;
        }
        let fullscreen = self.snapshot.view.phase == ViewPhase::Viewing && viewport_fullscreen;

        if fullscreen {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(Color32::BLACK))
                .show(ui, |ui| {
                    let action = self.player.show(
                        ui,
                        self.locale,
                        &self.snapshot.view,
                        self.playback_texture.as_ref(),
                        self.playback_identity,
                    );
                    self.handle_player_action(action);
                });
            self.diagnostics.show_window(&context, self.locale);
            context.request_repaint_after(std::time::Duration::from_millis(33));
            return;
        }

        self.desktop_shell(ui);
        let logs_visible = self.diagnostics.visible();
        self.diagnostics.show_window(&context, self.locale);
        if logs_visible && !self.diagnostics.visible() {
            self.settings_open = true;
        }
        context.request_repaint_after(std::time::Duration::from_millis(if viewing {
            33
        } else {
            100
        }));
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(
            "moqcast.theme",
            match self.theme_preference {
                egui::ThemePreference::Light => "light",
                egui::ThemePreference::Dark => "dark",
                egui::ThemePreference::System => "system",
            }
            .to_owned(),
        );
        storage.set_string(
            STORAGE_LOCALE,
            match self.locale {
                Locale::Chinese => "zh",
                Locale::English => "en",
            }
            .to_owned(),
        );
        storage.set_string(
            STORAGE_DEVELOPER_MODE,
            if self.developer_mode { "true" } else { "false" }.to_owned(),
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
    }
}

fn has_active_media(snapshot: &RuntimeSnapshot) -> bool {
    matches!(
        snapshot.media.phase,
        MediaPhase::Preparing | MediaPhase::Sharing | MediaPhase::Stopping
    ) || matches!(
        snapshot.view.phase,
        ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
    )
}

fn visible_peers(snapshot: &RuntimeSnapshot) -> Vec<PeerView> {
    snapshot
        .peers
        .values()
        .filter(|peer| {
            peer.present
                || peer.screen == ScreenAvailability::Available
                || peer.transport.direction == Some(TransportDirectionView::Outbound)
                    && peer.transport.phase == TransportPhaseView::Connected
        })
        .cloned()
        .collect()
}

fn peer_display_name(locale: Locale, ordinal: usize) -> String {
    let base = match locale {
        Locale::Chinese => "附近设备",
        Locale::English => "Nearby device",
    };
    format!("{base} {ordinal}")
}

#[cfg(test)]
fn local_device_id_label(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "本机 ID",
        Locale::English => "This device ID",
    }
}

#[cfg(test)]
fn remote_device_id_label(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "设备 ID",
        Locale::English => "Device ID",
    }
}

#[cfg(test)]
fn local_device_description(
    locale: Locale,
    device_name: &str,
    status: &str,
    peer_id: Option<&str>,
) -> String {
    let identity = match (locale, peer_id) {
        (Locale::Chinese, Some(peer_id)) => format!("本次运行：{peer_id}"),
        (Locale::English, Some(peer_id)) => format!("Current run: {peer_id}"),
        (Locale::Chinese, None) => "本次运行尚未生成 ID".to_owned(),
        (Locale::English, None) => "No ID for the current run yet".to_owned(),
    };
    format!("{device_name} · {status} · {identity}")
}

#[cfg(test)]
fn peer_list_subtitle(locale: Locale, peer: &PeerView) -> String {
    format!(
        "{} · {}: {}",
        peer_status(locale, peer),
        remote_device_id_label(locale),
        peer.id
    )
}

fn peer_status(locale: Locale, peer: &PeerView) -> &'static str {
    if !peer.present {
        return match locale {
            Locale::Chinese => "设备不再可发现",
            Locale::English => "Device no longer discoverable",
        };
    }
    if peer.screen == ScreenAvailability::Available {
        return match locale {
            Locale::Chinese => "屏幕可观看",
            Locale::English => "Screen available",
        };
    }
    match (locale, peer.transport.phase) {
        (Locale::Chinese, TransportPhaseView::Connected) => "已连接",
        (Locale::English, TransportPhaseView::Connected) => "Connected",
        (Locale::Chinese, TransportPhaseView::Connecting) => "正在连接",
        (Locale::English, TransportPhaseView::Connecting) => "Connecting",
        (Locale::Chinese, TransportPhaseView::Rejected) => "无法验证设备",
        (Locale::English, TransportPhaseView::Rejected) => "Could not verify device",
        (Locale::Chinese, TransportPhaseView::Failed) => "连接不可用",
        (Locale::English, TransportPhaseView::Failed) => "Connection unavailable",
        (Locale::Chinese, TransportPhaseView::Disconnected) => "等待设备",
        (Locale::English, TransportPhaseView::Disconnected) => "Waiting for device",
        (Locale::Chinese, TransportPhaseView::Waiting) => "等待设备",
        (Locale::English, TransportPhaseView::Waiting) => "Waiting for device",
    }
}

fn discovery_summary(locale: Locale, snapshot: &RuntimeSnapshot) -> String {
    match (locale, snapshot.discovery) {
        (Locale::Chinese, DiscoveryState::Ready) => {
            format!("{} 台附近设备", snapshot.present_peer_count())
        }
        (Locale::English, DiscoveryState::Ready) => {
            format!("{} nearby", snapshot.present_peer_count())
        }
        (Locale::Chinese, DiscoveryState::Empty) => "附近设备已开启".to_owned(),
        (Locale::English, DiscoveryState::Empty) => "Nearby is on".to_owned(),
        (Locale::Chinese, DiscoveryState::Starting) => "正在开启".to_owned(),
        (Locale::English, DiscoveryState::Starting) => "Turning on".to_owned(),
        (Locale::Chinese, DiscoveryState::Stopping) => "正在关闭".to_owned(),
        (Locale::English, DiscoveryState::Stopping) => "Turning off".to_owned(),
        (Locale::Chinese, DiscoveryState::Failed) => "附近服务不可用".to_owned(),
        (Locale::English, DiscoveryState::Failed) => "Nearby unavailable".to_owned(),
        (Locale::Chinese, DiscoveryState::Stopped) => "附近设备已关闭".to_owned(),
        (Locale::English, DiscoveryState::Stopped) => "Nearby is off".to_owned(),
    }
}

fn capture_source_subtitle(locale: Locale, source: &CaptureSourceChoice) -> String {
    let dimensions = format!("{} × {}", source.width, source.height);
    match source.kind {
        CaptureSourceKind::Display => match locale {
            Locale::Chinese => format!("屏幕 · {dimensions}"),
            Locale::English => format!("Display · {dimensions}"),
        },
        CaptureSourceKind::Window => {
            let kind = match locale {
                Locale::Chinese => "窗口",
                Locale::English => "Window",
            };
            match source.application.as_deref().filter(|app| !app.is_empty()) {
                Some(app) => format!("{kind} · {app} · {dimensions}"),
                None => format!("{kind} · {dimensions}"),
            }
        }
    }
}

fn watch_projection(phase: ViewPhase) -> WatchProjection {
    match phase {
        ViewPhase::Idle => WatchProjection::Empty,
        ViewPhase::Failed => WatchProjection::Failed,
        ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping => WatchProjection::Player,
    }
}

fn should_commit_playback_frame(
    current_view_generation: u64,
    previous: Option<PlaybackFrameIdentity>,
    incoming: PlaybackFrameIdentity,
) -> bool {
    incoming.view_generation == current_view_generation
        && previous.is_none_or(|previous| incoming > previous)
}

fn video_encoding_label(locale: Locale, policy: VideoEncodingPolicy) -> &'static str {
    match (locale, policy) {
        (Locale::Chinese, VideoEncodingPolicy::Compatible) => "兼容（原生尺寸，最长边 1920）",
        (Locale::English, VideoEncodingPolicy::Compatible) => "Compatible (native, max edge 1920)",
        (Locale::Chinese, VideoEncodingPolicy::NativeQhdHardware) => "原生横屏 QHD（仅硬件）",
        (Locale::English, VideoEncodingPolicy::NativeQhdHardware) => {
            "Native landscape QHD (hardware only)"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_fonts_cover_core_simplified_chinese() {
        let context = egui::Context::default();
        install_ui_font(
            &context,
            std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC),
        );

        let mut output = context.run_ui(Default::default(), |ui| {
            for font_id in [
                egui::FontId::proportional(14.0),
                egui::FontId::monospace(14.0),
            ] {
                ui.fonts_mut(|fonts| {
                    assert!(fonts.has_glyphs(&font_id, "附近设备屏幕共享设置"));
                });
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn routing_and_language_are_independent() {
        let locale = Locale::English;
        assert_eq!(locale.settings(), "Settings");
        assert_eq!(locale.watch(), "Watch");
        assert_ne!(Page::Nearby, Page::ScreenShare);
        assert_ne!(Page::Watch, Page::Nearby);
    }

    #[test]
    fn nearby_projection_keeps_presence_and_healthy_outbound_sessions_separate() {
        let mut snapshot = RuntimeSnapshot::default();
        let present = peer("present");
        let mut connected = peer("connected");
        connected.present = false;
        connected.transport.direction = Some(TransportDirectionView::Outbound);
        connected.transport.phase = TransportPhaseView::Connected;
        let mut historical = peer("historical");
        historical.present = false;
        let mut inbound = peer("inbound");
        inbound.present = false;
        inbound.transport.direction = Some(TransportDirectionView::Inbound);
        inbound.transport.phase = TransportPhaseView::Connected;
        for peer in [present, connected, historical, inbound] {
            snapshot.peers.insert(peer.id.clone(), peer);
        }

        assert_eq!(snapshot.present_peer_count(), 1);
        let projected = visible_peers(&snapshot);
        assert_eq!(projected.len(), 2);
        assert!(projected.iter().any(|peer| peer.id == "present"));
        let connected = projected
            .iter()
            .find(|peer| peer.id == "connected")
            .expect("healthy outbound session remains visible");
        assert_eq!(
            peer_status(Locale::English, connected),
            "Device no longer discoverable"
        );
    }

    #[test]
    fn detailed_diagnostics_storage_is_opt_in() {
        assert!(!parse_stored_bool(None));
        assert!(!parse_stored_bool(Some("false".to_owned())));
        assert!(!parse_stored_bool(Some("unexpected".to_owned())));
        assert!(parse_stored_bool(Some("true".to_owned())));
        assert_eq!(parse_stored_locale(None), Locale::Chinese);
        assert_eq!(parse_stored_locale(Some("zh".to_owned())), Locale::Chinese);
        assert_eq!(parse_stored_locale(Some("en".to_owned())), Locale::English);
    }

    #[test]
    fn inactive_watch_states_do_not_project_a_player_or_live_toolbar() {
        assert_eq!(watch_projection(ViewPhase::Idle), WatchProjection::Empty);
        assert_eq!(watch_projection(ViewPhase::Failed), WatchProjection::Failed);
        for phase in [
            ViewPhase::Preparing,
            ViewPhase::Viewing,
            ViewPhase::Stopping,
        ] {
            assert_eq!(watch_projection(phase), WatchProjection::Player);
        }
    }

    #[test]
    fn nearby_turn_off_confirmation_tracks_active_media() {
        let mut snapshot = RuntimeSnapshot::default();
        assert!(!has_active_media(&snapshot));

        snapshot.media.phase = MediaPhase::Sharing;
        assert!(has_active_media(&snapshot));

        snapshot.media.phase = MediaPhase::Idle;
        snapshot.view.phase = ViewPhase::Viewing;
        assert!(has_active_media(&snapshot));
    }

    #[test]
    fn nearby_identity_copy_shows_peer_ids_without_transport_internals() {
        let mut visible = peer("0123-abcd");
        visible.candidates.push("192.0.2.1:4443".to_owned());
        visible.transport.phase = TransportPhaseView::Connecting;
        assert_eq!(peer_display_name(Locale::Chinese, 1), "附近设备 1");
        assert_eq!(peer_display_name(Locale::English, 2), "Nearby device 2");
        assert!(!peer_display_name(Locale::English, 1).contains("abcd"));
        assert_eq!(peer_status(Locale::English, &visible), "Connecting");
        assert_eq!(local_device_id_label(Locale::English), "This device ID");
        assert_eq!(remote_device_id_label(Locale::English), "Device ID");
        let local = local_device_description(
            Locale::English,
            "Workstation",
            "Nearby is on",
            Some("local-peer"),
        );
        let remote = peer_list_subtitle(Locale::English, &visible);
        assert!(local.contains("Current run: local-peer"));
        assert!(remote.contains("Device ID: 0123-abcd"));
        assert!(!remote.contains("192.0.2.1"));

        visible.screen = ScreenAvailability::Available;
        assert_eq!(peer_status(Locale::Chinese, &visible), "屏幕可观看");
    }

    #[test]
    fn capture_source_subtitles_distinguish_displays_and_windows() {
        let display = CaptureSourceChoice {
            kind: CaptureSourceKind::Display,
            id: "display:0".to_owned(),
            name: "Display 1".to_owned(),
            application: None,
            width: 1920,
            height: 1080,
        };
        let window = CaptureSourceChoice {
            kind: CaptureSourceKind::Window,
            id: "window:7".to_owned(),
            name: "Document".to_owned(),
            application: Some("Editor".to_owned()),
            width: 1280,
            height: 720,
        };

        assert_eq!(
            capture_source_subtitle(Locale::English, &display),
            "Display · 1920 × 1080"
        );
        assert_eq!(
            capture_source_subtitle(Locale::Chinese, &window),
            "窗口 · Editor · 1280 × 720"
        );
    }

    #[test]
    fn stale_view_decoder_and_sequence_cannot_replace_the_current_texture() {
        let current = PlaybackFrameIdentity {
            view_generation: 4,
            decoder_generation: 2,
            sequence: 10,
        };
        assert!(!should_commit_playback_frame(
            4,
            Some(current),
            PlaybackFrameIdentity {
                view_generation: 3,
                decoder_generation: 99,
                sequence: 99,
            },
        ));
        assert!(!should_commit_playback_frame(
            4,
            Some(current),
            PlaybackFrameIdentity {
                decoder_generation: 1,
                sequence: 99,
                ..current
            },
        ));
        assert!(!should_commit_playback_frame(
            4,
            Some(current),
            PlaybackFrameIdentity {
                sequence: 9,
                ..current
            },
        ));
        assert!(should_commit_playback_frame(
            4,
            Some(current),
            PlaybackFrameIdentity {
                sequence: 11,
                ..current
            },
        ));
        assert!(should_commit_playback_frame(
            4,
            Some(current),
            PlaybackFrameIdentity {
                decoder_generation: 3,
                sequence: 1,
                ..current
            },
        ));
    }

    fn peer(id: &str) -> PeerView {
        PeerView {
            id: id.to_owned(),
            candidates: Vec::new(),
            should_dial: true,
            authenticated_discovery: false,
            tls_pinned: true,
            present: true,
            transport: Default::default(),
            screen: ScreenAvailability::Unavailable,
        }
    }
}
