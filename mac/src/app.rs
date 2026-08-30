//! Native macOS application shell for Nearby and direct-only sessions.

mod capture_picker;
mod player;
mod theme;
mod view;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Key, Layout, Margin,
    Modifiers, RichText, Sense, Stroke,
};

use self::view::{
    ConnectionView, ContentLayout, NavigationLayout, PeerPresentation, PresenceView,
    screen_availability, selected_peer,
};
use crate::network::PeerSession;
use crate::playback::FrameIdentity;
use crate::remote::ScreenAvailability;
use crate::runtime::{
    AppSnapshot, DiscoveryPhase, MediaOwner, MediaPhase, NearbyIssue, PeerSnapshot, RuntimeOwner,
    SessionPhase, ShareAudioPhase,
};

const STORAGE_LOCALE: &str = "moqcast.macos.locale";
const MAX_CONTENT_WIDTH: f32 = 1040.0;
const DETAIL_WIDTH: f32 = 360.0;
const DEVICE_ROW_HEIGHT: f32 = 60.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Nearby,
    ScreenShare,
    Settings,
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
    selected_peer: Option<String>,
    runtime: RuntimeOwner,
    player: player::Player,
    playback_texture: Option<egui::TextureHandle>,
    playback_identity: Option<FrameIdentity>,
    playback_display: Option<(u32, u32)>,
    capture_picker: Option<capture_picker::Picker>,
    capture_permission: CapturePermission,
    picker_pending: bool,
    picker_error: bool,
}

