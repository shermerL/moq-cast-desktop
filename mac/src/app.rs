//! Native macOS application shell for Nearby and direct-only sessions.

mod capture_picker;
mod desktop;
mod diagnostics;
mod player;
mod system_lifecycle;
mod view;

use eframe::egui::{self, Align, Color32, Frame, Key, Layout, Modifiers};
use moqcast_ui::{PlayerFrameSample, install_ui_font};

#[cfg(test)]
use self::view::{ConnectionView, PresenceView};
use self::view::{PeerPresentation, screen_availability};
use crate::network::PeerSession;
use crate::playback::FrameIdentity;
use crate::remote::ScreenAvailability;
use crate::runtime::{
    AppSnapshot, DiscoveryPhase, Generation, MediaOwner, MediaPhase, NearbyIssue, PeerSnapshot,
    RuntimeOwner, RuntimePhase, SessionPhase, ShareAudioPhase,
};

const STORAGE_LOCALE: &str = "moqcast.macos.locale";
const STORAGE_DEVELOPER_MODE: &str = "moqcast.macos.developer-mode";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Page {
    #[default]
    Nearby,
    ScreenShare,
    Watch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Locale {
    #[default]
    Chinese,
    English,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapturePermission {
    NotRequested,
    Allowed,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchProjection {
    Empty,
    Failed,
    Player,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NearbyAction {
    TurnOn,
    TurnOff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NearbyActionPending {
    action: NearbyAction,
    discovery_generation: Generation,
}

impl Locale {
    fn stored(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en",
        }
    }

    fn from_storage(value: Option<String>) -> Self {
        match value.as_deref() {
            Some("en") => Self::English,
            _ => Self::Chinese,
        }
    }
}

pub(crate) struct MoqCastApp {
    page: Page,
    locale: Locale,
    developer_mode: bool,
    diagnostics: diagnostics::DiagnosticsUi,
    detail_peer: Option<String>,
    settings_open: bool,
    theme_preference: egui::ThemePreference,
    command_error: bool,
    confirm_turn_off_nearby: bool,
    nearby_action_pending: Option<NearbyActionPending>,
    runtime: RuntimeOwner,
    system_lifecycle: Option<system_lifecycle::Observer>,
    player: player::Player,
    playback_texture: Option<egui::TextureHandle>,
    playback_identity: Option<FrameIdentity>,
    playback_display: Option<(u32, u32)>,
    capture_picker: Option<capture_picker::Picker>,
    capture_permission: CapturePermission,
    picker_kind: capture_picker::Kind,
    picker_pending: bool,
    picker_error: bool,
}

impl MoqCastApp {
    pub(crate) fn new(
        context: &eframe::CreationContext<'_>,
        diagnostics: moqcast_diagnostics::Handle,
    ) -> anyhow::Result<Self> {
        install_ui_font(
            &context.egui_ctx,
            std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC),
        );
        moqcast_ui::v2::install(&context.egui_ctx);
        let theme_preference = match context
            .storage
            .and_then(|s| s.get_string("moqcast.macos.theme"))
            .as_deref()
        {
            Some("light") => egui::ThemePreference::Light,
            Some("dark") => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        };
        context.egui_ctx.set_theme(theme_preference);
        let locale = Locale::from_storage(
            context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_LOCALE)),
        );
        let developer_mode = developer_mode_from_storage(
            context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_DEVELOPER_MODE)),
        );
        let repaint = context.egui_ctx.clone();
        let runtime = RuntimeOwner::start(move || repaint.request_repaint())?;
        let system_lifecycle = Some(system_lifecycle::Observer::new(runtime.system_lifecycle()));
        let capture_permission = if capture_picker::permission_allowed() {
            CapturePermission::Allowed
        } else {
            CapturePermission::NotRequested
        };
        Ok(Self {
            page: Page::Nearby,
            locale,
            developer_mode,
            diagnostics: diagnostics::DiagnosticsUi::new(diagnostics, developer_mode),
            detail_peer: None,
            settings_open: false,
            theme_preference,
            command_error: false,
            confirm_turn_off_nearby: false,
            nearby_action_pending: None,
            runtime,
            system_lifecycle,
            player: player::Player::default(),
            playback_texture: None,
            playback_identity: None,
            playback_display: None,
            capture_picker: None,
            capture_permission,
            picker_kind: capture_picker::Kind::default(),
            picker_pending: false,
            picker_error: false,
        })
    }

    fn text(&self, chinese: &'static str, english: &'static str) -> &'static str {
        text(self.locale, chinese, english)
    }

    fn handle_shortcuts(&mut self, context: &egui::Context) {
        let shortcut = |key| egui::KeyboardShortcut::new(Modifiers::COMMAND, key);
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Num1))) {
            self.page = Page::Nearby;
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Num2))) {
            self.page = Page::ScreenShare;
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Num3))) {
            self.page = Page::Watch;
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Num4)))
            || context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Comma)))
        {
            self.settings_open = true;
        }
    }

    fn update_playback_texture(&mut self, context: &egui::Context, snapshot: &AppSnapshot) {
        let active = Self::player_active(snapshot);
        if !active {
            self.playback_texture = None;
            self.playback_identity = None;
            self.playback_display = None;
            return;
        }
        let Some(frame) = self.runtime.playback_frame() else {
            if snapshot.media.phase() != MediaPhase::Watching {
                self.playback_texture = None;
                self.playback_identity = None;
                self.playback_display = None;
            }
            return;
        };
        if frame.identity.view_generation != snapshot.media.generation().value()
            || self
                .playback_identity
                .is_some_and(|identity| identity >= frame.identity)
        {
            return;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width, frame.height],
            frame.rgba.as_slice(),
        );
        match self.playback_texture.as_mut() {
            Some(texture) if texture.size() == [frame.width, frame.height] => {
                texture.set(image, egui::TextureOptions::LINEAR);
            }
            _ => {
                self.playback_texture = Some(context.load_texture(
                    "moqcast-remote-screen",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        self.playback_identity = Some(frame.identity);
        self.playback_display = Some((frame.display_width, frame.display_height));
    }

    fn player_active(snapshot: &AppSnapshot) -> bool {
        watch_projection(snapshot.media_owner, snapshot.media.phase()) == WatchProjection::Player
    }

    fn poll_capture_picker(&mut self, context: &egui::Context) {
        let Some(picker) = self.capture_picker.as_ref() else {
            return;
        };
        while let Some(event) = picker.poll() {
            self.picker_pending = false;
            match event {
                capture_picker::Event::Selected(selection) => {
                    self.picker_kind =
                        if matches!(selection, crate::publication::Selection::Window { .. }) {
                            capture_picker::Kind::Window
                        } else {
                            capture_picker::Kind::Display
                        };
                    self.capture_permission = if capture_picker::permission_allowed() {
                        CapturePermission::Allowed
                    } else {
                        CapturePermission::Denied
                    };
                    self.picker_error = false;
                    self.runtime.select_share_source(selection);
                }
                capture_picker::Event::Cancelled => {
                    self.capture_permission = if capture_picker::permission_allowed() {
                        CapturePermission::Allowed
                    } else {
                        CapturePermission::NotRequested
                    };
                }
                capture_picker::Event::Failed => {
                    self.capture_permission = if capture_picker::permission_allowed() {
                        CapturePermission::Allowed
                    } else {
                        CapturePermission::Denied
                    };
                    self.picker_error = true;
                }
            }
            context.request_repaint();
        }
    }

    fn player_device_name(&self, snapshot: &AppSnapshot) -> String {
        snapshot
            .media_peer
            .as_deref()
            .and_then(|peer| snapshot.peers.get(peer))
            .map(|peer| device_name(peer, self.locale))
            .unwrap_or_else(|| self.text("附近设备", "Nearby device").to_owned())
    }

    fn watch_player(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let device_name = self.player_device_name(snapshot);
        let texture = self.playback_texture.as_ref().zip(self.playback_display);
        match self.player.show(
            ui,
            player::PlayerView {
                locale: self.locale,
                generation: snapshot.media.generation().value(),
                phase: snapshot.media.phase(),
                frame: self.playback_identity.map(|identity| PlayerFrameSample {
                    view_generation: identity.view_generation,
                    decoder_generation: identity.decoder_generation,
                    sequence: identity.sequence,
                }),
                video_codec: snapshot.media_video_codec.as_deref(),
                decoder: snapshot.media_decoder.as_deref(),
                audio: &snapshot.watch_audio,
                device_name: &device_name,
                texture,
            },
        ) {
            Some(player::PlayerAction::Stop) => {
                self.runtime.stop_watching();
            }
            Some(player::PlayerAction::SetVolume {
                generation,
                percent,
            }) => {
                self.runtime.set_playback_volume(generation, percent);
            }
            None => {}
        }
    }

    fn request_turn_on_nearby(&mut self, snapshot: &AppSnapshot) {
        let pending = &mut self.nearby_action_pending;
        let runtime = &self.runtime;
        self.command_error = !begin_nearby_action(
            pending,
            NearbyAction::TurnOn,
            snapshot.discovery.generation(),
            || runtime.restart_network(),
        );
    }

    fn request_turn_off_nearby(&mut self, snapshot: &AppSnapshot) {
        let pending = &mut self.nearby_action_pending;
        let runtime = &self.runtime;
        self.command_error = !begin_nearby_action(
            pending,
            NearbyAction::TurnOff,
            snapshot.discovery.generation(),
            || runtime.stop_network(),
        );
    }

    fn show_turn_off_nearby_confirmation(&mut self, context: &egui::Context) {
        if !self.confirm_turn_off_nearby {
            return;
        }
        let response = egui::Modal::new(egui::Id::new("mac-turn-off-nearby-dialog")).frame(moqcast_ui::v2::Palette::new(context.global_style().visuals.dark_mode).dialog()).show(context, |ui| {
            ui.set_max_width(400.0);
            if moqcast_ui::v2::dialog_header(ui, self.text("关闭附近设备？", "Turn off Nearby?"), self.text("取消关闭附近设备", "Cancel turning off Nearby")).clicked() {
                self.confirm_turn_off_nearby = false;
            }
            ui.label(self.text("关闭会停止当前观看或共享，并断开附近连接。", "Turning off Nearby stops the current watch or share and closes nearby connections."));
            ui.horizontal(|ui| {
                if (moqcast_ui::v2::Action { label: self.text("取消", "Cancel"), size: egui::vec2(108.0, 42.0), enabled: true, tone: moqcast_ui::v2::ActionTone::Neutral }).show(ui).clicked() { self.confirm_turn_off_nearby = false; }
                if (moqcast_ui::v2::Action { label: self.text("关闭附近设备", "Turn off Nearby"), size: egui::vec2(148.0,42.0), enabled: true, tone: moqcast_ui::v2::ActionTone::Danger }).show(ui).clicked() {
                    self.confirm_turn_off_nearby = false;
                    let snapshot = self.runtime.snapshot();
                    self.request_turn_off_nearby(&snapshot);
                }
            });
        });
        if response.should_close() {
            self.confirm_turn_off_nearby = false;
        }
    }
}

impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_capture_picker(ui.ctx());
        let snapshot = self.runtime.snapshot();
        reconcile_nearby_action(&mut self.nearby_action_pending, &snapshot);
        if matches!(
            snapshot.discovery.phase(),
            DiscoveryPhase::Stopped | DiscoveryPhase::Failed
        ) {
            self.confirm_turn_off_nearby = false;
        }
        self.update_playback_texture(ui.ctx(), &snapshot);
        let player_active = Self::player_active(&snapshot);
        let fullscreen = self.player.reconcile_fullscreen(ui.ctx(), player_active);
        if fullscreen {
            egui::CentralPanel::default()
                .frame(Frame::NONE.fill(Color32::BLACK))
                .show(ui, |ui| self.watch_player(ui, &snapshot));
            return;
        }
        self.handle_shortcuts(ui.ctx());
        self.desktop_shell(ui, &snapshot);
        self.show_turn_off_nearby_confirmation(ui.ctx());
        if self.diagnostics.show_window(ui.ctx(), self.locale) {
            self.settings_open = true;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(
            "moqcast.macos.theme",
            match self.theme_preference {
                egui::ThemePreference::Light => "light",
                egui::ThemePreference::Dark => "dark",
                egui::ThemePreference::System => "system",
            }
            .to_owned(),
        );
        storage.set_string(STORAGE_LOCALE, self.locale.stored().to_owned());
        storage.set_string(STORAGE_DEVELOPER_MODE, self.developer_mode.to_string());
    }

    fn on_exit(&mut self) {
        self.system_lifecycle.take();
        self.runtime.shutdown();
    }
}

fn text(locale: Locale, chinese: &'static str, english: &'static str) -> &'static str {
    match locale {
        Locale::Chinese => chinese,
        Locale::English => english,
    }
}

