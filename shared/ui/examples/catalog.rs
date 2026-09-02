use std::borrow::Cow;

use eframe::egui::{
    self, Align, Context, Frame, Id, Layout, Margin, ScrollArea, Sense, Stroke, Ui, UiBuilder,
    ViewportCommand, vec2,
};
use moqcast_ui::{
    BadgeTone, ButtonSpec, COLORS, CheckboxSpec, ControlRole, DeviceBadgeSpec, DeviceListItemSpec,
    DeviceListSpec, DeviceRowSpec, DialogSpec, IconButtonSpec, Interaction, NavItemSpec,
    SelectSpec, SettingRowSpec, Size, Spacing, StatePanelKind, StatePanelSpec, SwitchSpec, Theme,
    TypographyRole, checkbox, control_button, device_list, device_row, dialog, install_ui_font,
    nav_item, page_header, player_button, player_icon_button, player_surface, primary_button,
    secondary_button, section_header, select, setting_row, state_panel, status_badge, switch,
    typography,
};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("MoQCast shared UI catalog")
            .with_inner_size(Size::VIEWPORT_WINDOWS)
            .with_min_inner_size(Size::MIN_VIEWPORT),
        ..Default::default()
    };
    eframe::run_native(
        "MoQCast shared UI catalog",
        options,
        Box::new(|creation| Ok(Box::new(Catalog::new(&creation.egui_ctx)))),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlatformFixture {
    Windows,
    Linux,
    Macos,
}

impl PlatformFixture {
    const ALL: [Self; 3] = [Self::Windows, Self::Linux, Self::Macos];
    const LABELS: [&'static str; 3] = ["Windows", "Linux", "macOS"];

    fn viewport(self) -> [f32; 2] {
        match self {
            Self::Windows => Size::VIEWPORT_WINDOWS,
            Self::Linux => Size::VIEWPORT_LINUX,
            Self::Macos => Size::VIEWPORT_MACOS,
        }
    }

    fn capability(self) -> &'static str {
        match self {
            Self::Windows => "System audio adapter · WASAPI",
            Self::Linux => "System audio adapter · PipeWire",
            Self::Macos => "Screen source · System picker",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocaleFixture {
    Chinese,
    English,
}

impl LocaleFixture {
    fn text<'a>(self, chinese: &'a str, english: &'a str) -> &'a str {
        match self {
            Self::Chinese => chinese,
            Self::English => english,
        }
    }
}

struct Catalog {
    platform: usize,
    locale: LocaleFixture,
    compact: bool,
    enabled: bool,
    switch_value: bool,
    checkbox_value: bool,
    select_value: usize,
    dialog_open: bool,
    selected_device: &'static str,
}

impl Catalog {
    fn new(context: &Context) -> Self {
        install_ui_font(context, Cow::Borrowed(moqcast_ui::NOTO_SANS_SC));
        Theme.apply(context);
        Self {
            platform: 0,
            locale: LocaleFixture::Chinese,
            compact: false,
            enabled: true,
            switch_value: true,
            checkbox_value: true,
            select_value: 0,
            dialog_open: false,
            selected_device: "review-b",
        }
    }

    fn text<'a>(&self, chinese: &'a str, english: &'a str) -> &'a str {
        self.locale.text(chinese, english)
    }
}

impl eframe::App for Catalog {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        Theme.apply(ui.ctx());
        catalog_root(self, ui);

        if self.dialog_open {
            let cancel_id = Id::new("catalog-dialog-cancel");
            let response = dialog(
                ui.ctx(),
                DialogSpec::new(
                    Id::new("catalog-dialog"),
                    self.text("确认操作", "Confirm action"),
                    cancel_id,
                ),
                |ui| {
                    ui.label(typography(
                        self.text(
                            "平台负责业务文案和动作。这个最长文案用于验证窄窗口换行。",
                            "The platform owns business copy and actions. This longest copy verifies narrow wrapping.",
                        ),
                        TypographyRole::Body,
                        COLORS.text.into(),
                    ));
                    ui.add_space(Spacing::LG);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        primary_button(ui, self.text("继续", "Continue"), true);
                        ui.add_space(Size::DIALOG_ACTION_SPACING);
                        control_button(
                            ui,
                            ButtonSpec::new(self.text("取消", "Cancel"), ControlRole::Secondary)
                                .id(cancel_id),
                        );
                    });
                },
            );
            if response.should_close() {
                self.dialog_open = false;
            }
        }
    }
}

fn catalog_root(catalog: &mut Catalog, ui: &mut Ui) -> egui::Response {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::hover());
    ui.painter().rect_filled(rect, 0.0, COLORS.canvas);
    let inner = rect.shrink(Spacing::LG);
    let mut root = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    root.set_min_size(inner.size());
    fixture_toolbar(catalog, &mut root);
    root.add_space(Spacing::LG);
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(&mut root, |ui| catalog_content(catalog, ui));
    response
}