impl MoqCastApp {
    pub(crate) fn new(context: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        configure_fonts(&context.egui_ctx);
        theme::apply(&context.egui_ctx);
        let locale = Locale::from_storage(
            context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_LOCALE)),
        );
        let repaint = context.egui_ctx.clone();
        let runtime = RuntimeOwner::start(move || repaint.request_repaint())?;
        let capture_permission = if capture_picker::permission_allowed() {
            CapturePermission::Allowed
        } else {
            CapturePermission::NotRequested
        };
        Ok(Self {
            page: Page::Nearby,
            locale,
            selected_peer: None,
            runtime,
            player: player::Player::default(),
            playback_texture: None,
            playback_identity: None,
            playback_display: None,
            capture_picker: None,
            capture_permission,
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
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Num3)))
            || context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Comma)))
        {
            self.page = Page::Settings;
        }
    }

    fn update_playback_texture(&mut self, context: &egui::Context, snapshot: &AppSnapshot) {
        let active = Self::media_active(snapshot);
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

    fn media_active(snapshot: &AppSnapshot) -> bool {
        snapshot.media_owner == Some(MediaOwner::Watch)
            && matches!(
                snapshot.media.phase(),
                MediaPhase::PreparingWatch
                    | MediaPhase::Watching
                    | MediaPhase::Stopping
                    | MediaPhase::Failed
            )
    }

    fn poll_capture_picker(&mut self, context: &egui::Context) {
        let Some(picker) = self.capture_picker.as_ref() else {
            return;
        };
        while let Some(event) = picker.poll() {
            self.picker_pending = false;
            match event {
                capture_picker::Event::Selected(selection) => {
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
        if matches!(
            self.player.show(
                ui,
                self.locale,
                snapshot.media.phase(),
                &device_name,
                texture
            ),
            Some(player::PlayerAction::Stop)
        ) {
            self.runtime.stop_watching();
        }
    }

    fn top_bar(&mut self, root: &mut egui::Ui, snapshot: &AppSnapshot) {
        let summary = global_summary(snapshot, self.locale);
        egui::Panel::top("navigation")
            .frame(
                Frame::NONE
                    .fill(theme::SURFACE)
                    .inner_margin(Margin::symmetric(18, 8)),
            )
            .show(root, |ui| {
                match NavigationLayout::for_width(ui.ctx().content_rect().width()) {
                    NavigationLayout::OneRow => {
                        ui.set_min_height(36.0);
                        ui.horizontal_centered(|ui| {
                            ui.label(
                                RichText::new("MoQCast")
                                    .size(15.0)
                                    .strong()
                                    .color(theme::BRAND_PRESSED),
                            );
                            ui.add_space(18.0);
                            self.navigation(ui, false);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(summary).small().color(theme::MUTED));
                            });
                        });
                    }
                    NavigationLayout::TwoRows => {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("MoQCast")
                                    .size(15.0)
                                    .strong()
                                    .color(theme::BRAND_PRESSED),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(summary).small().color(theme::MUTED));
                            });
                        });
                        self.navigation(ui, true);
                    }
                }
            });
    }

    fn navigation(&mut self, ui: &mut egui::Ui, equal_width: bool) {
        let labels = [
            self.text("附近设备", "Nearby"),
            self.text("屏幕共享", "Screen Share"),
            self.text("设置与诊断", "Settings & Diagnostics"),
        ];
        let pages = [Page::Nearby, Page::ScreenShare, Page::Settings];
        let width = if equal_width {
            (ui.available_width() - 6.0) / 3.0
        } else {
            0.0
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for (page, label) in pages.into_iter().zip(labels) {
                let button = egui::Button::new(label)
                    .selected(self.page == page)
                    .corner_radius(CornerRadius::same(6));
                let response = if equal_width {
                    ui.add_sized([width, 32.0], button)
                } else {
                    ui.add_sized([0.0, 32.0], button)
                };
                if response.clicked() {
                    self.page = page;
                }
            }
        });
    }

    fn page_header(&self, ui: &mut egui::Ui, title: &'static str, subtitle: &'static str) {
        ui.heading(title);
        ui.add_space(2.0);
        ui.label(RichText::new(subtitle).small().color(theme::MUTED));
        ui.add_space(18.0);
    }

    fn nearby(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        self.selected_peer = selected_peer(self.selected_peer.as_deref(), &snapshot.peers);
        self.page_header(
            ui,
            self.text("附近设备", "Nearby Devices"),
            self.text(
                "自动发现同一局域网中的 MoQCast 设备，并分别显示附近状态与安全连接。",
                "Discover MoQCast devices on this LAN and keep presence separate from the secure session.",
            ),
        );
        self.local_summary(ui, snapshot);
        ui.add_space(12.0);

        if let Some(issue) = snapshot.nearby_issue {
            issue_notice(ui, issue, self.locale);
            let placeholder_has_retry = snapshot.peers.is_empty()
                && matches!(
                    snapshot.discovery.phase(),
                    DiscoveryPhase::Failed | DiscoveryPhase::Stopped
                );
            if !placeholder_has_retry
                && matches!(
                    issue,
                    NearbyIssue::DiscoveryStopped | NearbyIssue::ListenerStopped
                )
            {
                ui.add_space(8.0);
                if ui
                    .button(self.text("重新启动附近设备服务", "Restart Nearby services"))
                    .clicked()
                {
                    self.runtime.restart_network();
                }
            }
            ui.add_space(12.0);
        }

        match snapshot.discovery.phase() {
            DiscoveryPhase::Starting => self.placeholder(
                ui,
                true,
                self.text("正在启动附近设备服务", "Starting Nearby services"),
                self.text(
                    "正在准备本地网络监听与安全身份。",
                    "Preparing the local network listener and secure identity.",
                ),
            ),
            DiscoveryPhase::Scanning if snapshot.peers.is_empty() => self.placeholder(
                ui,
                true,
                self.text("正在查找附近设备", "Searching for nearby devices"),
                self.text(
                    "请确认其他设备已打开 MoQCast，并连接到同一局域网。",
                    "Confirm that other devices have MoQCast open and are on the same LAN.",
                ),
            ),
            DiscoveryPhase::Empty | DiscoveryPhase::Ready if snapshot.peers.is_empty() => {
                self.placeholder(
                    ui,
                    false,
                    self.text("未找到附近设备", "No nearby devices found"),
                    self.text(
                        "当前扫描已完成。设备出现后会自动加入列表。",
                        "The current scan is complete. Devices will appear here automatically.",
                    ),
                );
            }
            DiscoveryPhase::Failed if snapshot.peers.is_empty() => self.recovery_placeholder(
                ui,
                self.text("无法搜索本地网络", "Could not search the local network"),
                self.text(
                    "请检查本地网络权限，然后重试。",
                    "Check Local Network permission, then try again.",
                ),
            ),
            DiscoveryPhase::Stopped if snapshot.peers.is_empty() => self.recovery_placeholder(
                ui,
                self.text("附近设备服务已停止", "Nearby services stopped"),
                self.text(
                    "可以重新启动附近设备服务。",
                    "Nearby services can be started again.",
                ),
            ),
            _ => self.device_workspace(ui, snapshot),
        }
    }

    fn local_summary(&self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let title = match snapshot.local_device_name.as_deref() {
            Some(device_name) => {
                format!("{} · {device_name}", self.text("这台 Mac", "This Mac"))
            }
            None => self.text("这台 Mac", "This Mac").to_owned(),
        };
        let status = local_status(snapshot, self.locale);
        let counts = local_count_summary(snapshot, self.locale);

        Frame::NONE
            .fill(Color32::from_white_alpha(150))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_min_height(36.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).strong());
                        ui.label(RichText::new(status).small().color(theme::MUTED));
                        if ui.ctx().content_rect().width() < 760.0 && snapshot.inbound_sessions > 0
                        {
                            ui.label(RichText::new(&counts).small().color(theme::MUTED));
                        }
                    });
                    if ui.ctx().content_rect().width() >= 760.0 {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(counts).small().color(theme::MUTED));
                        });
                    }
                });
            });
    }

    fn placeholder(&self, ui: &mut egui::Ui, busy: bool, title: &str, body: &str) {
        Frame::NONE
            .inner_margin(Margin::symmetric(24, 24))
            .show(ui, |ui| {
                ui.set_min_height(172.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(28.0);
                    if busy {
                        ui.spinner();
                    }
                    ui.label(RichText::new(title).size(16.0).strong());
                    ui.label(RichText::new(body).small().color(theme::MUTED));
                });
            });
    }

    fn recovery_placeholder(&mut self, ui: &mut egui::Ui, title: &str, body: &str) {
        Frame::NONE
            .inner_margin(Margin::symmetric(24, 24))
            .show(ui, |ui| {
                ui.set_min_height(172.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(28.0);
                    ui.label(RichText::new(title).size(16.0).strong());
                    ui.label(RichText::new(body).small().color(theme::MUTED));
                    ui.add_space(12.0);
                    if ui
                        .add_sized(
                            [132.0, 36.0],
                            egui::Button::new(self.text("重试", "Try Again")),
                        )
                        .clicked()
                    {
                        self.runtime.restart_network();
                    }
                });
            });
    }

    fn device_workspace(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let selected = self.selected_peer.clone();
        match ContentLayout::for_width(ui.ctx().content_rect().width()) {
            ContentLayout::ListDetail => {
                let list_width = (ui.available_width() - DETAIL_WIDTH - 16.0).max(320.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(list_width, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| self.device_list(ui, snapshot, selected.as_deref()),
                    );
                    ui.add_space(8.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(DETAIL_WIDTH, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| self.device_detail(ui, snapshot, selected.as_deref()),
                    );
                });
            }
            ContentLayout::SingleColumn => {
                self.device_list(ui, snapshot, selected.as_deref());
                ui.add_space(16.0);
                self.device_detail(ui, snapshot, selected.as_deref());
            }
        }
    }

    fn device_list(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot, selected: Option<&str>) {
        Frame::NONE
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(CornerRadius::same(8))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, (id, peer)) in snapshot.peers.iter().enumerate() {
                    if index > 0 {
                        ui.separator();
                    }
                    let response = device_row(ui, peer, selected == Some(id), self.locale);
                    let keyboard_activated = response.has_focus()
                        && ui.input(|input| {
                            input.key_pressed(Key::Enter) || input.key_pressed(Key::Space)
                        });
                    if response.clicked() || keyboard_activated {
                        self.selected_peer = Some(id.clone());
                    }
                }
            });
    }

    fn device_detail(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot, selected: Option<&str>) {
        let Some((peer_id, peer)) =
            selected.and_then(|id| snapshot.peers.get(id).map(|peer| (id, peer)))
        else {
            return;
        };
        let presentation = PeerPresentation::from(peer);
        let screen_path = crate::contract::screen_path(peer_id);
        let screen = screen_availability(peer_id, &snapshot.remote_screens);
        Frame::NONE
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(18))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(device_name(peer, self.locale))
                        .size(16.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(peer_line(presentation, self.locale))
                        .small()
                        .color(theme::MUTED),
                );
                ui.add_space(10.0);
                ui.separator();
                detail_row(
                    ui,
                    self.text("附近状态", "Nearby status"),
                    presence_label(presentation.presence, self.locale),
                    None,
                );
                detail_row(
                    ui,
                    self.text("安全连接", "Secure session"),
                    connection_label(presentation.connection, self.locale),
                    Some(connection_tone(presentation.connection)),
                );
                detail_row(
                    ui,
                    self.text("共享屏幕", "Shared screen"),
                    screen_label(screen, self.locale),
                    Some(screen_tone(screen)),
                );
                detail_row(
                    ui,
                    self.text("最近发现", "Last seen"),
                    if peer.discovered {
                        self.text("刚刚", "Just now")
                    } else {
                        self.text("已离开附近范围", "No longer nearby")
                    },
                    None,
                );
                if snapshot.can_watch(peer_id, &screen_path) {
                    ui.add_space(14.0);
                    let watch = ui.add_sized(
                        [ui.available_width(), 36.0],
                        egui::Button::new(
                            RichText::new(self.text("观看", "Watch"))
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(theme::BRAND)
                        .corner_radius(CornerRadius::same(6)),
                    );
                    if watch.clicked()
                        && self
                            .runtime
                            .watch_screen(peer_id.to_owned(), screen_path.clone())
                    {
                        self.page = Page::Nearby;
                    }
                }
            });
    }

    fn screen_share(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        self.page_header(
            ui,
            self.text("屏幕共享", "Screen Share"),
            self.text(
                "选择一个屏幕或窗口，并将画面发布给附近设备。",
                "Choose a display or window to share with nearby devices.",
            ),
        );
        ui.separator();
        ui.add_space(12.0);

        let permission = match self.capture_permission {
            CapturePermission::NotRequested => self.text("尚未请求", "Not requested"),
            CapturePermission::Allowed => self.text("已允许", "Allowed"),
            CapturePermission::Denied => self.text("需要系统权限", "System permission required"),
        };
        action_row(
            ui,
            62.0,
            self.text("屏幕录制权限", "Screen recording permission"),
            permission,
            |_| {},
        );
        ui.separator();

        let source = snapshot
            .share_selection
            .as_ref()
            .map(|selection| selection.label().to_owned())
            .unwrap_or_else(|| self.text("尚未选择", "Nothing selected").to_owned());
        let network_ready = snapshot.session.phase() == SessionPhase::Listening;
        let can_choose = network_ready && matches!(snapshot.media.phase(), MediaPhase::Idle);
        let choose_label = if self.picker_pending {
            self.text("正在打开…", "Opening…")
        } else {
            self.text("选择屏幕…", "Choose Screen…")
        };
        let mut choose_clicked = false;
        action_row(
            ui,
            62.0,
            self.text("共享来源", "Share source"),
            &source,
            |ui| {
                choose_clicked = ui
                    .add_enabled(
                        can_choose && !self.picker_pending,
                        egui::Button::new(choose_label).min_size(egui::vec2(132.0, 32.0)),
                    )
                    .clicked();
            },
        );
        if choose_clicked {
            self.picker_error = false;
            if self.capture_permission == CapturePermission::Allowed
                || capture_picker::request_permission()
            {
                self.capture_permission = CapturePermission::Allowed;
                self.picker_pending = true;
                let picker = self.capture_picker.get_or_insert_with(|| {
                    let repaint = ui.ctx().clone();
                    capture_picker::Picker::new(move || repaint.request_repaint())
                });
                picker.present();
            } else {
                self.capture_permission = CapturePermission::Denied;
            }
        }
        let audio_supported = snapshot
            .share_selection
            .as_ref()
            .is_some_and(crate::publication::Selection::supports_system_audio);
        let audio_status = if snapshot.share_audio == ShareAudioPhase::Failed {
            snapshot.share_audio_error.as_deref().unwrap_or_else(|| {
                self.text(
                    "系统音频不可用，视频共享继续。",
                    "System audio is unavailable. Video sharing continues.",
                )
            })
        } else if snapshot.share_audio == ShareAudioPhase::Included {
            self.text("已包含，按观看需求采集", "Included, captured on demand")
        } else if snapshot.share_system_audio {
            self.text("已开启", "On")
        } else if audio_supported {
            self.text("关闭", "Off")
        } else {
            self.text(
                "仅共享主显示器时可用",
                "Available only for the main display",
            )
        };
        let mut audio_enabled = snapshot.share_system_audio;
        let mut audio_changed = false;
        action_row(
            ui,
            62.0,
            self.text("系统音频", "System audio"),
            audio_status,
            |ui| {
                audio_changed = ui
                    .add_enabled(
                        system_audio_action_available(snapshot),
                        egui::Checkbox::without_text(&mut audio_enabled),
                    )
                    .changed();
            },
        );
        if audio_changed {
            self.runtime.set_share_system_audio(audio_enabled);
        }
        ui.label(
            RichText::new(self.text(
                "共享画面包含光标。系统音频不包含麦克风，当前仅支持主显示器。",
                "The pointer is included. System audio excludes the microphone and currently supports only the main display.",
            ))
            .small()
            .color(theme::MUTED),
        );
        ui.add_space(14.0);

        let share_owned = snapshot.media_owner == Some(MediaOwner::Share);
        let watch_owned = snapshot.media_owner == Some(MediaOwner::Watch);
        let can_start = share_action_available(self.capture_permission, snapshot);
        let (title, body, tone) = if self.picker_error {
            (
                self.text("无法打开系统选择器", "The system picker could not open"),
                self.text(
                    "请检查系统设置中的屏幕录制权限，然后重试。",
                    "Check Screen Recording permission in System Settings, then try again.",
                ),
                Tone::Error,
            )
        } else if !network_ready {
            (
                self.text("屏幕共享不可用", "Screen sharing unavailable"),
                self.text(
                    "安全直连服务尚未就绪。",
                    "The secure direct-session service is not ready.",
                ),
                Tone::Error,
            )
        } else if self.capture_permission == CapturePermission::Denied {
            (
                self.text("需要屏幕录制权限", "Screen recording permission required"),
                self.text(
                    "请在系统设置中允许 MoQCast 录制屏幕，然后重新打开应用。",
                    "Allow MoQCast to record the screen in System Settings, then reopen the app.",
                ),
                Tone::Error,
            )
        } else if watch_owned {
            (
                self.text("正在观看另一台设备", "Watching another device"),
                self.text(
                    "停止观看后才能共享这台 Mac 的屏幕。",
                    "Stop watching before sharing this Mac's screen.",
                ),
                Tone::Neutral,
            )
        } else if share_owned && snapshot.media.phase() == MediaPhase::PreparingShare {
            (
                self.text("正在准备屏幕共享", "Preparing screen sharing"),
                self.text(
                    "正在验证来源并准备 H.264 视频。安全连接保持不变。",
                    "Validating the source and preparing H.264 video. Secure sessions stay active.",
                ),
                Tone::Warning,
            )
        } else if share_owned && snapshot.media.phase() == MediaPhase::Sharing {
            if snapshot.share_audio == ShareAudioPhase::Failed {
                (
                    self.text("视频共享已就绪", "Video sharing is ready"),
                    self.text(
                        "系统音频不可用，视频仍会按观看需求发布。停止共享不会断开健康连接。",
                        "System audio is unavailable, but video continues on demand. Stopping keeps healthy sessions active.",
                    ),
                    Tone::Warning,
                )
            } else {
                (
                    self.text("屏幕共享已就绪", "Screen sharing is ready"),
                    self.text(
                    "附近设备开始观看时，将按需启动捕获与 H.264 编码。停止共享不会断开健康连接。",
                    "Capture and H.264 encoding start on demand when a nearby device watches. Stopping keeps healthy sessions active.",
                    ),
                    Tone::Success,
                )
            }
        } else if share_owned && snapshot.media.phase() == MediaPhase::Stopping {
            (
                self.text("正在停止共享", "Stopping screen sharing"),
                self.text(
                    "正在释放捕获与发布资源。",
                    "Releasing capture and publishing resources.",
                ),
                Tone::Neutral,
            )
        } else if share_owned && snapshot.media.phase() == MediaPhase::Failed {
            (
                self.text("屏幕共享已停止", "Screen sharing stopped"),
                snapshot.media_error.as_deref().unwrap_or_else(|| {
                    self.text(
                        "无法继续捕获或编码所选画面。",
                        "The selected content could not continue capturing or encoding.",
                    )
                }),
                Tone::Error,
            )
        } else if can_start {
            (
                self.text("可以开始共享", "Ready to share"),
                self.text(
                    "开始后，附近设备可通过现有直连会话观看。",
                    "Nearby devices can watch through existing direct sessions after you start.",
                ),
                Tone::Success,
            )
        } else {
            (
                self.text("选择共享来源", "Choose what to share"),
                self.text(
                    "使用系统选择器明确选择屏幕或窗口。",
                    "Use the system picker to explicitly choose a display or window.",
                ),
                Tone::Neutral,
            )
        };

        let mut action = None;
        status_panel(ui, title, body, tone, |ui| {
            if share_owned
                && matches!(
                    snapshot.media.phase(),
                    MediaPhase::PreparingShare | MediaPhase::Sharing
                )
            {
                if ui
                    .add_sized(
                        [132.0, 36.0],
                        egui::Button::new(self.text("停止共享", "Stop Sharing")),
                    )
                    .clicked()
                {
                    action = Some(false);
                }
            } else if share_owned && snapshot.media.phase() == MediaPhase::Failed {
                if ui
                    .add_sized(
                        [132.0, 36.0],
                        egui::Button::new(self.text("返回", "Return")),
                    )
                    .clicked()
                {
                    action = Some(false);
                }
            } else if !watch_owned
                && ui
                    .add_enabled(
                        can_start,
                        egui::Button::new(
                            RichText::new(self.text("开始共享", "Start Sharing"))
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(theme::BRAND)
                        .corner_radius(CornerRadius::same(6))
                        .min_size(egui::vec2(132.0, 36.0)),
                    )
                    .clicked()
            {
                action = Some(true);
            }
        });
        match action {
            Some(true) => {
                self.runtime.start_sharing();
            }
            Some(false) => {
                self.runtime.stop_sharing();
            }
            None => {}
        }
        ui.separator();
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        self.page_header(
            ui,
            self.text("设置与诊断", "Settings & Diagnostics"),
            self.text(
                "仅显示用户偏好与公开产品信息。",
                "Only user preferences and public product information are shown.",
            ),
        );
        ui.separator();
        ui.add_space(12.0);
        ui.label(
            RichText::new(self.text("通用", "General"))
                .small()
                .strong()
                .color(theme::MUTED),
        );
        ui.add_space(8.0);
        let language = self.text("语言", "Language");
        let language_hint = self.text(
            "立即应用并保存在这台 Mac 上",
            "Applies immediately and is saved on this Mac",
        );
        action_row(ui, 58.0, language, language_hint, |ui| {
            egui::ComboBox::from_id_salt("settings-locale")
                .width(180.0)
                .selected_text(match self.locale {
                    Locale::Chinese => "简体中文",
                    Locale::English => "English",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.locale, Locale::Chinese, "简体中文");
                    ui.selectable_value(&mut self.locale, Locale::English, "English");
                });
        });
        ui.separator();
        action_row(
            ui,
            58.0,
            self.text("版本", "Version"),
            self.text("公开产品版本", "Public product version"),
            |ui| {
                ui.label(RichText::new(format!("MoQCast {}", env!("CARGO_PKG_VERSION"))).strong());
            },
        );
    }
}

impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_capture_picker(ui.ctx());
        let snapshot = self.runtime.snapshot();
        self.update_playback_texture(ui.ctx(), &snapshot);
        let media_active = Self::media_active(&snapshot);
        let fullscreen = self.player.reconcile_fullscreen(ui.ctx(), media_active);
        if fullscreen {
            egui::CentralPanel::default()
                .frame(Frame::NONE.fill(Color32::BLACK))
                .show(ui, |ui| self.watch_player(ui, &snapshot));
            return;
        }
        self.handle_shortcuts(ui.ctx());
        self.top_bar(ui, &snapshot);
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(theme::CANVAS))
            .show(ui, |ui| {
                if self.page == Page::Nearby && media_active {
                    let content_width = ui.available_width().min(MAX_CONTENT_WIDTH);
                    let side_margin = ((ui.available_width() - content_width) / 2.0).max(0.0);
                    let page_padding = if ui.available_width() < 760.0 {
                        20.0
                    } else {
                        28.0
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(side_margin + page_padding);
                        ui.allocate_ui_with_layout(
                            egui::vec2(
                                (content_width - page_padding * 2.0).max(1.0),
                                (ui.available_height() - page_padding * 2.0).max(1.0),
                            ),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.add_space(page_padding);
                                self.watch_player(ui, &snapshot);
                            },
                        );
                    });
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let content_width = ui.available_width().min(MAX_CONTENT_WIDTH);
                    let side_margin = ((ui.available_width() - content_width) / 2.0).max(0.0);
                    let page_padding = if ui.available_width() < 760.0 {
                        20.0
                    } else {
                        28.0
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(side_margin);
                        ui.allocate_ui_with_layout(
                            egui::vec2(content_width, 0.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.add_space(page_padding);
                                ui.horizontal(|ui| {
                                    ui.add_space(page_padding);
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(
                                            (content_width - page_padding * 2.0).max(1.0),
                                            0.0,
                                        ),
                                        Layout::top_down(Align::Min),
                                        |ui| match self.page {
                                            Page::Nearby => self.nearby(ui, &snapshot),
                                            Page::ScreenShare => self.screen_share(ui, &snapshot),
                                            Page::Settings => self.settings(ui),
                                        },
                                    );
                                });
                                ui.add_space(page_padding);
                            },
                        );
                    });
                });
            });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_LOCALE, self.locale.stored().to_owned());
    }

    fn on_exit(&mut self) {
        self.runtime.shutdown();
    }
}

