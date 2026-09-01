//! Compact native UI for discovery and direct peer sessions.

use eframe::egui::{self, Align, Color32, Frame, Layout, RichText, Sense, Stroke};

use crate::{
    audio::AudioPhase,
    diagnostics::DiagnosticsUi,
    media::{MediaPhase, VideoEncodingPolicy},
    playback::{PlaybackFrameIdentity, ViewAudioPhase, ViewPhase},
    player::{LivePlayer, PlayerAction, TOOLBAR_HEIGHT},
    remote::ScreenAvailability,
    runtime::{
        DiscoveryState, PeerView, RuntimeCommand, RuntimeOwner, RuntimeSnapshot, TransportPhaseView,
    },
};

const CONTENT_MAX_WIDTH: f32 = 1040.0;
const CONTENT_TOP_SPACING: f32 = 24.0;
const NEARBY_SPLIT_MIN_WIDTH: f32 = 760.0;
const NEARBY_LIST_MAX_WIDTH: f32 = 360.0;
const COMPACT_NAVIGATION_HEIGHT: f32 = 96.0;
const DESKTOP_NAVIGATION_HEIGHT: f32 = 52.0;
const SETTINGS_STACK_WIDTH: f32 = 720.0;
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

fn content_rect(available: egui::Rect) -> egui::Rect {
    let width = available.width().min(CONTENT_MAX_WIDTH);
    egui::Rect::from_min_size(
        egui::pos2(
            available.center().x - width / 2.0,
            available.top() + CONTENT_TOP_SPACING,
        ),
        egui::vec2(width, (available.height() - CONTENT_TOP_SPACING).max(1.0)),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Nearby,
    ScreenShare,
    Watch,
    Settings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SettingsPane {
    #[default]
    Preferences,
    Diagnostics,
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

    fn screen_share(self) -> &'static str {
        match self {
            Self::Chinese => "屏幕共享",
            Self::English => "Screen share",
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
    settings_pane: SettingsPane,
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
        configure_fonts(&context.egui_ctx);
        context.egui_ctx.set_visuals(egui::Visuals::light());
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
            settings_pane: SettingsPane::Preferences,
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

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        if navigation_height(ui.available_width()) == COMPACT_NAVIGATION_HEIGHT {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("MoQCast Desktop").size(17.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.small(discovery_summary(self.locale, &self.snapshot));
                    });
                });
                ui.add_space(4.0);
                self.navigation_buttons(ui);
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new("MoQCast Desktop").size(17.0).strong());
                ui.add_space(24.0);
                self.navigation_buttons(ui);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.small(discovery_summary(self.locale, &self.snapshot));
                });
            });
        }
    }

    fn navigation_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.selectable_value(&mut self.page, Page::Nearby, self.locale.nearby());
            ui.selectable_value(
                &mut self.page,
                Page::ScreenShare,
                self.locale.screen_share(),
            );
            ui.selectable_value(&mut self.page, Page::Watch, self.locale.watch());
            ui.selectable_value(&mut self.page, Page::Settings, self.locale.settings());
        });
    }

    fn request_turn_off_nearby(&mut self) {
        if self.nearby_turn_off_pending || self.snapshot.discovery == DiscoveryState::Stopping {
            return;
        }
        if self.send(RuntimeCommand::StopScan) {
            self.nearby_turn_off_pending = true;
        }
    }

    fn show_turn_off_nearby_confirmation(&mut self, context: &egui::Context) {
        if !self.confirm_turn_off_nearby {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(match self.locale {
            Locale::Chinese => "关闭附近设备？",
            Locale::English => "Turn off Nearby?",
        })
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(context, |ui| {
            ui.label(match self.locale {
                Locale::Chinese => "关闭会停止当前观看或共享，并断开附近连接。",
                Locale::English => {
                    "Turning off Nearby stops the current watch or share and closes nearby connections."
                }
            });
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                cancel = ui
                    .button(match self.locale {
                        Locale::Chinese => "取消",
                        Locale::English => "Cancel",
                    })
                    .clicked();
                confirm = ui
                    .button(match self.locale {
                        Locale::Chinese => "关闭附近设备",
                        Locale::English => "Turn off Nearby",
                    })
                    .clicked();
            });
        });
        if cancel {
            self.confirm_turn_off_nearby = false;
        } else if confirm {
            self.confirm_turn_off_nearby = false;
            self.request_turn_off_nearby();
        }
    }

    fn nearby(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            self.locale.nearby(),
            match self.locale {
                Locale::Chinese => "查看局域网中的设备、连接与可观看屏幕。",
                Locale::English => {
                    "Review LAN devices, connections, and screens available to watch."
                }
            },
        );
        let nearby_active = self.snapshot.discovery.is_active();
        let turning_off =
            self.nearby_turn_off_pending || self.snapshot.discovery == DiscoveryState::Stopping;
        let nearby_action = match (self.locale, self.snapshot.discovery, turning_off) {
            (Locale::Chinese, _, true) => "正在关闭",
            (Locale::English, _, true) => "Turning off",
            (Locale::Chinese, DiscoveryState::Failed, false) => "重试",
            (Locale::English, DiscoveryState::Failed, false) => "Try again",
            (Locale::Chinese, _, false) if nearby_active => "关闭附近设备",
            (Locale::English, _, false) if nearby_active => "Turn off Nearby",
            (Locale::Chinese, _, false) => "开启附近设备",
            (Locale::English, _, false) => "Turn on Nearby",
        };
        ui.separator();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(match self.locale {
                        Locale::Chinese => "本机设备",
                        Locale::English => "This device",
                    })
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
                ui.label(RichText::new(&self.snapshot.local_device_name).strong());
                ui.small(match (self.locale, self.snapshot.discovery) {
                    (Locale::Chinese, DiscoveryState::Starting) => "正在开启附近设备",
                    (Locale::English, DiscoveryState::Starting) => "Turning on Nearby",
                    (Locale::Chinese, DiscoveryState::Ready | DiscoveryState::Empty) => {
                        "附近设备已开启，正在自动查找"
                    }
                    (Locale::English, DiscoveryState::Ready | DiscoveryState::Empty) => {
                        "Nearby is on and discovering automatically"
                    }
                    (Locale::Chinese, DiscoveryState::Stopping) => "正在停止媒体并关闭附近连接",
                    (Locale::English, DiscoveryState::Stopping) => {
                        "Stopping media and closing Nearby connections"
                    }
                    (Locale::Chinese, DiscoveryState::Stopped) => "附近设备已关闭",
                    (Locale::English, DiscoveryState::Stopped) => "Nearby is off",
                    (Locale::Chinese, DiscoveryState::Failed) => "附近设备不可用",
                    (Locale::English, DiscoveryState::Failed) => "Nearby is unavailable",
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(!turning_off, egui::Button::new(nearby_action))
                    .clicked()
                {
                    if nearby_active {
                        if has_active_media(&self.snapshot) {
                            self.confirm_turn_off_nearby = true;
                        } else {
                            self.request_turn_off_nearby();
                        }
                    } else {
                        self.send(RuntimeCommand::StartScan);
                    }
                }
            });
        });
        ui.separator();
        ui.add_space(8.0);

        let peers: Vec<_> = self
            .snapshot
            .peers
            .values()
            .filter(|peer| peer.present)
            .cloned()
            .collect();
        self.selected_peer = selected_peer(&peers, self.selected_peer.as_deref());

        if peers.is_empty() {
            self.nearby_empty(ui);
            return;
        }

        let selected = self
            .selected_peer
            .as_deref()
            .and_then(|id| peers.iter().find(|peer| peer.id == id))
            .cloned();
        if nearby_uses_stacked_layout(ui.available_width()) {
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| self.peer_list(ui, &peers));
            ui.separator();
            self.peer_detail(ui, selected.as_ref());
        } else {
            let list_width = nearby_list_width(ui.available_width());
            let height = ui.available_height();
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(list_width, height),
                    Layout::top_down(Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.peer_list(ui, &peers));
                    },
                );
                ui.separator();
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), height),
                    Layout::top_down(Align::Min),
                    |ui| self.peer_detail(ui, selected.as_ref()),
                );
            });
        }
    }

    fn nearby_empty(&mut self, ui: &mut egui::Ui) {
        let (title, help) = match (self.locale, self.snapshot.discovery) {
            (Locale::Chinese, DiscoveryState::Starting) => {
                ("正在启动附近服务", "正在准备扫描和安全连接。")
            }
            (Locale::English, DiscoveryState::Starting) => (
                "Starting Nearby services",
                "Preparing scanning and secure connections.",
            ),
            (Locale::Chinese, DiscoveryState::Failed) => {
                ("附近服务不可用", "请检查本地网络和防火墙设置，然后重试。")
            }
            (Locale::English, DiscoveryState::Failed) => (
                "Nearby services unavailable",
                "Check local network and firewall settings, then try again.",
            ),
            (Locale::Chinese, DiscoveryState::Stopping) => {
                ("正在关闭附近设备", "正在停止媒体并关闭局域网连接。")
            }
            (Locale::English, DiscoveryState::Stopping) => (
                "Turning off Nearby",
                "Stopping media and closing LAN connections.",
            ),
            (Locale::Chinese, DiscoveryState::Stopped) => {
                ("附近设备已关闭", "需要时可以重新开启附近设备。")
            }
            (Locale::English, DiscoveryState::Stopped) => {
                ("Nearby is off", "Turn on Nearby again when you need it.")
            }
            (Locale::Chinese, _) => (
                "暂未发现附近设备",
                "请确认另一台设备已打开 MoQCast，然后重新扫描。",
            ),
            (Locale::English, _) => (
                "No nearby devices found",
                "Make sure MoQCast is open on another device, then scan again.",
            ),
        };
        Frame::new()
            .stroke(Stroke::new(
                1.0,
                ui.visuals().widgets.noninteractive.bg_stroke.color,
            ))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(24, 24))
            .show(ui, |ui| {
                ui.label(RichText::new(title).size(18.0).strong());
                ui.add_space(4.0);
                ui.label(help);
            });
    }

    fn peer_list(&mut self, ui: &mut egui::Ui, peers: &[PeerView]) {
        for peer in peers {
            let selected = self.selected_peer.as_deref() == Some(peer.id.as_str());
            let response = ui.add_sized(
                [ui.available_width(), 72.0],
                egui::Button::selectable(
                    selected,
                    format!(
                        "{}\n{}",
                        peer_display_name(self.locale, &peer.id),
                        peer_status(self.locale, peer)
                    ),
                ),
            );
            if response.clicked() {
                self.selected_peer = Some(peer.id.clone());
            }
        }
    }

    fn peer_detail(&mut self, ui: &mut egui::Ui, peer: Option<&PeerView>) {
        let Some(peer) = peer else {
            return;
        };
        ui.add_space(8.0);
        ui.label(
            RichText::new(match self.locale {
                Locale::Chinese => "所选设备",
                Locale::English => "Selected device",
            })
            .small()
            .color(ui.visuals().weak_text_color()),
        );
        ui.label(
            RichText::new(peer_display_name(self.locale, &peer.id))
                .size(18.0)
                .strong(),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        if peer.screen == ScreenAvailability::Available {
            ui.label(match self.locale {
                Locale::Chinese => "选择“观看”打开此屏幕。",
                Locale::English => "Choose Watch to open this screen.",
            });
            ui.add_space(16.0);
            let enabled = self.snapshot.discovery.is_active()
                && !self.nearby_turn_off_pending
                && self.snapshot.has_mesh_session()
                && matches!(
                    self.snapshot.media.phase,
                    MediaPhase::Idle | MediaPhase::Failed
                )
                && matches!(
                    self.snapshot.view.phase,
                    ViewPhase::Idle | ViewPhase::Failed
                );
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(self.locale.watch()).min_size(egui::vec2(96.0, 38.0)),
                )
                .clicked()
            {
                self.page = Page::Watch;
                self.send(RuntimeCommand::WatchScreen {
                    path: crate::screen_path::for_peer(&peer.id),
                });
            }
        } else {
            ui.label(match self.locale {
                Locale::Chinese => "共享屏幕可用后会在这里出现观看动作。",
                Locale::English => "The Watch action appears here when a screen is shared.",
            });
        }
    }

    fn screen_share(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            match self.locale {
                Locale::Chinese => "共享这台电脑的屏幕",
                Locale::English => "Share this computer's screen",
            },
            match self.locale {
                Locale::Chinese => "管理本次共享使用的屏幕来源和系统音频。",
                Locale::English => "Manage the screen source and system audio used for this share.",
            },
        );
        ui.separator();
        setting_value_row(
            ui,
            match self.locale {
                Locale::Chinese => "来源",
                Locale::English => "Source",
            },
            match self.locale {
                Locale::Chinese => "此版本共享主显示器。",
                Locale::English => "This version shares the primary display.",
            },
            match self.locale {
                Locale::Chinese => "主显示器",
                Locale::English => "Primary display",
            },
        );
        ui.separator();
        setting_value_row(
            ui,
            match self.locale {
                Locale::Chinese => "系统音频",
                Locale::English => "System audio",
            },
            match self.locale {
                Locale::Chinese => "可用时随屏幕共享捕获；没有单独开关。",
                Locale::English => {
                    "Captured with screen sharing when available; there is no separate switch."
                }
            },
            share_audio_status(self.locale, self.snapshot.media.audio.phase),
        );
        ui.separator();
        ui.add_space(20.0);

        let status = match (self.locale, self.snapshot.media.phase) {
            (Locale::Chinese, MediaPhase::Idle) => "准备共享",
            (Locale::English, MediaPhase::Idle) => "Ready to share",
            (Locale::Chinese, MediaPhase::Preparing) => "正在准备共享",
            (Locale::English, MediaPhase::Preparing) => "Preparing screen share",
            (Locale::Chinese, MediaPhase::Sharing) => "正在共享屏幕",
            (Locale::English, MediaPhase::Sharing) => "Sharing screen",
            (Locale::Chinese, MediaPhase::Stopping) => "正在停止共享",
            (Locale::English, MediaPhase::Stopping) => "Stopping screen share",
            (Locale::Chinese, MediaPhase::Failed) => "无法开始共享",
            (Locale::English, MediaPhase::Failed) => "Screen share could not start",
        };
        ui.label(RichText::new(status).size(17.0).strong());
        ui.add_space(6.0);
        ui.label(match (self.locale, self.snapshot.media.phase) {
            (Locale::Chinese, MediaPhase::Preparing) => "正在请求系统捕获并准备共享。",
            (Locale::English, MediaPhase::Preparing) => {
                "Requesting system capture and preparing to share."
            }
            (Locale::Chinese, MediaPhase::Sharing) => "屏幕正在共享给已连接的附近设备。",
            (Locale::English, MediaPhase::Sharing) => {
                "Your screen is being shared with connected nearby devices."
            }
            (Locale::Chinese, MediaPhase::Stopping) => "停止共享不会断开附近设备。",
            (Locale::English, MediaPhase::Stopping) => {
                "Stopping screen share does not disconnect nearby devices."
            }
            (Locale::Chinese, MediaPhase::Failed) => "检查屏幕捕获状态后重试；附近连接不受影响。",
            (Locale::English, MediaPhase::Failed) => {
                "Check screen capture and try again; Nearby connections are not affected."
            }
            (Locale::Chinese, MediaPhase::Idle) => {
                if matches!(
                    self.snapshot.view.phase,
                    ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
                ) {
                    "正在观看远端屏幕。停止观看后才能开始共享。"
                } else {
                    "开始后会共享主显示器，并尝试捕获系统音频。"
                }
            }
            (Locale::English, MediaPhase::Idle) => {
                if matches!(
                    self.snapshot.view.phase,
                    ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
                ) {
                    "A remote screen is active. Stop watching before sharing."
                } else {
                    "Starting shares the primary display and attempts system audio capture."
                }
            }
        });
        ui.add_space(16.0);
        match self.snapshot.media.phase {
            MediaPhase::Idle | MediaPhase::Failed => {
                let enabled = self.snapshot.discovery.is_active()
                    && !self.nearby_turn_off_pending
                    && self.snapshot.local_id.is_some()
                    && matches!(
                        self.snapshot.view.phase,
                        ViewPhase::Idle | ViewPhase::Failed
                    );
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(match self.locale {
                            Locale::Chinese => "开始共享",
                            Locale::English => "Start sharing",
                        })
                        .min_size(egui::vec2(112.0, 40.0)),
                    )
                    .clicked()
                {
                    self.send(RuntimeCommand::ShareScreen);
                }
            }
            MediaPhase::Sharing => {
                if ui
                    .add(
                        egui::Button::new(match self.locale {
                            Locale::Chinese => "停止共享",
                            Locale::English => "Stop sharing",
                        })
                        .min_size(egui::vec2(112.0, 40.0)),
                    )
                    .clicked()
                {
                    self.send(RuntimeCommand::StopSharing);
                }
            }
            MediaPhase::Preparing | MediaPhase::Stopping => {
                ui.add_enabled(false, egui::Button::new(status));
            }
        }
        if let Some(error) = self.snapshot.media.last_error {
            ui.add_space(8.0);
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if let Some(error) = self.snapshot.media.audio.last_error {
            ui.add_space(8.0);
            ui.colored_label(Color32::ORANGE, error);
        }
    }

    fn watch(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            match self.locale {
                Locale::Chinese => "观看附近屏幕",
                Locale::English => "Watch a nearby screen",
            },
            match self.locale {
                Locale::Chinese => "画面保持原始宽高比，播放状态不会改变舞台尺寸。",
                Locale::English => {
                    "Video keeps its original aspect ratio, and playback state does not resize the stage."
                }
            },
        );
        let active = matches!(
            self.snapshot.view.phase,
            ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
        );
        let stage = watch_player_size(ui.available_size());
        if active {
            ui.allocate_ui_with_layout(stage, Layout::top_down(Align::Center), |ui| {
                if matches!(
                    self.player.show(
                        ui,
                        self.locale,
                        &self.snapshot.view,
                        self.playback_texture.as_ref(),
                    ),
                    Some(PlayerAction::Stop)
                ) {
                    self.send(RuntimeCommand::StopWatching);
                }
            });
            return;
        }

        let empty_stage = egui::vec2(stage.x, (stage.y - TOOLBAR_HEIGHT).max(1.0));
        let (rect, _) = ui.allocate_exact_size(empty_stage, Sense::hover());
        ui.painter().rect_filled(rect, 6.0, Color32::BLACK);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(match (self.locale, self.snapshot.view.phase) {
                            (Locale::Chinese, ViewPhase::Failed) => "无法播放",
                            (Locale::English, ViewPhase::Failed) => "Playback unavailable",
                            (Locale::Chinese, _) => "选择一个可观看屏幕",
                            (Locale::English, _) => "Choose a screen to watch",
                        })
                        .size(18.0)
                        .color(Color32::WHITE),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(match (self.locale, self.snapshot.view.phase) {
                            (Locale::Chinese, ViewPhase::Failed) => {
                                "媒体已停止，附近设备仍保持连接。"
                            }
                            (Locale::English, ViewPhase::Failed) => {
                                "Media stopped, but the nearby device stays connected."
                            }
                            (Locale::Chinese, _) => "前往附近设备，选择正在共享屏幕的设备。",
                            (Locale::English, _) => {
                                "Open Nearby and select a device that is sharing a screen."
                            }
                        })
                        .color(Color32::from_gray(185)),
                    );
                    ui.add_space(14.0);
                    if ui
                        .button(match self.locale {
                            Locale::Chinese => "打开附近设备",
                            Locale::English => "Open Nearby",
                        })
                        .clicked()
                    {
                        self.page = Page::Nearby;
                    }
                });
            });
        });
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

    fn settings(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            self.locale.settings(),
            match self.locale {
                Locale::Chinese => "调整日常偏好；需要时可开启开发者模式查看本地日志。",
                Locale::English => {
                    "Adjust everyday preferences; enable Developer mode when local logs are needed."
                }
            },
        );
        if self.developer_mode {
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut self.settings_pane,
                    SettingsPane::Preferences,
                    match self.locale {
                        Locale::Chinese => "偏好设置",
                        Locale::English => "Preferences",
                    },
                );
                ui.selectable_value(
                    &mut self.settings_pane,
                    SettingsPane::Diagnostics,
                    match self.locale {
                        Locale::Chinese => "诊断",
                        Locale::English => "Diagnostics",
                    },
                );
            });
            ui.separator();
            ui.add_space(16.0);
        } else {
            self.settings_pane = SettingsPane::Preferences;
        }

        match self.settings_pane {
            SettingsPane::Preferences => self.preferences(ui),
            SettingsPane::Diagnostics => self.diagnostics_settings(ui),
        }
    }

    fn preferences(&mut self, ui: &mut egui::Ui) {
        section_heading(
            ui,
            match self.locale {
                Locale::Chinese => "通用",
                Locale::English => "General",
            },
        );
        setting_control_row(
            ui,
            match self.locale {
                Locale::Chinese => "语言",
                Locale::English => "Language",
            },
            match self.locale {
                Locale::Chinese => "语言设置会保留。",
                Locale::English => "Language preference is retained.",
            },
            |ui| {
                egui::ComboBox::from_id_salt("locale")
                    .selected_text(match self.locale {
                        Locale::Chinese => "简体中文",
                        Locale::English => "English",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.locale, Locale::Chinese, "简体中文");
                        ui.selectable_value(&mut self.locale, Locale::English, "English");
                    });
            },
        );
        ui.separator();
        setting_value_row(
            ui,
            match self.locale {
                Locale::Chinese => "产品版本",
                Locale::English => "Product version",
            },
            "",
            self.snapshot.version,
        );
        ui.separator();
        ui.add_space(24.0);

        section_heading(
            ui,
            match self.locale {
                Locale::Chinese => "屏幕共享",
                Locale::English => "Screen sharing",
            },
        );
        let current_video_encoding = self.snapshot.media.video_encoding;
        let mut selected_video_encoding = current_video_encoding;
        let can_change_video_encoding = matches!(
            self.snapshot.media.phase,
            MediaPhase::Idle | MediaPhase::Failed
        );
        ui.add_enabled_ui(can_change_video_encoding, |ui| {
            setting_control_row(
                ui,
                match self.locale {
                    Locale::Chinese => "视频编码",
                    Locale::English => "Video encoding",
                },
                match self.locale {
                    Locale::Chinese => "QHD 要求硬件 H.264，不提供软件降级。",
                    Locale::English => {
                        "QHD requires hardware H.264 and does not fall back to software."
                    }
                },
                |ui| {
                    egui::ComboBox::from_id_salt("video-encoding-policy")
                        .selected_text(video_encoding_label(self.locale, selected_video_encoding))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut selected_video_encoding,
                                VideoEncodingPolicy::Compatible,
                                video_encoding_label(self.locale, VideoEncodingPolicy::Compatible),
                            );
                            ui.selectable_value(
                                &mut selected_video_encoding,
                                VideoEncodingPolicy::NativeQhdHardware,
                                video_encoding_label(
                                    self.locale,
                                    VideoEncodingPolicy::NativeQhdHardware,
                                ),
                            );
                        });
                },
            );
        });
        if selected_video_encoding != current_video_encoding {
            self.send(RuntimeCommand::SetVideoEncodingPolicy(
                selected_video_encoding,
            ));
        }
        ui.small(match (self.locale, current_video_encoding) {
            (Locale::Chinese, VideoEncodingPolicy::Compatible) => {
                "保持显示器原生尺寸；最长边不超过 1920。编码器自动选择。"
            }
            (Locale::English, VideoEncodingPolicy::Compatible) => {
                "Keeps the display's native size up to a 1920-pixel edge; encoder selection is automatic."
            }
            (Locale::Chinese, VideoEncodingPolicy::NativeQhdHardware) => {
                "请求原生横屏 2560x1440；启动共享时验证硬件 H.264，失败时不会降级。"
            }
            (Locale::English, VideoEncodingPolicy::NativeQhdHardware) => {
                "Requests native landscape 2560x1440; hardware H.264 is checked when sharing starts, with no fallback."
            }
        });
        if !can_change_video_encoding {
            ui.small(match self.locale {
                Locale::Chinese => "停止共享后才能更改编码模式。",
                Locale::English => "Stop sharing before changing the encoding mode.",
            });
        }
        ui.separator();
        ui.add_space(24.0);

        section_heading(
            ui,
            match self.locale {
                Locale::Chinese => "高级",
                Locale::English => "Advanced",
            },
        );
        let mut auto_watch = false;
        setting_control_row(
            ui,
            match self.locale {
                Locale::Chinese => "自动观看唯一可用屏幕",
                Locale::English => "Auto-watch the only available screen",
            },
            match self.locale {
                Locale::Chinese => "仅在没有活跃媒体且恰好一个可观看屏幕时开始。",
                Locale::English => {
                    "Starts only when no media is active and exactly one screen is available."
                }
            },
            |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled(false, egui::Checkbox::new(&mut auto_watch, ""));
                    ui.label(
                        RichText::new(match self.locale {
                            Locale::Chinese => "暂不可开启",
                            Locale::English => "Not available yet",
                        })
                        .weak(),
                    );
                });
            },
        );
        ui.separator();
        let was_developer_mode = self.developer_mode;
        setting_control_row(
            ui,
            match self.locale {
                Locale::Chinese => "开发者模式",
                Locale::English => "Developer mode",
            },
            match self.locale {
                Locale::Chinese => "开启后显示本地日志与诊断入口。",
                Locale::English => "Shows local log and diagnostics tools when enabled.",
            },
            |ui| {
                ui.checkbox(&mut self.developer_mode, "");
            },
        );
        if was_developer_mode && !self.developer_mode {
            self.settings_pane = SettingsPane::Preferences;
            self.diagnostics.hide_window();
        }
        ui.separator();
    }

    fn diagnostics_settings(&mut self, ui: &mut egui::Ui) {
        section_heading(
            ui,
            match self.locale {
                Locale::Chinese => "连接信息",
                Locale::English => "Connection information",
            },
        );
        egui::Grid::new("connection-info")
            .num_columns(2)
            .spacing([18.0, 8.0])
            .show(ui, |ui| {
                ui.label("Version");
                ui.monospace(self.snapshot.version);
                ui.end_row();
                ui.label("MoQ baseline");
                ui.monospace(crate::build_info::moq_baseline());
                ui.end_row();
                ui.label("Discovery");
                ui.monospace(format!("{:?}", self.snapshot.discovery));
                ui.end_row();
                ui.label("Listener");
                ui.monospace(self.snapshot.listener.as_deref().unwrap_or("unavailable"));
                ui.end_row();
                ui.label("Local peer");
                ui.monospace(self.snapshot.local_id.as_deref().unwrap_or("unavailable"));
                ui.end_row();
                ui.label("Inbound sessions");
                ui.monospace(self.snapshot.inbound_sessions.to_string());
                ui.end_row();
                ui.label("Screen media");
                ui.monospace(format!("{:?}", self.snapshot.media.phase));
                ui.end_row();
                ui.label("Remote playback");
                ui.monospace(format!("{:?}", self.snapshot.view.phase));
                ui.end_row();
                ui.label("Decoder");
                ui.monospace(
                    self.snapshot
                        .view
                        .decoder
                        .as_deref()
                        .unwrap_or("not active"),
                );
                ui.end_row();
                ui.label("Playback audio");
                let audio = match self.snapshot.view.audio.phase {
                    ViewAudioPhase::Idle => "not active".to_owned(),
                    ViewAudioPhase::Pending => "pending".to_owned(),
                    ViewAudioPhase::TrackSelected => "track selected".to_owned(),
                    ViewAudioPhase::Decoded => "PCM decoded".to_owned(),
                    ViewAudioPhase::NotPublished => "not published".to_owned(),
                    ViewAudioPhase::Writing => "sink write accepted".to_owned(),
                    ViewAudioPhase::CallbackConsumed => {
                        let codec = self.snapshot.view.audio.codec.as_deref().unwrap_or("audio");
                        let sample_rate = self.snapshot.view.audio.sample_rate.unwrap_or_default();
                        let channels = self.snapshot.view.audio.channels.unwrap_or_default();
                        format!(
                            "output callback consumed non-zero PCM · {codec} · {sample_rate} Hz · {channels} ch"
                        )
                    }
                    ViewAudioPhase::Failed => "unavailable (video continues)".to_owned(),
                };
                ui.monospace(audio);
                ui.end_row();
            });
        if let Some(error) = &self.snapshot.view.audio.last_error {
            ui.add_space(10.0);
            ui.colored_label(Color32::ORANGE, error);
        }
        if let Some(error) = self.snapshot.last_error {
            ui.add_space(10.0);
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        ui.separator();
        ui.add_space(20.0);
        self.diagnostics.show_settings(ui, self.locale);
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
                    if matches!(
                        self.player.show(
                            ui,
                            self.locale,
                            &self.snapshot.view,
                            self.playback_texture.as_ref(),
                        ),
                        Some(PlayerAction::Stop)
                    ) {
                        self.send(RuntimeCommand::StopWatching);
                    }
                });
            self.diagnostics.show_window(&context, self.locale);
            context.request_repaint_after(std::time::Duration::from_millis(33));
            return;
        }

        egui::Panel::top("navigation")
            .exact_size(navigation_height(context.content_rect().width()))
            .show(ui, |ui| {
                ui.add_space(7.0);
                self.top_bar(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            let content = content_rect(ui.available_rect_before_wrap());
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                ui.set_width(content.width());
                if let Some(error) = &self.command_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                    ui.add_space(6.0);
                }
                match self.page {
                    Page::Nearby => self.nearby(ui),
                    Page::ScreenShare => self.screen_share(ui),
                    Page::Watch => self.watch(ui),
                    Page::Settings => {
                        egui::ScrollArea::vertical().show(ui, |ui| self.settings(ui));
                    }
                }
            });
        });
        self.show_turn_off_nearby_confirmation(&context);
        self.diagnostics.show_window(&context, self.locale);
        context.request_repaint_after(std::time::Duration::from_millis(if viewing {
            33
        } else {
            100
        }));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
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