fn fixture_toolbar(catalog: &mut Catalog, ui: &mut Ui) {
    Frame::new()
        .fill(COLORS.chrome.into())
        .stroke(Stroke::new(Size::BORDER, COLORS.border))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::same(Spacing::MD as i8))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(typography(
                    "Prototype states",
                    TypographyRole::Meta,
                    COLORS.muted.into(),
                ));
                let previous = catalog.platform;
                let spec =
                    SelectSpec::new(Id::new("platform"), "Platform", &PlatformFixture::LABELS)
                        .expect("fixture options are not empty");
                select(ui, &mut catalog.platform, spec).expect("fixture selection stays valid");
                if previous != catalog.platform && !catalog.compact {
                    ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(
                        PlatformFixture::ALL[catalog.platform].viewport().into(),
                    ));
                }
                if secondary_button(
                    ui,
                    match catalog.locale {
                        LocaleFixture::Chinese => "中文",
                        LocaleFixture::English => "English",
                    },
                    true,
                )
                .clicked()
                {
                    catalog.locale = match catalog.locale {
                        LocaleFixture::Chinese => LocaleFixture::English,
                        LocaleFixture::English => LocaleFixture::Chinese,
                    };
                }
                if secondary_button(
                    ui,
                    if catalog.compact {
                        "680×640"
                    } else {
                        "Preset"
                    },
                    true,
                )
                .clicked()
                {
                    catalog.compact = !catalog.compact;
                    let size = if catalog.compact {
                        Size::MIN_VIEWPORT
                    } else {
                        PlatformFixture::ALL[catalog.platform].viewport()
                    };
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::InnerSize(size.into()));
                }
                checkbox(ui, &mut catalog.enabled, CheckboxSpec::new("Enabled"));
            });
        });
}

fn catalog_content(catalog: &mut Catalog, ui: &mut Ui) {
    let platform = PlatformFixture::ALL[catalog.platform];
    let width = catalog_content_width(ui.available_width());
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        page_header(
            ui,
            catalog.text("共享视觉基础", "Shared visual foundation"),
            Some(platform.capability()),
        );
        navigation_catalog(catalog, ui);
        ui.add_space(Spacing::XL);
        controls_catalog(catalog, ui);
        ui.add_space(Spacing::XXL);
        rows_catalog(catalog, ui);
        ui.add_space(Size::MAJOR_SECTION_SPACING);
        device_list_catalog(catalog, ui);
        ui.add_space(Size::MAJOR_SECTION_SPACING);
        player_catalog(catalog, ui);
        ui.add_space(Spacing::XXL);
        state_catalog(catalog, ui);
    });
}

fn catalog_content_width(available_width: f32) -> f32 {
    available_width.min(Size::PAGE_WIDE_MAX)
}

fn navigation_catalog(catalog: &Catalog, ui: &mut Ui) {
    Frame::new()
        .fill(COLORS.chrome.into())
        .inner_margin(Margin::symmetric(Spacing::LG as i8, 0))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                nav_item(
                    ui,
                    NavItemSpec::new(Id::new("nearby"), catalog.text("附近设备", "Nearby"))
                        .selected(true),
                );
                nav_item(
                    ui,
                    NavItemSpec::new(Id::new("share"), catalog.text("屏幕共享", "Screen share")),
                );
                nav_item(
                    ui,
                    NavItemSpec::new(Id::new("settings"), catalog.text("设置", "Settings"))
                        .enabled(false),
                );
            });
        });
}

