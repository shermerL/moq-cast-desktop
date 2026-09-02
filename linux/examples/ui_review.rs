//! Static Linux desktop UI review shell for macOS design inspection.

use std::borrow::Cow;

use eframe::egui::{self, Align, Frame, Layout, ScrollArea, Sense};
use moqcast_ui::{
    BadgeTone, COLORS, DetailRowSpec, DeviceBadgeSpec, DeviceListItemSpec, DeviceListSpec,
    IconButtonSpec, NavItemSpec, PageWidth, SelectSpec, SettingRowSpec, Size, Spacing,
    StatePanelKind, StatePanelSpec, SwitchSpec, Theme, TypographyRole, app_bar_content_rect,
    control_button, detail_row, device_list, install_ui_font, nav_item, page_content_rect,
    page_header, player_icon_button, player_rects, player_stage_at, player_toolbar_at,
    section_header, select, setting_row, state_panel, status_badge, status_strip, switch,
    typography,
};

const WORKSPACE_GAP: f32 = Spacing::LG;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("MoQCast Linux UI review")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size(Size::MIN_VIEWPORT),
        ..Default::default()
    };
    eframe::run_native(
        "MoQCast Linux UI review",
        options,
        Box::new(|creation| Ok(Box::new(Review::new(&creation.egui_ctx)))),
    )
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
enum Page {
    #[default]
    Nearby,
    Share,
    Watch,
    Settings,
}

impl Page {
    const ALL: [Self; 4] = [Self::Nearby, Self::Share, Self::Watch, Self::Settings];

    fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::Nearby => locale.text("附近设备", "Nearby"),
            Self::Share => locale.text("屏幕共享", "Screen share"),
            Self::Watch => locale.text("观看", "Watch"),
            Self::Settings => locale.text("设置", "Settings"),
        }
    }

    fn width(self) -> PageWidth {
        match self {
            Self::Nearby => PageWidth::Wide,
            Self::Share | Self::Watch => PageWidth::Medium,
            Self::Settings => PageWidth::Narrow,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Locale {
    Chinese,
    #[default]
    English,
}

impl Locale {
    fn text<'a>(self, chinese: &'a str, english: &'a str) -> &'a str {
        match self {
            Self::Chinese => chinese,
            Self::English => english,
        }
    }
}

struct Review {
    page: Page,
    selected: usize,
    system_audio: bool,
    developer_mode: bool,
    locale: Locale,
}

impl Review {
    fn new(context: &egui::Context) -> Self {
        install_ui_font(context, Cow::Borrowed(moqcast_ui::NOTO_SANS_SC));
        Theme.apply(context);
        Self {
            page: Page::Nearby,
            selected: 0,
            system_audio: false,
            developer_mode: false,
            locale: Locale::English,
        }
    }
}

impl eframe::App for Review {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        Theme.apply(ui.ctx());
        app_bar(ui, &mut self.page, self.locale);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(COLORS.surface.into()))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt(("linux-ui-review", self.page))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let rect =
                            page_content_rect(ui.available_rect_before_wrap(), self.page.width());
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            ui.set_width(rect.width());
                            match self.page {
                                Page::Nearby => nearby(ui, &mut self.selected, self.locale),
                                Page::Share => share(ui, &mut self.system_audio, self.locale),
                                Page::Watch => watch(ui, self.locale),
                                Page::Settings => {
                                    settings(ui, &mut self.developer_mode, &mut self.locale)
                                }
                            }
                        });
                    });
            });
    }
}

