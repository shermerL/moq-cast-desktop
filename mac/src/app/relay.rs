//! Relay page composition; connection and playback remain runtime-owned.
use super::*;
use crate::relay::{Config, ConfigError, Endpoint, Phase, Selection};
use crate::runtime::MediaSource;
use moqcast_ui::v2::{Action, ActionTone, Palette, StatusTone, notice};

#[derive(Default)]
pub(super) struct Ui {
    address: String,
    pub(super) prefix: String,
    pub(super) publish_pending: Option<(Generation, u64)>,
    fingerprint: String,
    error: Option<ConfigError>,
    pending: Option<Generation>,
    pin_endpoint: Option<Endpoint>,
    pin_cleared: bool,
    suppress_connect_gesture: bool,
    confirm_disconnect: Option<Generation>,
    pub(super) watch_pending: Option<(u64, Selection)>,
}

impl Ui {
    // Compare only completed edits. Invalid intermediate text is never a new endpoint.
    fn finish_address_edit(&mut self) -> bool {
        if self.fingerprint.is_empty() {
            return false;
        }
        let Ok(config) = Config::parse(&self.address, "") else {
            return false;
        };
        let endpoint = config.endpoint();
        if self
            .pin_endpoint
            .as_ref()
            .is_some_and(|old| *old != endpoint)
        {
            self.fingerprint.clear();
            self.pin_endpoint = None;
            self.pin_cleared = true;
            self.error = None;
            return true;
        }
        self.pin_endpoint = Some(endpoint);
        false
    }

    fn affects_media(&self, snapshot: &AppSnapshot) -> bool {
        let generation = snapshot.relay.lifecycle.generation();
        snapshot.relay.lifecycle.phase() == Phase::Connected
            && ((snapshot.media_owner == Some(MediaOwner::Watch)
                && matches!(&snapshot.media_source, MediaSource::Relay(s) if s.generation == generation))
                || (snapshot.media_owner == Some(MediaOwner::Share)
                    && snapshot.media_source == MediaSource::RelayShare(generation))
                || self.publish_pending.is_some_and(|(g, _)| g == generation)
                || self
                    .watch_pending
                    .as_ref()
                    .is_some_and(|(_, s)| s.generation == generation))
    }

    pub(super) fn reconcile(&mut self, snapshot: &AppSnapshot) -> bool {
        if self.pending.is_some_and(|g| {
            g != snapshot.relay.lifecycle.generation()
                || snapshot.runtime.phase() != RuntimePhase::Ready
        }) {
            self.pending = None;
        }
        if self.publish_pending.is_some_and(|(g, n)| {
            g != snapshot.relay.lifecycle.generation() || n != snapshot.relay.publish_request
        }) {
            self.publish_pending = None;
        }
        let mut navigate = false;
        if let Some((request, selection)) = &self.watch_pending
            && (*request != snapshot.relay.watch_request
                || selection.generation != snapshot.relay.lifecycle.generation())
        {
            let accepted = *request != snapshot.relay.watch_request
                && selection.generation == snapshot.relay.lifecycle.generation()
                && snapshot.media_owner == Some(MediaOwner::Watch)
                && snapshot.media_source == MediaSource::Relay(selection.clone());
            self.watch_pending = None;
            navigate = accepted && self.confirm_disconnect.is_none();
        }
        if self.confirm_disconnect != Some(snapshot.relay.lifecycle.generation())
            || !self.affects_media(snapshot)
        {
            self.confirm_disconnect = None;
        }
        navigate
    }
}

fn source_badge(ui: &mut egui::Ui, label: &str) {
    let p = Palette::for_ui(ui);
    StatusTone::Active.indicator(ui);
    egui::Frame::new()
        .fill(p.soft)
        .corner_radius(8)
        .inner_margin(5)
        .show(ui, |ui| ui.colored_label(p.accent, label));
}

impl MoqCastApp {
    pub(super) fn watch_return_page(&self, snapshot: &AppSnapshot) -> Page {
        if self.page == Page::RelayShare
            || (self.page == Page::Watch && matches!(snapshot.media_source, MediaSource::Relay(_)))
        {
            Page::Relay
        } else {
            Page::Nearby
        }
    }

