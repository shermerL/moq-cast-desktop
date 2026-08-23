//! Native Windows desktop entry point for MoQCast.

mod app;
mod audio;
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
use runtime::{RuntimeConfig, RuntimeOwner};
use tracing_subscriber::EnvFilter;
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
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("moq_tokio=warn".parse().expect("valid log directive"))
        .add_directive("mdns_sd=warn".parse().expect("valid log directive"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    let runtime = RuntimeOwner::start(RuntimeConfig {
        bind: args.bind,
        node: args.node,
        secret_file: args.secret_file,
    })?;
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([680.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "MoQCast",
        options,
        Box::new(move |context| Ok(Box::new(app::MoqCastApp::new(context, runtime)))),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}
