//! MoQCast macOS application state and runtime ownership.

#[cfg(feature = "app")]
mod app;
#[cfg(test)]
mod build_info;
mod contract;
#[cfg(feature = "network")]
mod network;
#[cfg(feature = "watch")]
mod playback;
#[cfg(feature = "publish")]
mod publication;
#[cfg(feature = "network")]
mod remote;
#[cfg(feature = "watch")]
mod runtime;

/// Start the native MoQCast macOS application.
#[cfg(feature = "app")]
pub fn run() -> anyhow::Result<()> {
    use eframe::egui;

    let _ = tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_names(true)
        .try_init();
    tracing::info!(
        stage = "startup",
        version = env!("CARGO_PKG_VERSION"),
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
