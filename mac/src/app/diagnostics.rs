//! macOS application diagnostics controls and log window state.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use eframe::egui::{self, RichText};
use moqcast_diagnostics::{Entry, ExportRequest, FileStatus, Handle, LogLevel, Snapshot};

use super::Locale;

enum Status {
    Exported(PathBuf),
    Error(String),
}

pub(super) struct DiagnosticsUi {
    handle: Handle,
    visible: bool,
    pause_auto_scroll: bool,
    minimum_level: LogLevel,
    search: String,
    snapshot: Snapshot,
    display_cache: DisplayCache,
    open_rx: Option<mpsc::Receiver<Result<(), String>>>,
    export_rx: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    status: Option<Status>,
}

impl DiagnosticsUi {
    pub(super) fn new(handle: Handle, detailed: bool) -> Self {
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
            open_rx: None,
            export_rx: None,
            status: None,
        }
    }

    pub(super) fn set_detailed(&self, detailed: bool) {
        self.handle.set_detailed(detailed);
    }

    pub(super) fn hide_window(&mut self) {
        self.visible = false;
    }

    pub(super) fn show_settings(&mut self, ui: &mut egui::Ui, locale: Locale) {
        ui.label(RichText::new(text(locale, "本地日志", "Local logs")).strong());
        ui.small(text(
            locale,
            "日志仅保存在这台 Mac 上，只有你主动选择时才会导出。",
            "Logs stay on this Mac and are exported only when you choose.",
        ));
        ui.add_space(8.0);

        ui.checkbox(
            &mut self.visible,
            text(locale, "显示应用日志", "Show application logs"),
        );

        let file_available = match self.handle.file_status() {
            FileStatus::Available(directory) => {
                ui.small(format!(
                    "{}: {}",
                    text(locale, "日志目录", "Log directory"),
                    directory.display()
                ));
                true
            }
            FileStatus::Unavailable(reason) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    match locale {
                        Locale::Chinese => {
                            format!("文件日志不可用：{reason}。应用仍可继续运行。")
                        }
                        Locale::English => format!(
                            "File logs are unavailable: {reason}. The application can continue."
                        ),
                    },
                );
                false
            }
        };
        ui.small(format!(
            "{}: {}",
            text(locale, "已丢弃日志项", "Dropped log entries"),
            self.handle.dropped_count()
        ));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    file_available && self.open_rx.is_none(),
                    egui::Button::new(text(locale, "打开日志目录", "Open log directory")),
                )
                .clicked()
            {
                self.open_directory(ui.ctx().clone());
            }
            if ui
                .add_enabled(
                    file_available && self.export_rx.is_none(),
                    egui::Button::new(text(locale, "导出本地日志", "Export local logs")),
                )
                .clicked()
            {
                self.choose_export(ui.ctx().clone());
            }
        });
        if let Some(status) = self.localized_status(locale) {
            ui.small(status);
        }
    }

    pub(super) fn show_window(&mut self, context: &egui::Context, locale: Locale) {
        self.poll_open();
        self.poll_export();
        if !self.visible {
            return;
        }
        context.request_repaint_after(Duration::from_millis(250));
        self.refresh_snapshot();
        let mut open = self.visible;
        egui::Window::new(text(locale, "应用日志", "Application logs"))
            .id(egui::Id::new("diagnostics-log-window"))
            .open(&mut open)
            .default_size(egui::vec2(860.0, 520.0))
            .min_size(egui::vec2(520.0, 320.0))
            .show(context, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(text(locale, "最低级别", "Minimum level"));
                    egui::ComboBox::from_id_salt("diagnostics-level")
                        .selected_text(self.minimum_level.as_str())
                        .show_ui(ui, |ui| {
                            for level in [
                                LogLevel::Error,
                                LogLevel::Warn,
                                LogLevel::Info,
                                LogLevel::Debug,
                            ] {
                                ui.selectable_value(&mut self.minimum_level, level, level.as_str());
                            }
                        });
                    ui.label(text(locale, "搜索", "Search"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .desired_width(220.0)
                            .hint_text(text(
                                locale,
                                "模块、线程或事件",
                                "Target, thread, or event",
                            )),
                    );
                    ui.checkbox(
                        &mut self.pause_auto_scroll,
                        text(locale, "暂停自动滚动", "Pause auto-scroll"),
                    );
                });

                self.display_cache
                    .refresh(&self.snapshot, self.minimum_level, &self.search);
                ui.horizontal(|ui| {
                    if ui
                        .button(text(locale, "复制当前结果", "Copy visible logs"))
                        .clicked()
                    {
                        context.copy_text(self.display_cache.lines.join("\n"));
                    }
                    ui.label(
                        RichText::new(format!(
                            "{}: {}",
                            text(locale, "已丢弃日志项", "Dropped log entries"),
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

    fn open_directory(&mut self, context: egui::Context) {
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
        let log_dir = log_dir.to_owned();
        let (tx, rx) = mpsc::channel();
        self.open_rx = Some(rx);
        self.status = None;
        if let Err(error) = std::thread::Builder::new()
            .name("moqcast-open-log-directory".to_owned())
            .spawn(move || {
                let result = directory_command(&log_dir)
                    .status()
                    .map_err(|error| error.to_string())
                    .and_then(|status| {
                        status
                            .success()
                            .then_some(())
                            .ok_or_else(|| format!("system open tool exited with status {status}"))
                    });
                let _ = tx.send(result);
                context.request_repaint();
            })
        {
            self.open_rx = None;
            self.status = Some(Status::Error(error.to_string()));
        }
    }

    fn choose_export(&mut self, context: egui::Context) {
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

        let handle = self.handle.clone();
        let (tx, rx) = mpsc::channel();
        self.export_rx = Some(rx);
        self.status = None;
        if let Err(error) = std::thread::Builder::new()
            .name("moqcast-log-export".to_owned())
            .spawn(move || {
                let result = handle
                    .export(ExportRequest::new(destination))
                    .map(|result| result.path().to_owned())
                    .map_err(|error| error.to_string());
                let _ = tx.send(result);
                context.request_repaint();
            })
        {
            self.export_rx = None;
            self.status = Some(Status::Error(error.to_string()));
        }
    }

    fn localized_status(&mut self, locale: Locale) -> Option<String> {
        self.poll_open();
        self.poll_export();
        match self.status.as_ref()? {
            Status::Exported(path) => Some(match locale {
                Locale::Chinese => format!("本地日志已导出到 {}", path.display()),
                Locale::English => format!("Local logs exported to {}", path.display()),
            }),
            Status::Error(error) => Some(match locale {
                Locale::Chinese => format!("本地日志操作失败：{error}"),
                Locale::English => format!("Local log action failed: {error}"),
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

    fn poll_open(&mut self) {
        let Some(receiver) = &self.open_rx else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(())) => {
                self.open_rx = None;
            }
            Ok(Err(error)) => {
                self.status = Some(Status::Error(error));
                self.open_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = Some(Status::Error("log directory worker stopped".to_owned()));
                self.open_rx = None;
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
    let mut command = Command::new("/usr/bin/open");
    command.arg(path);
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
    fn log_directory_uses_the_system_open_tool() {
        let command = directory_command(Path::new("/tmp/logs"));
        assert_eq!(command.get_program(), "/usr/bin/open");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["/tmp/logs"]);
    }
}