fn app_bar(ui: &mut egui::Ui, page: &mut Page, locale: Locale) {
    egui::Panel::top("review-app-bar")
        .exact_size(Size::APP_BAR)
        .frame(Frame::new().fill(COLORS.chrome.into()))
        .show(ui, |ui| {
            let rect = app_bar_content_rect(ui.max_rect()).shrink2(egui::vec2(0.0, Spacing::SM));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    ui.label(typography(
                        "MoQCast Desktop",
                        TypographyRole::Row,
                        COLORS.text.into(),
                    ));
                    ui.add_space(Spacing::XL);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = Spacing::XS;
                        for (index, target) in Page::ALL.into_iter().enumerate() {
                            let label = target.label(locale);
                            if nav_item(
                                ui,
                                NavItemSpec::new(egui::Id::new(("review-nav", index)), label)
                                    .selected(*page == target),
                            )
                            .clicked()
                            {
                                *page = target;
                            }
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(typography(
                            locale
                                .text("发现服务运行中 · 2 台设备", "Discovery active · 2 devices"),
                            TypographyRole::Meta,
                            COLORS.muted.into(),
                        ));
                    });
                },
            );
        });
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum WorkspaceLayout {
    Split { list: f32, detail: f32 },
    Single,
}

fn workspace_layout(width: f32) -> WorkspaceLayout {
    if width < Size::SPLIT_BREAKPOINT {
        return WorkspaceLayout::Single;
    }
    let detail = Size::NEARBY_LIST.min((width - WORKSPACE_GAP).max(1.0));
    let list = (width - detail - WORKSPACE_GAP).max(1.0);
    WorkspaceLayout::Split { list, detail }
}

fn nearby(ui: &mut egui::Ui, selected: &mut usize, locale: Locale) {
    page_header(
        ui,
        locale.text("附近设备", "Nearby"),
        Some(locale.text(
            "自动连接同一网络内的 MoQCast 设备。",
            "Automatically connect to MoQCast devices on this network.",
        )),
    );
    setting_row(
        ui,
        SettingRowSpec::new(locale.text("本机", "This device")).description(locale.text(
            "Studio Linux · 发现服务运行中",
            "Studio Linux · Discovery active",
        )),
        |ui| {
            moqcast_ui::secondary_button(ui, locale.text("停止扫描", "Stop scan"), true);
        },
    );
    ui.add_space(Spacing::LG);

    match workspace_layout(ui.available_width()) {
        WorkspaceLayout::Split { list, detail } => {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(list, 1.0),
                    Layout::top_down(Align::Min),
                    |ui| review_device_list(ui, selected, locale),
                );
                ui.add_space(WORKSPACE_GAP);
                ui.allocate_ui_with_layout(
                    egui::vec2(detail, 1.0),
                    Layout::top_down(Align::Min),
                    |ui| review_device_detail(ui, *selected, locale, detail),
                );
            });
        }
        WorkspaceLayout::Single => {
            review_device_list(ui, selected, locale);
            ui.add_space(Spacing::LG);
            let width = ui.available_width();
            review_device_detail(ui, *selected, locale, width);
        }
    }
}

fn review_device_list(ui: &mut egui::Ui, selected: &mut usize, locale: Locale) {
    let fixtures = [
        (
            locale.text("客厅显示器", "Living room display"),
            locale.text("屏幕可观看", "Screen available"),
            "review-peer-a",
        ),
        (
            locale.text("客厅显示器", "Living room display"),
            locale.text("已连接", "Connected"),
            "review-peer-b",
        ),
    ];
    let subtitles = fixtures
        .iter()
        .map(|(_, detail, id)| format!("{detail} · {}: {id}", locale.text("设备 ID", "Device ID")))
        .collect::<Vec<_>>();
    let items = fixtures
        .iter()
        .zip(&subtitles)
        .enumerate()
        .map(|(index, ((name, detail, _), subtitle))| {
            DeviceListItemSpec::new(index, *name)
                .subtitle(subtitle)
                .badge(DeviceBadgeSpec::new(detail, BadgeTone::Info))
                .selected(*selected == index)
        })
        .collect::<Vec<_>>();
    if let Some(index) = device_list(
        ui,
        DeviceListSpec::new(egui::Id::new("linux-review-devices"), &items),
    ) {
        *selected = index;
    }
}

