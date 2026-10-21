//! Native S2 shell. Runtime snapshots remain the source of network and media state.

use super::*;
use egui::{RichText, Sense, UiBuilder, vec2};
use moqcast_ui::v2::{
    Action, ActionTone, Palette, StatusTone, action, dialog_header, monitor, notice,
};

impl MoqCastApp {
    pub(super) fn desktop_shell(&mut self, root: &mut egui::Ui, snapshot: &AppSnapshot) {
        let p = Palette::for_ui(root);
        let metrics = moqcast_ui::v2::Metrics::for_width(root.ctx().content_rect().width());
        egui::Panel::top("s2-header")
            .exact_size(metrics.header)
            .frame(
                Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(metrics.inset, 0)),
            )
            .show(root, |ui| {
                let title = match self.page {
                    Page::Nearby => None,
                    Page::Watch => Some(self.player_device_name(snapshot)),
                    _ => Some(self.text("共享设置", "Sharing settings").to_owned()),
                };
                let response = moqcast_ui::v2::header(
                    ui,
                    moqcast_ui::v2::Header {
                        title: title.as_deref(),
                        back: self.text("‹  附近设备", "‹  Nearby"),
                        settings: self.text("设置", "Settings"),
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
            egui::Panel::bottom("s2-footer")
                .exact_size(metrics.footer)
                .frame(
                    Frame::new()
                        .fill(p.canvas)
                        .inner_margin(egui::Margin::symmetric(metrics.inset, 0)),
                )
                .show(root, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let active = !matches!(
                                snapshot.discovery.phase(),
                                DiscoveryPhase::Stopped | DiscoveryPhase::Failed
                            );
                            let label = if snapshot.nearby_stopping {
                                self.text("正在关闭", "Turning off")
                            } else if self.nearby_action_pending.is_some() {
                                self.text("处理中", "Pending")
                            } else if active {
                                self.text("关闭附近设备", "Turn off Nearby")
                            } else {
                                self.text("开启附近设备", "Turn on Nearby")
                            };
                            if (Action {
                                label,
                                size: vec2(148.0, 40.0),
                                enabled: !snapshot.nearby_stopping
                                    && nearby_action_enabled(
                                        self.nearby_action_pending,
                                        snapshot.runtime.phase(),
                                    ),
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
                                    if has_active_media(snapshot) {
                                        self.confirm_turn_off_nearby = true;
                                    } else {
                                        self.request_turn_off_nearby(snapshot);
                                    }
                                } else {
                                    self.request_turn_on_nearby(snapshot);
                                }
                            }
                            let (tone, status) = discovery_status(
                                snapshot,
                                self.nearby_action_pending.map(|pending| pending.action),
                                self.locale,
                            );
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                tone.indicator(ui);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(status).size(12.0).color(p.muted),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(global_summary(snapshot, self.locale));
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
                if self.page == Page::Nearby {
                    let width = ui.available_width().min(1240.0);
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(ui.max_rect().center().x - width / 2.0, ui.cursor().top()),
                        vec2(width, ui.available_height()),
                    );
                    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                        // At extreme zoom, keep controls reachable without changing native minimum size.
                        if ui.available_width() < 580.0 || ui.available_height() < 260.0 {
                            egui::ScrollArea::both()
                                .id_salt("v2-nearby-safety-scroll")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                )
                                .show(ui, |ui| {
                                    ui.set_min_width(580.0);
                                    self.s2_nearby(ui, snapshot, metrics);
                                });
                        } else {
                            self.s2_nearby(ui, snapshot, metrics);
                        }
                    });
                } else {
                    match self.page {
                        Page::ScreenShare => {
                            let width = ui.available_width().min(792.0);
                            let rect = egui::Rect::from_center_size(
                                ui.available_rect_before_wrap().center(),
                                vec2(width, ui.available_height()),
                            );
                            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                                self.s2_share(ui, snapshot)
                            });
                        }
                        Page::Watch if Self::player_active(snapshot) => {
                            self.watch_player(ui, snapshot)
                        }
                        Page::Watch => {
                            self.s2_watch_empty(ui, snapshot);
                        }
                        Page::Nearby => {}
                    }
                }
            });
        if self.settings_open {
            let response = egui::Modal::new(egui::Id::new("v2-settings"))
                .frame(p.dialog())
                .show(root.ctx(), |ui| {
                    ui.set_width((root.ctx().content_rect().width() - 96.0).min(480.0));
                    let close = dialog_header(
                        ui,
                        self.text("设置", "Settings"),
                        self.text("关闭设置", "Close settings"),
                    )
                    .clicked();
                    self.s2_settings(ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        Action {
                            label: self.text("关闭", "Close"),
                            size: vec2(108.0, 42.0),
                            enabled: true,
                            tone: ActionTone::Neutral,
                        }
                        .show(ui)
                        .clicked()
                    })
                    .inner
                        || close
                });
            if response.should_close() || response.inner {
                self.settings_open = false;
            }
        }
        if let Some(peer_id) = self.detail_peer.clone() {
            let response = egui::Modal::new(egui::Id::new("v2-device-details"))
                .frame(p.dialog())
                .show(root.ctx(), |ui| {
                    ui.set_width((root.ctx().content_rect().width() - 96.0).min(412.0));
                    let close = dialog_header(
                        ui,
                        self.text("设备详情", "Device details"),
                        self.text("关闭设备详情", "Close device details"),
                    )
                    .clicked();
                    egui::ScrollArea::vertical()
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .max_height((root.ctx().content_rect().height() - 220.0).max(120.0))
                        .show(ui, |ui| {
                            if let Some(peer) = snapshot.peers.get(&peer_id) {
                                let view = PeerPresentation::from(peer);
                                ui.label(
                                    RichText::new(device_name(peer, self.locale))
                                        .size(19.0)
                                        .variation(b"wght", 600.0),
                                );
                                ui.label(
                                    RichText::new(remote_device_id_label(self.locale))
                                        .color(p.muted),
                                );
                                ui.label(&peer_id).on_hover_text(
                                    self.text("标识属于本次运行。", "Identifier for this run."),
                                );
                                ui.separator();
                                ui.label(peer_line(view, self.locale));
                                ui.label(screen_label(
                                    screen_availability(&peer_id, &snapshot.remote_screens),
                                    self.locale,
                                ));
                            } else {
                                ui.label(self.text(
                                    "设备已离开，当前没有可用信息。",
                                    "The device is no longer available.",
                                ));
                            }
                        });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        Action {
                            label: self.text("关闭", "Close"),
                            size: vec2(108.0, 42.0),
                            enabled: true,
                            tone: ActionTone::Neutral,
                        }
                        .show(ui)
                        .clicked()
                    })
                    .inner
                        || close
                });
            if response.should_close() || response.inner {
                self.detail_peer = None;
            }
        }
    }

    fn s2_nearby(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &AppSnapshot,
        metrics: moqcast_ui::v2::Metrics,
    ) {
        let p = Palette::for_ui(ui);
        let compact = metrics.compact;
        ui.spacing_mut().item_spacing.y = 0.0;
        let watch = snapshot.media_owner == Some(MediaOwner::Watch);
        let owned = snapshot.media_owner.is_some();
        let status = match snapshot.media.phase() {
            MediaPhase::Idle => self.text("空闲", "Idle"),
            MediaPhase::PreparingShare => self.text("正在准备共享", "Preparing to share"),
            MediaPhase::Sharing => self.text("正在共享", "Sharing"),
            MediaPhase::PreparingWatch => self.text("正在连接画面", "Opening stream"),
            MediaPhase::Watching => self.text("正在观看", "Watching"),
            MediaPhase::Stopping => self.text("正在停止", "Stopping"),
            MediaPhase::Failed => self.text("媒体任务需要处理", "Media needs attention"),
        };
        let name = snapshot
            .local_device_name
            .as_deref()
            .unwrap_or(self.text("这台 Mac", "This Mac"));
        let identity = format!(
            "{}: {}",
            local_device_id_label(self.locale),
            snapshot.local_peer_id.as_deref().unwrap_or("—")
        );
        let label = if watch {
            self.text("返回播放", "Return to player")
        } else if owned {
            self.text("管理共享", "Manage sharing")
        } else {
            self.text("开始共享…", "Share…")
        };
        if moqcast_ui::v2::context_card(
            ui,
            moqcast_ui::v2::ContextCard {
                heading: self.text("本机", "This device"),
                name,
                identity: &identity,
                status,
                tone: media_tone(snapshot.media.phase()),
                action: label,
            },
            metrics,
        )
        .on_hover_text(local_device_description(
            self.locale,
            name,
            local_status(snapshot, self.locale),
            snapshot.local_peer_id.as_deref(),
        ))
        .clicked()
        {
            self.page = if watch {
                Page::Watch
            } else {
                Page::ScreenShare
            };
        }
        ui.add_space(metrics.gap);
        if let Some(issue) = snapshot.nearby_issue {
            // Keep existing recovery actions and error explanations, not just a colored dot.
            ui.scope(|ui| {
                notice(ui, StatusTone::Warning, issue_message(issue, self.locale));
                if matches!(
                    issue,
                    NearbyIssue::DiscoveryStopped
                        | NearbyIssue::ListenerStopped
                        | NearbyIssue::ServicesStopped
                ) && action(
                    ui,
                    self.text("重新启动附近设备", "Restart Nearby"),
                    vec2(180.0, 40.0),
                    nearby_action_enabled(self.nearby_action_pending, snapshot.runtime.phase()),
                )
                .clicked()
                {
                    self.request_turn_on_nearby(snapshot);
                }
            });
        }
        self.s2_command_notice(ui);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.text("附近设备", "Nearby devices"))
                    .size(16.0)
                    .variation(b"wght", 600.0),
            );
            ui.label(
                RichText::new(snapshot.peers.len().to_string())
                    .size(12.0)
                    .color(p.muted),
            );
        });
        ui.add_space(12.0);
        let row_height = if compact { 76.0 } else { 88.0 };
        let desired_height = if snapshot.peers.is_empty() {
            216.0
        } else {
            snapshot.peers.len() as f32 * row_height + 36.0
        };
        let height = desired_height.min(ui.available_height().clamp(100.0, 440.0));
        p.card()
            .inner_margin(egui::Margin::symmetric(if compact { 18 } else { 24 }, 18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(height - 38.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                let list_height = (height - 38.0).max(40.0);
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .id_salt("s2-nearby-list")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if snapshot.peers.is_empty() {
                            // The wide shell leaves less height at the minimum window size.
                            ui.add_space(((list_height - 98.0) / 2.0).max(0.0));
                            ui.vertical_centered(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(vec2(56.0, 42.0), Sense::hover());
                                let (tone, _) = discovery_status(
                                    snapshot,
                                    self.nearby_action_pending.map(|pending| pending.action),
                                    self.locale,
                                );
                                monitor(ui, rect, tone.color(p));
                                let title = match snapshot.discovery.phase() {
                                    DiscoveryPhase::Stopped => self.text(
                                        "发现同一网络的设备",
                                        "Discover devices on your network",
                                    ),
                                    DiscoveryPhase::Failed => self
                                        .text("暂时无法搜索设备", "Device discovery unavailable"),
                                    DiscoveryPhase::Starting => {
                                        self.text("正在开启附近设备", "Starting Nearby")
                                    }
                                    _ => self.text("暂无附近设备", "No nearby devices"),
                                };
                                ui.label(
                                    RichText::new(title)
                                        .size(17.0)
                                        .variation(b"wght", 500.0)
                                        .color(p.text),
                                );
                                ui.label(
                                    RichText::new(match snapshot.discovery.phase() {
                                        DiscoveryPhase::Stopped => self.text("", ""),
                                        DiscoveryPhase::Failed => self.text(
                                            "查看上方提示，恢复后重新开启附近设备",
                                            "Check the notice above, then turn on Nearby again",
                                        ),
                                        DiscoveryPhase::Starting => {
                                            self.text("请稍候", "Please wait")
                                        }
                                        _ => self.text(
                                            "在同一网络的其他设备上打开 MoQCast",
                                            "Open MoQCast on another device on this network",
                                        ),
                                    })
                                    .size(13.0)
                                    .color(p.muted),
                                );
                            });
                        }
                        for (id, peer) in &snapshot.peers {
                            let availability = screen_availability(id, &snapshot.remote_screens);
                            let path = crate::contract::screen_path(id);
                            let owned = snapshot.media_owner == Some(MediaOwner::Watch)
                                && snapshot.media_peer.as_deref() == Some(id.as_str());
                            let current = owned && snapshot.media.phase() == MediaPhase::Watching;
                            let (tone, status) = peer_status(
                                PeerPresentation::from(peer),
                                availability,
                                owned.then_some(snapshot.media.phase()),
                                self.locale,
                            );
                            let name = device_name(peer, self.locale);
                            let subtitle = if compact {
                                status.to_owned()
                            } else {
                                peer_list_subtitle(self.locale, id)
                            };
                            let response = moqcast_ui::v2::nearby_row(
                                ui,
                                moqcast_ui::v2::NearbyRow {
                                    id: egui::Id::new(id),
                                    name: &name,
                                    subtitle: &subtitle,
                                    status,
                                    tone,
                                    current,
                                    watch_label: if owned {
                                        self.text("返回播放", "Return")
                                    } else {
                                        self.text("观看", "Watch")
                                    },
                                    watch_enabled: owned || snapshot.can_watch(id, &path),
                                    details_label: self.text("设备详情", "Device details"),
                                    compact,
                                },
                            );
                            if response.details {
                                self.detail_peer = Some(id.clone());
                            }
                            if response.watch {
                                if owned || self.runtime.watch_screen(id.clone(), path) {
                                    self.page = Page::Watch;
                                } else {
                                    self.command_error = true;
                                }
                            }
                        }
                    });
            });
    }

    fn s2_settings(&mut self, ui: &mut egui::Ui) {
        let preferred_height = if self.developer_mode { 312.0 } else { 256.0 };
        let body_height = (ui.ctx().content_rect().height() - 160.0).clamp(120.0, preferred_height);
        egui::ScrollArea::vertical()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .max_height(body_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::NONE.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let appearance = self.text("外观", "Appearance");
                    moqcast_ui::v2::settings_row(ui, appearance, |ui| {
                        for (preference, label) in [
                            (egui::ThemePreference::Dark, self.text("深色", "Dark")),
                            (egui::ThemePreference::Light, self.text("浅色", "Light")),
                            (
                                egui::ThemePreference::System,
                                self.text("跟随系统", "System"),
                            ),
                        ] {
                            if ui
                                .selectable_value(&mut self.theme_preference, preference, label)
                                .changed()
                            {
                                ui.ctx().set_theme(self.theme_preference);
                            }
                        }
                    });
                    let language = self.text("语言", "Language");
                    moqcast_ui::v2::settings_row(ui, language, |ui| {
                        egui::ComboBox::from_id_salt("v2-language")
                            .selected_text(if self.locale == Locale::Chinese {
                                "简体中文"
                            } else {
                                "English"
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.locale, Locale::Chinese, "简体中文");
                                ui.selectable_value(&mut self.locale, Locale::English, "English");
                            });
                    });
                    self.diagnostics.show_settings(ui, self.locale);
                    let developer = self.text("开发者模式", "Developer mode");
                    let previous = self.developer_mode;
                    moqcast_ui::v2::settings_row(ui, developer, |ui| {
                        ui.checkbox(&mut self.developer_mode, "")
                            .on_hover_text(developer);
                    });
                    if previous != self.developer_mode {
                        self.diagnostics.set_detailed(self.developer_mode);
                        if !self.developer_mode {
                            self.diagnostics.hide_window();
                        }
                    }
                    if self.developer_mode
                        && self.diagnostics.show_developer_settings(ui, self.locale)
                    {
                        self.settings_open = false;
                    }
                });
            });
    }
    fn s2_share(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let p = Palette::for_ui(ui);
        let share_owned = snapshot.media_owner == Some(MediaOwner::Share);
        let stopping = snapshot.media.phase() == MediaPhase::Stopping;
        let idle = snapshot.media.phase() == MediaPhase::Idle;
        let ready = snapshot.access().ready;
        // Keep actions outside scrolling content at the minimum window size.
        let available = ui.available_rect_before_wrap();
        let actions = egui::Rect::from_min_max(
            egui::pos2(available.left(), available.bottom() - 58.0),
            available.right_bottom(),
        );
        let body = egui::Rect::from_min_max(
            available.min,
            egui::pos2(available.right(), actions.top() - 16.0),
        );
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            egui::ScrollArea::vertical().id_salt("v2-share-content")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .max_height(body.height()).show(ui, |ui| {
                p.card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(self.text("共享来源", "Sharing source")).size(15.0).variation(b"wght", 600.0));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        for (kind, label) in [(capture_picker::Kind::Display, self.text("屏幕", "Screen")), (capture_picker::Kind::Window,self.text("窗口", "Window"))] {
                            ui.add_enabled_ui(idle && !self.picker_pending, |ui| { ui.selectable_value(&mut self.picker_kind,kind,label); });
                        }
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if (Action { label: if self.picker_pending { self.text("正在打开…", "Opening…") } else { self.text("更改来源…", "Change source…") }, size: vec2(136.0,40.0), enabled: idle && ready && !self.picker_pending, tone: ActionTone::Neutral }).show(ui).clicked() { self.present_source_picker(ui.ctx()); }
                            let label = snapshot.share_selection.as_ref().filter(|s| self.picker_kind.matches(s)).map(|s| s.label()).unwrap_or(self.text("未选择来源", "No source selected"));
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                let (_,rect)=ui.allocate_space(vec2(32.0,40.0)); monitor(ui,rect,p.muted);
                                ui.add(egui::Label::new(RichText::new(label).size(15.0)).truncate()).on_hover_text(label);
                            });
                        });
                    });
                });
                ui.add_space(14.0);
                p.card().inner_margin(16).show(ui,|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let audio_available = self.picker_kind == capture_picker::Kind::Display && system_audio_action_available(snapshot);
                            let mut audio = share_audio_selected(snapshot, self.picker_kind);
                            if ui.add_enabled(audio_available,egui::Checkbox::without_text(&mut audio)).on_hover_text(self.text("共享系统声音","Share system audio")).changed() {
                                self.command_error = !self.runtime.set_share_system_audio(audio);
                            }
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                ui.label(RichText::new(self.text("共享系统声音","Share system audio")).size(14.0));
                                ui.label(RichText::new(self.text("仅主显示器支持，不含麦克风。", "Main display only; microphone excluded.")).size(12.0).color(p.muted));
                            });
                        });
                    });
                });
                ui.add_space(12.0);
                ui.scope(|ui| {
                    if snapshot.share_audio == ShareAudioPhase::Failed {
                        notice(ui, StatusTone::Warning, snapshot.share_audio_error.as_deref().unwrap_or(self.text("系统声音不可用，画面共享继续。", "System audio unavailable; video sharing continues.")));
                    }
                    ui.add_space(8.0);
                    if self.picker_error || self.capture_permission == CapturePermission::Denied
                        || !ready || snapshot.media_owner.is_some() {
                    let message = if self.picker_error {
                        self.text("无法打开系统选择器，请检查屏幕录制权限后重试。", "Could not open the picker. Check Screen Recording permission and try again.")
                    } else if self.capture_permission == CapturePermission::Denied {
                        self.text("需要屏幕录制权限。请在系统设置中允许 MoQCast，然后重新选择。", "Screen Recording permission is required. Allow MoQCast in System Settings, then choose again.")
                    } else if !ready { self.text("请先开启附近设备并等待服务就绪。", "Turn on Nearby and wait for its services to become ready.") }
                    else if snapshot.media_owner == Some(MediaOwner::Watch) { self.text("正在观看。请先停止播放，再开始共享。", "Stop the current playback before sharing.") }
                    else { match snapshot.media.phase() {
                        MediaPhase::PreparingShare => self.text("正在准备共享…", "Preparing to share…"),
                        MediaPhase::Sharing => self.text("正在共享", "Sharing"),
                        MediaPhase::Stopping => self.text("正在停止共享…", "Stopping sharing…"),
                        MediaPhase::Failed => snapshot.media_error.as_deref().unwrap_or(self.text("共享失败，请结束当前任务后重新选择来源。", "Sharing failed. End the task and choose a source again.")),
                        _ => self.text("媒体任务尚未结束。", "The media task is not finished."),
                    }};
                    let tone = if self.picker_error { StatusTone::Error }
                        else if self.capture_permission == CapturePermission::Denied || !ready { StatusTone::Warning }
                        else if self.picker_pending { StatusTone::Busy }
                        else if share_owned { media_tone(snapshot.media.phase()) }
                        else { StatusTone::Neutral };
                    notice(ui, tone, message);
                    }
                    self.s2_command_notice(ui);
                });
            });
        });
        ui.painter().line_segment(
            [actions.left_top(), actions.right_top()],
            egui::Stroke::new(1.0, p.border),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(actions)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                if share_owned {
                    let label = if stopping {
                        self.text("正在停止", "Stopping")
                    } else if snapshot.media.phase() == MediaPhase::PreparingShare {
                        self.text("取消共享", "Cancel sharing")
                    } else if snapshot.media.phase() == MediaPhase::Failed {
                        self.text("结束任务", "End task")
                    } else {
                        self.text("停止共享", "Stop sharing")
                    };
                    if (Action {
                        label,
                        size: vec2(156.0, 44.0),
                        enabled: !stopping,
                        tone: ActionTone::Danger,
                    })
                    .show(ui)
                    .clicked()
                    {
                        self.command_error = !self.runtime.stop_sharing();
                    }
                } else if (Action {
                    label: self.text("开始共享", "Start sharing"),
                    size: vec2(156.0, 44.0),
                    enabled: !self.picker_pending
                        && snapshot
                            .share_selection
                            .as_ref()
                            .is_some_and(|s| self.picker_kind.matches(s))
                        && share_action_available(self.capture_permission, snapshot),
                    tone: ActionTone::Primary,
                })
                .show(ui)
                .clicked()
                {
                    self.command_error = !self.runtime.start_sharing();
                }
                if !ready
                    && matches!(
                        snapshot.discovery.phase(),
                        DiscoveryPhase::Stopped | DiscoveryPhase::Failed
                    )
                    && (Action {
                        label: self.text("开启附近设备", "Turn on Nearby"),
                        size: vec2(160.0, 40.0),
                        enabled: !snapshot.nearby_stopping
                            && nearby_action_enabled(
                                self.nearby_action_pending,
                                snapshot.runtime.phase(),
                            ),
                        tone: ActionTone::Neutral,
                    })
                    .show(ui)
                    .clicked()
                {
                    self.request_turn_on_nearby(snapshot);
                }
            },
        );
    }

    fn present_source_picker(&mut self, context: &egui::Context) {
        self.picker_error = false;
        if self.capture_permission == CapturePermission::Allowed
            || capture_picker::request_permission()
        {
            self.capture_permission = CapturePermission::Allowed;
            self.picker_pending = true;
            let picker = self.capture_picker.get_or_insert_with(|| {
                let repaint = context.clone();
                capture_picker::Picker::new(move || repaint.request_repaint())
            });
            picker.present(self.picker_kind);
        } else {
            self.capture_permission = CapturePermission::Denied;
        }
    }

    fn s2_watch_empty(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let failed = watch_projection(snapshot.media_owner, snapshot.media.phase())
            == WatchProjection::Failed;
        let detail = if self.command_error {
            self.text(
                "操作未能发送，请稍后重试。",
                "The action could not be sent. Try again.",
            )
        } else if failed {
            snapshot.media_error.as_deref().unwrap_or(self.text(
                "停止当前任务后，可重新选择附近设备。",
                "End the current task, then select a nearby device again.",
            ))
        } else {
            self.text(
                "返回附近设备，选择有可观看内容的设备。",
                "Return to Nearby and choose a device with available content.",
            )
        };
        if moqcast_ui::v2::canvas_message(
            ui,
            moqcast_ui::v2::CanvasMessage {
                title: if failed {
                    self.text("无法继续播放", "Playback unavailable")
                } else {
                    self.text("播放已结束", "Playback ended")
                },
                detail,
                action: if failed {
                    self.text("停止并返回", "Stop and return")
                } else {
                    self.text("返回附近设备", "Return to Nearby")
                },
            },
        )
        .clicked()
        {
            if !failed || self.runtime.stop_watching() {
                self.page = Page::Nearby;
                self.command_error = false;
            } else {
                self.command_error = true;
            }
        }
    }

    fn s2_command_notice(&mut self, ui: &mut egui::Ui) {
        if self.command_error {
            notice(
                ui,
                StatusTone::Error,
                self.text(
                    "操作未能发送，请检查服务状态后重试。",
                    "The action could not be sent. Check service status before trying again.",
                ),
            );
            if ui.small_button(self.text("关闭提示", "Dismiss")).clicked() {
                self.command_error = false;
            }
        }
    }
}

