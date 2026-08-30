//! MoQCast macOS application state and runtime ownership.

#[cfg(feature = "app")]
mod app;
mod build_info;
mod contract;
mod runtime;

/// Start the native MoQCast macOS application.
#[cfg(feature = "app")]
pub fn run() -> anyhow::Result<()> {
    use eframe::egui;

    let build = build_info::BuildInfo::current();
    let _ = tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_names(true)
        .try_init();
    tracing::info!(
        stage = "startup",
        version = build.version,
        build_identity = build.build_identity,
        source_identity = build.source_identity,
        dependency_identity = build.dependency_identity,
        minimum_macos = build_info::MINIMUM_MACOS,
        "MoQCast macOS starting"
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("dev.moq.moqcast.macos")
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([680.0, 520.0]),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "MoQCast",
        options,
        Box::new(|context| Ok(Box::new(app::MoqCastApp::new(context)?))),
    )
    .map_err(anyhow::Error::from)
}