fn controls_catalog(catalog: &mut Catalog, ui: &mut Ui) {
    section_header(
        ui,
        catalog.text("交互状态", "Interaction states"),
        Some(catalog.text(
            "所有生产控件使用同一状态解析。",
            "All production controls use one state resolver.",
        )),
    );
    for state in [
        Interaction::Rest,
        Interaction::Hovered,
        Interaction::Pressed,
        Interaction::Selected,
        Interaction::Focused,
        Interaction::Disabled,
    ] {
        ui.horizontal_wrapped(|ui| {
            ui.label(typography(
                format!("{state:?}"),
                TypographyRole::Mono,
                COLORS.muted.into(),
            ));
            nav_item(
                ui,
                NavItemSpec::new(Id::new(("state-nav", format!("{state:?}"))), "Nav")
                    .preview_interaction(state),
            );
            for (label, role) in [
                ("Primary", ControlRole::Primary),
                ("Secondary", ControlRole::Secondary),
                ("Danger", ControlRole::Danger),
            ] {
                control_button(ui, ButtonSpec::new(label, role).preview_interaction(state));
            }
            moqcast_ui::icon_button(
                ui,
                IconButtonSpec::new("⋯", "More").preview_interaction(state),
            );
            player_icon_button(
                ui,
                IconButtonSpec::player("⛶", "Fullscreen").preview_interaction(state),
            );
            let mut switch_value = state == Interaction::Selected;
            switch(
                ui,
                &mut switch_value,
                SwitchSpec::new("Switch").preview_interaction(state),
            );
            let mut checkbox_value = state == Interaction::Selected;
            checkbox(
                ui,
                &mut checkbox_value,
                CheckboxSpec::new("Checkbox").preview_interaction(state),
            );
            let options = ["Auto", "Compatible"];
            let mut selected = 0;
            let spec = SelectSpec::new(
                Id::new(("state-select", format!("{state:?}"))),
                "Select",
                &options,
            )
            .expect("fixture options are not empty")
            .preview_interaction(state);
            select(ui, &mut selected, spec).expect("fixture selection stays valid");
        });
        ui.add_space(Spacing::SM);
    }
    ui.horizontal_wrapped(|ui| {
        switch(
            ui,
            &mut catalog.switch_value,
            SwitchSpec::new("Detailed logging").preview_interaction(Interaction::Hovered),
        );
        checkbox(
            ui,
            &mut catalog.checkbox_value,
            CheckboxSpec::new("Remember choice").preview_interaction(Interaction::Focused),
        );
        let options = [
            catalog.text("自动", "Auto"),
            catalog.text("兼容", "Compatible"),
        ];
        let spec = SelectSpec::new(Id::new("mode"), catalog.text("模式", "Mode"), &options)
            .expect("fixture options are not empty")
            .preview_interaction(Interaction::Pressed);
        select(ui, &mut catalog.select_value, spec).expect("fixture selection stays valid");
        if primary_button(ui, catalog.text("打开对话框", "Open dialog"), true).clicked() {
            catalog.dialog_open = true;
        }
    });
}

fn rows_catalog(catalog: &Catalog, ui: &mut Ui) {
    section_header(ui, catalog.text("行与状态", "Rows and status"), None);
    setting_row(
        ui,
        SettingRowSpec::new(catalog.text("详细日志", "Detailed logging")).description(
            catalog.text(
                "记录本地诊断，不自动上传。",
                "Stores local diagnostics without upload.",
            ),
        ),
        |ui| {
            let mut value = true;
            switch(ui, &mut value, SwitchSpec::new("Detailed logging"));
        },
    );
    let (_, ()) = device_row(
        ui,
        DeviceRowSpec::new(
            Id::new("device"),
            catalog.text("客厅设备", "Living room device"),
        )
        .detail(catalog.text("在线，仅表示已发现", "Online, discovery presence only"))
        .selected(true),
        |ui| {
            status_badge(ui, catalog.text("在线", "Online"), BadgeTone::Info);
        },
    );
}

