//! Controls shared by the three native application shells.
use eframe::egui;
use moqcast_diagnostics::media;

pub(crate) fn settings(ui: &mut egui::Ui, chinese: bool) {
    let mut software = media::next_software();
    moqcast_ui::v2::settings_row(
        ui,
        if chinese {
            "下次观看解码"
        } else {
            "Next watch decoder"
        },
        |ui| {
            let automatic = if chinese { "自动" } else { "Auto" };
            let software_only = if chinese {
                "仅软件"
            } else {
                "Software only"
            };
            egui::ComboBox::from_id_salt("next-watch-decoder")
                .selected_text(if software { software_only } else { automatic })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut software, false, automatic);
                    ui.selectable_value(&mut software, true, software_only);
                });
        },
    );
    if software != media::next_software() {
        media::set_next_software(software);
    }
    ui.small(if chinese { "新观看时生效，重启后恢复自动。导出 ZIP 包含日志、环境及媒体会话摘要。" }
        else { "Applies when a watch starts. Restarts reset to Auto. ZIP includes logs, environment and media sessions." });
}

pub(crate) fn sharing(ui: &mut egui::Ui, chinese: bool) {
    let Some(record) = media::publication() else {
        return;
    };
    let text = |zh, en| if chinese { zh } else { en };
    let render = |ui: &mut egui::Ui| {
        let unavailable = text("未提供", "Unavailable");
        let source = if record.source() == "window" {
            text("窗口", "Window")
        } else {
            text("屏幕", "Display")
        };
        let policy = if record.policy == "hardware" {
            text("仅硬件", "Hardware only")
        } else {
            text("自动", "Auto")
        };
        let missing = if record.awaiting_backend() {
            text("等待中", "Waiting")
        } else {
            unavailable
        };
        let execution = match record.execution() {
            Some("hardware") => text("硬件编码", "Hardware encoding"),
            Some("software") => text("软件编码", "Software encoding"),
            _ => missing,
        };
        let size = record.dimensions.map(|(w, h)| format!("{w} × {h}"));
        egui::Grid::new("publication-observations").show(ui, |ui| {
            for (label, value) in [
                (text("角色", "Role"), text("发布", "Publisher")),
                (text("来源类型", "Source type"), source),
                (
                    text("编码格式", "Codec"),
                    record.codec.unwrap_or(unavailable),
                ),
                (
                    text("来源尺寸", "Source size"),
                    size.as_deref().unwrap_or(unavailable),
                ),
                (text("请求策略", "Requested policy"), policy),
                (
                    text("实际编码器", "Actual encoder"),
                    record.backend.unwrap_or(missing),
                ),
                (text("执行方式", "Execution"), execution),
            ] {
                ui.label(label);
                ui.label(value);
                ui.end_row();
            }
        });
    };
    let label = text("共享信息", "Sharing information");
    #[cfg(target_os = "macos")]
    moqcast_ui::v2::disclosure(
        ui,
        egui::Id::new("publication-information"),
        label,
        false,
        render,
    );
    #[cfg(not(target_os = "macos"))]
    ui.collapsing(label, render);
}

pub(crate) fn software_failure(generation: u64, chinese: bool) -> Option<&'static str> {
    media::software_decoder_failed(generation).then_some(if chinese {
        "没有可用的软件解码器。可在设置中将下次观看改为自动。"
    } else {
        "No software decoder could open this video. Select Auto in Settings for the next watch."
    })
}
