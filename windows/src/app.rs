//! Compact native UI for discovery and direct peer sessions.

use eframe::egui::{self, Align, Color32, Frame, Layout, RichText, Stroke};

use crate::{
    audio::AudioPhase,
    diagnostics::DiagnosticsUi,
    media::{COMPATIBLE_MAX_SCREEN_EDGE, MediaPhase, VideoEncodingPolicy},
    playback::{PlaybackFrameIdentity, ViewAudioPhase, ViewPhase},
    player::{LivePlayer, PlayerAction},
    remote::ScreenAvailability,
    runtime::{
        DiscoveryState, PeerView, RuntimeCommand, RuntimeOwner, RuntimeSnapshot, TransportPhaseView,
    },
};

const CONTENT_MAX_WIDTH: f32 = 900.0;
const CONTENT_TOP_SPACING: f32 = 18.0;
const STORAGE_DETAILED_DIAGNOSTICS: &str = "moqcast.detailed-diagnostics";

fn parse_stored_bool(value: Option<String>) -> bool {
    value.as_deref() == Some("true")
}

fn content_rect(available: egui::Rect, page: Page, viewing: bool) -> egui::Rect {
    let width = if page == Page::ScreenShare && viewing {
        available.width()
    } else {
        available.width().min(CONTENT_MAX_WIDTH)
    };
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
    Settings,
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
}