fn developer_mode_from_storage(value: Option<String>) -> bool {
    value.as_deref() == Some("true")
}

fn watch_projection(owner: Option<MediaOwner>, phase: MediaPhase) -> WatchProjection {
    if owner != Some(MediaOwner::Watch) {
        return WatchProjection::Empty;
    }
    match phase {
        MediaPhase::PreparingWatch | MediaPhase::Watching | MediaPhase::Stopping => {
            WatchProjection::Player
        }
        MediaPhase::Failed => WatchProjection::Failed,
        MediaPhase::Idle | MediaPhase::PreparingShare | MediaPhase::Sharing => {
            WatchProjection::Empty
        }
    }
}

fn has_active_media(snapshot: &AppSnapshot) -> bool {
    matches!(
        snapshot.media.phase(),
        MediaPhase::PreparingWatch
            | MediaPhase::Watching
            | MediaPhase::PreparingShare
            | MediaPhase::Sharing
            | MediaPhase::Stopping
    )
}

fn begin_nearby_action(
    pending: &mut Option<NearbyActionPending>,
    action: NearbyAction,
    discovery_generation: Generation,
    send: impl FnOnce() -> bool,
) -> bool {
    if pending.is_some() || !send() {
        return false;
    }
    *pending = Some(NearbyActionPending {
        action,
        discovery_generation,
    });
    true
}

