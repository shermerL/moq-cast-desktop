//! Minimal native shell for macOS foundation milestones.

use std::time::Duration;

use eframe::egui::{self, Align, Layout, RichText};

use crate::build_info::{BuildInfo, MINIMUM_MACOS};
use crate::contract;
use crate::runtime::{AppSnapshot, CapabilityPhase, RuntimeOwner, RuntimePhase};

const STORAGE_LOCALE: &str = "moqcast.macos.locale";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Nearby,
    ScreenShare,
    Settings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Locale {
    #[default]
    Chinese,
    English,
}

impl Locale {
    fn stored(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en",
        }
    }

    fn from_storage(value: Option<String>) -> Self {
        match value.as_deref() {
            Some("en") => Self::English,
            _ => Self::Chinese,
        }
    }
}

pub(crate) struct MoqCastApp {
    page: Page,
    locale: Locale,
    runtime: RuntimeOwner,
    build: BuildInfo,
}

impl MoqCastApp {
    pub(crate) fn new(context: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        context.egui_ctx.set_visuals(egui::Visuals::light());
        let locale = Locale::from_storage(
            context
                .storage
                .and_then(|storage| storage.get_string(STORAGE_LOCALE)),
        );
        let runtime = RuntimeOwner::start()?;
        Ok(Self {
            page: Page::Nearby,
            locale,
            runtime,
            build: BuildInfo::current(),
        })
    }

    fn text(&self, chinese: &'static str, english: &'static str) -> &'static str {
        match self.locale {
            Locale::Chinese => chinese,
            Locale::English => english,
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        let nearby = self.text("附近设备", "Nearby");
        let screen_share = self.text("屏幕共享", "Screen share");
        let settings = self.text("设置", "Settings");
        ui.horizontal(|ui| {
            ui.label(RichText::new("MoQCast").size(20.0).strong());
            ui.add_space(18.0);
            ui.selectable_value(&mut self.page, Page::Nearby, nearby);
            ui.selectable_value(&mut self.page, Page::ScreenShare, screen_share);
            ui.selectable_value(&mut self.page, Page::Settings, settings);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.small(runtime_label(snapshot.runtime.phase()));
            });
        });
    }

    fn nearby(&self, ui: &mut egui::Ui) {
        ui.heading(self.text("附近设备", "Nearby devices"));
        ui.label(self.text(
            "M1 仅建立原生应用与资源所有权。Bonjour、listener 与 direct-only session 将在 M2 接入。",
            "M1 establishes the native app and resource ownership only. Bonjour, the listener, and direct-only sessions arrive in M2.",
        ));
        ui.add_space(12.0);
        ui.group(|ui| {
            ui.label(RichText::new(self.text("跨平台网络契约", "Cross-platform network contract")).strong());
            ui.monospace(format!("service: {}", contract::SERVICE_TYPE));
            ui.monospace(format!(
                "authenticated path: {}",
                contract::cluster_path("<credential>")
            ));
            ui.monospace(format!(
                "screen path: {}",
                contract::screen_path("<peer-id>")
            ));
            ui.label(self.text(
                "发现、transport、媒体与 decoder 保持独立 generation。短暂 Lost 不会拆健康 session。",
                "Discovery, transport, media, and decoder keep independent generations. A transient Lost event will not tear down a healthy session.",
            ));
        });
        ui.add_space(10.0);
        let mut unavailable = false;
        ui.add_enabled(
            false,
            egui::Checkbox::new(
                &mut unavailable,
                self.text("启动附近设备服务（M2）", "Start Nearby services (M2)"),
            ),
        );
    }

    fn screen_share(&self, ui: &mut egui::Ui) {
        ui.heading(self.text("屏幕共享", "Screen share"));
        ui.label(self.text(
            "观看 H.264 属于 M3，发布 ScreenCaptureKit 屏幕属于 M4，系统音频属于 M5。当前骨架不会报告虚假的媒体就绪状态。",
            "H.264 viewing is M3, ScreenCaptureKit publishing is M4, and system audio is M5. This foundation does not report false media readiness.",
        ));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_enabled(
                false,
                egui::Button::new(self.text("观看远端屏幕", "Watch remote screen")),
            );
            ui.add_enabled(
                false,
                egui::Button::new(self.text("共享本机屏幕", "Share this Mac")),
            );
        });
        ui.small(self.text(
            "Stop Share/Watch 只结束媒体所有权，不会拆除健康的 direct-only session。",
            "Stop Share/Watch ends media ownership without tearing down a healthy direct-only session.",
        ));
    }

    fn settings(&mut self, ui: &mut egui::Ui, snapshot: &AppSnapshot) {
        ui.heading(self.text("设置", "Settings"));
        ui.horizontal(|ui| {
            ui.label(self.text("语言", "Language"));
            ui.selectable_value(&mut self.locale, Locale::Chinese, "中文");
            ui.selectable_value(&mut self.locale, Locale::English, "English");
        });
        ui.separator();
        ui.label(RichText::new(self.text("构建来源", "Build provenance")).strong());
        provenance_row(ui, "Version", self.build.version);
        provenance_row(ui, "Build", self.build.build_identity);
        provenance_row(ui, "Source", self.build.source_identity);
        provenance_row(ui, "Target", &self.build.target);
        provenance_row(ui, "MoQ", self.build.moq_baseline);
        provenance_row(ui, "Dependency", self.build.dependency_identity);
        provenance_row(ui, "Minimum macOS", MINIMUM_MACOS);
        ui.separator();
        ui.label(RichText::new(self.text("运行时诊断", "Runtime diagnostics")).strong());
        lifecycle_row(
            ui,
            "runtime",
            snapshot.runtime.generation().value(),
            runtime_label(snapshot.runtime.phase()),
        );
        lifecycle_row(
            ui,
            "discovery",
            snapshot.discovery.generation().value(),
            capability_label(snapshot.discovery.phase()),
        );
        lifecycle_row(
            ui,
            "session",
            snapshot.session.generation().value(),
            capability_label(snapshot.session.phase()),
        );
        lifecycle_row(
            ui,
            "capture",
            snapshot.capture.generation().value(),
            capability_label(snapshot.capture.phase()),
        );
        lifecycle_row(
            ui,
            "decoder",
            snapshot.decoder.generation().value(),
            capability_label(snapshot.decoder.phase()),
        );
        ui.small(format!(
            "event {}: {}",
            snapshot.event_revision, snapshot.last_event
        ));
        if let Some(error) = &snapshot.last_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.small(self.text(
            "M1 diagnostics 使用结构化 stderr 与上述 typed snapshot。持久化 DLOG/export 尚未实现。",
            "M1 diagnostics use structured stderr and the typed snapshot above. Persistent DLOG/export is not implemented yet.",
        ));
    }
}