fn text(locale: Locale, chinese: &'static str, english: &'static str) -> &'static str {
    match locale {
        Locale::Chinese => chinese,
        Locale::English => english,
    }
}

fn global_summary(snapshot: &AppSnapshot, locale: Locale) -> String {
    match snapshot.discovery.phase() {
        DiscoveryPhase::Starting => text(locale, "正在启动", "Starting").to_owned(),
        DiscoveryPhase::Scanning => text(locale, "正在扫描", "Scanning").to_owned(),
        DiscoveryPhase::Failed | DiscoveryPhase::Stopped => {
            text(locale, "扫描不可用", "Scan unavailable").to_owned()
        }
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
        Locale::English => format!("{} devices · {} sessions", snapshot.peers.len(), connected),
    }
}

fn local_count_summary(snapshot: &AppSnapshot, locale: Locale) -> String {
    let summary = count_summary(snapshot, locale);
    if snapshot.inbound_sessions == 0 {
        return summary;
    }
    match locale {
        Locale::Chinese => format!("{summary} · {} 个未归属传入连接", snapshot.inbound_sessions),
        Locale::English => format!(
            "{summary} · {} unattributed incoming sessions",
            snapshot.inbound_sessions
        ),
    }
}

fn local_status(snapshot: &AppSnapshot, locale: Locale) -> &'static str {
    match (snapshot.discovery.phase(), snapshot.session.phase()) {
        (DiscoveryPhase::Starting, _) => {
            text(locale, "正在启动附近设备服务", "Starting Nearby services")
        }
        (DiscoveryPhase::Scanning, _) => {
            text(locale, "正在查找附近设备", "Searching for nearby devices")
        }
        (DiscoveryPhase::Failed | DiscoveryPhase::Stopped, _) => {
            text(locale, "本地网络不可用", "Local network unavailable")
        }
        (_, SessionPhase::Listening) => text(locale, "可被发现", "Available on the local network"),
        (_, SessionPhase::Starting) => {
            text(locale, "正在启动安全连接", "Starting secure connections")
        }
        (_, SessionPhase::Failed | SessionPhase::Stopped) => {
            text(locale, "安全连接不可用", "Secure connections unavailable")
        }
    }
}