fn device_list_catalog(catalog: &mut Catalog, ui: &mut Ui) {
    section_header(
        ui,
        catalog.text("设备列表审阅矩阵", "Device-list review matrix"),
        Some(catalog.text(
            "仅用于共享组件静态审阅，不代表运行时发现或连接状态。",
            "Static review fixture only. It does not represent runtime discovery or connection state.",
        )),
    );

    fixture_label(ui, catalog.text("空列表", "Empty list"));
    let empty: [DeviceListItemSpec<'_, &'static str>; 0] = [];
    let _ = device_list(
        ui,
        DeviceListSpec::new(Id::new("catalog-device-list-empty"), &empty),
    );
    ui.label(typography(
        catalog.text("没有分配设备行。", "No device rows allocated."),
        TypographyRole::Help,
        COLORS.muted.into(),
    ));
    ui.add_space(Spacing::LG);

    fixture_label(ui, catalog.text("单项", "Single item"));
    let single = [DeviceListItemSpec::new(
        "review-single",
        catalog.text("客厅设备", "Living room device"),
    )
    .subtitle(catalog.text("设备 ID：A1B2C3D4", "Device ID: A1B2C3D4"))
    .badge(DeviceBadgeSpec::new(
        catalog.text("已发现", "Found"),
        BadgeTone::Neutral,
    ))];
    let _ = device_list(
        ui,
        DeviceListSpec::new(Id::new("catalog-device-list-single"), &single),
    );
    ui.add_space(Spacing::LG);

    fixture_label(
        ui,
        catalog.text(
            "多项、同名与混合状态",
            "Multiple, duplicate, and mixed states",
        ),
    );
    let mixed = [
        DeviceListItemSpec::new("review-a", catalog.text("会议室", "Meeting room"))
            .subtitle(catalog.text("设备 ID：7F2A104C", "Device ID: 7F2A104C"))
            .badge(DeviceBadgeSpec::new(
                catalog.text("可观看", "Watchable"),
                BadgeTone::Info,
            ))
            .selected(catalog.selected_device == "review-a")
            .preview_interaction(Interaction::Hovered),
        DeviceListItemSpec::new("review-b", catalog.text("会议室", "Meeting room"))
            .subtitle(catalog.text("设备 ID：9C4D22E1", "Device ID: 9C4D22E1"))
            .badge(DeviceBadgeSpec::new(
                catalog.text("已连接", "Connected"),
                BadgeTone::Info,
            ))
            .selected(catalog.selected_device == "review-b"),
        DeviceListItemSpec::new(
            "review-c",
            catalog.text(
                "用于跨设备联调与演示的超长中文设备名称",
                "A very long interoperability review workstation name",
            ),
        )
        .subtitle(catalog.text("设备 ID：D81E5A60", "Device ID: D81E5A60"))
        .badge(DeviceBadgeSpec::new(
            catalog.text("已发现", "Found"),
            BadgeTone::Neutral,
        ))
        .selected(catalog.selected_device == "review-c")
        .preview_interaction(Interaction::Focused),
        DeviceListItemSpec::new("review-d", catalog.text("离线设备", "Unavailable device"))
            .subtitle(catalog.text("设备 ID：11AA22BB", "Device ID: 11AA22BB"))
            .badge(DeviceBadgeSpec::new(
                catalog.text("不可用", "Unavailable"),
                BadgeTone::Warning,
            ))
            .enabled(false),
    ];
    if let Some(selected) = device_list(
        ui,
        DeviceListSpec::new(Id::new("catalog-device-list-mixed"), &mixed),
    ) {
        catalog.selected_device = selected;
    }
    ui.add_space(Spacing::LG);

    fixture_label(
        ui,
        catalog.text("窄列与七项滚动", "Narrow column and seven-item scrolling"),
    );
    ui.allocate_ui_with_layout(
        vec2(ui.available_width().min(240.0), 0.0),
        Layout::top_down(Align::Min),
        |ui| {
            let ids = [
                "scroll-1", "scroll-2", "scroll-3", "scroll-4", "scroll-5", "scroll-6", "scroll-7",
            ];
            let items = ids.map(|id| {
                DeviceListItemSpec::new(id, catalog.text("滚动审阅设备", "Scrolling review device"))
                    .subtitle(catalog.text("设备 ID：FIXTURE", "Device ID: FIXTURE"))
                    .badge(DeviceBadgeSpec::new(
                        catalog.text("已发现", "Found"),
                        BadgeTone::Neutral,
                    ))
            });
            let _ = device_list(
                ui,
                DeviceListSpec::new(Id::new("catalog-device-list-scroll"), &items),
            );
        },
    );
}

fn fixture_label(ui: &mut Ui, label: &str) {
    ui.label(typography(label, TypographyRole::Meta, COLORS.muted.into()));
}

fn player_catalog(catalog: &Catalog, ui: &mut Ui) {
    section_header(ui, catalog.text("直播播放器", "Live player"), None);
    let _ = player_surface(
        ui,
        |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(typography(
                    catalog.text(
                        "16:9 舞台 · contain · 允许黑边",
                        "16:9 stage · contain · letterbox allowed",
                    ),
                    TypographyRole::Body,
                    COLORS.player_muted.into(),
                ));
            });
        },
        |ui| {
            status_badge(ui, "LIVE", BadgeTone::Danger);
            ui.label(typography(
                "moqcast.screen/device",
                TypographyRole::Mono,
                COLORS.player_text.into(),
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                player_icon_button(
                    ui,
                    IconButtonSpec::player("⛶", catalog.text("全屏", "Fullscreen")),
                )
                .on_hover_text(catalog.text("全屏", "Fullscreen"));
                player_button(ui, catalog.text("停止观看", "Stop watching"), true);
            });
        },
    );
}

fn state_catalog(catalog: &Catalog, ui: &mut Ui) {
    state_panel(
        ui,
        StatePanelSpec::new(
            StatePanelKind::Empty,
            catalog.text("未发现设备", "No devices found"),
            catalog.text(
                "开始扫描以查找局域网设备。",
                "Start scanning to find LAN devices.",
            ),
        ),
        |ui| {
            primary_button(ui, catalog.text("开始扫描", "Start scan"), true);
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_root_consumes_the_available_viewport() {
        egui::__run_test_ui(|ui| {
            ui.set_width(1000.0);
            ui.set_height(700.0);
            let available = ui.available_rect_before_wrap();
            let mut catalog = Catalog::new(ui.ctx());
            let response = catalog_root(&mut catalog, ui);

            assert_eq!(response.rect, available);
        });
    }

    #[test]
    fn catalog_content_uses_available_width_up_to_the_shared_limit() {
        assert_eq!(catalog_content_width(760.0), 760.0);
        assert_eq!(
            catalog_content_width(Size::PAGE_WIDE_MAX + 200.0),
            Size::PAGE_WIDE_MAX
        );
    }
}