fn page_header(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).size(28.0).strong());
    ui.add_space(4.0);
    ui.label(RichText::new(description).color(ui.visuals().weak_text_color()));
    ui.add_space(24.0);
}

fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(18.0).strong());
    ui.add_space(6.0);
}

fn setting_control_row(
    ui: &mut egui::Ui,
    title: &str,
    help: &str,
    control: impl FnOnce(&mut egui::Ui),
) {
    if settings_use_stacked_rows(ui.available_width()) {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong());
            if !help.is_empty() {
                ui.small(help);
            }
            ui.add_space(8.0);
            control(ui);
        });
    } else {
        ui.horizontal(|ui| {
            ui.set_min_height(56.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(title).strong());
                if !help.is_empty() {
                    ui.small(help);
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), control);
        });
    }
}

fn setting_value_row(ui: &mut egui::Ui, title: &str, help: &str, value: &str) {
    setting_control_row(ui, title, help, |ui| {
        ui.label(
            RichText::new(value)
                .strong()
                .color(ui.visuals().weak_text_color()),
        );
    });
}

fn selected_peer(peers: &[PeerView], selected: Option<&str>) -> Option<String> {
    if let Some(selected) = selected
        && peers.iter().any(|peer| peer.id == selected)
    {
        return Some(selected.to_owned());
    }
    peers.first().map(|peer| peer.id.clone())
}