fn reconcile_nearby_action(pending: &mut Option<NearbyActionPending>, snapshot: &AppSnapshot) {
    let acknowledged = pending.is_some_and(|pending| {
        if pending.discovery_generation == snapshot.discovery.generation() {
            return false;
        }
        match pending.action {
            NearbyAction::TurnOn => !matches!(snapshot.discovery.phase(), DiscoveryPhase::Stopped),
            NearbyAction::TurnOff => {
                snapshot.runtime.phase() != RuntimePhase::Suspended
                    && snapshot.discovery.phase() == DiscoveryPhase::Stopped
            }
        }
    });
    if acknowledged {
        *pending = None;
    }
}

fn nearby_action_enabled(
    pending: Option<NearbyActionPending>,
    runtime_phase: RuntimePhase,
) -> bool {
    pending.is_none() && runtime_phase != RuntimePhase::Suspended
}

fn global_summary(snapshot: &AppSnapshot, locale: Locale) -> String {
    if snapshot.nearby_stopping {
        return text(locale, "正在关闭附近设备", "Turning off Nearby").into();
    }
    if snapshot.runtime.phase() == RuntimePhase::Suspended {
        return text(locale, "已暂停", "Suspended").to_owned();
    }
    match snapshot.discovery.phase() {
        DiscoveryPhase::Starting => {
            text(locale, "正在开启附近设备", "Turning on Nearby").to_owned()
        }
        DiscoveryPhase::Scanning => text(locale, "附近设备已开启", "Nearby is on").to_owned(),
        DiscoveryPhase::Failed => text(locale, "附近设备不可用", "Nearby unavailable").to_owned(),
        DiscoveryPhase::Stopped => text(locale, "附近设备已关闭", "Nearby is off").to_owned(),
        DiscoveryPhase::Empty => text(locale, "未发现设备", "No devices found").to_owned(),
        DiscoveryPhase::Ready => count_summary(snapshot, locale),
    }
}