fn issue_notice(ui: &mut egui::Ui, issue: NearbyIssue, locale: Locale) {
    let message = match issue {
        NearbyIssue::LocalNetworkUnavailable => text(
            locale,
            "无法使用本地网络。请检查系统权限。",
            "The local network is unavailable. Check system permission.",
        ),
        NearbyIssue::DirectConnectionsUnavailable => text(
            locale,
            "无法接受安全直连。",
            "Secure direct connections are unavailable.",
        ),
        NearbyIssue::DiscoveryStopped => text(
            locale,
            "附近设备扫描已停止；现有连接状态保持独立。",
            "Nearby scanning stopped; existing session state remains independent.",
        ),
        NearbyIssue::ListenerStopped => text(
            locale,
            "无法接受新的安全连接；现有设备结果仍会保留。",
            "New secure sessions cannot be accepted; existing device results remain visible.",
        ),
        NearbyIssue::ServicesStopped => text(
            locale,
            "附近设备服务已结束。可以重试恢复扫描与安全连接。",
            "Nearby services ended. Try again to restore scanning and secure connections.",
        ),
        NearbyIssue::DeviceRejected => text(
            locale,
            "一个传入连接未能通过验证。",
            "An incoming connection could not be verified.",
        ),
    };
    Frame::NONE
        .fill(theme::ERROR_SOFT)
        .stroke(Stroke::new(1.0, Color32::from_rgb(0xec, 0xc6, 0xc4)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.label(RichText::new(message).small().color(theme::ERROR));
        });
}

