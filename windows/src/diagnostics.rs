//! Windows application diagnostics controls and log window state.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

use eframe::egui::{self, RichText};
use moqcast_diagnostics::{Entry, FileStatus, Handle, LogLevel, Snapshot};

use crate::app::Locale;

enum Status {
    Exported(PathBuf),
    Error(String),
}

pub(crate) struct DiagnosticsUi {
    handle: Handle,
    visible: bool,
    pause_auto_scroll: bool,
    minimum_level: LogLevel,
    search: String,
    snapshot: Snapshot,
    display_cache: DisplayCache,
    export_rx: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    status: Option<Status>,
}

impl DiagnosticsUi {
    pub(crate) fn new(handle: Handle, detailed: bool) -> Self {
        handle.set_detailed(detailed);
        let snapshot = handle.snapshot();
        Self {
            handle,
            visible: false,
            pause_auto_scroll: false,
            minimum_level: LogLevel::Info,
            search: String::new(),
            snapshot,
            display_cache: DisplayCache::default(),
            export_rx: None,
            status: None,
        }
    }

    pub(crate) fn detailed(&self) -> bool {
        self.handle.detailed()
    }

    pub(crate) fn hide_window(&mut self) {
        self.visible = false;
    }

    pub(crate) fn visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn show_settings(&mut self, ui: &mut egui::Ui, locale: Locale) {
        use moqcast_ui::v2::{self, Action, ActionTone, StatusTone};
        let available = matches!(self.handle.file_status(), FileStatus::Available(_));
        v2::settings_row(ui, text(locale, "日志", "Logs"), |ui| {
            if (Action {
                label: text(locale, "导出日志", "Export logs"),
                size: egui::vec2(100.0, 40.0),
                enabled: available && self.export_rx.is_none(),
                tone: ActionTone::Neutral,
            })
            .show(ui)
            .clicked()
            {
                self.choose_export();
            }
            if (Action {
                label: text(locale, "打开目录", "Open folder"),
                size: egui::vec2(100.0, 40.0),
                enabled: available,
                tone: ActionTone::Neutral,
            })
            .show(ui)
            .clicked()
            {
                self.open_directory();
            }
        });
        if !available {
            v2::notice(
                ui,
                StatusTone::Warning,
                text(locale, "文件日志不可用", "File diagnostics unavailable"),
            );
        }
        if let Some(status) = self.localized_status(locale) {
            ui.label(status);
        }
    }
    pub(crate) fn show_developer_settings(&mut self, ui: &mut egui::Ui, locale: Locale) {
        use moqcast_ui::v2::{self, Action, ActionTone, StatusTone};
        let mut detailed = self.handle.detailed();
        let detailed_label = text(locale, "详细诊断", "Detailed diagnostics");
        v2::settings_row(ui, detailed_label, |ui| {
            if v2::switch(ui, &mut detailed, detailed_label).changed() {
                self.handle.set_detailed(detailed);
            }
        });
        v2::settings_row(ui, text(locale, "应用日志", "Application logs"), |ui| {
            if (Action {
                label: text(locale, "打开日志", "Open logs"),
                size: egui::vec2(108.0, 40.0),
                enabled: true,
                tone: ActionTone::Neutral,
            })
            .show(ui)
            .clicked()
            {
                self.visible = true;
            }
        });
        if self.handle.dropped_count() > 0 {
            v2::notice(
                ui,
                StatusTone::Warning,
                &format!(
                    "{}: {}",
                    text(locale, "已丢弃日志", "Dropped log entries"),
                    self.handle.dropped_count()
                ),
            );
        }
    }

