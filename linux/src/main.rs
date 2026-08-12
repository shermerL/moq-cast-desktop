use eframe::egui;
use moq_cast_desktop::app::MoqCastApp;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    if std::env::args()
        .skip(1)
        .any(|argument| argument == "--version")
    {
        println!("MoQCast {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("dev.moq.moqcast.desktop")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([680.0, 520.0]),
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "MoQCast",
        options,
        Box::new(|creation_context| Ok(Box::new(MoqCastApp::new(creation_context)?))),
    )
    .map_err(anyhow::Error::from)
}
