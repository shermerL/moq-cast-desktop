//! Native macOS application shell for Nearby and direct-only sessions.

mod capture_picker;
mod diagnostics;
mod player;
mod system_lifecycle;
mod view;

use eframe::egui::{self, Align, Color32, Frame, Key, Layout, Modifiers};
use moqcast_ui::{
    BadgeTone, COLORS, DetailRowSpec, DeviceRowSpec, NavItemSpec, PageWidth, SelectSpec,
    SettingRowSpec, Size, Spacing, StatePanelKind, StatePanelSpec, SwitchSpec, Theme,
    TypographyRole, app_bar_content_rect, danger_button, detail_row as compact_detail_row,
    device_row, install_ui_font, nav_item, page_header, page_shell, primary_button,
    secondary_button, section_header, select, setting_row, state_panel, status_badge, status_strip,
    switch, typography,
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
    RuntimePhase, SessionPhase, ShareAudioPhase,
};

const STORAGE_LOCALE: &str = "moqcast.macos.locale";
const STORAGE_DEVELOPER_MODE: &str = "moqcast.macos.developer-mode";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Page {
    #[default]
    Nearby,
    ScreenShare,
    Watch,
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
        egui::Id::new(("mac-page-scroll", self))
    }
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
    selected_peer: Option<String>,
    runtime: RuntimeOwner,
    system_lifecycle: Option<system_lifecycle::Observer>,
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
    pub(crate) fn new(
        context: &eframe::CreationContext<'_>,
        diagnostics: moqcast_diagnostics::Handle,
    ) -> anyhow::Result<Self> {
        install_ui_font(
            &context.egui_ctx,
            std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC),
        );
        Theme.apply(&context.egui_ctx);
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
            selected_peer: None,
            runtime,
            system_lifecycle,
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
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Num3))) {
            self.page = Page::Watch;
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Num4)))
            || context.input_mut(|input| input.consume_shortcut(&shortcut(Key::Comma)))
        {
            self.page = Page::Settings;
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
            .frame(Frame::new().fill(COLORS.chrome.into()))
            .exact_size(navigation_height(root.ctx().content_rect().width()))
            .show(root, |ui| {
                let content =
                    app_bar_content_rect(ui.max_rect()).shrink2(egui::vec2(0.0, Spacing::SM));
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(content)
                        .layout(Layout::top_down(Align::Min)),
                    |ui| match NavigationLayout::for_width(ui.ctx().content_rect().width()) {
                        NavigationLayout::OneRow => {
                            ui.allocate_ui_with_layout(
                                content.size(),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.label(typography(
                                        "MoQCast Desktop",
                                        TypographyRole::Row,
                                        COLORS.text.into(),
                                    ));
                                    ui.add_space(Spacing::XL);
                                    self.navigation(ui);
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(typography(
                                            &summary,
                                            TypographyRole::Meta,
                                            COLORS.muted.into(),
                                        ));
                                    });
                                },
                            );
                        }
                        NavigationLayout::TwoRows => {
                            let top_height = (content.height() - Spacing::XS - Size::NAV).max(1.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(content.width(), top_height),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.label(typography(
                                        "MoQCast Desktop",
                                        TypographyRole::Row,
                                        COLORS.text.into(),
                                    ));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(typography(
                                            &summary,
                                            TypographyRole::Meta,
                                            COLORS.muted.into(),
                                        ));
                                    });
                                },
                            );
                            ui.add_space(Spacing::XS);
                            self.navigation(ui);
                        }
                    },
                );
            });
    }

    fn navigation(&mut self, ui: &mut egui::Ui) {
        let labels = [
            self.text("附近设备", "Nearby"),
            self.text("屏幕共享", "Screen Share"),
            self.text("观看", "Watch"),
            self.text("设置", "Settings"),
        ];
        let pages = [Page::Nearby, Page::ScreenShare, Page::Watch, Page::Settings];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = Spacing::XS;
            for (index, (page, label)) in pages.into_iter().zip(labels).enumerate() {
                if nav_item(
                    ui,
                    NavItemSpec::new(egui::Id::new(("mac-nav", index)), label)
                        .selected(self.page == page),
                )
                .clicked()
                {
                    self.page = page;
                }
            }
        });
    }

    fn nearby(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        self.selected_peer = selected_peer(self.selected_peer.as_deref(), &snapshot.peers);
        page_header(
            ui,
            self.text("附近设备", "Nearby Devices"),
            Some(self.text(
                "查找同一局域网中的 MoQCast 设备，选择可用屏幕开始观看。",
                "Find MoQCast devices on this local network and choose an available screen to watch.",
            )),
        );
        self.local_summary(ui, snapshot);
        ui.add_space(Spacing::LG);

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
                ui.add_space(Spacing::SM);
                if secondary_button(
                    ui,
                    self.text("重新启动附近设备服务", "Restart Nearby services"),
                    true,
                )
                .clicked()
                {
                    self.runtime.restart_network();
                }
            }
            ui.add_space(Spacing::LG);
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
            DiscoveryPhase::Stopped
                if snapshot.runtime.phase() == RuntimePhase::Suspended
                    && snapshot.peers.is_empty() =>
            {
                self.placeholder(
                    ui,
                    false,
                    self.text("Mac 正在睡眠", "Mac is sleeping"),
                    self.text(
                        "附近设备和媒体服务将在唤醒后自动恢复。",
                        "Nearby and media services will resume after wake.",
                    ),
                );
            }
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
        let counts = count_summary(snapshot, self.locale);

        setting_row(ui, SettingRowSpec::new(&title).description(status), |ui| {
            ui.label(typography(
                counts,
                TypographyRole::Meta,
                COLORS.muted.into(),
            ));
        });
    }

    fn placeholder(&self, ui: &mut egui::Ui, busy: bool, title: &str, body: &str) {
        state_panel(
            ui,
            StatePanelSpec::new(
                if busy {
                    StatePanelKind::Pending
                } else {
                    StatePanelKind::Empty
                },
                title,
                body,
            ),
            |ui| {
                if busy {
                    ui.spinner();
                }
            },
        );
    }

    fn recovery_placeholder(&mut self, ui: &mut egui::Ui, title: &str, body: &str) {
        let mut retry = false;
        state_panel(
            ui,
            StatePanelSpec::new(StatePanelKind::Failed, title, body),
            |ui| {
                retry = primary_button(ui, self.text("重试", "Try Again"), true).clicked();
            },
        );
        if retry {
            self.runtime.restart_network();
        }
    }

    fn device_workspace(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let selected = self.selected_peer.clone();
        match ContentLayout::for_width(ui.ctx().content_rect().width()) {
            ContentLayout::ListDetail => {
                let list_width = (ui.available_width() - Size::NEARBY_LIST - Spacing::LG)
                    .max(Size::WORKSPACE_MIN);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(list_width, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| self.device_list(ui, snapshot, selected.as_deref()),
                    );
                    ui.add_space(Spacing::SM);
                    ui.allocate_ui_with_layout(
                        egui::vec2(Size::NEARBY_LIST, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| self.device_detail(ui, snapshot, selected.as_deref()),
                    );
                });
            }
            ContentLayout::SingleColumn => {
                self.device_list(ui, snapshot, selected.as_deref());
                ui.add_space(Spacing::LG);
                self.device_detail(ui, snapshot, selected.as_deref());
            }
        }
    }

    fn device_list(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot, selected: Option<&str>) {
        ui.spacing_mut().item_spacing.y = Spacing::XS;
        for (id, peer) in &snapshot.peers {
            let presentation = PeerPresentation::from(peer);
            let title = device_name(peer, self.locale);
            let detail = peer_line(presentation, self.locale);
            let (response, ()) = device_row(
                ui,
                DeviceRowSpec::new(egui::Id::new(("nearby-peer", id)), &title)
                    .detail(detail)
                    .selected(selected == Some(id)),
                |ui| {
                    status_badge(
                        ui,
                        connection_badge(presentation.connection, self.locale),
                        connection_tone(presentation.connection),
                    );
                },
            );
            let keyboard_activated = response.has_focus()
                && ui.input(|input| input.key_pressed(Key::Enter) || input.key_pressed(Key::Space));
            if response.clicked() || keyboard_activated {
                self.selected_peer = Some(id.clone());
            }
        }
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
        let peer_name = device_name(peer, self.locale);
        section_header(ui, &peer_name, Some(peer_line(presentation, self.locale)));
        detail_row(
            ui,
            self.text("附近状态", "Nearby status"),
            presence_label(presentation.presence, self.locale),
            None,
        );
        detail_row(
            ui,
            self.text("连接", "Connection"),
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
            ui.add_space(Spacing::LG);
            if primary_button(ui, self.text("观看", "Watch"), true).clicked()
                && self
                    .runtime
                    .watch_screen(peer_id.to_owned(), screen_path.clone())
            {
                self.page = Page::Watch;
            }
        }
    }

    fn screen_share(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        page_header(
            ui,
            self.text("屏幕共享", "Screen Share"),
            Some(self.text(
                "选择一个屏幕或窗口，并将画面发布给附近设备。",
                "Choose a display or window to share with nearby devices.",
            )),
        );
        section_header(
            ui,
            self.text("共享设置", "Share settings"),
            Some(self.text(
                "系统选择器只显示实际可共享的屏幕和窗口。",
                "The system picker shows only screens and windows that can be shared.",
            )),
        );

        let permission = match self.capture_permission {
            CapturePermission::NotRequested => self.text("尚未请求", "Not requested"),
            CapturePermission::Allowed => self.text("已允许", "Allowed"),
            CapturePermission::Denied => self.text("需要系统权限", "System permission required"),
        };
        detail_row(
            ui,
            self.text("屏幕录制权限", "Screen recording permission"),
            permission,
            None,
        );

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
            self.text("共享来源", "Share source"),
            &source,
            |ui| {
                choose_clicked =
                    secondary_button(ui, choose_label, can_choose && !self.picker_pending)
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
        ui.separator();
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
            self.text("系统音频", "System audio"),
            audio_status,
            |ui| {
                audio_changed = switch(
                    ui,
                    &mut audio_enabled,
                    SwitchSpec::new(self.text("系统音频", "System audio"))
                        .enabled(system_audio_action_available(snapshot)),
                )
                .changed();
            },
        );
        if audio_changed {
            self.runtime.set_share_system_audio(audio_enabled);
        }
        ui.label(typography(
            self.text(
                "共享画面包含光标。系统音频不包含麦克风，当前仅支持主显示器。",
                "The pointer is included. System audio excludes the microphone and currently supports only the main display.",
            ),
            TypographyRole::Help,
            COLORS.muted.into(),
        ));
        ui.add_space(Spacing::XL);

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
                    "附近设备连接尚未就绪。",
                    "Nearby device connections are not ready.",
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
                    "正在验证来源并准备画面。附近连接保持不变。",
                    "Validating the source and preparing video. Nearby connections stay active.",
                ),
                Tone::Warning,
            )
        } else if share_owned && snapshot.media.phase() == MediaPhase::Sharing {
            if snapshot.share_audio == ShareAudioPhase::Failed {
                (
                    self.text("视频共享已就绪", "Video sharing is ready"),
                    self.text(
                        "系统音频不可用，视频仍会按观看需求发布。停止共享不会断开附近连接。",
                        "System audio is unavailable, but video continues on demand. Stopping keeps Nearby connections active.",
                    ),
                    Tone::Warning,
                )
            } else {
                (
                    self.text("屏幕共享已就绪", "Screen sharing is ready"),
                    self.text(
                    "附近设备开始观看时，将按需启动画面共享。停止共享不会断开附近连接。",
                    "Screen sharing starts on demand when a nearby device watches. Stopping keeps Nearby connections active.",
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
                    "开始后，附近设备即可观看。",
                    "Nearby devices can watch after you start.",
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
        status_section(ui, title, body, tone, |ui| {
            if share_owned
                && matches!(
                    snapshot.media.phase(),
                    MediaPhase::PreparingShare | MediaPhase::Sharing
                )
            {
                if danger_button(ui, self.text("停止共享", "Stop Sharing"), true).clicked() {
                    action = Some(false);
                }
            } else if share_owned && snapshot.media.phase() == MediaPhase::Failed {
                if secondary_button(ui, self.text("返回", "Return"), true).clicked() {
                    action = Some(false);
                }
            } else if !watch_owned
                && primary_button(ui, self.text("开始共享", "Start Sharing"), can_start).clicked()
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
    }

    fn watch(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        page_header(
            ui,
            self.text("观看附近屏幕", "Watch a Nearby Screen"),
            Some(self.text(
                "画面保持原始宽高比，播放状态不会改变舞台尺寸。",
                "Video keeps its original aspect ratio, and playback state does not resize the stage.",
            )),
        );
        let projection = watch_projection(snapshot.media_owner, snapshot.media.phase());
        if projection == WatchProjection::Player {
            self.watch_player(ui, snapshot);
            return;
        }

        let failed = projection == WatchProjection::Failed;
        let mut open_nearby = false;
        state_panel(
            ui,
            StatePanelSpec::new(
                if failed {
                    StatePanelKind::Failed
                } else {
                    StatePanelKind::Empty
                },
                if failed {
                    self.text("无法播放", "Playback unavailable")
                } else {
                    self.text("选择一个可观看屏幕", "Choose a screen to watch")
                },
                if failed {
                    self.text(
                        "播放未能继续。停止观看后，附近连接仍会保留。",
                        "Playback could not continue. Stopping keeps the Nearby session connected.",
                    )
                } else {
                    self.text(
                        "打开附近设备，选择正在共享屏幕的设备。",
                        "Open Nearby and select a device that is sharing a screen.",
                    )
                },
            ),
            |ui| {
                open_nearby = primary_button(
                    ui,
                    if failed {
                        self.text("停止并返回", "Stop and Return")
                    } else {
                        self.text("打开附近设备", "Open Nearby")
                    },
                    true,
                )
                .clicked();
            },
        );
        if open_nearby {
            if failed {
                self.runtime.stop_watching();
            }
            self.page = Page::Nearby;
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            self.text("设置", "Settings"),
            Some(if self.developer_mode {
                self.text(
                    "开发者诊断已启用，本地日志工具仅保留在这台 Mac 上。",
                    "Developer diagnostics are enabled; local log tools stay on this Mac.",
                )
            } else {
                self.text(
                    "仅显示用户偏好与公开产品信息。",
                    "Only user preferences and public product information are shown.",
                )
            }),
        );
        section_header(ui, self.text("通用", "General"), None);
        let language = self.text("语言", "Language");
        let language_hint = self.text(
            "立即应用并保存在这台 Mac 上",
            "Applies immediately and is saved on this Mac",
        );
        action_row(ui, language, language_hint, |ui| {
            const OPTIONS: [&str; 2] = ["简体中文", "English"];
            let mut selected = usize::from(self.locale == Locale::English);
            if select(
                ui,
                &mut selected,
                SelectSpec::new(egui::Id::new("settings-locale"), language, &OPTIONS)
                    .expect("the locale list is not empty"),
            )
            .expect("the selected locale index is valid")
            .changed()
            {
                self.locale = if selected == 0 {
                    Locale::Chinese
                } else {
                    Locale::English
                };
            }
        });
        ui.separator();
        action_row(
            ui,
            self.text("版本", "Version"),
            self.text("公开产品版本", "Public product version"),
            |ui| {
                ui.label(typography(
                    format!("MoQCast {}", env!("CARGO_PKG_VERSION")),
                    TypographyRole::Meta,
                    COLORS.muted.into(),
                ));
            },
        );
        ui.add_space(Spacing::XL);
        section_header(ui, self.text("高级", "Advanced"), None);
        let was_developer_mode = self.developer_mode;
        let developer_mode_label = self.text("开发者模式", "Developer mode");
        action_row(
            ui,
            developer_mode_label,
            self.text(
                "开启后显示本地日志与导出工具。",
                "Shows local logs and export tools when enabled.",
            ),
            |ui| {
                switch(
                    ui,
                    &mut self.developer_mode,
                    SwitchSpec::new(developer_mode_label),
                );
            },
        );
        if was_developer_mode != self.developer_mode {
            self.diagnostics.set_detailed(self.developer_mode);
            if !self.developer_mode {
                self.diagnostics.hide_window();
            }
        }
        if self.developer_mode {
            ui.add_space(Spacing::XL);
            self.diagnostics.show_settings(ui, self.locale);
        }
    }
}

impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_capture_picker(ui.ctx());
        let snapshot = self.runtime.snapshot();
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
        self.top_bar(ui, &snapshot);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(COLORS.surface.into()))
            .show(ui, |ui| {
                let page = self.page;
                if page == Page::Watch {
                    page_shell(ui, page.content_width(), |ui| self.watch(ui, &snapshot));
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt(page.scroll_id())
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            page_shell(ui, page.content_width(), |ui| match page {
                                Page::Nearby => self.nearby(ui, &snapshot),
                                Page::ScreenShare => self.screen_share(ui, &snapshot),
                                Page::Settings => self.settings(ui),
                                Page::Watch => {
                                    unreachable!("Watch does not use the page scroller")
                                }
                            });
                        });
                }
            });
        self.diagnostics.show_window(ui.ctx(), self.locale);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
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

fn global_summary(snapshot: &AppSnapshot, locale: Locale) -> String {
    if snapshot.runtime.phase() == RuntimePhase::Suspended {
        return text(locale, "已暂停", "Suspended").to_owned();
    }
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
            text(locale, "正在准备设备连接", "Preparing device connections")
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
        NearbyIssue::DeviceRejected => text(
            locale,
            "一个传入连接未能通过验证。",
            "An incoming connection could not be verified.",
        ),
    };
    state_panel(
        ui,
        StatePanelSpec::new(
            StatePanelKind::Failed,
            text(locale, "附近服务需要处理", "Nearby needs attention"),
            message,
        ),
        |_| {},
    );
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str, tone: Option<BadgeTone>) {
    compact_detail_row(ui, DetailRowSpec::new(label), |ui| match tone {
        Some(tone) => {
            status_badge(ui, value, tone);
        }
        None => {
            ui.label(typography(value, TypographyRole::Meta, COLORS.muted.into()));
        }
    });
    ui.separator();
}