fn count_summary(snapshot: &AppSnapshot, locale: Locale) -> String {
    let connected = snapshot
        .peers
        .values()
        .filter(|peer| peer.session == PeerSession::Connected)
        .count()
        + snapshot.inbound_sessions;
    match locale {
        Locale::Chinese => format!("{} 台设备 · {} 个连接", snapshot.peers.len(), connected),
        Locale::English => format!(
            "{} devices · {} connections",
            snapshot.peers.len(),
            connected
        ),
    }
}

fn local_status(snapshot: &AppSnapshot, locale: Locale) -> &'static str {
    if snapshot.runtime.phase() == RuntimePhase::Suspended {
        return text(locale, "睡眠期间已暂停", "Paused while the Mac sleeps");
    }
    match (snapshot.discovery.phase(), snapshot.session.phase()) {
        (DiscoveryPhase::Starting, _) => text(locale, "正在开启附近设备", "Turning on Nearby"),
        (DiscoveryPhase::Scanning, _) => text(
            locale,
            "附近设备已开启，正在自动查找",
            "Nearby is on and discovering automatically",
        ),
        (DiscoveryPhase::Failed, _) => text(locale, "附近设备不可用", "Nearby is unavailable"),
        (DiscoveryPhase::Stopped, _) => text(locale, "附近设备已关闭", "Nearby is off"),
        (_, SessionPhase::Listening) => text(
            locale,
            "附近设备已开启，正在自动查找",
            "Nearby is on and discovering automatically",
        ),
        (_, SessionPhase::Starting) => {
            text(locale, "正在准备设备连接", "Preparing device connections")
        }
        (_, SessionPhase::Failed | SessionPhase::Stopped) => {
            text(locale, "安全连接不可用", "Secure connections unavailable")
        }
    }
}

