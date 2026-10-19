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
    last_action: String,
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
                    ui.label("附近设备 · 可用 / 未共享 / 连接中 / 已离开");
                    ui.add_space(12.0);
                    egui::ScrollArea::vertical()
                        .max_height((ui.available_height() - 45.0).max(80.0))
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            for n in 0..if self.long_list { 40 } else { 4 } {
                                let name =
                                    format!("设备 {n} · Very long desktop name with 中文内容");
                                let status =
                                    ["可观看", "未共享", "连接中", "已离开附近范围"][n % 4];
                                let response = v2::nearby_row(
                                    ui,
                                    NearbyRow {
                                        id: egui::Id::new(n),
                                        name: &name,
                                        subtitle: if m.compact {
                                            status
                                        } else {
                                            "Device ID: 0123456789abcdef0123456789abcdef"
                                        },
                                        status,
                                        tone: if n % 4 == 0 {
                                            v2::StatusTone::Active
                                        } else {
                                            v2::StatusTone::Neutral
                                        },
                                        current: false,
                                        watch_label: "观看",
                                        watch_enabled: n % 4 == 0,
                                        details_label: "设备详情",
                                        compact: m.compact,
                                    },
                                );
                                if response.details {
                                    self.last_action = format!("Details {n}");
                                }
                                if response.watch {
                                    self.last_action = format!("Watch {n}");
                                }
                            }
                        });
                });
                ui.label(&self.last_action);
            });
    }
}
