//! Native Windows desktop entry point for MoQCast.

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod audio;
mod build_info;
mod diagnostics;
mod media;
mod playback;
mod player;
mod registry;
mod remote;
mod runtime;
mod screen_path;
mod session;

use std::{net::SocketAddr, path::PathBuf};

use anyhow::Result;
use clap::Parser;
use moqcast_diagnostics::{BuildInfo, Config, Paths};
use runtime::{RuntimeConfig, RuntimeOwner};
use url::Url;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Address for the direct MoQ/QUIC listener.
    #[arg(long, default_value = "[::]:0")]
    bind: SocketAddr,

    /// Canonical node URL advertised instead of address-derived candidates.
    #[arg(long)]
    node: Option<Url>,

    /// File containing a 32-byte LAN secret as 64 hexadecimal characters.
    #[arg(long, value_name = "PATH")]
    secret_file: Option<PathBuf>,
}

fn main() -> Result<()> {
    let build = BuildInfo::new(env!("CARGO_PKG_VERSION"))
        .with_build_identity(option_env!("MOQCAST_BUILD_IDENTITY").unwrap_or("local"))
        .with_source_identity(option_env!("MOQCAST_SOURCE_COMMIT").unwrap_or("unknown"))
        .with_dependency_identity(build_info::dependency_identity());
    let diagnostics_config = match Paths::discover() {
        Ok(paths) => Config::new(paths, build),
        Err(error) => {
            eprintln!("MoQCast diagnostics path unavailable: {error}");
            Config::without_file(build, error.to_string())
        }
    };
    let diagnostics = moqcast_diagnostics::init(diagnostics_config);
    let diagnostics_handle = diagnostics.handle();

    let args = Args::parse();
    let runtime = match RuntimeOwner::start(RuntimeConfig {
        bind: args.bind,
        node: args.node,
        secret_file: args.secret_file,
    }) {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(
                stage = "runtime-owner-start",
                error = %error,
                "MoQCast Windows startup failed"
            );
            return Err(error.into());
        }
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([680.0, 480.0]),
        ..Default::default()
    };

    let result = match eframe::run_native(
        "MoQCast",
        options,
        Box::new(move |context| {
            Ok(Box::new(app::MoqCastApp::new(
                context,
                runtime,
                diagnostics_handle,
            )))
        }),
    ) {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::error!(
                stage = "eframe-run-native",
                error = %error,
                "MoQCast Windows startup failed"
            );
            Err(anyhow::anyhow!(error.to_string()))
        }
    };
    drop(diagnostics);
    result
}
