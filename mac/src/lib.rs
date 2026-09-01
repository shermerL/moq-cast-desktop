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
    use std::time::Duration;

    use eframe::egui;
    use moqcast_diagnostics::{BuildInfo, Config, Paths};

    let build = BuildInfo::new(env!("CARGO_PKG_VERSION"));
    let diagnostics_config = match Paths::discover() {
        Ok(paths) => Config::new(paths, build),
        Err(error) => {
            eprintln!("MoQCast diagnostics path unavailable: {error}");
            Config::without_file(build, error.to_string())
        }
    }
    .with_file_retention(Duration::from_secs(7 * 24 * 60 * 60))
    .with_owner_only_file_permissions()
    .with_minimal_export_metadata()
    .with_private_location_redaction();
    let diagnostics = moqcast_diagnostics::init(diagnostics_config);
    let diagnostics_handle = diagnostics.handle();
    tracing::info!(
        stage = "startup",
        version = env!("CARGO_PKG_VERSION"),
        "local diagnostics initialized"
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("dev.moq.moqcast.macos")
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([680.0, 520.0]),
        persist_window: true,
        ..Default::default()
    };
    let result = eframe::run_native(
        "MoQCast",
        options,
        Box::new(move |context| {
            Ok(Box::new(app::MoqCastApp::new(
                context,
                diagnostics_handle.clone(),
            )?))
        }),
    )
    .map_err(anyhow::Error::from);
    if let Err(error) = &result {
        tracing::error!(stage = "application", %error, "native application exited with an error");
    }
    drop(diagnostics);
    result
}