fn issue_message(issue: NearbyIssue, locale: Locale) -> &'static str {
    match issue {
        NearbyIssue::LocalNetworkUnavailable => text(
            locale,
            "无法使用本地网络。请检查系统权限。",
            "The local network is unavailable. Check system permission.",
        ),
        NearbyIssue::DirectConnectionsUnavailable => text(
            locale,
            "无法连接附近设备。",
            "Nearby device connections are unavailable.",
        ),
        NearbyIssue::DiscoveryStopped => text(
            locale,
            "附近设备扫描已停止；现有连接仍可继续。",
            "Nearby scanning stopped; existing connections can continue.",
        ),
        NearbyIssue::ListenerStopped => text(
            locale,
            "无法接受新的设备连接；现有设备结果仍会保留。",
            "New device connections cannot be accepted; existing results remain visible.",
        ),
        NearbyIssue::ServicesStopped => text(
            locale,
            "附近设备服务已结束。可以重试恢复扫描与连接。",
            "Nearby services ended. Try again to restore scanning and connections.",
        ),
        NearbyIssue::ShutdownFailed => text(
            locale,
            "关闭附近设备时发生错误，请查看日志。",
            "Nearby shutdown reported an error. Check the logs.",
        ),
        NearbyIssue::DeviceRejected => text(
            locale,
            "一个传入连接未能通过验证。",
            "An incoming connection could not be verified.",
        ),
    }
}

fn share_action_available(permission: CapturePermission, snapshot: &AppSnapshot) -> bool {
    permission == CapturePermission::Allowed
        && snapshot.access().publish()
        && snapshot.share_selection.is_some()
}

fn system_audio_action_available(snapshot: &AppSnapshot) -> bool {
    snapshot.access().publish()
        && snapshot
            .share_selection
            .as_ref()
            .is_some_and(crate::publication::Selection::supports_system_audio)
}

fn device_name(peer: &PeerSnapshot, locale: Locale) -> String {
    match locale {
        Locale::Chinese => format!("附近设备 {}", peer.ordinal),
        Locale::English => format!("Nearby device {}", peer.ordinal),
    }
}

fn local_device_id_label(locale: Locale) -> &'static str {
    text(locale, "本机 ID", "This device ID")
}

fn remote_device_id_label(locale: Locale) -> &'static str {
    text(locale, "设备 ID", "Device ID")
}

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

fn peer_list_subtitle(locale: Locale, peer_id: &str) -> String {
    format!("{}: {peer_id}", remote_device_id_label(locale))
}

fn device_status(
    peer: PeerPresentation,
    screen: ScreenAvailability,
    locale: Locale,
) -> &'static str {
    peer.facts(screen == ScreenAvailability::Available)
        .status()
        .label(locale == Locale::Chinese)
}

fn peer_line(peer: PeerPresentation, locale: Locale) -> &'static str {
    peer.facts(false).status().label(locale == Locale::Chinese)
}

fn screen_label(screen: ScreenAvailability, locale: Locale) -> &'static str {
    match screen {
        ScreenAvailability::Unavailable => text(locale, "未共享", "Not shared"),
        ScreenAvailability::Available => text(locale, "可观看", "Available"),
        ScreenAvailability::Withdrawn => text(locale, "共享已结束", "Share ended"),
    }
}

#[cfg(test)]
mod tests;