fn action_row(ui: &mut egui::Ui, label: &str, hint: &str, action: impl FnOnce(&mut egui::Ui)) {
    setting_row(ui, SettingRowSpec::new(label).description(hint), action);
}

fn status_section(
    ui: &mut egui::Ui,
    title: &str,
    body: &str,
    tone: Tone,
    action: impl FnOnce(&mut egui::Ui),
) {
    let kind = match tone {
        Tone::Neutral | Tone::Success => StatePanelKind::Empty,
        Tone::Warning => StatePanelKind::Pending,
        Tone::Error => StatePanelKind::Failed,
    };
    status_strip(ui, StatePanelSpec::new(kind, title, body), action);
}

fn share_action_available(permission: CapturePermission, snapshot: &AppSnapshot) -> bool {
    permission == CapturePermission::Allowed
        && snapshot.session.phase() == SessionPhase::Listening
        && snapshot.media.phase() == MediaPhase::Idle
        && snapshot.share_selection.is_some()
}

fn system_audio_action_available(snapshot: &AppSnapshot) -> bool {
    snapshot.session.phase() == SessionPhase::Listening
        && snapshot.media.phase() == MediaPhase::Idle
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

fn connection_tone(connection: ConnectionView) -> BadgeTone {
    match connection {
        ConnectionView::Connected => BadgeTone::Info,
        ConnectionView::ConnectingSecurely => BadgeTone::Warning,
        ConnectionView::Rejected | ConnectionView::Failed => BadgeTone::Danger,
        ConnectionView::Waiting | ConnectionView::Disconnected => BadgeTone::Neutral,
    }
}

fn screen_tone(screen: ScreenAvailability) -> BadgeTone {
    match screen {
        ScreenAvailability::Available => BadgeTone::Info,
        ScreenAvailability::Withdrawn => BadgeTone::Warning,
        ScreenAvailability::Unavailable => BadgeTone::Neutral,
    }
}

fn navigation_height(width: f32) -> f32 {
    if width < Size::SPLIT_BREAKPOINT {
        Size::APP_BAR_COMPACT
    } else {
        Size::APP_BAR
    }
}

#[cfg(test)]
mod tests;
