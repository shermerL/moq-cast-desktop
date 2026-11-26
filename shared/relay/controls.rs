//! Native Relay form and directory for desktop supervisors.
use super::{
    Command, Config, ConfigError, Endpoint, Generation, MediaRoute, Phase, PublishPrefix,
    Selection, Snapshot,
};
use eframe::egui;
use moqcast_ui::v2::{self, Action, ActionTone, Palette, StatusTone, notice};
pub(crate) enum ActionRequest {
    Command(Command),
    Share,
    Player,
}
#[derive(Default)]
pub(crate) struct Ui {
    address: String,
    pub(crate) prefix: String,
    pub(crate) publish_pending: Option<(Generation, u64)>,
    fingerprint: String,
    error: Option<ConfigError>,
    pending: Option<Generation>,
    pin_endpoint: Option<Endpoint>,
    pin_cleared: bool,
    suppress_connect_gesture: bool,
    confirm_disconnect: Option<Generation>,
    pub(crate) watch_pending: Option<(u64, Selection)>,
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

    pub(crate) fn queued(&mut self, command: &Command, relay: &Snapshot) {
        match command {
            Command::Connect(_) | Command::Disconnect(_) => {
                self.pending = Some(relay.lifecycle.generation())
            }
            Command::Watch(selection) => {
                self.watch_pending = Some((relay.watch_request, selection.clone()))
            }
            _ => {}
        }
    }

    pub(crate) fn reconcile(&mut self, relay: &Snapshot, route: &MediaRoute, active: bool) -> bool {
        let generation = relay.lifecycle.generation();
        if self.pending.is_some_and(|g| g != generation) {
            self.pending = None;
        }
        if self.confirm_disconnect.is_some_and(|g| g != generation) {
            self.confirm_disconnect = None;
        }
        if self
            .publish_pending
            .is_some_and(|(g, n)| g != generation || n != relay.publish_request)
        {
            self.publish_pending = None;
        }
        let mut navigate = false;
        if let Some((n, selection)) = &self.watch_pending
            && (selection.generation != generation || *n != relay.watch_request)
        {
            navigate = matches!(route, MediaRoute::Watching(s, _) if s == selection)
                && active
                && self.confirm_disconnect.is_none();
            self.watch_pending = None;
        }
        if !(active && route.is_relay())
            && self.watch_pending.is_none()
            && self.publish_pending.is_none()
        {
            self.confirm_disconnect = None;
        }
        navigate
    }
    pub(crate) fn prefix_input(
        &mut self,
        ui: &mut egui::Ui,
        chinese: bool,
        editable: bool,
    ) -> Option<PublishPrefix> {
        let text = |zh, en| if chinese { zh } else { en };
        let invalid = !self.prefix.is_empty() && PublishPrefix::parse(&self.prefix).is_none();
        (v2::TextField {
            id: egui::Id::new("relay-prefix"),
            label: text("发布前缀", "Publication prefix"),
            hint: "screen",
            enabled: editable,
            error: invalid.then(|| text("请输入有效的相对前缀", "Enter a valid relative prefix")),
        })
        .show(ui, &mut self.prefix);
        PublishPrefix::parse(&self.prefix)
    }