pub(crate) struct MoqCastApp {
    page: Page,
    locale: Locale,
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
        let snapshot = runtime.snapshot();
        Self {
            page: Page::Nearby,
            locale: Locale::Chinese,
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

    fn send(&mut self, command: RuntimeCommand) {
        self.command_error = self
            .runtime
            .try_send(command)
            .err()
            .map(|error| match self.locale {
                Locale::Chinese => format!("命令未发送：{error}"),
                Locale::English => format!("Command was not sent: {error}"),
            });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("MoQCast").size(19.0).strong());
            ui.add_space(18.0);
            ui.selectable_value(&mut self.page, Page::Nearby, self.locale.nearby());
            ui.selectable_value(
                &mut self.page,
                Page::ScreenShare,
                self.locale.screen_share(),
            );
            ui.selectable_value(&mut self.page, Page::Settings, self.locale.settings());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = match self.snapshot.discovery {
                    DiscoveryState::Ready => format!("{} peers", self.snapshot.peers.len()),
                    DiscoveryState::Empty => "LAN ready".to_owned(),
                    DiscoveryState::Starting => "Starting".to_owned(),
                    DiscoveryState::Failed => "Discovery error".to_owned(),
                    DiscoveryState::Stopped => "Stopped".to_owned(),
                };
                ui.small(text);
            });
        });
    }

    fn nearby(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.locale.nearby());
        ui.label(match self.locale {
            Locale::Chinese => "发现与连接保持分层。只有确定的拨号侧可以发起安全直连。",
            Locale::English => {
                "Discovery and transport stay separate. Only the deterministic dial side can connect."
            }
        });
        ui.add_space(12.0);

        if self.snapshot.peers.values().all(|peer| !peer.present) {
            ui.label(match self.snapshot.discovery {
                DiscoveryState::Starting => match self.locale {
                    Locale::Chinese => "正在启动局域网发现…",
                    Locale::English => "Starting LAN discovery…",
                },
                DiscoveryState::Failed => match self.locale {
                    Locale::Chinese => "局域网发现不可用。请在设置中查看状态。",
                    Locale::English => "LAN discovery is unavailable. See Settings for status.",
                },
                _ => match self.locale {
                    Locale::Chinese => "暂未发现附近设备。",
                    Locale::English => "No nearby devices found.",
                },
            });
            return;
        }

        let peers: Vec<_> = self
            .snapshot
            .peers
            .values()
            .filter(|peer| peer.present)
            .cloned()
            .collect();
        for peer in peers {
            self.peer_row(ui, &peer);
            ui.add_space(6.0);
        }
    }

    fn peer_row(&mut self, ui: &mut egui::Ui, peer: &PeerView) {
        Frame::new()
            .stroke(Stroke::new(
                1.0,
                ui.visuals().widgets.noninteractive.bg_stroke.color,
            ))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&peer.id).strong());
                        let role = if peer.should_dial {
                            "outbound"
                        } else {
                            "inbound"
                        };
                        ui.small(format!(
                            "{role} · {} · generation {}",
                            phase(peer.transport.phase),
                            peer.transport.generation
                        ));
                        for candidate in &peer.candidates {
                            ui.label(
                                RichText::new(candidate)
                                    .monospace()
                                    .small()
                                    .color(Color32::GRAY),
                            );
                        }
                        let security = if peer.authenticated_discovery && peer.tls_pinned {
                            "authenticated discovery · TLS pinned"
                        } else if peer.tls_pinned {
                            "open discovery · TLS pinned"
                        } else {
                            "security metadata incomplete"
                        };
                        ui.small(security);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if peer.screen == ScreenAvailability::Available {
                            let path = crate::screen_path::for_peer(&peer.id);
                            let enabled = self.snapshot.has_mesh_session()
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
                                    egui::Button::new(match self.locale {
                                        Locale::Chinese => "观看",
                                        Locale::English => "Watch",
                                    }),
                                )
                                .clicked()
                            {
                                self.page = Page::ScreenShare;
                                self.send(RuntimeCommand::WatchScreen { path });
                            }
                        } else {
                            ui.small(match self.locale {
                                Locale::Chinese => "自动连接",
                                Locale::English => "Automatic",
                            });
                        }
                    });
                });
            });
    }

    fn screen_share(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.locale.screen_share());
        if matches!(
            self.snapshot.view.phase,
            ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
        ) {
            if matches!(
                self.player.show(
                    ui,
                    &self.snapshot.view,
                    self.playback_texture.as_ref(),
                    false,
                ),
                Some(PlayerAction::Stop)
            ) {
                self.send(RuntimeCommand::StopWatching);
            }
            return;
        }
        let status = match (self.locale, self.snapshot.media.phase) {
            (Locale::Chinese, MediaPhase::Idle) => "尚未共享屏幕。",
            (Locale::English, MediaPhase::Idle) => "Screen sharing is idle.",
            (Locale::Chinese, MediaPhase::Preparing) => "正在准备主显示器…",
            (Locale::English, MediaPhase::Preparing) => "Preparing the primary display…",
            (Locale::Chinese, MediaPhase::Sharing) => "正在共享主显示器。",
            (Locale::English, MediaPhase::Sharing) => "The primary display is being shared.",
            (Locale::Chinese, MediaPhase::Stopping) => "正在停止共享…",
            (Locale::English, MediaPhase::Stopping) => "Stopping screen sharing…",
            (Locale::Chinese, MediaPhase::Failed) => "屏幕共享未能启动或已意外结束。",
            (Locale::English, MediaPhase::Failed) => {
                "Screen sharing could not start or ended unexpectedly."
            }
        };
        ui.label(status);
        ui.add_space(8.0);
        match self.snapshot.media.phase {
            MediaPhase::Idle | MediaPhase::Failed => {
                let enabled = self.snapshot.local_id.is_some()
                    && matches!(
                        self.snapshot.view.phase,
                        ViewPhase::Idle | ViewPhase::Failed
                    );
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(match self.locale {
                            Locale::Chinese => "共享屏幕",
                            Locale::English => "Share screen",
                        }),
                    )
                    .clicked()
                {
                    self.send(RuntimeCommand::ShareScreen);
                }
            }
            MediaPhase::Sharing => {
                if ui
                    .button(match self.locale {
                        Locale::Chinese => "停止共享",
                        Locale::English => "Stop sharing",
                    })
                    .clicked()
                {
                    self.send(RuntimeCommand::StopSharing);
                }
            }
            MediaPhase::Preparing | MediaPhase::Stopping => {
                ui.add_enabled(false, egui::Button::new(status));
            }
        }
        ui.add_space(12.0);
        egui::Grid::new("screen-share-stats")
            .num_columns(2)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                ui.label("Broadcast");
                ui.monospace(
                    self.snapshot
                        .media
                        .path
                        .as_deref()
                        .unwrap_or("not announced"),
                );
                ui.end_row();
                ui.label("Codec");
                ui.monospace("H.264");
                ui.end_row();
                ui.label(match self.locale {
                    Locale::Chinese => "编码模式",
                    Locale::English => "Encoding mode",
                });
                ui.monospace(video_encoding_label(
                    self.locale,
                    self.snapshot.media.video_encoding,
                ));
                ui.end_row();
                ui.label("Encoder");
                ui.monospace(match self.snapshot.media.video_encoding {
                    VideoEncodingPolicy::Compatible => {
                        "Auto: Media Foundation preferred, OpenH264 fallback"
                    }
                    VideoEncodingPolicy::NativeQhdHardware => {
                        "Media Foundation hardware required; checked at start"
                    }
                });
                ui.end_row();
                ui.label(match self.locale {
                    Locale::Chinese => "系统音频",
                    Locale::English => "System audio",
                });
                ui.monospace(match self.snapshot.media.audio.phase {
                    AudioPhase::Idle => "idle",
                    AudioPhase::Preparing => "preparing",
                    AudioPhase::Publishing => "Opus",
                    AudioPhase::Silent => "silent",
                    AudioPhase::Recovering => "recovering",
                    AudioPhase::Stopping => "stopping",
                    AudioPhase::Failed => "unavailable (video continues)",
                });
                ui.end_row();
                ui.label("Output");
                let output = self
                    .snapshot
                    .media
                    .width
                    .zip(self.snapshot.media.height)
                    .map_or_else(
                        || match self.snapshot.media.video_encoding {
                            VideoEncodingPolicy::Compatible => {
                                format!("native, max edge {COMPATIBLE_MAX_SCREEN_EDGE}px")
                            }
                            VideoEncodingPolicy::NativeQhdHardware => {
                                "native landscape 2560x1440".to_owned()
                            }
                        },
                        |(width, height)| format!("{width}x{height} @ 30 fps"),
                    );
                ui.monospace(output);
                ui.end_row();
            });
        if let Some(error) = self.snapshot.media.last_error {
            ui.add_space(8.0);
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if let Some(error) = self.snapshot.media.audio.last_error {
            ui.add_space(8.0);
            ui.colored_label(Color32::ORANGE, error);
        }
        if let Some(error) = &self.snapshot.view.last_error {
            ui.add_space(8.0);
            ui.colored_label(Color32::LIGHT_RED, error);
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

    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.locale.settings());
        ui.horizontal(|ui| {
            ui.label(match self.locale {
                Locale::Chinese => "语言",
                Locale::English => "Language",
            });
            egui::ComboBox::from_id_salt("locale")
                .selected_text(match self.locale {
                    Locale::Chinese => "简体中文",
                    Locale::English => "English",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.locale, Locale::Chinese, "简体中文");
                    ui.selectable_value(&mut self.locale, Locale::English, "English");
                });
        });
        ui.add_space(8.0);
        let current_video_encoding = self.snapshot.media.video_encoding;
        let mut selected_video_encoding = current_video_encoding;
        let can_change_video_encoding = matches!(
            self.snapshot.media.phase,
            MediaPhase::Idle | MediaPhase::Failed
        );
        ui.add_enabled_ui(can_change_video_encoding, |ui| {
            ui.horizontal(|ui| {
                ui.label(match self.locale {
                    Locale::Chinese => "视频编码",
                    Locale::English => "Video encoding",
                });
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
            });
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
        egui::Grid::new("connection-info")
            .num_columns(2)
            .spacing([18.0, 8.0])
            .show(ui, |ui| {
                ui.label("Version");
                ui.monospace(self.snapshot.version);
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
                    ViewAudioPhase::NotPublished => "not published".to_owned(),
                    ViewAudioPhase::Playing => {
                        let codec = self.snapshot.view.audio.codec.as_deref().unwrap_or("audio");
                        let sample_rate = self.snapshot.view.audio.sample_rate.unwrap_or_default();
                        let channels = self.snapshot.view.audio.channels.unwrap_or_default();
                        format!("{codec} · {sample_rate} Hz · {channels} ch")
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
        self.diagnostics.show_settings(ui, self.locale);
    }
}

impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.snapshot = self.runtime.snapshot();
        let context = ui.ctx().clone();
        self.update_playback_texture(&context);
        let viewing = matches!(
            self.snapshot.view.phase,
            ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
        );
        let viewport_fullscreen = LivePlayer::fullscreen(&context);
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
        if viewport_fullscreen && !viewing {
            context.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        let fullscreen = viewing && viewport_fullscreen;

        if fullscreen {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(Color32::BLACK))
                .show(ui, |ui| {
                    if matches!(
                        self.player.show(
                            ui,
                            &self.snapshot.view,
                            self.playback_texture.as_ref(),
                            true,
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
            .exact_size(52.0)
            .show(ui, |ui| {
                ui.add_space(7.0);
                self.top_bar(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            let content = content_rect(ui.available_rect_before_wrap(), self.page, viewing);
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                ui.set_width(content.width());
                if let Some(error) = &self.command_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                    ui.add_space(6.0);
                }
                match self.page {
                    Page::Nearby => self.nearby(ui),
                    Page::ScreenShare => self.screen_share(ui),
                    Page::Settings => {
                        egui::ScrollArea::vertical().show(ui, |ui| self.settings(ui));
                    }
                }
            });
        });
        self.diagnostics.show_window(&context, self.locale);
        context.request_repaint_after(std::time::Duration::from_millis(if viewing {
            33
        } else {
            100
        }));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
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

fn phase(phase: TransportPhaseView) -> &'static str {
    match phase {
        TransportPhaseView::Waiting => "waiting",
        TransportPhaseView::Connecting => "connecting",
        TransportPhaseView::Connected => "connected",
        TransportPhaseView::Rejected => "rejected",
        TransportPhaseView::Failed => "failed",
        TransportPhaseView::Disconnected => "disconnected",
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
        let page = Page::Settings;
        let locale = Locale::English;
        assert_eq!(page, Page::Settings);
        assert_eq!(locale.settings(), "Settings");
        assert_ne!(Page::Nearby, Page::ScreenShare);
    }

    #[test]
    fn detailed_diagnostics_storage_is_opt_in() {
        assert!(!parse_stored_bool(None));
        assert!(!parse_stored_bool(Some("false".to_owned())));
        assert!(!parse_stored_bool(Some("unexpected".to_owned())));
        assert!(parse_stored_bool(Some("true".to_owned())));
    }

    #[test]
    fn active_player_content_keeps_panel_height_without_expanding_other_pages() {
        let available =
            egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(1200.0, 700.0));

        let player = content_rect(available, Page::ScreenShare, true);
        assert_eq!(player.min, egui::pos2(20.0, 48.0));
        assert_eq!(player.size(), egui::vec2(1200.0, 682.0));

        for page in [Page::Nearby, Page::Settings] {
            let content = content_rect(available, page, true);
            assert_eq!(content.min, egui::pos2(170.0, 48.0));
            assert_eq!(content.size(), egui::vec2(900.0, 682.0));
        }
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
}