// Project lifecycle facts once, so labels and colors cannot independently imply success.
fn discovery_status(
    snapshot: &AppSnapshot,
    pending: Option<NearbyAction>,
    locale: Locale,
) -> (StatusTone, &'static str) {
    if snapshot.runtime.phase() == RuntimePhase::Suspended {
        return (
            StatusTone::Neutral,
            text(locale, "睡眠期间已暂停", "Paused while the Mac sleeps"),
        );
    }
    if snapshot.runtime.phase() == RuntimePhase::Stopped {
        return (
            StatusTone::Error,
            text(locale, "应用服务已停止", "Application services stopped"),
        );
    }
    if snapshot.nearby_stopping {
        return (
            StatusTone::Busy,
            text(locale, "正在关闭附近设备", "Turning off Nearby"),
        );
    }
    if let Some(action) = pending {
        return (
            StatusTone::Busy,
            match action {
                NearbyAction::TurnOn => text(locale, "正在开启附近设备", "Turning on Nearby"),
                NearbyAction::TurnOff => text(locale, "正在关闭附近设备", "Turning off Nearby"),
            },
        );
    }
    match snapshot.discovery.phase() {
        DiscoveryPhase::Stopped => (
            StatusTone::Neutral,
            text(locale, "附近设备已关闭", "Nearby is off"),
        ),
        DiscoveryPhase::Failed => (
            StatusTone::Error,
            text(locale, "附近设备不可用", "Nearby is unavailable"),
        ),
        DiscoveryPhase::Starting => (
            StatusTone::Busy,
            text(locale, "正在开启附近设备", "Turning on Nearby"),
        ),
        _ => match snapshot.session.phase() {
            SessionPhase::Starting => (
                StatusTone::Busy,
                text(locale, "正在准备设备连接", "Preparing device connections"),
            ),
            SessionPhase::Failed | SessionPhase::Stopped => (
                StatusTone::Warning,
                text(
                    locale,
                    "正在查找，但设备连接服务不可用",
                    "Discovering; device connections unavailable",
                ),
            ),
            SessionPhase::Listening => (
                StatusTone::Success,
                text(locale, "正在查找附近设备", "Discovering nearby devices"),
            ),
        },
    }
}