fn nearby_uses_stacked_layout(width: f32) -> bool {
    width < NEARBY_SPLIT_MIN_WIDTH
}

fn nearby_list_width(available_width: f32) -> f32 {
    available_width.min(NEARBY_LIST_MAX_WIDTH).max(1.0)
}

fn navigation_height(width: f32) -> f32 {
    if width < NEARBY_SPLIT_MIN_WIDTH {
        COMPACT_NAVIGATION_HEIGHT
    } else {
        DESKTOP_NAVIGATION_HEIGHT
    }
}

fn settings_use_stacked_rows(width: f32) -> bool {
    width < SETTINGS_STACK_WIDTH
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

fn peer_display_name(locale: Locale, id: &str) -> String {
    let compact: String = id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect();
    let suffix: String = compact
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
        .to_ascii_uppercase();
    let base = match locale {
        Locale::Chinese => "附近设备",
        Locale::English => "Nearby device",
    };
    if suffix.is_empty() {
        base.to_owned()
    } else {
        format!("{base} · {suffix}")
    }
}

fn peer_status(locale: Locale, peer: &PeerView) -> &'static str {
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

fn share_audio_status(locale: Locale, phase: AudioPhase) -> &'static str {
    match (locale, phase) {
        (Locale::Chinese, AudioPhase::Publishing) => "正在共享",
        (Locale::English, AudioPhase::Publishing) => "Sharing",
        (Locale::Chinese, AudioPhase::Preparing) => "正在准备",
        (Locale::English, AudioPhase::Preparing) => "Preparing",
        (Locale::Chinese, AudioPhase::Silent) => "当前静音",
        (Locale::English, AudioPhase::Silent) => "Silent",
        (Locale::Chinese, AudioPhase::Recovering) => "正在恢复",
        (Locale::English, AudioPhase::Recovering) => "Recovering",
        (Locale::Chinese, AudioPhase::Failed) => "不可用，视频继续",
        (Locale::English, AudioPhase::Failed) => "Unavailable, video continues",
        (Locale::Chinese, AudioPhase::Stopping) => "正在停止",
        (Locale::English, AudioPhase::Stopping) => "Stopping",
        (Locale::Chinese, AudioPhase::Idle) => "随共享开启",
        (Locale::English, AudioPhase::Idle) => "Starts with sharing",
    }
}