    pub(super) fn relay_navigation(
        &mut self,
        root: &mut egui::Ui,
        snapshot: &AppSnapshot,
        metrics: moqcast_ui::v2::Metrics,
    ) {
        if matches!(self.page, Page::Watch | Page::RelayShare) {
            return;
        }
        let palette = Palette::for_ui(root);
        egui::Panel::top("relay-navigation")
            .frame(
                Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(egui::Margin::symmetric(metrics.inset, 4)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    for (page, label) in [
                        (Page::Nearby, self.text("附近设备", "Nearby")),
                        (Page::Relay, "Relay"),
                    ] {
                        if (Action {
                            label,
                            size: egui::vec2(128.0, 40.0),
                            enabled: true,
                            tone: if self.page == page {
                                ActionTone::Primary
                            } else {
                                ActionTone::Neutral
                            },
                        })
                        .show(ui)
                        .clicked()
                        {
                            self.page = page;
                        }
                    }
                });
                if snapshot.media_owner.is_some() {
                    ui.horizontal(|ui| {
                        let relay = snapshot.relay_media();
                        let source = if relay {
                            "Relay"
                        } else {
                            self.text("附近设备", "Nearby")
                        };
                        source_badge(ui, source);
                        let label_width =
                            (ui.available_width() - 176.0 - 2.0 * ui.spacing().item_spacing.x)
                                .max(0.0);
                        ui.add_sized(
                            egui::vec2(label_width, 40.0),
                            egui::Label::new(
                                snapshot
                                    .media_path
                                    .as_deref()
                                    .unwrap_or(self.text("屏幕共享", "Screen share")),
                            )
                            .halign(egui::Align::Min)
                            .truncate(),
                        );
                        if (Action {
                            label: self.text("返回", "Return"),
                            size: egui::vec2(88.0, 40.0),
                            enabled: true,
                            tone: ActionTone::Neutral,
                        })
                        .show(ui)
                        .clicked()
                        {
                            self.page = if snapshot.media_owner == Some(MediaOwner::Watch) {
                                Page::Watch
                            } else if relay {
                                Page::RelayShare
                            } else {
                                Page::ScreenShare
                            };
                        }
                        if (Action {
                            label: self.text("停止", "Stop"),
                            size: egui::vec2(88.0, 40.0),
                            enabled: snapshot.media.phase() != MediaPhase::Stopping,
                            tone: ActionTone::Danger,
                        })
                        .show(ui)
                        .clicked()
                        {
                            self.command_error = !if snapshot.media_owner == Some(MediaOwner::Watch)
                            {
                                self.runtime.stop_watching(snapshot.media.generation())
                            } else {
                                self.runtime.stop_sharing(snapshot.media.generation())
                            };
                        }
                    });
                }
            });
    }

    pub(super) fn relay_page(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let connected = matches!(
            snapshot.relay.lifecycle.phase(),
            Phase::Connected | Phase::Disconnecting
        );
        let width = ui
            .available_width()
            .min(if connected { 1100.0 } else { 700.0 });
        ui.vertical_centered(|ui| {
            ui.set_max_width(width);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                if connected {
                    self.s2_command_notice(ui);
                    Palette::for_ui(ui).card().inner_margin(16).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                self.connection_action(ui, snapshot, false);
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.colored_label(
                                        Palette::for_ui(ui).accent,
                                        if snapshot.relay.lifecycle.phase() == Phase::Disconnecting
                                        {
                                            self.text("正在断开", "Disconnecting")
                                        } else {
                                            self.text("已连接", "Connected")
                                        },
                                    );
                                    ui.add(egui::Label::new(&snapshot.relay.server).truncate())
                                        .on_hover_text(&snapshot.relay.server);
                                });
                            });
                        });
                    });
                    if let Some(error) = &snapshot.relay.error {
                        notice(ui, StatusTone::Error, error);
                    }
                    if snapshot.relay.lifecycle.phase() == Phase::Connected {
                        ui.add_space(12.0);
                        let owns = snapshot.media_owner == Some(MediaOwner::Share)
                            && snapshot.relay_media();
                        if moqcast_ui::v2::context_card_with_action(
                            ui,
                            moqcast_ui::v2::ContextCard {
                                heading: self.text("本机", "This device"),
                                name: snapshot
                                    .local_device_name
                                    .as_deref()
                                    .unwrap_or(self.text("这台 Mac", "This Mac")),
                                identity: "Relay",
                                status: match snapshot.media.phase() {
                                    MediaPhase::Idle => self.text("空闲", "Idle"),
                                    MediaPhase::PreparingShare | MediaPhase::PreparingWatch => {
                                        self.text("准备中", "Preparing")
                                    }
                                    MediaPhase::Sharing => self.text("正在共享", "Sharing"),
                                    MediaPhase::Watching => self.text("正在观看", "Watching"),
                                    MediaPhase::Stopping => self.text("正在停止", "Stopping"),
                                    MediaPhase::Failed => {
                                        self.text("媒体任务需要处理", "Media needs attention")
                                    }
                                },
                                tone: super::desktop::media_tone(snapshot.media.phase()),
                                action: if owns {
                                    self.text("管理共享", "Manage sharing")
                                } else {
                                    self.text("共享到 Relay…", "Share to Relay…")
                                },
                            },
                            moqcast_ui::v2::Metrics::for_width(ui.available_width()),
                            owns || (snapshot.media.phase() == MediaPhase::Idle
                                && self.relay_ui.watch_pending.is_none()
                                && self.relay_ui.publish_pending.is_none()
                                && !self.picker_pending),
                        )
                        .clicked()
                        {
                            self.page = Page::RelayShare;
                        }
                        ui.add_space(12.0);
                        Palette::for_ui(ui).card().inner_margin(20).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            self.directory(ui, snapshot);
                        });
                    }
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("relay-connection-form")
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            Palette::for_ui(ui).card().inner_margin(24).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.heading(self.text("连接 Relay", "Connect to Relay"));
                                self.s2_command_notice(ui);
                                self.connection_form(ui, snapshot);
                                if let Some(error) = &snapshot.relay.error {
                                    notice(ui, StatusTone::Error, error);
                                }
                            });
                        });
                }
                self.disconnect_confirmation(ui, snapshot);
            });
        });
    }

    fn connection_form(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let editable = matches!(
            snapshot.relay.lifecycle.phase(),
            Phase::Disconnected | Phase::Failed
        ) && self.relay_ui.pending.is_none();
        let error = self.relay_ui.error.map(|error| match error {
            ConfigError::Url => self.text("请输入有效的服务器地址", "Enter a valid server URL"),
            ConfigError::Scheme => self.text(
                "使用 https:// 或 moqt:// 地址，不包含用户名或 # 片段",
                "Use https:// or moqt:// without user information or a fragment",
            ),
            ConfigError::Fingerprint => self.text(
                "证书指纹须为 SHA-256 十六进制值",
                "Certificate pin must be a hex-encoded SHA-256 fingerprint",
            ),
        });
        let address_label = self.text("服务器地址", "Server URL");
        let address_error = if self.relay_ui.error == Some(ConfigError::Fingerprint) {
            None
        } else {
            error
        };
        let address = (moqcast_ui::v2::TextField {
            id: egui::Id::new("relay-address"),
            label: address_label,
            hint: "moqt://relay.example/",
            enabled: editable,
            error: address_error,
        })
        .show(ui, &mut self.relay_ui.address);
        if address.changed() {
            self.relay_ui.error = None;
        }
        if editable && address.lost_focus() && self.relay_ui.finish_address_edit() {
            self.relay_ui.suppress_connect_gesture = true;
        }
        let pin_set = !self.relay_ui.fingerprint.trim().is_empty()
            && moq_tokio::tls::parse_fingerprint(self.relay_ui.fingerprint.trim()).is_ok();
        let title = if pin_set {
            self.text("证书校验 · 已设置", "Certificate verification · Set")
        } else {
            self.text("证书校验", "Certificate verification")
        };
        ui.add_enabled_ui(editable, |ui| {
            moqcast_ui::v2::disclosure(
                ui,
                egui::Id::new("relay-pin-section"),
                title,
                self.relay_ui.error == Some(ConfigError::Fingerprint),
                |ui| {
                    let label = self.text(
                        "SHA-256 证书指纹（留空使用系统信任）",
                        "SHA-256 certificate pin (empty for system trust)",
                    );
                    if (moqcast_ui::v2::TextField {
                        id: egui::Id::new("relay-fingerprint"),
                        label,
                        hint: "",
                        enabled: true,
                        error: if self.relay_ui.error == Some(ConfigError::Fingerprint) {
                            error
                        } else {
                            None
                        },
                    })
                    .show(ui, &mut self.relay_ui.fingerprint)
                    .changed()
                    {
                        self.relay_ui.error = None;
                        self.relay_ui.pin_cleared = false;
                        self.relay_ui.pin_endpoint = Config::parse(&self.relay_ui.address, "")
                            .ok()
                            .map(|c| c.endpoint());
                    }
                    if !self.relay_ui.fingerprint.is_empty()
                        && (Action {
                            label: self.text("清空指纹", "Clear pin"),
                            size: egui::vec2(118.0, 40.0),
                            enabled: editable,
                            tone: ActionTone::Neutral,
                        })
                        .show(ui)
                        .clicked()
                    {
                        self.relay_ui.fingerprint.clear();
                        self.relay_ui.pin_endpoint = None;
                        self.relay_ui.error = None;
                        self.relay_ui.pin_cleared = false;
                    }
                },
            );
        });
        if self.relay_ui.pin_cleared {
            notice(ui, StatusTone::Warning, self.text("服务器已更改，旧指纹已清除。请确认新服务器的证书设置后再连接。", "Server changed. The previous pin was cleared. Check certificate settings before connecting."));
        }
        self.connection_action(ui, snapshot, editable);
        if !ui.input(|i| i.pointer.any_down()) {
            self.relay_ui.suppress_connect_gesture = false;
        }
        if snapshot.relay.lifecycle.phase() == Phase::Connecting {
            notice(ui, StatusTone::Busy, self.text("正在连接", "Connecting"));
        }
    }

    fn connection_action(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot, editable: bool) {
        let phase = snapshot.relay.lifecycle.phase();
        let label = match phase {
            Phase::Connecting => self.text("取消连接", "Cancel connection"),
            Phase::Connected => self.text("断开 Relay", "Disconnect Relay"),
            Phase::Disconnecting => self.text("正在断开", "Disconnecting"),
            Phase::Disconnected | Phase::Failed => self.text("连接", "Connect"),
        };
        if (Action {
            label,
            size: egui::vec2(180.0, 44.0),
            enabled: phase != Phase::Disconnecting
                && self.relay_ui.pending.is_none()
                && snapshot.runtime.phase() == RuntimePhase::Ready,
            tone: if phase == Phase::Connected {
                ActionTone::Danger
            } else {
                ActionTone::Primary
            },
        })
        .show(ui)
        .clicked()
        {
            if editable {
                if self.relay_ui.suppress_connect_gesture || self.relay_ui.finish_address_edit() {
                    return;
                }
                match Config::parse(&self.relay_ui.address, &self.relay_ui.fingerprint) {
                    Ok(config) => {
                        self.command_error = !self.runtime.connect_relay(config);
                        if !self.command_error {
                            self.relay_ui.pin_cleared = false;
                            self.relay_ui.pending = Some(snapshot.relay.lifecycle.generation());
                            self.relay_ui.error = None;
                        }
                    }
                    Err(error) => {
                        self.relay_ui.error = Some(error);
                        if error == ConfigError::Fingerprint {
                            ui.memory_mut(|memory| {
                                memory.request_focus(egui::Id::new("relay-fingerprint"))
                            });
                        }
                    }
                }
            } else if self.relay_ui.affects_media(snapshot) {
                self.relay_ui.confirm_disconnect = Some(snapshot.relay.lifecycle.generation());
            } else {
                self.disconnect_relay(snapshot);
            }
        }
    }

    fn disconnect_confirmation(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        if self.relay_ui.confirm_disconnect == Some(snapshot.relay.lifecycle.generation())
            && self.relay_ui.affects_media(snapshot)
        {
            let response = egui::Modal::new(egui::Id::new("disconnect-relay"))
                .frame(Palette::for_ui(ui).dialog())
                .show(ui.ctx(), |ui| {
                    if moqcast_ui::v2::dialog_header(
                        ui,
                        self.text("断开 Relay？", "Disconnect Relay?"),
                        self.text("取消", "Cancel"),
                    )
                    .clicked()
                    {
                        self.relay_ui.confirm_disconnect = None;
                    }
                    let sharing = matches!(snapshot.media_source, MediaSource::RelayShare(_))
                        || self.relay_ui.publish_pending.is_some();
                    ui.label(if sharing {
                        self.text(
                            "将停止当前共享并从 Relay 撤回广播。",
                            "Sharing will stop and the broadcast will be withdrawn.",
                        )
                    } else {
                        self.text(
                            "这会停止当前 Relay 观看。附近设备设置保持不变。",
                            "This stops Relay playback. Your Nearby setting stays the same.",
                        )
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if (Action {
                            label: if sharing {
                                self.text("停止并断开", "Stop and disconnect")
                            } else {
                                self.text("断开 Relay", "Disconnect Relay")
                            },
                            size: egui::vec2(180.0, 44.0),
                            enabled: true,
                            tone: ActionTone::Danger,
                        })
                        .show(ui)
                        .clicked()
                        {
                            self.relay_ui.confirm_disconnect = None;
                            self.disconnect_relay(snapshot);
                        }
                        if (Action {
                            label: self.text("取消", "Cancel"),
                            size: egui::vec2(96.0, 44.0),
                            enabled: true,
                            tone: ActionTone::Neutral,
                        })
                        .show(ui)
                        .clicked()
                        {
                            self.relay_ui.confirm_disconnect = None;
                        }
                    });
                });
            if response.should_close() {
                self.relay_ui.confirm_disconnect = None;
            }
        }
    }

    fn directory(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        ui.label(self.text("共享列表", "Shared broadcasts"));
        if snapshot.relay.entries.is_empty() {
            // The pinned SDK has no directory-complete signal. Silence is not an empty or refused directory.
            ui.label(self.text("当前未发现共享", "No broadcasts found at the moment"));
        }
        egui::ScrollArea::vertical()
            .id_salt("relay-broadcasts")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (path, revision) in &snapshot.relay.entries {
                    let selection = Selection {
                        generation: snapshot.relay.lifecycle.generation(),
                        path: path.clone(),
                        revision: *revision,
                    };
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let current = snapshot.media_owner == Some(MediaOwner::Watch)
                                && snapshot.media_source == MediaSource::Relay(selection.clone());
                            if (Action {
                                label: if current {
                                    self.text("返回播放", "Return to player")
                                } else {
                                    self.text("观看", "Watch")
                                },
                                size: egui::vec2(144.0, 40.0),
                                enabled: current
                                    || (self.relay_ui.pending.is_none()
                                        && self.relay_ui.watch_pending.is_none()
                                        && self.relay_ui.publish_pending.is_none()
                                        && !self.picker_pending
                                        && snapshot.can_watch_relay(&selection)),
                                tone: ActionTone::Primary,
                            })
                            .show(ui)
                            .clicked()
                            {
                                if current {
                                    self.page = Page::Watch;
                                } else {
                                    self.command_error =
                                        !self.runtime.watch_relay(selection.clone());
                                    if !self.command_error {
                                        self.relay_ui.watch_pending =
                                            Some((snapshot.relay.watch_request, selection));
                                    }
                                }
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(egui::Label::new(path).truncate())
                                    .on_hover_text(path);
                            });
                        });
                    });
                }
            });
    }

    fn disconnect_relay(&mut self, snapshot: &AppSnapshot) {
        self.command_error = !self
            .runtime
            .disconnect_relay(snapshot.relay.lifecycle.generation());
        if !self.command_error {
            self.relay_ui.pending = Some(snapshot.relay.lifecycle.generation());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_follows_tls_endpoint_across_completed_edits() {
        for changed in [
            "https://example.com/a",
            "moqt://peer.example/b",
            "moqt://example.com:4443/a",
        ] {
            let mut ui = Ui {
                address: "moqt://EXAMPLE.com/a?token=one".into(),
                fingerprint: "ab".repeat(32),
                ..Default::default()
            };
            assert!(!ui.finish_address_edit());
            ui.address = "moqt://".into();
            assert!(!ui.finish_address_edit());
            ui.address = "moqt://example.com:443/b?token=two".into();
            assert!(!ui.finish_address_edit());
            assert_eq!(ui.fingerprint.len(), 64);
            ui.address = changed.into();
            assert!(ui.finish_address_edit());
            assert!(ui.fingerprint.is_empty() && ui.pin_cleared);
            assert!(!ui.finish_address_edit());
        }
    }

    #[test]
    fn disconnect_confirmation_covers_queued_watch_and_expires_on_replacement() {
        let mut snapshot = AppSnapshot::default();
        let generation = snapshot.relay.lifecycle.begin(Phase::Connected);
        let selection = Selection {
            generation,
            path: "sample".into(),
            revision: 1,
        };
        let mut ui = Ui {
            watch_pending: Some((0, selection.clone())),
            ..Default::default()
        };
        assert!(ui.affects_media(&snapshot));
        ui.confirm_disconnect = Some(generation);
        snapshot.media_source = MediaSource::Relay(selection);
        snapshot.media_owner = Some(MediaOwner::Watch);
        snapshot.relay.watch_request = 1;
        assert!(!ui.reconcile(&snapshot));
        assert_eq!(ui.confirm_disconnect, Some(generation));
        snapshot.relay.lifecycle.begin(Phase::Connected);
        assert!(!ui.reconcile(&snapshot));
        assert!(ui.confirm_disconnect.is_none());
        snapshot.media_source = MediaSource::Nearby;
        assert!(!ui.affects_media(&snapshot));
    }

    #[test]
    fn refused_connection_can_be_retried_without_editing_the_form() {
        let (mut app, mut received) = crate::app::tests::layout::app();
        let mut snapshot = AppSnapshot::default();
        snapshot.runtime.begin(RuntimePhase::Ready);
        app.relay_ui.address = "moqt://localhost/".into();
        while app.runtime.stop_watching(snapshot.media.generation()) {}
        let ctx = egui::Context::default();
        let render = |app: &mut MoqCastApp, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default()
                        .show(root, |ui| app.connection_action(ui, &snapshot, true));
                },
            );
            output.textures_delta.clear();
            output
        };
        let output = render(&mut app, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == "Connect" => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .unwrap();
        for accepted in [false, true] {
            for pressed in [true, false] {
                let _ = render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            assert_eq!(app.command_error, !accepted);
            assert_eq!(app.relay_ui.pending.is_some(), accepted);
            assert!(received());
            while received() {}
        }
    }

    #[test]
    fn invalid_pin_is_visible_when_certificate_section_was_collapsed() {
        let (mut app, _commands) = crate::app::tests::layout::app();
        app.relay_ui.address = "moqt://localhost/".into();
        app.relay_ui.fingerprint = "invalid".into();
        app.relay_ui.error = Config::parse(&app.relay_ui.address, &app.relay_ui.fingerprint).err();
        let ctx = egui::Context::default();
        moqcast_ui::v2::install(&ctx);
        let snapshot = AppSnapshot::default();
        let mut visible = false;
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |root| {
                egui::CentralPanel::default().show(root, |ui| app.relay_page(ui, &snapshot));
            });
            visible = output.shapes.iter().any(|s| {
                matches!(&s.shape, egui::epaint::Shape::Text(t)
                if t.galley.job.text.contains("Certificate pin must"))
            });
            output.textures_delta.clear();
        }
        assert!(
            visible,
            "pin validation must not disappear inside a collapsed section"
        );
    }

    #[test]
    fn watch_navigation_waits_for_acceptance_and_releases_rejected_click() {
        let mut snapshot = AppSnapshot::default();
        let generation = snapshot.relay.lifecycle.begin(Phase::Connected);
        let selection = Selection {
            generation,
            path: "test/one".into(),
            revision: 1,
        };
        let mut ui = Ui {
            watch_pending: Some((0, selection.clone())),
            ..Default::default()
        };
        assert!(!ui.reconcile(&snapshot));
        assert!(ui.watch_pending.is_some());
        snapshot.relay.watch_request = 1;
        snapshot.relay.error = Some("withdrawn".into());
        assert!(!ui.reconcile(&snapshot));
        assert!(ui.watch_pending.is_none());
        ui.watch_pending = Some((1, selection.clone()));
        snapshot.relay.watch_request = 2;
        snapshot.media_owner = Some(MediaOwner::Watch);
        snapshot.media_source = MediaSource::Relay(selection);
        assert!(ui.reconcile(&snapshot));
        assert!(ui.watch_pending.is_none());
        let MediaSource::Relay(selection) = snapshot.media_source.clone() else {
            unreachable!()
        };
        ui.watch_pending = Some((snapshot.relay.watch_request, selection));
        snapshot.relay.lifecycle.begin(Phase::Disconnected);
        assert!(!ui.reconcile(&snapshot));
        assert!(ui.watch_pending.is_none());
    }
}

#[cfg(test)]
mod publish_confirmation_tests {
    use super::*;

    #[test]
    fn queued_and_running_publication_require_confirmation_only_for_their_connection() {
        let mut state = Ui::default();
        let mut s = AppSnapshot::default();
        let connection = s.relay.lifecycle.begin(Phase::Connected);
        state.publish_pending = Some((connection, s.relay.publish_request));
        assert!(state.affects_media(&s));
        state.confirm_disconnect = Some(connection);
        s.relay.publish_request += 1;
        s.media_source = MediaSource::RelayShare(connection);
        s.media_owner = Some(MediaOwner::Share);
        state.reconcile(&s);
        assert!(state.publish_pending.is_none());
        assert_eq!(state.confirm_disconnect, Some(connection));
        s.relay.lifecycle.begin(Phase::Connected);
        state.reconcile(&s);
        assert!(state.confirm_disconnect.is_none());
        assert!(!state.affects_media(&s));
    }
}