    pub(crate) fn page(
        &mut self,
        ui: &mut egui::Ui,
        chinese: bool,
        relay: &Snapshot,
        route: &MediaRoute,
        active: bool,
    ) -> Option<ActionRequest> {
        let text = |zh, en| if chinese { zh } else { en };
        let mut result = None;
        let generation = relay.lifecycle.generation();
        let palette = Palette::for_ui(ui);
        let phase = relay.lifecycle.phase();
        if phase != Phase::Connected {
            palette.card().inner_margin(18).show(ui, |ui| {
                ui.heading(text("连接 Relay", "Connect to Relay"));
                let editable =
                    matches!(phase, Phase::Disconnected | Phase::Failed) && self.pending.is_none();
                self.connection_fields(ui, chinese, editable);
                let label = if phase == Phase::Connecting {
                    text("取消连接", "Cancel connection")
                } else {
                    text("连接", "Connect")
                };
                if v2::action(
                    ui,
                    label,
                    egui::vec2(160.0, 40.0),
                    editable || phase == Phase::Connecting && self.pending.is_none(),
                )
                .clicked()
                {
                    if phase == Phase::Connecting {
                        result = Some(ActionRequest::Command(Command::Disconnect(generation)));
                    } else if !self.suppress_connect_gesture && !self.finish_address_edit() {
                        match Config::parse(&self.address, &self.fingerprint) {
                            Ok(config) => {
                                result = Some(ActionRequest::Command(Command::Connect(config)))
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                }
                if !ui.input(|i| i.pointer.any_down()) {
                    self.suppress_connect_gesture = false;
                }
                if let Some(e) = &relay.error {
                    notice(ui, StatusTone::Error, e);
                }
            });
            return result;
        }
        palette.card().inner_margin(16).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(&relay.server);
                if let Some(version) = &relay.version {
                    ui.label(version);
                }
                if ui
                    .add_enabled(
                        self.pending.is_none(),
                        egui::Button::new(text("断开 Relay", "Disconnect Relay")),
                    )
                    .clicked()
                {
                    if active && route.is_relay()
                        || self.watch_pending.is_some()
                        || self.publish_pending.is_some()
                    {
                        self.confirm_disconnect = Some(generation);
                    } else {
                        result = Some(ActionRequest::Command(Command::Disconnect(generation)));
                    }
                }
            });
        });
        let available = self.pending.is_none()
            && !active
            && self.publish_pending.is_none()
            && self.watch_pending.is_none();
        if ui
            .add_enabled(
                available || (active && matches!(route, MediaRoute::Publishing { .. })),
                egui::Button::new(text("共享到 Relay…", "Share to Relay…")),
            )
            .clicked()
        {
            result = Some(ActionRequest::Share);
        }
        ui.heading(text("共享目录", "Shared broadcasts"));
        if relay.entries.is_empty() {
            ui.label(text("当前未发现共享", "No broadcasts found"));
        }
        egui::ScrollArea::vertical()
            .id_salt("relay-directory")
            .show(ui, |ui| {
                for (path, revision) in &relay.entries {
                    ui.horizontal(|ui| {
                        let selected = Selection {
                            generation,
                            path: path.clone(),
                            revision: *revision,
                        };
                        let current =
                            active && matches!(route, MediaRoute::Watching(s, _) if s == &selected);
                        let label = if current {
                            text("返回播放器", "Return to player")
                        } else {
                            text("观看", "Watch")
                        };
                        if ui
                            .add_enabled(available || current, egui::Button::new(label))
                            .clicked()
                        {
                            result = Some(if current {
                                ActionRequest::Player
                            } else {
                                ActionRequest::Command(Command::Watch(selected))
                            });
                        }
                        ui.add(egui::Label::new(path.as_str()).truncate());
                    });
                }
            });
        if self.confirm_disconnect == Some(generation) {
            egui::Modal::new("relay-disconnect".into()).show(ui.ctx(), |ui| {
                ui.label(text(
                    "断开会结束本场 Relay 媒体。",
                    "Disconnecting will stop this Relay session.",
                ));
                if ui.button(text("取消", "Cancel")).clicked() {
                    self.confirm_disconnect = None;
                }
                if ui
                    .button(text("停止并断开", "Stop and disconnect"))
                    .clicked()
                {
                    self.confirm_disconnect = None;
                    result = Some(ActionRequest::Command(Command::Disconnect(generation)));
                }
            });
        }
        result
    }

    fn connection_fields(&mut self, ui: &mut egui::Ui, chinese: bool, editable: bool) {
        let text = |zh, en| if chinese { zh } else { en };
        let error = self.error.map(|error| match error {
            ConfigError::Url => text("请输入有效的服务器地址", "Enter a valid server URL"),
            ConfigError::Scheme => text(
                "使用 https:// 或 moqt:// 地址，不包含用户名或 # 片段",
                "Use https:// or moqt:// without user information or a fragment",
            ),
            ConfigError::Fingerprint => text(
                "证书指纹须为 SHA-256 十六进制值",
                "Certificate pin must be a hex-encoded SHA-256 fingerprint",
            ),
        });
        let address_label = text("服务器地址", "Server URL");
        let address_error = if self.error == Some(ConfigError::Fingerprint) {
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
        .show(ui, &mut self.address);
        if address.changed() {
            self.error = None;
        }
        if editable && address.lost_focus() && self.finish_address_edit() {
            self.suppress_connect_gesture = true;
        }
        let pin_set = !self.fingerprint.trim().is_empty()
            && moq_tokio::tls::parse_fingerprint(self.fingerprint.trim()).is_ok();
        let title = if pin_set {
            text("证书校验 · 已设置", "Certificate verification · Set")
        } else {
            text("证书校验", "Certificate verification")
        };
        ui.add_enabled_ui(editable, |ui| {
            moqcast_ui::v2::disclosure(
                ui,
                egui::Id::new("relay-pin-section"),
                title,
                self.error == Some(ConfigError::Fingerprint),
                |ui| {
                    let label = text(
                        "SHA-256 证书指纹（留空使用系统信任）",
                        "SHA-256 certificate pin (empty for system trust)",
                    );
                    if (moqcast_ui::v2::TextField {
                        id: egui::Id::new("relay-fingerprint"),
                        label,
                        hint: "",
                        enabled: true,
                        error: if self.error == Some(ConfigError::Fingerprint) {
                            error
                        } else {
                            None
                        },
                    })
                    .show(ui, &mut self.fingerprint)
                    .changed()
                    {
                        self.error = None;
                        self.pin_cleared = false;
                        self.pin_endpoint =
                            Config::parse(&self.address, "").ok().map(|c| c.endpoint());
                    }
                    if !self.fingerprint.is_empty()
                        && (Action {
                            label: text("清空指纹", "Clear pin"),
                            size: egui::vec2(118.0, 40.0),
                            enabled: editable,
                            tone: ActionTone::Neutral,
                        })
                        .show(ui)
                        .clicked()
                    {
                        self.fingerprint.clear();
                        self.pin_endpoint = None;
                        self.error = None;
                        self.pin_cleared = false;
                    }
                },
            );
        });
        if self.pin_cleared {
            notice(
                ui,
                StatusTone::Warning,
                text(
                    "服务器已更改，旧指纹已清除。请确认新服务器的证书设置后再连接。",
                    "Server changed. The previous pin was cleared. Check certificate settings before connecting.",
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_survives_scope_edit_but_retarget_requires_another_gesture() {
        let mut form = Ui {
            address: "https://example.test:443/start".into(),
            fingerprint: "aa".repeat(32),
            ..Default::default()
        };
        assert!(!form.finish_address_edit());
        form.address = "https://EXAMPLE.test/end?scope=2".into();
        assert!(!form.finish_address_edit());
        assert!(!form.fingerprint.is_empty());
        form.address = "moqt://example.test/end".into();
        assert!(form.finish_address_edit());
        assert!(form.pin_cleared && form.fingerprint.is_empty());
    }

    #[test]
    fn watch_acknowledgement_matches_the_requested_route_and_connection() {
        let mut state = Snapshot::default();
        let generation = state.lifecycle.begin(Phase::Connected);
        let selected = Selection {
            generation,
            path: "test/one".into(),
            revision: 7,
        };
        let mut form = Ui::default();
        form.queued(&Command::Watch(selected.clone()), &state);
        assert!(!form.reconcile(&state, &MediaRoute::Nearby, false));
        state.watch_request += 1;
        assert!(form.reconcile(&state, &MediaRoute::Watching(selected.clone(), 1), true));
        form.queued(&Command::Watch(selected.clone()), &state);
        state.lifecycle.begin(Phase::Disconnected);
        assert!(!form.reconcile(&state, &MediaRoute::Nearby, false));
        assert!(form.watch_pending.is_none());
    }
}