fn device_row(
    ui: &mut egui::Ui,
    peer: &PeerSnapshot,
    selected: bool,
    locale: Locale,
) -> egui::Response {
    let presentation = PeerPresentation::from(peer);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), DEVICE_ROW_HEIGHT),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);
    let fill = if selected {
        theme::BRAND_SOFT
    } else if response.hovered() {
        Color32::from_rgb(0xf7, 0xf9, 0xf8)
    } else {
        theme::SURFACE
    };
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    if selected {
        painter.rect_filled(
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 3.0, rect.bottom())),
            CornerRadius::ZERO,
            theme::BRAND,
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(2.0),
            CornerRadius::same(6),
            Stroke::new(2.0, theme::FOCUS),
            egui::StrokeKind::Inside,
        );
    }

    let name = device_name(peer, locale);
    let line = peer_line(presentation, locale);
    let badge_text = connection_badge(presentation.connection, locale);
    let badge_font = FontId::new(11.0, FontFamily::Proportional);
    let badge_galley = painter.layout_no_wrap(badge_text.to_owned(), badge_font, theme::TEXT);
    let badge_size = egui::vec2(badge_galley.size().x + 16.0, 22.0);
    let badge_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 12.0 - badge_size.x / 2.0, rect.center().y),
        badge_size,
    );
    let (badge_fill, badge_stroke, badge_color) =
        tone_colors(connection_tone(presentation.connection));
    painter.rect_filled(badge_rect, CornerRadius::same(8), badge_fill);
    painter.rect_stroke(
        badge_rect,
        CornerRadius::same(8),
        Stroke::new(1.0, badge_stroke),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        egui::pos2(
            badge_rect.center().x - badge_galley.size().x / 2.0,
            badge_rect.center().y - badge_galley.size().y / 2.0,
        ),
        badge_galley,
        badge_color,
    );

    let text_left = rect.left() + 12.0;
    let clip = egui::Rect::from_min_max(
        rect.min,
        egui::pos2((badge_rect.left() - 12.0).max(text_left), rect.bottom()),
    );
    let text_painter = painter.with_clip_rect(clip);
    text_painter.text(
        egui::pos2(text_left, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        name,
        FontId::new(13.0, FontFamily::Proportional),
        theme::TEXT,
    );
    text_painter.text(
        egui::pos2(text_left, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        line,
        FontId::new(12.0, FontFamily::Proportional),
        theme::MUTED,
    );
    response
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str, tone: Option<Tone>) {
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        ui.label(RichText::new(label).small().color(theme::MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match tone {
            Some(tone) => badge(ui, value, tone),
            None => {
                ui.label(RichText::new(value).small());
            }
        });
    });
    ui.separator();
}

