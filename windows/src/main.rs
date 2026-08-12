//! Windows CLI spike for Luke-compatible MoQ LAN discovery.

mod registry;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use moq_native::mdns;
use registry::{PeerRegistry, RegistryChange, sanitize_identity};
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Port of the MoQ listener represented by this advertisement.
    #[arg(long)]
    port: u16,

    /// SHA-256 certificate fingerprint advertised for TLS pinning.
    #[arg(long)]
    fingerprint: Option<String>,

    /// Canonical node URL advertised instead of address-derived candidates.
    #[arg(long)]
    node: Option<Url>,

    /// File containing a 32-byte LAN secret as 64 hexadecimal characters.
    #[arg(long, value_name = "PATH")]
    secret_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("moq_native=warn".parse().expect("valid log directive"))
        .add_directive("mdns_sd=warn".parse().expect("valid log directive"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    let authenticated_discovery = args.secret_file.is_some();
    let mut config = mdns::Config::new(args.port);
    if let Some(fingerprint) = args.fingerprint {
        config = config.with_fingerprint(fingerprint);
    }
    if let Some(node) = args.node {
        config = config.with_node(node);
    }
    if let Some(path) = args.secret_file {
        let secret = mdns::Secret::load(path.to_string_lossy().as_ref())
            .with_context(|| format!("failed to load LAN secret from {}", path.display()))?;
        config = config.with_secret(secret);
    } else {
        tracing::warn!("discovery is open and does not authenticate LAN membership");
    }

    let mut discovery = config
        .advertise()
        .await
        .context("failed to start LAN discovery")?;
    let mut registry = PeerRegistry::new(discovery.id(), authenticated_discovery);
    tracing::info!(local_id = %sanitize_identity(discovery.id()), port = args.port, "LAN discovery started");

    loop {
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("failed to listen for Ctrl+C")?;
                tracing::info!("LAN discovery stopping");
                return Ok(());
            }
            event = discovery.recv() => {
                let Some(event) = event else {
                    anyhow::bail!("LAN discovery stopped unexpectedly");
                };
                let change = match event {
                    mdns::Event::Found(peer) => {
                        let should_dial = discovery.should_dial(&peer.id);
                        registry.found(&peer, should_dial)
                    }
                    mdns::Event::Lost(id) => registry.lost(&id),
                    _ => continue,
                };
                log_change(change);
            }
        }
    }
}

fn log_change(change: RegistryChange) {
    match change {
        RegistryChange::Added(peer) => tracing::info!(
            peer = %peer.id,
            candidates = ?peer.candidates,
            should_dial = peer.should_dial,
            authenticated_discovery = peer.authenticated_discovery,
            tls_pinned = peer.tls_pinned,
            "peer found"
        ),
        RegistryChange::Updated(peer) => tracing::info!(
            peer = %peer.id,
            candidates = ?peer.candidates,
            should_dial = peer.should_dial,
            authenticated_discovery = peer.authenticated_discovery,
            tls_pinned = peer.tls_pinned,
            "peer updated"
        ),
        RegistryChange::Removed { id } => tracing::info!(peer = %id, "peer lost"),
        RegistryChange::Unchanged | RegistryChange::IgnoredSelf => {}
    }
}
