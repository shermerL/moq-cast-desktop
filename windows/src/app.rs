//! Compact native UI for discovery and direct peer sessions.

use eframe::egui::{self, Align, Color32, Frame, Layout, RichText, Stroke};

use crate::runtime::{
    DiscoveryState, PeerView, RuntimeCommand, RuntimeOwner, RuntimeSnapshot, TransportPhaseView,
};

const CONTENT_MAX_WIDTH: f32 = 900.0;

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
    runtime: RuntimeOwner,
    snapshot: RuntimeSnapshot,
    command_error: Option<String>,
}

impl MoqCastApp {
    pub(crate) fn new(context: &eframe::CreationContext<'_>, mut runtime: RuntimeOwner) -> Self {
        context.egui_ctx.set_visuals(egui::Visuals::light());
        let snapshot = runtime.snapshot();
        Self {
            page: Page::Nearby,
            locale: Locale::Chinese,
            runtime,
            snapshot,
            command_error: None,
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
                        if matches!(
                            peer.transport.phase,
                            TransportPhaseView::Connecting | TransportPhaseView::Connected
                        ) {
                            if ui
                                .button(match self.locale {
                                    Locale::Chinese => "断开",
                                    Locale::English => "Disconnect",
                                })
                                .clicked()
                            {
                                self.send(RuntimeCommand::Disconnect(peer.id.clone()));
                            }
                        } else {
                            let enabled = peer.should_dial;
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(match self.locale {
                                        Locale::Chinese => "连接",
                                        Locale::English => "Connect",
                                    }),
                                )
                                .clicked()
                            {
                                self.send(RuntimeCommand::Connect(peer.id.clone()));
                            }
                        }
                    });
                });
            });
    }

    fn screen_share(&self, ui: &mut egui::Ui) {
        ui.heading(self.locale.screen_share());
        ui.label(match self.locale {
            Locale::Chinese => "当前版本尚不可用。Windows 屏幕采集与 H.264 发布将在后续版本提供。",
            Locale::English => {
                "Unavailable in this build. Windows capture and H.264 publishing will arrive in a later build."
            }
        });
        ui.add_space(8.0);
        ui.small(match self.locale {
            Locale::Chinese => "当前应用不会请求屏幕权限，也不会伪装正在共享。",
            Locale::English => {
                "The app does not request capture permission or present a false sharing state."
            }
        });
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
            });
        if let Some(error) = self.snapshot.last_error {
            ui.add_space(10.0);
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
}

impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.snapshot = self.runtime.snapshot();
        let context = ui.ctx().clone();
        context.request_repaint_after(std::time::Duration::from_millis(100));

        egui::Panel::top("navigation")
            .exact_size(52.0)
            .show(ui, |ui| {
                ui.add_space(7.0);
                self.top_bar(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            let available = ui.available_width();
            let width = available.min(CONTENT_MAX_WIDTH);
            ui.horizontal(|ui| {
                ui.add_space(((available - width) / 2.0).max(0.0));
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(18.0);
                    if let Some(error) = &self.command_error {
                        ui.colored_label(Color32::LIGHT_RED, error);
                        ui.add_space(6.0);
                    }
                    match self.page {
                        Page::Nearby => self.nearby(ui),
                        Page::ScreenShare => self.screen_share(ui),
                        Page::Settings => self.settings(ui),
                    }
                });
            });
        });
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
    fn routing_and_language_are_independent() {
        let page = Page::Settings;
        let locale = Locale::English;
        assert_eq!(page, Page::Settings);
        assert_eq!(locale.settings(), "Settings");
        assert_ne!(Page::Nearby, Page::ScreenShare);
    }
}