fn badge(ui: &mut egui::Ui, label: &str, tone: Tone) {
    let (fill, stroke, color) = tone_colors(tone);
    Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(11.0).strong().color(color));
        });
}

fn action_row(
    ui: &mut egui::Ui,
    min_height: f32,
    label: &str,
    hint: &str,
    action: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.set_min_height(min_height);
        ui.vertical(|ui| {
            ui.label(RichText::new(label).strong());
            ui.label(RichText::new(hint).small().color(theme::MUTED));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), action);
    });
}

fn status_panel(
    ui: &mut egui::Ui,
    title: &str,
    body: &str,
    tone: Tone,
    action: impl FnOnce(&mut egui::Ui),
) {
    let (fill, stroke, color) = tone_colors(tone);
    Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).strong().color(color));
                    ui.label(RichText::new(body).small().color(theme::MUTED));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), action);
            });
        });
}

fn share_action_available(permission: CapturePermission, snapshot: &AppSnapshot) -> bool {
    permission == CapturePermission::Allowed
        && snapshot.session.phase() == SessionPhase::Listening
        && snapshot.media.phase() == MediaPhase::Idle
        && snapshot.share_selection.is_some()
}

fn system_audio_action_available(snapshot: &AppSnapshot) -> bool {
    snapshot.media.phase() == MediaPhase::Idle
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

fn peer_line(peer: PeerPresentation, locale: Locale) -> &'static str {
    match (peer.presence, peer.connection) {
        (PresenceView::NotNearby, ConnectionView::Connected) => text(
            locale,
            "已离开附近范围 · 连接仍在",
            "No longer nearby · Session active",
        ),
        (PresenceView::NotNearby, _) => text(locale, "已离开附近范围", "No longer nearby"),
        (PresenceView::Nearby, ConnectionView::Waiting) => {
            text(locale, "附近 · 等待对端连接", "Nearby · Waiting for device")
        }
        (PresenceView::Nearby, ConnectionView::ConnectingSecurely) => text(
            locale,
            "附近 · 正在安全连接",
            "Nearby · Connecting securely",
        ),
        (PresenceView::Nearby, ConnectionView::Connected) => {
            text(locale, "附近 · 已连接", "Nearby · Connected")
        }
        (PresenceView::Nearby, ConnectionView::Rejected) => text(
            locale,
            "附近 · 无法验证连接",
            "Nearby · Could not verify connection",
        ),
        (PresenceView::Nearby, ConnectionView::Failed) => {
            text(locale, "附近 · 连接失败", "Nearby · Connection failed")
        }
        (PresenceView::Nearby, ConnectionView::Disconnected) => {
            text(locale, "附近 · 连接已结束", "Nearby · Session ended")
        }
    }
}