    pub(crate) fn show_window(&mut self, context: &egui::Context, locale: Locale) {
        self.poll_export();
        if !self.visible {
            return;
        }
        self.refresh_snapshot();
        let mut open = self.visible;
        egui::Window::new(text(locale, "应用日志", "Application logs"))
            .id(egui::Id::new("diagnostics-log-window"))
            .open(&mut open)
            .default_size(egui::vec2(860.0, 520.0))
            .min_size(egui::vec2(520.0, 320.0))
            .show(context, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let level_label = ui.label(text(locale, "最低级别", "Minimum level"));
                    moqcast_ui::v2::select(
                        ui,
                        egui::ComboBox::from_id_salt("diagnostics-level")
                            .selected_text(self.minimum_level.as_str()),
                        |ui| {
                            for level in [
                                LogLevel::Error,
                                LogLevel::Warn,
                                LogLevel::Info,
                                LogLevel::Debug,
                            ] {
                                ui.selectable_value(&mut self.minimum_level, level, level.as_str());
                            }
                        },
                    )
                    .response
                    .labelled_by(level_label.id);
                    ui.label(text(locale, "搜索", "Search"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .desired_width(220.0)
                            .hint_text(text(
                                locale,
                                "目标、线程或事件",
                                "Target, thread, or event",
                            )),
                    );
                    moqcast_ui::v2::switch_with_label(
                        ui,
                        &mut self.pause_auto_scroll,
                        text(locale, "暂停自动滚动", "Pause auto-scroll"),
                    );
                });

                self.display_cache
                    .refresh(&self.snapshot, self.minimum_level, &self.search);
                ui.horizontal(|ui| {
                    if (moqcast_ui::v2::Action {
                        label: text(locale, "复制当前结果", "Copy visible logs"),
                        size: egui::vec2(124.0, 40.0),
                        enabled: true,
                        tone: moqcast_ui::v2::ActionTone::Neutral,
                    })
                    .show(ui)
                    .clicked()
                    {
                        context.copy_text(self.display_cache.lines.join("\n"));
                    }
                    ui.label(
                        RichText::new(format!(
                            "{}: {}",
                            text(locale, "已丢弃诊断项", "Dropped diagnostics"),
                            self.snapshot.dropped()
                        ))
                        .small(),
                    );
                });
                ui.separator();
                let scroll = egui::ScrollArea::both()
                    .stick_to_bottom(!self.pause_auto_scroll)
                    .auto_shrink([false, false]);
                if self.display_cache.lines.is_empty() {
                    scroll.show(ui, |ui| {
                        ui.label(text(
                            locale,
                            "当前筛选条件下没有日志。",
                            "No logs match the current filters.",
                        ));
                    });
                } else {
                    let lines = &self.display_cache.lines;
                    scroll.show_rows(ui, 16.0, lines.len(), |ui, visible_rows| {
                        for row in visible_rows {
                            ui.add(
                                egui::Label::new(RichText::new(&lines[row]).monospace().size(11.0))
                                    .extend(),
                            );
                        }
                    });
                }
            });
        self.visible = open;
    }

    fn open_directory(&mut self) {
        let status = self.handle.file_status();
        let Some(log_dir) = status.directory() else {
            self.status = Some(Status::Error(
                status
                    .unavailable_reason()
                    .unwrap_or("persistent diagnostics unavailable")
                    .to_owned(),
            ));
            return;
        };
        if let Err(error) = directory_command(log_dir).spawn() {
            self.status = Some(Status::Error(error.to_string()));
        }
    }

    fn choose_export(&mut self) {
        if self.export_rx.is_some() {
            return;
        }
        let status = self.handle.file_status();
        let Some(log_dir) = status.directory() else {
            self.status = Some(Status::Error(
                status
                    .unavailable_reason()
                    .unwrap_or("persistent diagnostics unavailable")
                    .to_owned(),
            ));
            return;
        };
        let destination = rfd::FileDialog::new()
            .add_filter("ZIP archive", &["zip"])
            .set_directory(log_dir)
            .set_file_name(self.handle.recommended_export_name())
            .save_file();
        let Some(destination) = destination else {
            return;
        };

        let write_archive = self.handle.prepare_export(destination);
        let (tx, rx) = mpsc::channel();
        self.export_rx = Some(rx);
        self.status = None;
        if let Err(error) = std::thread::Builder::new()
            .name("moqcast-log-export".to_owned())
            .spawn(move || {
                let result = write_archive()
                    .map(|result| result.path().to_owned())
                    .map_err(|error| error.to_string());
                let _ = tx.send(result);
            })
        {
            self.export_rx = None;
            self.status = Some(Status::Error(error.to_string()));
        }
    }

    fn localized_status(&mut self, locale: Locale) -> Option<String> {
        self.poll_export();
        match self.status.as_ref()? {
            Status::Exported(path) => Some(match locale {
                Locale::Chinese => format!("本地日志已导出到 {}", path.display()),
                Locale::English => format!("Local logs exported to {}", path.display()),
            }),
            Status::Error(error) => Some(match locale {
                Locale::Chinese => format!("本地诊断操作失败：{error}"),
                Locale::English => format!("Local diagnostics action failed: {error}"),
            }),
        }
    }

    fn refresh_snapshot(&mut self) {
        if self.handle.revision() != self.snapshot.revision()
            || self.handle.dropped_count() != self.snapshot.dropped()
        {
            self.snapshot = self.handle.snapshot();
        }
    }

    fn poll_export(&mut self) {
        let Some(receiver) = &self.export_rx else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(path)) => {
                self.status = Some(Status::Exported(path));
                self.export_rx = None;
            }
            Ok(Err(error)) => {
                self.status = Some(Status::Error(error));
                self.export_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = Some(Status::Error("log export worker stopped".to_owned()));
                self.export_rx = None;
            }
        }
    }
}

