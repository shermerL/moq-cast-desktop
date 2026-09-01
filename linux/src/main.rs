use eframe::egui;
use moq_cast_desktop::app::MoqCastApp;
use moqcast_diagnostics::{BuildInfo, Config, Paths};

const APP_ICON_SIZE: u32 = 64;
const APP_ICON_RGBA: &[u8; 64 * 64 * 4] = include_bytes!("../assets/icons/moqcast-window-64.rgba");

fn main() -> anyhow::Result<()> {
    if std::env::args()
        .skip(1)
        .any(|argument| argument == "--version")
    {
        println!("MoQCast {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let build = BuildInfo::new(env!("CARGO_PKG_VERSION"))
        .with_build_identity(option_env!("MOQCAST_BUILD_IDENTITY").unwrap_or("local"))
        .with_source_identity(option_env!("MOQCAST_SOURCE_COMMIT").unwrap_or("unknown"));
    let diagnostics_config = match Paths::discover() {
        Ok(paths) => Config::new(paths, build),
        Err(error) => {
            eprintln!("MoQCast file diagnostics unavailable: {error}");
            Config::without_file(build, error.to_string())
        }
    };
    let diagnostics = moqcast_diagnostics::init(diagnostics_config);
    let diagnostics_handle = diagnostics.handle();
    tracing::info!(
        stage = "startup",
        version = env!("CARGO_PKG_VERSION"),
        "local diagnostics initialized"
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("dev.moq.moqcast.desktop")
            .with_icon(app_icon())
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([680.0, 520.0]),
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "MoQCast",
        options,
        Box::new(move |creation_context| {
            Ok(Box::new(MoqCastApp::new(
                creation_context,
                diagnostics_handle.clone(),
            )?))
        }),
    )
    .map_err(anyhow::Error::from)
}

fn app_icon() -> egui::IconData {
    egui::IconData {
        rgba: APP_ICON_RGBA.to_vec(),
        width: APP_ICON_SIZE,
        height: APP_ICON_SIZE,
    }
}