fn watch_player_size(available: egui::Vec2) -> egui::Vec2 {
    let available = egui::vec2(available.x.max(1.0), available.y.max(1.0));
    let width = available.x.min(960.0);
    let stage_height = width * 9.0 / 16.0;
    if stage_height + TOOLBAR_HEIGHT <= available.y {
        egui::vec2(width, stage_height + TOOLBAR_HEIGHT)
    } else {
        let stage_height = (available.y - TOOLBAR_HEIGHT).max(1.0);
        egui::vec2(stage_height * 16.0 / 9.0, stage_height + TOOLBAR_HEIGHT)
    }
}

fn configure_fonts(context: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};

    context.add_font(FontInsert::new(
        "Noto Sans SC",
        egui::FontData::from_static(include_bytes!("../assets/fonts/NotoSansSC-Regular.otf")),
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
        configure_fonts(&context);

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
        assert_ne!(Page::Watch, Page::Settings);
    }

    #[test]
    fn nearby_count_uses_presence_instead_of_historical_rows() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.peers.insert(
            "present".to_owned(),
            PeerView {
                id: "present".to_owned(),
                candidates: Vec::new(),
                should_dial: true,
                authenticated_discovery: false,
                tls_pinned: true,
                present: true,
                transport: Default::default(),
                screen: ScreenAvailability::Unavailable,
            },
        );
        let historical = snapshot.peers["present"].clone();
        snapshot.peers.insert(
            "historical".to_owned(),
            PeerView {
                id: "historical".to_owned(),
                present: false,
                ..historical
            },
        );

        assert_eq!(snapshot.present_peer_count(), 1);
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
    fn content_uses_the_shared_desktop_width_and_keeps_panel_height() {
        let available =
            egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(1200.0, 700.0));

        let content = content_rect(available);
        assert_eq!(content.min, egui::pos2(100.0, 54.0));
        assert_eq!(content.size(), egui::vec2(1040.0, 676.0));
    }

    #[test]
    fn narrow_windows_use_two_row_navigation_and_stacked_settings() {
        assert_eq!(navigation_height(720.0), 96.0);
        assert_eq!(navigation_height(1024.0), 52.0);
        assert!(settings_use_stacked_rows(680.0));
        assert!(!settings_use_stacked_rows(900.0));
    }

    #[test]
    fn nearby_and_watch_layouts_reserve_stable_regions() {
        assert_eq!(nearby_list_width(1040.0), 360.0);
        assert_eq!(nearby_list_width(800.0), 360.0);

        assert_eq!(
            watch_player_size(egui::vec2(1000.0, 700.0)),
            egui::vec2(960.0, 592.0)
        );
        assert_size(
            watch_player_size(egui::vec2(800.0, 300.0)),
            egui::vec2(440.888_9, 300.0),
        );
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
    fn nearby_layout_and_selection_are_stable_across_resizes_and_lost_peers() {
        assert!(!nearby_uses_stacked_layout(900.0));
        assert!(nearby_uses_stacked_layout(700.0));

        let peers = vec![peer("alpha"), peer("beta")];
        assert_eq!(selected_peer(&peers, Some("beta")).as_deref(), Some("beta"));
        assert_eq!(
            selected_peer(&peers, Some("lost")).as_deref(),
            Some("alpha")
        );
        assert_eq!(selected_peer(&[], Some("alpha")), None);
    }

    #[test]
    fn nearby_names_and_statuses_hide_transport_internals() {
        let mut visible = peer("0123-abcd");
        visible.transport.phase = TransportPhaseView::Connecting;
        assert_eq!(
            peer_display_name(Locale::Chinese, &visible.id),
            "附近设备 · ABCD"
        );
        assert_eq!(peer_status(Locale::English, &visible), "Connecting");

        visible.screen = ScreenAvailability::Available;
        assert_eq!(peer_status(Locale::Chinese, &visible), "屏幕可观看");
    }

    fn assert_size(actual: egui::Vec2, expected: egui::Vec2) {
        assert!((actual.x - expected.x).abs() < 0.001);
        assert!((actual.y - expected.y).abs() < 0.001);
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