#[derive(Default)]
struct DisplayCache {
    revision: Option<u64>,
    minimum_level: Option<LogLevel>,
    search: String,
    lines: Vec<String>,
}

impl DisplayCache {
    fn refresh(&mut self, snapshot: &Snapshot, minimum_level: LogLevel, search: &str) {
        if !self.should_rebuild(snapshot.revision(), minimum_level, search) {
            return;
        }

        let normalized_search = search.to_lowercase();
        self.lines = snapshot
            .entries()
            .iter()
            .filter(|entry| includes_level(entry.level(), minimum_level))
            .filter(|entry| {
                normalized_search.is_empty()
                    || entry.target().to_lowercase().contains(&normalized_search)
                    || entry.event().to_lowercase().contains(&normalized_search)
                    || entry.thread().to_lowercase().contains(&normalized_search)
            })
            .map(display_line)
            .collect();
        self.revision = Some(snapshot.revision());
        self.minimum_level = Some(minimum_level);
        self.search.clear();
        self.search.push_str(search);
    }

    fn should_rebuild(&self, revision: u64, minimum_level: LogLevel, search: &str) -> bool {
        self.revision != Some(revision)
            || self.minimum_level != Some(minimum_level)
            || self.search != search
    }
}

fn directory_command(path: &Path) -> Command {
    #[cfg(target_os = "windows")]
    let command = {
        let mut command = Command::new("explorer");
        command.arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    command
}

fn text(locale: Locale, chinese: &'static str, english: &'static str) -> &'static str {
    match locale {
        Locale::Chinese => chinese,
        Locale::English => english,
    }
}

fn includes_level(level: LogLevel, minimum: LogLevel) -> bool {
    level_rank(level) <= level_rank(minimum)
}

fn level_rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}