fn media_tone(phase: MediaPhase) -> StatusTone {
    match phase {
        MediaPhase::Idle => StatusTone::Neutral,
        MediaPhase::PreparingWatch | MediaPhase::PreparingShare | MediaPhase::Stopping => {
            StatusTone::Busy
        }
        MediaPhase::Watching | MediaPhase::Sharing => StatusTone::Active,
        MediaPhase::Failed => StatusTone::Error,
    }
}

fn peer_status(
    peer: PeerPresentation,
    availability: ScreenAvailability,
    owned: Option<MediaPhase>,
    locale: Locale,
) -> (StatusTone, &'static str) {
    if let Some(phase) = owned {
        let label = match phase {
            MediaPhase::Watching => Some(text(locale, "正在观看", "Watching")),
            MediaPhase::PreparingWatch => Some(text(locale, "正在连接画面", "Opening stream")),
            MediaPhase::Stopping => Some(text(locale, "正在停止", "Stopping")),
            MediaPhase::Failed => Some(text(locale, "播放失败", "Playback failed")),
            _ => None,
        };
        if let Some(label) = label {
            return (media_tone(phase), label);
        }
    }
    // An mDNS loss must not hide a still-available route or healthy session.
    use moqcast_lan_state::Status;
    let tone = match peer
        .facts(availability == ScreenAvailability::Available)
        .status()
    {
        Status::Available | Status::Connected => StatusTone::Active,
        Status::Connecting => StatusTone::Busy,
        Status::Rejected | Status::Failed => StatusTone::Error,
        _ => StatusTone::Neutral,
    };
    (tone, device_status(peer, availability, locale))
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn pending_off_and_suspension_never_claim_healthy_discovery() {
        let mut snapshot = AppSnapshot::default();
        snapshot.runtime.begin(RuntimePhase::Ready);
        snapshot.discovery.begin(DiscoveryPhase::Scanning);
        snapshot.session.begin(SessionPhase::Listening);
        assert_eq!(
            discovery_status(&snapshot, Some(NearbyAction::TurnOff), Locale::English),
            (StatusTone::Busy, "Turning off Nearby")
        );
        snapshot.nearby_stopping = true;
        assert_eq!(
            discovery_status(&snapshot, None, Locale::English),
            (StatusTone::Busy, "Turning off Nearby")
        );
        snapshot.nearby_stopping = false;
        snapshot.runtime.begin(RuntimePhase::Suspended);
        assert_eq!(
            discovery_status(&snapshot, None, Locale::English),
            (StatusTone::Neutral, "Paused while the Mac sleeps")
        );
        snapshot.runtime.begin(RuntimePhase::Stopped);
        assert_eq!(
            discovery_status(&snapshot, None, Locale::English).0,
            StatusTone::Error
        );
        snapshot.runtime.begin(RuntimePhase::Ready);
        snapshot.discovery.begin(DiscoveryPhase::Failed);
        assert_eq!(
            discovery_status(&snapshot, None, Locale::English).0,
            StatusTone::Error
        );
        snapshot.discovery.begin(DiscoveryPhase::Stopped);
        assert_eq!(
            discovery_status(&snapshot, None, Locale::English).0,
            StatusTone::Neutral
        );
    }

    #[test]
    fn available_route_survives_discovery_loss_but_is_not_playback() {
        let peer = PeerPresentation {
            presence: PresenceView::NotNearby,
            connection: ConnectionView::Connected,
        };
        assert_eq!(
            peer_status(peer, ScreenAvailability::Available, None, Locale::English),
            (StatusTone::Active, "Available")
        );
        assert_eq!(
            peer_status(peer, ScreenAvailability::Unavailable, None, Locale::English),
            (StatusTone::Active, "Connected")
        );
    }

    #[test]
    fn watch_owner_reports_startup_and_failure_even_with_available_content() {
        let peer = PeerPresentation {
            presence: PresenceView::Nearby,
            connection: ConnectionView::Connected,
        };
        for (phase, tone, label) in [
            (
                MediaPhase::PreparingWatch,
                StatusTone::Busy,
                "Opening stream",
            ),
            (MediaPhase::Watching, StatusTone::Active, "Watching"),
            (MediaPhase::Failed, StatusTone::Error, "Playback failed"),
        ] {
            assert_eq!(
                peer_status(
                    peer,
                    ScreenAvailability::Available,
                    Some(phase),
                    Locale::English
                ),
                (tone, label)
            );
        }
    }
}

fn share_audio_selected(snapshot: &AppSnapshot, kind: capture_picker::Kind) -> bool {
    snapshot.share_system_audio
        && snapshot
            .share_selection
            .as_ref()
            .is_some_and(|s| kind.matches(s) && s.supports_system_audio())
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn sharing_disables_edits_without_hiding_selected_audio() {
        let mut snapshot = AppSnapshot {
            share_selection: Some(crate::publication::Selection::Display {
                display_id: 1,
                primary: true,
                label: "Main display".into(),
            }),
            share_system_audio: true,
            ..Default::default()
        };
        snapshot.media.begin(MediaPhase::Sharing);
        assert!(share_audio_selected(
            &snapshot,
            capture_picker::Kind::Display
        ));
        assert!(!system_audio_action_available(&snapshot));
        assert!(!share_audio_selected(
            &snapshot,
            capture_picker::Kind::Window
        ));
        snapshot.share_selection = Some(crate::publication::Selection::Window {
            window_id: 2,
            label: "Window".into(),
        });
        assert!(!share_audio_selected(
            &snapshot,
            capture_picker::Kind::Window
        ));
    }
}