fn review_device_detail(ui: &mut egui::Ui, selected: usize, locale: Locale, available: f32) {
    ui.set_width(available);
    section_header(
        ui,
        if selected == 0 {
            locale.text("客厅显示器", "Living room display")
        } else {
            locale.text("工作电脑", "Work laptop")
        },
        Some(locale.text("可在此网络使用", "Available on this network")),
    );
    let rows = [
        (
            locale.text("设备 ID", "Device ID"),
            if selected == 0 {
                "review-peer-a"
            } else {
                "review-peer-b"
            },
        ),
        (
            locale.text("连接", "Connection"),
            locale.text("已连接", "Connected"),
        ),
        (
            locale.text("共享屏幕", "Shared screen"),
            if selected == 0 {
                locale.text("可观看", "Available")
            } else {
                locale.text("无", "None")
            },
        ),
    ];
    for (index, (label, value)) in rows.into_iter().enumerate() {
        detail_row(ui, DetailRowSpec::new(label), |ui| {
            status_badge(ui, value, BadgeTone::Neutral);
        });
        if index + 1 < rows.len() {
            ui.separator();
        }
    }
    if selected == 0 {
        ui.add_space(Spacing::LG);
        moqcast_ui::primary_button(ui, locale.text("观看", "Watch"), true);
    }
}

fn share(ui: &mut egui::Ui, system_audio: &mut bool, locale: Locale) {
    page_header(
        ui,
        locale.text("屏幕共享", "Screen share"),
        Some(locale.text(
            "将本机屏幕共享给附近的 MoQCast 设备。",
            "Share this desktop with nearby MoQCast devices.",
        )),
    );
    section_header(
        ui,
        locale.text("共享本机屏幕", "Share this screen"),
        Some(locale.text(
            "开始共享后由系统选择屏幕。",
            "Choose a source when sharing starts.",
        )),
    );
    setting_row(
        ui,
        SettingRowSpec::new(locale.text("系统音频", "System audio")).description(locale.text(
            "同时共享此设备正在播放的声音。",
            "Also share sound playing on this device.",
        )),
        |ui| {
            switch(
                ui,
                system_audio,
                SwitchSpec::new(locale.text("系统音频", "System audio")),
            );
        },
    );
    ui.add_space(Spacing::XL);
    full_width_status_strip(
        ui,
        StatePanelSpec::new(
            StatePanelKind::Empty,
            locale.text("屏幕媒体空闲", "Screen media is idle"),
            locale.text(
                "选择一个本机屏幕开始共享。",
                "Choose a local display to start sharing.",
            ),
        ),
        |ui| {
            moqcast_ui::primary_button(ui, locale.text("选择屏幕", "Choose screen"), true);
        },
    );
}

fn full_width_status_strip<R>(
    ui: &mut egui::Ui,
    spec: StatePanelSpec<'_>,
    action: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let inner_width = (ui.available_width() - Spacing::LG * 2.0).max(1.0);
    status_strip(ui, spec, |ui| {
        ui.set_min_width(inner_width);
        action(ui)
    })
}

fn watch(ui: &mut egui::Ui, locale: Locale) {
    page_header(
        ui,
        locale.text("观看", "Watch"),
        Some(locale.text(
            "观看附近设备正在共享的屏幕。",
            "Watch a screen shared by a nearby device.",
        )),
    );
    let rects = player_rects(ui.available_rect_before_wrap(), false);
    ui.allocate_rect(rects.stage.union(rects.toolbar), Sense::hover());
    player_stage_at(ui, rects.stage, |ui| {
        ui.centered_and_justified(|ui| {
            ui.label(typography(
                locale.text("实时屏幕预览", "Live screen preview"),
                TypographyRole::Body,
                COLORS.player_muted.into(),
            ));
        });
    });
    player_toolbar_at(ui, rects.toolbar, |ui| {
        let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 4.0, COLORS.live);
        ui.label(typography("LIVE", TypographyRole::Meta, COLORS.live.into()));
        ui.label(typography(
            locale.text(
                "客厅显示器 · 1920 × 1080 · 音频可用",
                "Living room display · 1920 × 1080 · Audio available",
            ),
            TypographyRole::Meta,
            COLORS.player_text.into(),
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            player_icon_button(
                ui,
                IconButtonSpec::player("⛶", locale.text("进入全屏", "Fullscreen")),
            );
            control_button(
                ui,
                moqcast_ui::ButtonSpec::new(
                    locale.text("停止观看", "Stop watching"),
                    moqcast_ui::ControlRole::PlayerIcon,
                ),
            );
        });
    });
}