fn display_line(entry: &Entry) -> String {
    format!(
        "{} {:<5} {} [{}] {}",
        entry.timestamp(),
        entry.level().as_str(),
        entry.target(),
        entry.thread(),
        entry.event()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_filter_includes_more_severe_entries() {
        assert!(includes_level(LogLevel::Error, LogLevel::Info));
        assert!(includes_level(LogLevel::Warn, LogLevel::Info));
        assert!(includes_level(LogLevel::Info, LogLevel::Info));
        assert!(!includes_level(LogLevel::Debug, LogLevel::Info));
    }

    #[test]
    fn display_cache_rebuilds_only_when_revision_level_or_search_changes() {
        let cache = DisplayCache {
            revision: Some(7),
            minimum_level: Some(LogLevel::Info),
            search: "peer".to_owned(),
            lines: Vec::new(),
        };

        assert!(!cache.should_rebuild(7, LogLevel::Info, "peer"));
        assert!(cache.should_rebuild(8, LogLevel::Info, "peer"));
        assert!(cache.should_rebuild(7, LogLevel::Debug, "peer"));
        assert!(cache.should_rebuild(7, LogLevel::Info, "screen"));
    }

    #[test]
    fn log_window_v2_controls_toggle_pause_once_and_render_level_and_copy() {
        for (locale, level_label, pause_label, copy_label) in [
            (Locale::Chinese, "最低级别", "暂停自动滚动", "复制当前结果"),
            (
                Locale::English,
                "Minimum level",
                "Pause auto-scroll",
                "Copy visible logs",
            ),
        ] {
            let diagnostics = moqcast_diagnostics::init(moqcast_diagnostics::Config::without_file(
                moqcast_diagnostics::BuildInfo::new("test"),
                "UI fixture",
            ));
            let handle = diagnostics.handle();
            let mut ui = DiagnosticsUi::new(handle, false);
            ui.visible = true;
            let ctx = egui::Context::default();
            moqcast_ui::v2::install(&ctx);
            ctx.enable_accesskit();
            let render = |ui: &mut DiagnosticsUi, events| {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(960.0, 640.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |root| {
                        ui.show_window(root.ctx(), locale);
                    },
                );
                out.textures_delta.clear();
                out
            };
            let _ = render(&mut ui, vec![]);
            let out = render(&mut ui, vec![]);
            let update = out
                .platform_output
                .accesskit_update
                .as_ref()
                .expect("AccessKit update must be emitted");
            let (_, combo_node) = update
                .nodes
                .iter()
                .find(|(_, node)| node.role() == egui::accesskit::Role::ComboBox)
                .expect("v2 level select must expose an AccessKit ComboBox node");
            assert_eq!(
                combo_node.value(),
                Some("INFO"),
                "ComboBox must keep accessible current level value"
            );
            let labelled_texts: Vec<&str> = combo_node
                .labelled_by()
                .iter()
                .filter_map(|target_id| {
                    update
                        .nodes
                        .iter()
                        .find(|(id, _)| id == target_id)
                        .and_then(|(_, node)| node.value().or_else(|| node.label()))
                })
                .collect();
            assert!(
                combo_node.label() == Some(level_label) || labelled_texts.contains(&level_label),
                "locale={locale:?}: ComboBox label={:?}, labelled_by texts={labelled_texts:?}, expected {level_label:?}",
                combo_node.label()
            );

            // Switch level from INFO to ERROR via the v2 ComboBox and verify filtering + copy
            let combo_center = out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == "INFO" => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("v2 level select must render current level");
            for pressed in [true, false] {
                let _ = render(
                    &mut ui,
                    vec![
                        egui::Event::PointerMoved(combo_center),
                        egui::Event::PointerButton {
                            pos: combo_center,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            let popup_out = render(&mut ui, vec![]);
            let error_option = popup_out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == "ERROR" => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("ERROR level option must appear in v2 select popup");
            for pressed in [true, false] {
                let _ = render(
                    &mut ui,
                    vec![
                        egui::Event::PointerMoved(error_option),
                        egui::Event::PointerButton {
                            pos: error_option,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            let filtered_out = render(&mut ui, vec![]);
            assert_eq!(ui.minimum_level, LogLevel::Error);
            let filtered_update = filtered_out
                .platform_output
                .accesskit_update
                .as_ref()
                .expect("AccessKit update after level switch");
            let (_, filtered_combo) = filtered_update
                .nodes
                .iter()
                .find(|(_, node)| node.role() == egui::accesskit::Role::ComboBox)
                .expect("ComboBox must stay accessible after level switch");
            assert_eq!(filtered_combo.value(), Some("ERROR"));
            let filtered_labelled_texts: Vec<&str> = filtered_combo
                .labelled_by()
                .iter()
                .filter_map(|target_id| {
                    filtered_update
                        .nodes
                        .iter()
                        .find(|(id, _)| id == target_id)
                        .and_then(|(_, node)| node.value().or_else(|| node.label()))
                })
                .collect();
            assert!(
                filtered_combo.label() == Some(level_label)
                    || filtered_labelled_texts.contains(&level_label),
                "locale={locale:?}: ComboBox lost label after switch: {:?}",
                filtered_labelled_texts
            );

            ui.display_cache = DisplayCache {
                revision: Some(ui.snapshot.revision()),
                minimum_level: Some(LogLevel::Error),
                search: String::new(),
                lines: vec!["2026-11-26 ERROR moqcast::test [main] error-visible".to_owned()],
            };
            let copy_center = filtered_out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == copy_label => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("v2 copy action must render");
            let mut copied = None;
            for pressed in [true, false] {
                let copy_out = render(
                    &mut ui,
                    vec![
                        egui::Event::PointerMoved(copy_center),
                        egui::Event::PointerButton {
                            pos: copy_center,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
                for command in copy_out.platform_output.commands {
                    if let egui::OutputCommand::CopyText(text) = command {
                        copied = Some(text);
                    }
                }
            }
            let copied = copied.expect("clicking Copy visible logs must copy filtered lines");
            assert_eq!(
                copied,
                "2026-11-26 ERROR moqcast::test [main] error-visible"
            );

            let text_center = filtered_out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == pause_label => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("pause auto-scroll label must be painted");
            assert!(!ui.pause_auto_scroll);
            for expected in [true, false] {
                for pressed in [true, false] {
                    let _ = render(
                        &mut ui,
                        vec![
                            egui::Event::PointerMoved(text_center),
                            egui::Event::PointerButton {
                                pos: text_center,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                    );
                }
                assert_eq!(ui.pause_auto_scroll, expected, "locale={locale:?}");
            }
        }
    }
}