fn presence_label(presence: PresenceView, locale: Locale) -> &'static str {
    match presence {
        PresenceView::Nearby => text(locale, "附近", "Nearby"),
        PresenceView::NotNearby => text(locale, "已离开附近范围", "No longer nearby"),
    }
}

fn connection_label(connection: ConnectionView, locale: Locale) -> &'static str {
    match connection {
        ConnectionView::Waiting => text(locale, "等待中", "Waiting"),
        ConnectionView::ConnectingSecurely => text(locale, "正在安全连接", "Connecting securely"),
        ConnectionView::Connected => text(locale, "已连接", "Connected"),
        ConnectionView::Rejected => text(locale, "无法验证", "Could not verify"),
        ConnectionView::Failed => text(locale, "连接失败", "Connection failed"),
        ConnectionView::Disconnected => text(locale, "连接已结束", "Session ended"),
    }
}

fn connection_badge(connection: ConnectionView, locale: Locale) -> &'static str {
    match connection {
        ConnectionView::ConnectingSecurely => text(locale, "连接中", "Connecting"),
        _ => connection_label(connection, locale),
    }
}

fn screen_label(screen: ScreenAvailability, locale: Locale) -> &'static str {
    match screen {
        ScreenAvailability::Unavailable => text(locale, "未共享", "Not shared"),
        ScreenAvailability::Available => text(locale, "可观看", "Available"),
        ScreenAvailability::Withdrawn => text(locale, "共享已结束", "Share ended"),
    }
}

#[derive(Clone, Copy)]
enum Tone {
    Neutral,
    Success,
    Warning,
    Error,
}

fn connection_tone(connection: ConnectionView) -> Tone {
    match connection {
        ConnectionView::Connected => Tone::Success,
        ConnectionView::ConnectingSecurely => Tone::Warning,
        ConnectionView::Rejected | ConnectionView::Failed => Tone::Error,
        ConnectionView::Waiting | ConnectionView::Disconnected => Tone::Neutral,
    }
}

fn screen_tone(screen: ScreenAvailability) -> Tone {
    match screen {
        ScreenAvailability::Available => Tone::Success,
        ScreenAvailability::Withdrawn => Tone::Warning,
        ScreenAvailability::Unavailable => Tone::Neutral,
    }
}

fn tone_colors(tone: Tone) -> (Color32, Color32, Color32) {
    match tone {
        Tone::Neutral => (
            Color32::from_rgb(0xf5, 0xf6, 0xf6),
            Color32::from_rgb(0xd0, 0xd6, 0xd3),
            Color32::from_rgb(0x53, 0x60, 0x5b),
        ),
        Tone::Success => (
            Color32::from_rgb(0xea, 0xf6, 0xf2),
            Color32::from_rgb(0xa9, 0xd1, 0xc6),
            theme::BRAND_PRESSED,
        ),
        Tone::Warning => (
            theme::WARNING_SOFT,
            Color32::from_rgb(0xe1, 0xc9, 0x90),
            Color32::from_rgb(0x7a, 0x4c, 0x00),
        ),
        Tone::Error => (
            theme::ERROR_SOFT,
            Color32::from_rgb(0xe0, 0xb0, 0xad),
            theme::ERROR,
        ),
    }
}

fn configure_fonts(context: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};

    context.add_font(FontInsert::new(
        "Noto Sans SC",
        egui::FontData::from_static(include_bytes!("../assets/fonts/NotoSansSC-Regular.otf")),
        vec![InsertFontFamily {
            family: egui::FontFamily::Proportional,
            priority: FontPriority::Lowest,
        }],
    ));
}

#[cfg(test)]
mod tests;