fn settings(ui: &mut egui::Ui, developer_mode: &mut bool, locale: &mut Locale) {
    let current = *locale;
    page_header(
        ui,
        current.text("设置", "Settings"),
        Some(current.text(
            "设置应用语言和高级选项。",
            "Configure language and advanced options.",
        )),
    );
    section_header(ui, current.text("通用", "General"), None);
    const LANGUAGES: [&str; 2] = ["简体中文", "English"];
    let mut language_index = usize::from(current == Locale::English);
    setting_row(
        ui,
        SettingRowSpec::new(current.text("语言", "Language"))
            .description(current.text("选择界面显示语言。", "Choose the interface language.")),
        |ui| {
            select(
                ui,
                &mut language_index,
                SelectSpec::new(
                    egui::Id::new("review-language"),
                    current.text("语言", "Language"),
                    &LANGUAGES,
                )
                .expect("the review language list is not empty"),
            )
            .expect("the review language selection is valid");
        },
    );
    *locale = if language_index == 0 {
        Locale::Chinese
    } else {
        Locale::English
    };
    let locale = *locale;
    ui.add_space(Spacing::XL);
    section_header(ui, locale.text("高级", "Advanced"), None);
    let mut auto_watch = false;
    setting_row(
        ui,
        SettingRowSpec::new(locale.text("自动观看", "Auto-watch"))
            .description(locale.text("暂不可开启。", "Temporarily unavailable.")),
        |ui| {
            switch(
                ui,
                &mut auto_watch,
                SwitchSpec::new(locale.text("自动观看", "Auto-watch")).enabled(false),
            );
        },
    );
    setting_row(
        ui,
        SettingRowSpec::new(locale.text("开发者模式", "Developer mode")).description(locale.text(
            "显示仅用于本地排查的诊断工具。",
            "Show local diagnostic tools.",
        )),
        |ui| {
            switch(
                ui,
                developer_mode,
                SwitchSpec::new(locale.text("开发者模式", "Developer mode")),
            );
        },
    );
    if *developer_mode {
        ui.add_space(Spacing::XL);
        state_panel(
            ui,
            StatePanelSpec::new(
                StatePanelKind::Empty,
                locale.text("本地诊断", "Local diagnostics"),
                locale.text(
                    "日志仅保存在本机，由你选择是否导出。",
                    "Logs stay on this device and are exported only when you choose.",
                ),
            ),
            |ui| {
                moqcast_ui::secondary_button(
                    ui,
                    locale.text("显示应用日志", "Show application logs"),
                    true,
                );
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_review_uses_a_compact_detail_and_responsive_boundary() {
        assert_eq!(workspace_layout(680.0), WorkspaceLayout::Single);
        assert_eq!(
            workspace_layout(1024.0),
            WorkspaceLayout::Split {
                list: 1024.0 - Size::NEARBY_LIST - WORKSPACE_GAP,
                detail: Size::NEARBY_LIST,
            }
        );
    }

    #[test]
    fn review_fixture_uses_a_real_select_and_not_equal_columns() {
        let source = include_str!("ui_review.rs");
        let select_call = ["select", "("].concat();
        let equal_columns = ["ui.", "columns("].concat();
        assert!(source.contains(&select_call));
        assert!(source.contains("Size::NEARBY_LIST"));
        assert!(!source.contains(&equal_columns));
    }

    #[test]
    fn review_fixture_uses_the_shared_device_list_without_cursor_workarounds() {
        let source = include_str!("ui_review.rs");
        assert!(source.contains("device_list("));
        assert!(source.contains("Device ID"));
        assert!(!source.contains("settle_device_row"));
    }
}