impl eframe::App for MoqCastApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let snapshot = self.runtime.snapshot();
        if snapshot.runtime.phase() == RuntimePhase::Starting {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            self.top_bar(ui, &snapshot);
            ui.separator();
            ui.add_space(16.0);
            match self.page {
                Page::Nearby => self.nearby(ui),
                Page::ScreenShare => self.screen_share(ui),
                Page::Settings => self.settings(ui, &snapshot),
            }
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_LOCALE, self.locale.stored().to_owned());
    }
}

fn runtime_label(phase: RuntimePhase) -> &'static str {
    match phase {
        RuntimePhase::Starting => "Starting",
        RuntimePhase::Ready => "Foundation ready",
        RuntimePhase::Stopped => "Stopped",
    }
}

fn capability_label(phase: CapabilityPhase) -> &'static str {
    match phase {
        CapabilityPhase::Unavailable => "Unavailable in M1",
    }
}

fn provenance_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{label}:"));
        ui.monospace(value);
    });
}

fn lifecycle_row(ui: &mut egui::Ui, name: &str, generation: u64, phase: &str) {
    ui.monospace(format!("{name}: {phase} · generation {generation}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_locale_defaults_to_chinese_and_accepts_english() {
        assert_eq!(Locale::from_storage(None), Locale::Chinese);
        assert_eq!(Locale::from_storage(Some("en".to_owned())), Locale::English);
        assert_eq!(Locale::English.stored(), "en");
    }
}
