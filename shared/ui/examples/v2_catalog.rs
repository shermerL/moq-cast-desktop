//! Interactive UI v2 fixtures, separate from live platform discovery.
use eframe::egui;
use moqcast_ui::v2::{self, ContextCard, Metrics, NearbyRow, Palette};

fn main() -> eframe::Result {
    eframe::run_native(
        "MoQCast UI v2 catalog",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1040.0, 700.0])
                .with_min_inner_size([680.0, 520.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            moqcast_ui::install_ui_font(
                &cc.egui_ctx,
                std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC),
            );
            v2::install(&cc.egui_ctx);
            cc.egui_ctx.set_theme(egui::Theme::Light);
            Ok(Box::new(Catalog::default()))
        }),
    )
}

#[derive(Default)]
struct Catalog {
    dark: bool,
    long_list: bool,
    empty: bool,
    last_action: String,
    lan_stopping: bool,
    media_busy: bool,
}

impl eframe::App for Catalog {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        let p = Palette::for_ui(root);
        let m = Metrics::for_width(root.ctx().content_rect().width());
        egui::Panel::top("catalog-controls").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label("UI v2 component fixtures · not live devices");
                if ui.checkbox(&mut self.dark, "Dark").changed() {
                    ui.ctx().set_theme(if self.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                }
                ui.checkbox(&mut self.long_list, "40 rows / scroll");
                ui.checkbox(&mut self.empty, "No devices");
                ui.checkbox(&mut self.lan_stopping, "Nearby stopping");
                ui.checkbox(&mut self.media_busy, "Media slot occupied");
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.canvas).inner_margin(m.inset))
            .show(root, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if v2::context_card(
                    ui,
                    ContextCard {
                        heading: "本机",
                        name: "很长的本机名称 · MacBook Pro Studio",
                        identity: "本次运行 ID: 0123456789abcdef0123456789abcdef",
                        status: "空闲",
                        tone: v2::StatusTone::Neutral,
                        action: "共享屏幕 / 窗口",
                    },
                    m,
                )
                .clicked()
                {
                    self.last_action = "Share setup".into();
                }
                ui.add_space(m.gap);
                p.card().show(ui, |ui| {
                    ui.label("设备状态与动作 · 模拟快照，不建立连接");
                    ui.add_space(12.0);
                    egui::ScrollArea::vertical()
                        .max_height((ui.available_height() - 45.0).max(80.0))
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            // Platform snapshots decide permissions. These fixtures exercise the visual inputs only.
                            let states = [
                                ("已发现", v2::StatusTone::Neutral, false, false, false),
                                ("正在连接", v2::StatusTone::Busy, false, false, false),
                                ("连接失败", v2::StatusTone::Error, false, false, false),
                                ("无法验证连接", v2::StatusTone::Error, false, false, false),
                                ("已连接", v2::StatusTone::Active, false, false, false),
                                ("可观看", v2::StatusTone::Active, false, true, false),
                                ("正在准备观看", v2::StatusTone::Busy, false, false, true),
                                ("正在观看", v2::StatusTone::Active, true, false, true),
                                ("正在停止观看", v2::StatusTone::Busy, false, false, true),
                            ];
                            if self.empty {
                                ui.label("尚未发现设备");
                            }
                            let count = if self.empty {
                                0
                            } else if self.long_list {
                                40
                            } else {
                                states.len()
                            };
                            for n in 0..count {
                                let name =
                                    format!("设备 {n} · Very long desktop name with 中文内容");
                                let (status, tone, current, available, returning) =
                                    states[n % states.len()];
                                let response = v2::nearby_row(
                                    ui,
                                    NearbyRow {
                                        id: egui::Id::new(n),
                                        name: &name,
                                        subtitle: "Device ID: 0123456789abcdef0123456789abcdef",
                                        status,
                                        tone,
                                        current,
                                        watch_label: if returning {
                                            "返回播放器"
                                        } else {
                                            "观看"
                                        },
                                        watch_enabled: returning
                                            || (available
                                                && !self.lan_stopping
                                                && !self.media_busy),
                                        details_label: "设备详情",
                                        compact: m.compact,
                                    },
                                );
                                if response.details {
                                    self.last_action = format!("Details {n}");
                                }
                                if response.watch {
                                    self.last_action = format!(
                                        "{} {n}",
                                        if returning { "Return" } else { "Watch" }
                                    );
                                }
                            }
                        });
                });
                ui.label(&self.last_action);
            });
    }
}
