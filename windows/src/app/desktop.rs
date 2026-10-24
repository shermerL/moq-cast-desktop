//! Shared visual components with Windows-owned runtime commands and source validation.
use super::*;
use egui::{RichText, UiBuilder, vec2};
use moqcast_ui::v2::{self, Action, ActionTone, Metrics, Palette, StatusTone};
impl MoqCastApp {
    fn text<'a>(&self, zh: &'a str, en: &'a str) -> &'a str {
        match self.locale {
            Locale::Chinese => zh,
            Locale::English => en,
        }
    }
    pub(super) fn desktop_shell(&mut self, root: &mut egui::Ui) {
        let p = Palette::for_ui(root);
        let metrics = Metrics::for_width(root.ctx().content_rect().width());
        egui::Panel::top("v2-header")
            .exact_size(metrics.header)
            .frame(
                Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(metrics.inset, 0)),
            )
            .show(root, |ui| {
                let title = match self.page {
                    Page::Nearby => None,
                    Page::ScreenShare => Some(self.text("共享设置", "Sharing settings")),
                    Page::Watch => Some(self.text("播放器", "Player")),
                };
                let response = v2::header(
                    ui,
                    v2::Header {
                        title,
                        back: self.text("‹  附近设备", "‹  Nearby"),
                        settings: self.locale.settings(),
                    },
                );
                if response.back {
                    self.page = Page::Nearby;
                }
                if response.settings {
                    self.settings_open = !self.settings_open;
                }
            });
        if self.page == Page::Nearby {
            egui::Panel::bottom("v2-nearby-footer")
                .exact_size(metrics.footer)
                .frame(
                    Frame::new()
                        .fill(p.canvas)
                        .inner_margin(egui::Margin::symmetric(metrics.inset, 0)),
                )
                .show(root, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let pending = self.nearby_turn_off_pending
                                || matches!(
                                    self.snapshot.discovery,
                                    DiscoveryState::Starting | DiscoveryState::Stopping
                                );
                            let active = self.snapshot.discovery.is_active();
                            if (Action {
                                label: if pending {
                                    self.text("处理中", "Pending")
                                } else if active {
                                    self.text("关闭附近设备", "Turn off Nearby")
                                } else {
                                    self.text("开启附近设备", "Turn on Nearby")
                                },
                                size: vec2(148.0, 40.0),
                                enabled: !pending,
                                tone: if active {
                                    ActionTone::Danger
                                } else {
                                    ActionTone::Accent
                                },
                            })
                            .show(ui)
                            .clicked()
                            {
                                if active {
                                    if has_active_media(&self.snapshot) {
                                        self.confirm_turn_off_nearby = true;
                                    } else {
                                        self.request_turn_off_nearby();
                                    }
                                } else {
                                    self.send(RuntimeCommand::StartScan);
                                }
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                let tone = match self.snapshot.discovery {
                                    DiscoveryState::Starting | DiscoveryState::Stopping => {
                                        StatusTone::Busy
                                    }
                                    DiscoveryState::Ready | DiscoveryState::Empty => {
                                        StatusTone::Success
                                    }
                                    DiscoveryState::Failed => StatusTone::Error,
                                    DiscoveryState::Stopped => StatusTone::Neutral,
                                };
                                tone.indicator(ui);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(discovery_summary(
                                            self.locale,
                                            &self.snapshot,
                                        ))
                                        .size(12.0)
                                        .color(p.muted),
                                    )
                                    .truncate(),
                                );
                            });
                        });
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(p.canvas)
                    .inner_margin(if self.page == Page::Watch {
                        egui::Margin::ZERO
                    } else {
                        egui::Margin::symmetric(metrics.inset, 18)
                    }),
            )
            .show(root, |ui| {
                if self.page == Page::Watch {
                    self.watch(ui);
                    return;
                }
                let width = ui.available_width().min(if self.page == Page::Nearby {
                    1240.0
                } else {
                    792.0
                });
                let area = egui::Rect::from_min_size(
                    egui::pos2(ui.max_rect().center().x - width / 2.0, ui.cursor().top()),
                    vec2(width, ui.available_height()),
                );
                ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
                    if self.page == Page::Nearby {
                        if area.width() < 580.0 || area.height() < 260.0 {
                            egui::ScrollArea::vertical()
                                .id_salt("nearby-small-workspace")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                )
                                .show(ui, |ui| self.nearby(ui, metrics));
                        } else {
                            self.nearby(ui, metrics);
                        }
                    } else {
                        self.screen_share(ui);
                    }
                });
            });
        if self.command_error.is_some() && self.page != Page::Nearby {
            egui::Area::new(egui::Id::new("command-error"))
                .anchor(egui::Align2::CENTER_TOP, vec2(0.0, metrics.header + 4.0))
                .show(root.ctx(), |ui| {
                    p.card().show(ui, |ui| {
                        v2::notice(
                            ui,
                            StatusTone::Error,
                            self.text(
                                "操作未发送，请重试。",
                                "The action could not be sent. Retry.",
                            ),
                        )
                    });
                });
        }
        self.modals(root.ctx(), p);
    }
    fn nearby(&mut self, ui: &mut egui::Ui, metrics: Metrics) {
        let watching = matches!(
            self.snapshot.view.phase,
            ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
        );
        let (tone, status) = local_media_status(self.locale, &self.snapshot);
        if v2::context_card(
            ui,
            v2::ContextCard {
                heading: self.text("本机", "This computer"),
                name: &self.snapshot.local_device_name,
                identity: self.snapshot.local_id.as_deref().unwrap_or(""),
                tone,
                status,
                action: if watching {
                    self.text("返回播放器", "Return to player")
                } else {
                    self.text("共享设置", "Sharing settings")
                },
            },
            metrics,
        )
        .clicked()
        {
            self.page = if watching {
                Page::Watch
            } else {
                Page::ScreenShare
            };
        }
        ui.add_space(metrics.gap);
        if self.command_error.is_some() || self.snapshot.last_error.is_some() {
            v2::notice(
                ui,
                StatusTone::Error,
                self.text("操作失败，请重试。", "The operation failed. Retry."),
            );
        }
        ui.label(
            RichText::new(self.locale.nearby())
                .size(16.0)
                .variation(b"wght", 600.0),
        );
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .id_salt("v2-nearby-peers")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let peers = visible_peers(&self.snapshot);
                if peers.is_empty() {
                    ui.add_space(32.0);
                    ui.vertical_centered(|ui| {
                        ui.label(self.text("尚未发现设备", "No devices found"));
                    });
                }
                for (index, peer) in peers.iter().enumerate() {
                    let current = owns_playback(&self.snapshot, peer);
                    let (tone, status) = row_status(self.locale, &self.snapshot, peer);
                    let response = v2::nearby_row(
                        ui,
                        v2::NearbyRow {
                            id: egui::Id::new(("peer", &peer.id)),
                            name: &peer_display_name(self.locale, index + 1),
                            subtitle: &format!(
                                "{}: {}",
                                remote_device_id_label(self.locale),
                                peer.id
                            ),
                            status,
                            tone,
                            current,
                            watch_label: if current {
                                self.text("返回播放器", "Return")
                            } else {
                                self.locale.watch()
                            },
                            watch_enabled: current
                                || can_watch(&self.snapshot, peer, self.nearby_turn_off_pending),
                            details_label: self.text("设备详情", "Device details"),
                            compact: metrics.compact,
                        },
                    );
                    if response.details {
                        self.selected_peer = Some(peer.id.clone());
                    }
                    if response.watch
                        && (current
                            || self.send(RuntimeCommand::WatchScreen {
                                path: crate::screen_path::for_peer(&peer.id),
                            }))
                    {
                        self.page = Page::Watch;
                    }
                    ui.add_space(8.0);
                }
            });
    }
    fn screen_share(&mut self, ui: &mut egui::Ui) {
        let p = Palette::for_ui(ui);
        let outer = ui.available_rect_before_wrap();
        let action_rect = egui::Rect::from_min_max(
            egui::pos2(outer.left(), (outer.bottom() - 56.0).max(outer.top())),
            outer.right_bottom(),
        );
        let editable = matches!(
            self.snapshot.media.phase,
            MediaPhase::Idle | MediaPhase::Failed
        );
        ui.scope_builder(UiBuilder::new().max_rect(action_rect), |ui| {
            ui.horizontal_centered(|ui| {
                let (label, enabled, tone) = match self.snapshot.media.phase {
                    MediaPhase::Sharing => (
                        self.text("停止共享", "Stop sharing"),
                        true,
                        ActionTone::Danger,
                    ),
                    MediaPhase::Preparing => (
                        self.text("正在准备", "Preparing"),
                        false,
                        ActionTone::Neutral,
                    ),
                    MediaPhase::Stopping => {
                        (self.text("正在停止", "Stopping"), false, ActionTone::Danger)
                    }
                    _ if matches!(
                        self.snapshot.discovery,
                        DiscoveryState::Stopped | DiscoveryState::Failed
                    ) =>
                    {
                        (
                            self.text("开启附近设备", "Turn on Nearby"),
                            !self.nearby_turn_off_pending,
                            ActionTone::Accent,
                        )
                    }
                    _ => (
                        self.text("开始共享", "Start sharing"),
                        can_share(
                            &self.snapshot,
                            self.capture_kind,
                            self.nearby_turn_off_pending,
                        ),
                        ActionTone::Accent,
                    ),
                };
                if (Action {
                    label,
                    size: vec2(164.0, 40.0),
                    enabled,
                    tone,
                })
                .show(ui)
                .clicked()
                {
                    self.send(if self.snapshot.media.phase == MediaPhase::Sharing {
                        RuntimeCommand::StopSharing
                    } else if matches!(
                        self.snapshot.discovery,
                        DiscoveryState::Stopped | DiscoveryState::Failed
                    ) {
                        RuntimeCommand::StartScan
                    } else {
                        RuntimeCommand::ShareScreen
                    });
                }
            });
        });
        let content = egui::Rect::from_min_max(
            outer.min,
            egui::pos2(outer.right(), (action_rect.top() - 12.0).max(outer.top())),
        );
        ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
            ui.scope(|ui| {
                p.card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal(|ui| {
                            for (kind, label) in [
                                (CaptureSourceKind::Display, self.text("屏幕", "Screen")),
                                (CaptureSourceKind::Window, self.text("窗口", "Window")),
                            ] {
                                if ui
                                    .add(
                                        egui::Button::selectable(self.capture_kind == kind, label)
                                            .min_size(vec2(88.0, 40.0)),
                                    )
                                    .clicked()
                                {
                                    self.capture_kind = kind;
                                }
                            }
                        });
                        ui.add_space(12.0);
                        let catalog = self.snapshot.media.capture_sources.clone();
                        // Leave room for the tabs, refresh action and encoding/audio settings.
                        v2::source_list(ui, (content.height() - 320.0).clamp(48.0, 240.0), |ui| {
                            match catalog.phase {
                                CaptureSourceCatalogPhase::Loading => {
                                    ui.spinner();
                                }
                                CaptureSourceCatalogPhase::Empty => {
                                    ui.label(
                                        self.text(
                                            "没有可共享的来源",
                                            "No capture sources available",
                                        ),
                                    );
                                }
                                CaptureSourceCatalogPhase::Failed => v2::notice(
                                    ui,
                                    StatusTone::Error,
                                    self.text(
                                        "来源不可用，请刷新后重试。",
                                        "Sources unavailable. Refresh and retry.",
                                    ),
                                ),
                                CaptureSourceCatalogPhase::Ready => {
                                    let mut count = 0;
                                    let kind = self.capture_kind;
                                    for choice in
                                        catalog.choices.iter().filter(|choice| choice.kind == kind)
                                    {
                                        count += 1;
                                        let selected = catalog.selected.as_ref().is_some_and(|s| {
                                            s.kind == choice.kind && s.id == choice.id
                                        });
                                        let label = format!(
                                            "{} · {}",
                                            choice.name,
                                            capture_source_subtitle(self.locale, choice)
                                        );
                                        if v2::source_row(ui, &label, selected, editable).clicked()
                                        {
                                            self.send(RuntimeCommand::SelectCaptureSource {
                                                choice: choice.clone(),
                                            });
                                        }
                                    }
                                    if count == 0 {
                                        ui.label(self.text(
                                            "没有可用来源，请刷新。",
                                            "No sources available. Refresh to retry.",
                                        ));
                                    }
                                }
                            }
                        });
                        if (Action {
                            label: self.text("刷新来源", "Refresh sources"),
                            size: vec2(124.0, 40.0),
                            enabled: catalog.phase != CaptureSourceCatalogPhase::Loading,
                            tone: ActionTone::Neutral,
                        })
                        .show(ui)
                        .clicked()
                        {
                            self.send(RuntimeCommand::RefreshCaptureSources);
                        }
                    });
                });
                ui.add_space(16.0);
                // This sibling viewport cannot consume wheel events from the source list.
                egui::ScrollArea::vertical()
                    .id_salt("sharing-options")
                    .max_height((content.bottom() - ui.cursor().top()).max(0.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        v2::settings_row(ui, self.text("视频编码", "Video encoding"), |ui| {
                            let mut policy = self.snapshot.media.video_encoding;
                            ui.add_enabled_ui(editable, |ui| {
                                egui::ComboBox::from_id_salt("video-policy")
                                    .selected_text(video_encoding_label(self.locale, policy))
                                    .show_ui(ui, |ui| {
                                        for candidate in [
                                            VideoEncodingPolicy::Compatible,
                                            VideoEncodingPolicy::NativeQhdHardware,
                                        ] {
                                            ui.selectable_value(
                                                &mut policy,
                                                candidate,
                                                video_encoding_label(self.locale, candidate),
                                            );
                                        }
                                    });
                            });
                            if policy != self.snapshot.media.video_encoding {
                                self.send(RuntimeCommand::SetVideoEncodingPolicy(policy));
                            }
                        });
                        ui.label(
                            RichText::new(
                                if self.snapshot.media.video_encoding
                                    == VideoEncodingPolicy::Compatible
                                {
                                    self.text(
                                        "原生尺寸，最长边不超过 1920。",
                                        "Native size, longest edge at most 1920.",
                                    )
                                } else {
                                    self.text(
                                        "原生横屏 2560×1440，要求硬件 H.264。",
                                        "Native landscape 2560×1440; hardware H.264 required.",
                                    )
                                },
                            )
                            .small()
                            .color(p.muted),
                        );
                        ui.add_space(12.0);
                        ui.label(self.text(
                            "系统声音随共享采集，不限于所选窗口。",
                            "Sharing captures system sound, not only the selected window.",
                        ));
                        if self.capture_kind == CaptureSourceKind::Window {
                            ui.label(
                                RichText::new(self.text(
                                    "最小化会暂停画面；关闭窗口会结束共享。",
                                    "Minimizing pauses video; closing the window ends sharing.",
                                ))
                                .small()
                                .color(p.muted),
                            );
                        }
                        if !matches!(
                            self.snapshot.view.phase,
                            ViewPhase::Idle | ViewPhase::Failed
                        ) {
                            v2::notice(
                                ui,
                                StatusTone::Neutral,
                                self.text(
                                    "停止观看后可开始共享。",
                                    "Stop watching before sharing.",
                                ),
                            );
                        }
                        if self.snapshot.media.audio.phase == AudioPhase::Failed {
                            v2::notice(
                                ui,
                                StatusTone::Warning,
                                self.text(
                                    "系统声音不可用，画面共享继续。",
                                    "System sound unavailable; video sharing continues.",
                                ),
                            );
                        }
                        if let Some(warning) = self.snapshot.media.capture_sources.warning {
                            v2::notice(ui, StatusTone::Warning, warning);
                        }
                        if let Some(error) = self
                            .snapshot
                            .media
                            .capture_sources
                            .last_error
                            .or(self.snapshot.media.last_error)
                        {
                            v2::notice(ui, StatusTone::Error, error);
                        }
                    });
            });
        });
    }
    fn watch(&mut self, ui: &mut egui::Ui) {
        if watch_projection(self.snapshot.view.phase) == WatchProjection::Player {
            let action = self.player.show(
                ui,
                self.locale,
                &self.snapshot.view,
                self.playback_texture.as_ref(),
                self.playback_identity,
            );
            self.handle_player_action(action);
        } else if v2::canvas_message(
            ui,
            v2::CanvasMessage {
                title: if self.snapshot.view.phase == ViewPhase::Failed {
                    self.text("播放失败", "Playback failed")
                } else {
                    self.text("播放已结束", "Playback ended")
                },
                detail: if self.snapshot.view.phase == ViewPhase::Failed {
                    self.text("请返回附近设备后重试。", "Return to Nearby and retry.")
                } else {
                    ""
                },
                action: self.text("返回附近设备", "Back to Nearby"),
            },
        )
        .clicked()
        {
            self.page = Page::Nearby;
        }
    }
    fn modals(&mut self, ctx: &egui::Context, p: Palette) {
        if self.settings_open {
            let response = egui::Modal::new(egui::Id::new("v2-settings"))
                .frame(p.dialog())
                .show(ctx, |ui| {
                    ui.set_width((ctx.content_rect().width() - 96.0).clamp(240.0, 480.0));
                    let close = v2::dialog_header(
                        ui,
                        self.locale.settings(),
                        self.text("关闭设置", "Close settings"),
                    )
                    .clicked();
                    egui::ScrollArea::vertical()
                        .id_salt("settings-body")
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .max_height(
                            (ctx.content_rect().height() - 160.0)
                                .clamp(120.0, if self.developer_mode { 368.0 } else { 256.0 }),
                        )
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            v2::settings_row(ui, self.text("外观", "Appearance"), |ui| {
                                for (theme, label) in [
                                    (egui::ThemePreference::Dark, self.text("深色", "Dark")),
                                    (egui::ThemePreference::Light, self.text("浅色", "Light")),
                                    (egui::ThemePreference::System, self.text("系统", "System")),
                                ] {
                                    if ui
                                        .selectable_value(&mut self.theme_preference, theme, label)
                                        .changed()
                                    {
                                        ctx.set_theme(theme);
                                    }
                                }
                            });
                            v2::settings_row(ui, self.text("语言", "Language"), |ui| {
                                egui::ComboBox::from_id_salt("v2-language")
                                    .selected_text(if self.locale == Locale::Chinese {
                                        "简体中文"
                                    } else {
                                        "English"
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.locale,
                                            Locale::Chinese,
                                            "简体中文",
                                        );
                                        ui.selectable_value(
                                            &mut self.locale,
                                            Locale::English,
                                            "English",
                                        );
                                    });
                            });
                            self.diagnostics.show_settings(ui, self.locale);
                            v2::settings_row(
                                ui,
                                self.text("开发者模式", "Developer mode"),
                                |ui| {
                                    ui.checkbox(&mut self.developer_mode, "");
                                },
                            );
                            if self.developer_mode {
                                self.diagnostics.show_developer_settings(ui, self.locale);
                            } else {
                                self.diagnostics.hide_window();
                            }
                        });
                    close || self.diagnostics.visible()
                });
            if response.should_close() || response.inner {
                self.settings_open = false;
            }
        }
        if let Some(id) = self.selected_peer.clone() {
            let response = egui::Modal::new(egui::Id::new("v2-peer-details"))
                .frame(p.dialog())
                .show(ctx, |ui| {
                    ui.set_width((ctx.content_rect().width() - 96.0).clamp(240.0, 412.0));
                    let close = v2::dialog_header(
                        ui,
                        self.text("设备详情", "Device details"),
                        self.text("关闭设备详情", "Close device details"),
                    )
                    .clicked();
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 220.0).max(80.0))
                        .show(ui, |ui| {
                            if let Some(peer) = self.snapshot.peers.get(&id) {
                                ui.label(&id);
                                let (tone, status) = row_status(self.locale, &self.snapshot, peer);
                                v2::notice(ui, tone, status);
                                if !peer.present {
                                    ui.label(
                                        self.text(
                                            "设备不再可发现",
                                            "Device no longer discoverable",
                                        ),
                                    );
                                }
                            } else {
                                ui.label(self.text("设备已离开", "Device unavailable"));
                            }
                        });
                    close
                });
            if response.should_close() || response.inner {
                self.selected_peer = None;
            }
        }
        if self.confirm_turn_off_nearby {
            let response = egui::Modal::new(egui::Id::new("v2-nearby-off"))
                .frame(p.dialog())
                .show(ctx, |ui| {
                    ui.set_width((ctx.content_rect().width() - 96.0).clamp(240.0, 412.0));
                    let close = v2::dialog_header(
                        ui,
                        self.text("关闭附近设备？", "Turn off Nearby?"),
                        self.text("取消", "Cancel"),
                    )
                    .clicked();
                    ui.label(self.text(
                        "关闭会停止当前观看或共享，并断开附近连接。",
                        "Turning off Nearby stops media and closes nearby connections.",
                    ));
                    let confirm = (Action {
                        label: self.text("关闭附近设备", "Turn off Nearby"),
                        size: vec2(156.0, 40.0),
                        enabled: true,
                        tone: ActionTone::Danger,
                    })
                    .show(ui)
                    .clicked();
                    (close, confirm)
                });
            if response.inner.1 {
                self.request_turn_off_nearby();
            }
            if response.should_close() || response.inner.0 || response.inner.1 {
                self.confirm_turn_off_nearby = false;
            }
        }
    }
}
fn owns_playback(s: &RuntimeSnapshot, p: &PeerView) -> bool {
    matches!(
        s.view.phase,
        ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
    ) && s.view.path.as_deref() == Some(crate::screen_path::for_peer(&p.id).as_str())
}
fn can_watch(s: &RuntimeSnapshot, p: &PeerView, pending: bool) -> bool {
    !pending && s.can_watch(&crate::screen_path::for_peer(&p.id))
}
fn can_share(s: &RuntimeSnapshot, kind: CaptureSourceKind, pending: bool) -> bool {
    !pending
        && s.access().publish()
        && s.media.capture_sources.phase == CaptureSourceCatalogPhase::Ready
        && s.media
            .capture_sources
            .selected
            .as_ref()
            .is_some_and(|c| c.kind == kind)
}
fn local_media_status(locale: Locale, s: &RuntimeSnapshot) -> (StatusTone, &'static str) {
    let text = |zh, en| if locale == Locale::Chinese { zh } else { en };
    match s.view.phase {
        ViewPhase::Preparing => {
            return (StatusTone::Busy, text("正在准备观看", "Preparing playback"));
        }
        ViewPhase::Viewing => return (StatusTone::Active, text("正在观看", "Watching")),
        ViewPhase::Stopping => {
            return (StatusTone::Busy, text("正在停止观看", "Stopping playback"));
        }
        _ => {}
    }
    match s.media.phase {
        MediaPhase::Idle if s.view.phase == ViewPhase::Failed => {
            (StatusTone::Error, text("播放失败", "Playback failed"))
        }
        MediaPhase::Idle => (StatusTone::Neutral, text("空闲", "Idle")),
        MediaPhase::Preparing => (StatusTone::Busy, text("正在准备共享", "Preparing sharing")),
        MediaPhase::Sharing => (StatusTone::Active, text("正在共享", "Sharing")),
        MediaPhase::Stopping => (StatusTone::Busy, text("正在停止共享", "Stopping sharing")),
        MediaPhase::Failed => (StatusTone::Error, text("共享失败", "Sharing failed")),
    }
}
fn row_status(locale: Locale, s: &RuntimeSnapshot, p: &PeerView) -> (StatusTone, &'static str) {
    if owns_playback(s, p) {
        return local_media_status(locale, s);
    }
    use moqcast_lan_state::Status;
    let tone = match p.facts().status() {
        Status::Available | Status::Connected => StatusTone::Active,
        Status::Connecting => StatusTone::Busy,
        Status::Failed | Status::Rejected => StatusTone::Error,
        _ => StatusTone::Neutral,
    };
    (tone, peer_status(locale, p))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (MoqCastApp, tokio::sync::mpsc::Receiver<RuntimeCommand>) {
        let (runtime, commands) = RuntimeOwner::isolated_for_ui_test(
            tokio::sync::watch::channel(RuntimeSnapshot::default()).1,
        );
        let diagnostics = moqcast_diagnostics::init(moqcast_diagnostics::Config::without_file(
            moqcast_diagnostics::BuildInfo::new("test"),
            "UI fixture",
        ));
        let snapshot = RuntimeSnapshot {
            discovery: DiscoveryState::Stopped,
            ..Default::default()
        };
        (
            MoqCastApp {
                page: Page::Nearby,
                locale: Locale::English,
                developer_mode: false,
                settings_open: false,
                theme_preference: egui::ThemePreference::Light,
                capture_kind: CaptureSourceKind::Display,
                selected_peer: None,
                confirm_turn_off_nearby: false,
                nearby_turn_off_pending: false,
                diagnostics: DiagnosticsUi::new(diagnostics.handle(), false),
                runtime,
                snapshot,
                command_error: None,
                playback_texture: None,
                playback_identity: None,
                playback_timestamp_us: None,
                playback_high_water_timestamp_us: None,
                viewport_fullscreen: false,
                player: LivePlayer::default(),
            },
            commands,
        )
    }
    fn render(
        ctx: &egui::Context,
        app: &mut MoqCastApp,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |root| app.desktop_shell(root),
        );
        out.textures_delta.clear();
        out
    }
    fn painted(out: &egui::FullOutput, label: &str) -> egui::Rect {
        out.shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => Some(
                    text.galley
                        .rect
                        .translate(text.pos.to_vec2())
                        .intersect(shape.clip_rect),
                ),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {label}"))
    }
    fn click(ctx: &egui::Context, app: &mut MoqCastApp, size: egui::Vec2, pos: egui::Pos2) {
        for pressed in [true, false] {
            render(
                ctx,
                app,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    #[test]
    fn nearby_identity_is_visible_without_opening_details() {
        let (mut app, _commands) = fixture();
        app.snapshot.peers.insert(
            "remote-peer".into(),
            PeerView {
                id: "remote-peer".into(),
                candidates: vec![],
                should_dial: false,
                authenticated_discovery: false,
                tls_pinned: true,
                present: true,
                transport: Default::default(),
                screen: ScreenAvailability::Unavailable,
            },
        );
        let ctx = egui::Context::default();
        v2::install(&ctx);
        let out = render(&ctx, &mut app, vec2(1040.0, 640.0), vec![]);
        assert!(painted(&out, "Device ID: remote-peer").is_positive());
    }
    #[test]
    fn pages_and_settings_keep_real_actions_visible() {
        let (mut app, _commands) = fixture();
        for width in [680.0, 919.0, 920.0, 1440.0] {
            for dark in [false, true] {
                let ctx = egui::Context::default();
                install_ui_font(&ctx, std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC));
                v2::install(&ctx);
                ctx.set_theme(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                let size = vec2(width, 640.0);
                for page in [Page::Nearby, Page::ScreenShare, Page::Watch] {
                    app.page = page;
                    app.settings_open = false;
                    let out = render(&ctx, &mut app, size, vec![]);
                    let label = if page == Page::Watch {
                        "Back to Nearby"
                    } else {
                        "Turn on Nearby"
                    };
                    let rect = painted(&out, label);
                    assert!(rect.is_positive() && rect.bottom() <= size.y);
                }
                app.page = Page::Nearby;
                app.settings_open = true;
                render(&ctx, &mut app, size, vec![]);
                let out = render(&ctx, &mut app, size, vec![]);
                assert!(painted(&out, "Export logs").is_positive());
                assert!(!app.developer_mode);
            }
        }
    }
    #[test]
    fn back_navigation_does_not_stop_playback() {
        let (mut app, mut commands) = fixture();
        let ctx = egui::Context::default();
        v2::install(&ctx);
        app.page = Page::Watch;
        app.snapshot.view.phase = ViewPhase::Preparing;
        let size = vec2(680.0, 640.0);
        let out = render(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, painted(&out, "‹  Nearby").center());
        assert_eq!(app.page, Page::Nearby);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn source_tab_must_match_the_source_sent_for_sharing() {
        let mut s = RuntimeSnapshot {
            discovery: DiscoveryState::Ready,
            local_id: Some("local".into()),
            ..Default::default()
        };
        s.media.capture_sources.phase = CaptureSourceCatalogPhase::Ready;
        s.media.capture_sources.selected = Some(CaptureSourceChoice {
            kind: CaptureSourceKind::Display,
            id: "display".into(),
            name: "Display".into(),
            application: None,
            width: 1920,
            height: 1080,
        });
        assert!(can_share(&s, CaptureSourceKind::Display, false));
        assert!(!can_share(&s, CaptureSourceKind::Window, false));
        assert!(!can_share(&s, CaptureSourceKind::Display, true));
        s.stopping = true;
        assert!(!can_share(&s, CaptureSourceKind::Display, false));
    }
    #[test]
    fn missing_advertisement_does_not_hide_an_available_route() {
        let mut s = RuntimeSnapshot {
            discovery: DiscoveryState::Ready,
            local_id: Some("local".into()),
            ..Default::default()
        };
        let p = PeerView {
            id: "peer".into(),
            candidates: vec![],
            should_dial: false,
            authenticated_discovery: false,
            tls_pinned: true,
            present: false,
            transport: Default::default(),
            screen: ScreenAvailability::Available,
        };
        s.peers.insert(p.id.clone(), p.clone());
        s.remote_screens.insert(
            crate::screen_path::for_peer(&p.id),
            crate::remote::RemoteScreenView {
                peer_id: p.id.clone(),
                availability: ScreenAvailability::Available,
            },
        );
        assert_eq!(visible_peers(&s).len(), 1);
        assert!(can_watch(&s, &p, false));
        assert_eq!(row_status(Locale::English, &s, &p).1, "Available");
        s.stopping = true;
        assert!(!can_watch(&s, &p, false));
        s.stopping = false;
        s.view.phase = ViewPhase::Preparing;
        s.view.path = Some(crate::screen_path::for_peer(&p.id));
        assert_eq!(row_status(Locale::English, &s, &p).1, "Preparing playback");
        assert!(!can_watch(&s, &p, false));
    }
    #[test]
    fn long_source_list_does_not_push_start_out_of_view() {
        let (mut app, mut commands) = fixture();
        app.page = Page::ScreenShare;
        app.snapshot.discovery = DiscoveryState::Ready;
        app.snapshot.local_id = Some("local".into());
        let catalog = &mut app.snapshot.media.capture_sources;
        catalog.phase = CaptureSourceCatalogPhase::Ready;
        catalog.choices = (0..60)
            .map(|n| CaptureSourceChoice {
                kind: CaptureSourceKind::Display,
                id: n.to_string(),
                name: format!("Display {n} {}", "Long name".repeat(12)),
                application: None,
                width: 1920,
                height: 1080,
            })
            .collect();
        catalog.selected = catalog.choices.first().cloned();
        let ctx = egui::Context::default();
        v2::install(&ctx);
        let size = vec2(680.0, 640.0);
        let out = render(&ctx, &mut app, size, vec![]);
        let start = painted(&out, "Start sharing");
        assert!(start.is_positive() && start.bottom() < size.y);
        click(&ctx, &mut app, size, start.center());
        assert!(matches!(
            commands.try_recv(),
            Ok(RuntimeCommand::ShareScreen)
        ));
    }
    #[test]
    fn source_list_scroll_keeps_refresh_settings_and_start_fixed() {
        let (mut app, _commands) = fixture();
        app.page = Page::ScreenShare;
        app.capture_kind = CaptureSourceKind::Window;
        app.snapshot.discovery = DiscoveryState::Ready;
        app.snapshot.local_id = Some("local".into());
        let catalog = &mut app.snapshot.media.capture_sources;
        catalog.phase = CaptureSourceCatalogPhase::Ready;
        catalog.choices = (0..60)
            .map(|n| CaptureSourceChoice {
                kind: CaptureSourceKind::Window,
                id: n.to_string(),
                name: format!("Window {n}"),
                application: None,
                width: 1920,
                height: 1080,
            })
            .collect();
        catalog.selected = catalog.choices.first().cloned();
        let ctx = egui::Context::default();
        v2::install(&ctx);
        let size = vec2(1040.0, 760.0);
        let before = render(&ctx, &mut app, size, vec![]);
        let labels = ["Refresh sources", "Video encoding", "Start sharing"];
        let fixed = labels.map(|label| painted(&before, label));
        assert!(
            fixed.iter().all(|r| r.is_positive()),
            "all controls must be visible before scrolling"
        );
        let first = before
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text.starts_with("Window 0 ·") => Some(
                    t.galley
                        .rect
                        .translate(t.pos.to_vec2())
                        .intersect(s.clip_rect),
                ),
                _ => None,
            })
            .unwrap();
        let mut after = render(
            &ctx,
            &mut app,
            size,
            vec![
                egui::Event::PointerMoved(first.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(0.0, -400.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..20 {
            after = render(&ctx, &mut app, size, vec![]);
        }
        // Reach the end, then keep scrolling to catch propagation into a parent area.
        for _ in 0..3 {
            render(
                &ctx,
                &mut app,
                size,
                vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(0.0, -10000.0),
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            for _ in 0..20 {
                after = render(&ctx, &mut app, size, vec![]);
            }
        }
        assert!(
            after.shapes.iter().any(|s| matches!(&s.shape,
                egui::Shape::Text(t) if t.galley.job.text.starts_with("Window 59 ·")
                && t.galley.rect.translate(t.pos.to_vec2()).intersect(s.clip_rect).is_positive()
            )),
            "the last source must remain reachable"
        );
        for (label, rect) in labels.into_iter().zip(fixed) {
            assert_eq!(
                painted(&after, label),
                rect,
                "{label} moved with the source list"
            );
        }
        assert!(
            !after.shapes.iter().any(|s| matches!(&s.shape,
                egui::Shape::Text(t) if t.galley.job.text.starts_with("Window 0 ·")
                && t.galley.rect.translate(t.pos.to_vec2()).intersect(s.clip_rect).is_positive()
            )),
            "the source list must actually scroll"
        );
    }

    #[test]
    fn snapshot_channel_updates_render_rows_without_inbound_attribution() {
        let (mut app, _) = fixture();
        let (updates, receiver) = tokio::sync::watch::channel(RuntimeSnapshot::default());
        let (runtime, _commands) = RuntimeOwner::isolated_for_ui_test(receiver);
        app.runtime = runtime;
        let ctx = egui::Context::default();
        v2::install(&ctx);
        let size = vec2(1040.0, 640.0);
        let mut snapshot = RuntimeSnapshot {
            discovery: DiscoveryState::Ready,
            local_id: Some("local".into()),
            inbound_sessions: 1,
            ..Default::default()
        };
        let peer = PeerView {
            id: "peer".into(),
            candidates: vec![],
            should_dial: false,
            authenticated_discovery: false,
            tls_pinned: true,
            present: true,
            transport: Default::default(),
            screen: ScreenAvailability::Unavailable,
        };
        snapshot.peers.insert(peer.id.clone(), peer);
        // The same watch receiver feeds the real UI; multiple updates may coalesce.
        updates.send(snapshot.clone()).unwrap();
        app.snapshot = app.runtime.snapshot();
        let out = render(&ctx, &mut app, size, vec![]);
        assert!(painted(&out, "Found").is_positive());
        assert!(!can_watch(
            &app.snapshot,
            &app.snapshot.peers["peer"],
            false
        ));
        snapshot.peers.get_mut("peer").unwrap().present = false;
        snapshot.peers.get_mut("peer").unwrap().screen = ScreenAvailability::Available;
        snapshot.remote_screens.insert(
            crate::screen_path::for_peer("peer"),
            crate::remote::RemoteScreenView {
                peer_id: "peer".into(),
                availability: ScreenAvailability::Available,
            },
        );
        updates.send(snapshot.clone()).unwrap();
        app.snapshot = app.runtime.snapshot();
        let out = render(&ctx, &mut app, size, vec![]);
        assert!(painted(&out, "Available").is_positive());
        assert!(can_watch(&app.snapshot, &app.snapshot.peers["peer"], false));
        snapshot.remote_screens.clear();
        snapshot.peers.get_mut("peer").unwrap().screen = ScreenAvailability::Withdrawn;
        updates.send(snapshot).unwrap();
        app.snapshot = app.runtime.snapshot();
        assert_eq!(visible_peer_views(&app.snapshot).count(), 0);
        let out = render(&ctx, &mut app, size, vec![]);
        assert!(!out.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text == "Nearby device 1")));
    }

    #[test]
    fn active_sharing_takes_precedence_over_a_previous_view_failure() {
        let mut s = RuntimeSnapshot::default();
        s.view.phase = ViewPhase::Failed;
        s.media.phase = MediaPhase::Sharing;
        assert_eq!(
            local_media_status(Locale::English, &s),
            (StatusTone::Active, "Sharing")
        );
        s.media.phase = MediaPhase::Idle;
        assert_eq!(
            local_media_status(Locale::English, &s),
            (StatusTone::Error, "Playback failed")
        );
    }
}
