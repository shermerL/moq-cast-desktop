//! Visual shell; platform runtime remains the owner of all media and network work.
use super::*;
use egui::{Align, Layout, RichText, UiBuilder, vec2};
use moqcast_ui::v2::{self, Action, ActionTone, Metrics, Palette, StatusTone};

impl MoqCastApp {
    pub(super) fn desktop_shell(&mut self, root: &mut egui::Ui, snapshot: &AppSnapshot) {
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
                    Page::Watch => Some(
                        snapshot
                            .peers
                            .iter()
                            .find(|(id, _)| match &snapshot.media {
                                MediaState::PreparingView { path }
                                | MediaState::Viewing { path }
                                | MediaState::StoppingView { path } => {
                                    crate::screen_path::for_peer(id) == *path
                                }
                                _ => false,
                            })
                            .map(|(_, p)| p.name.clone())
                            .unwrap_or_else(|| self.text("播放器", "Player").to_owned()),
                    ),
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
                            let active = snapshot.discovery.is_active();
                            if (Action {
                                label: if snapshot.lan_stopping {
                                    self.text("正在关闭", "Turning off")
                                } else if active {
                                    self.text("关闭附近设备", "Turn off Nearby")
                                } else {
                                    self.text("开启附近设备", "Turn on Nearby")
                                },
                                size: vec2(148.0, 40.0),
                                enabled: !snapshot.lan_stopping,
                                tone: if active {
                                    ActionTone::Danger
                                } else {
                                    ActionTone::Accent
                                },
                            })
                            .show(ui)
                            .clicked()
                            {
                                if active && snapshot.media != MediaState::Idle {
                                    self.confirm_nearby_off = true;
                                } else {
                                    self.send(if active {
                                        UserCommand::StopDiscovery
                                    } else {
                                        if snapshot.discovery == DiscoveryState::Error {
                                            UserCommand::RetryDiscovery
                                        } else {
                                            UserCommand::StartDiscovery
                                        }
                                    });
                                }
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                let (tone, status) = discovery_status(snapshot, self.locale);
                                tone.indicator(ui);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(status).size(12.0).color(p.muted),
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
                    self.watch(ui, snapshot);
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
                                .show(ui, |ui| self.nearby(ui, snapshot, metrics));
                        } else {
                            self.nearby(ui, snapshot, metrics);
                        }
                    } else if let Some(command) = pages::screen_share::show(
                        ui,
                        self.locale,
                        snapshot,
                        &mut self.system_audio,
                        &mut self.selected_source,
                    ) {
                        self.send(command);
                    }
                });
            });
        if self.command_error.is_some() && self.page != Page::Nearby {
            egui::Area::new(egui::Id::new("command-error"))
                .anchor(egui::Align2::CENTER_TOP, vec2(0.0, metrics.header + 4.0))
                .show(root.ctx(), |ui| {
                    p.card().show(ui, |ui| {
                        v2::notice(ui, StatusTone::Error, self.locale.operation_failed());
                    });
                });
        }
        if self.settings_open {
            let result = egui::Modal::new(egui::Id::new("v2-settings"))
                .frame(p.dialog())
                .show(root.ctx(), |ui| {
                    ui.set_width((root.ctx().content_rect().width() - 96.0).clamp(240.0, 480.0));
                    let close = v2::dialog_header(
                        ui,
                        self.locale.settings(),
                        self.text("关闭设置", "Close settings"),
                    )
                    .clicked();
                    if let Some(locale) = pages::settings::show(
                        ui,
                        self.locale,
                        &mut self.theme_preference,
                        &mut self.developer_mode,
                        &mut self.diagnostics,
                    ) {
                        self.locale = locale;
                    }
                    close || self.diagnostics.visible()
                });
            if result.should_close() || result.inner {
                self.settings_open = false;
            }
        }
        if let Some(id) = self.selected_peer.clone() {
            let result = egui::Modal::new(egui::Id::new("v2-peer-details"))
                .frame(p.dialog())
                .show(root.ctx(), |ui| {
                    ui.set_width((root.ctx().content_rect().width() - 96.0).clamp(240.0, 412.0));
                    let close = v2::dialog_header(
                        ui,
                        self.locale.device_details(),
                        self.text("关闭设备详情", "Close device details"),
                    )
                    .clicked();
                    egui::ScrollArea::vertical()
                        .max_height((root.ctx().content_rect().height() - 220.0).max(80.0))
                        .show(ui, |ui| {
                            if let Some(peer) = snapshot.peers.get(&id) {
                                ui.label(&peer.name);
                                ui.label(RichText::new(&id).small().color(p.muted));
                                let (_, status) =
                                    pages::nearby::peer_status(snapshot, &id, peer, self.locale);
                                ui.label(status);
                                ui.label(match peer.discovery {
                                    PeerDiscoveryState::Found => self.locale.discovery_found(),
                                    PeerDiscoveryState::Lost => self.locale.discovery_lost(),
                                });
                            } else {
                                ui.label(self.text("设备已离开", "Device unavailable"));
                            }
                        });
                    close
                });
            if result.should_close() || result.inner {
                self.selected_peer = None;
            }
        }
        if self.confirm_nearby_off {
            let result = egui::Modal::new(egui::Id::new("v2-nearby-off"))
                .frame(p.dialog())
                .show(root.ctx(), |ui| {
                    ui.set_width((root.ctx().content_rect().width() - 96.0).clamp(240.0, 412.0));
                    let close = v2::dialog_header(
                        ui,
                        self.text("关闭附近设备？", "Turn off Nearby?"),
                        self.text("取消", "Cancel"),
                    )
                    .clicked();
                    ui.label(self.text(
                        "关闭会停止当前观看或共享，并断开附近连接。",
                        "Turning off Nearby stops the current watch or share and closes nearby connections.",
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
            if result.inner.1 {
                self.send(UserCommand::StopDiscovery);
            }
            if result.should_close() || result.inner.0 || result.inner.1 {
                self.confirm_nearby_off = false;
            }
        }
    }

    fn nearby(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot, metrics: Metrics) {
        let watching = matches!(
            snapshot.media,
            MediaState::PreparingView { .. }
                | MediaState::Viewing { .. }
                | MediaState::StoppingView { .. }
        );
        let (tone, status) = match snapshot.media {
            MediaState::Publishing => (StatusTone::Active, self.locale.sharing_screen()),
            MediaState::PreparingPublish => (StatusTone::Busy, self.locale.preparing_share()),
            MediaState::StoppingPublish => (StatusTone::Busy, self.locale.stopping_share()),
            MediaState::Viewing { .. } => (StatusTone::Active, self.locale.viewing_screen()),
            MediaState::PreparingView { .. } => (StatusTone::Busy, self.locale.preparing_view()),
            MediaState::StoppingView { .. } => (StatusTone::Busy, self.locale.stopping_view()),
            MediaState::Idle => (StatusTone::Neutral, self.locale.media_idle()),
        };
        let card = v2::ContextCard {
            heading: self.text("本机", "This computer"),
            name: &self.local_device_name,
            identity: snapshot.local_peer_id.as_deref().unwrap_or(""),
            status,
            tone,
            action: if watching {
                self.text("返回播放器", "Return to player")
            } else {
                self.text("共享设置", "Sharing settings")
            },
        };
        if v2::context_card(ui, card, metrics).clicked() {
            self.page = if watching {
                Page::Watch
            } else {
                Page::ScreenShare
            };
        }
        ui.add_space(metrics.gap);
        if self.command_error.is_some() || snapshot.last_error.is_some() {
            v2::notice(ui, StatusTone::Error, self.locale.operation_failed());
        }
        if let Some(action) = pages::nearby::show(ui, self.locale, snapshot, metrics.compact) {
            match action {
                pages::nearby::NearbyAction::Details(id) => self.selected_peer = Some(id),
                pages::nearby::NearbyAction::Watch(command) => {
                    self.page = Page::Watch;
                    self.send(command);
                }
                pages::nearby::NearbyAction::ReturnToPlayer => self.page = Page::Watch,
            }
        }
    }
}
fn discovery_status(snapshot: &AppSnapshot, locale: Locale) -> (StatusTone, &'static str) {
    if snapshot.lan_stopping {
        return (
            StatusTone::Busy,
            locale.text("正在关闭附近设备", "Turning off Nearby"),
        );
    }
    match snapshot.discovery {
        DiscoveryState::Idle => (
            StatusTone::Neutral,
            locale.text("附近设备已关闭", "Nearby is off"),
        ),
        DiscoveryState::Error => (StatusTone::Error, locale.discovery_error()),
        DiscoveryState::Scanning | DiscoveryState::Ready | DiscoveryState::Empty => (
            StatusTone::Success,
            locale.text(
                "附近设备已开启，正在自动查找",
                "Nearby is on. Discovering devices",
            ),
        ),
    }
}
