//! Windows CLI spike for Luke-compatible MoQ LAN discovery.

mod registry;
mod session;

use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use moq_native::mdns;
use registry::{PeerRegistry, RegistryChange, sanitize_identity};
use session::{SessionFoundation, SessionSubject, TransportUpdate};
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

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("moq_native=warn".parse().expect("valid log directive"))
        .add_directive("mdns_sd=warn".parse().expect("valid log directive"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    let bound = SessionFoundation::bind(args.bind).context("failed to bind MoQ listener")?;
    let advertisement = bound.advertisement().clone();
    let authenticated_discovery = args.secret_file.is_some();
    let mut config = mdns::Config::new(advertisement.addr.port())
        .with_fingerprint(advertisement.fingerprint.clone());
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
    let mut sessions = bound
        .start(discovery.credential().to_owned())
        .await
        .context("failed to start MoQ listener")?;
    tracing::info!(
        local_id = %sanitize_identity(discovery.id()),
        bind = %sessions.advertisement().addr,
        "LAN discovery and session listener started"
    );

    let result = loop {
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("failed to listen for Ctrl+C")?;
                tracing::info!("LAN discovery and sessions stopping");
                break Ok(());
            }
            event = discovery.recv() => {
                let Some(event) = event else {
                    break Err(anyhow::anyhow!("LAN discovery stopped unexpectedly"));
                };
                match event {
                    mdns::Event::Found(peer) => {
                        let should_dial = discovery.should_dial(&peer.id);
                        let change = registry.found(&peer, should_dial);
                        let should_connect = should_dial
                            && matches!(change, RegistryChange::Added(_) | RegistryChange::Updated(_));
                        log_change(change);
                        if should_connect {
                            match sessions.connect(&peer).await {
                                Ok(update) => log_transport(update),
                                Err(error) => tracing::warn!(
                                    peer = %sanitize_identity(&peer.id),
                                    stage = "transport",
                                    %error,
                                    "peer dial was not started"
                                ),
                            }
                        }
                    }
                    mdns::Event::Lost(id) => {
                        log_change(registry.lost(&id));
                        if let Some(update) = sessions.disconnect(&id).await {
                            log_transport(update);
                        }
                    }
                    _ => {}
                }
            }
            update = sessions.recv() => {
                let Some(update) = update else {
                    break Err(anyhow::anyhow!("MoQ session runtime stopped unexpectedly"));
                };
                log_transport(update);
            }
        }
    };

    sessions.shutdown().await;
    result
}

fn log_transport(update: TransportUpdate) {
    match update.subject {
        SessionSubject::Peer(peer) => tracing::info!(
            peer = %sanitize_identity(&peer),
            direction = ?update.state.direction(),
            phase = ?update.state.phase(),
            generation = update.state.generation(),
            stage = "transport",
            "peer transport state changed"
        ),
        SessionSubject::Inbound(id) => tracing::info!(
            inbound_id = id,
            direction = ?update.state.direction(),
            phase = ?update.state.phase(),
            generation = update.state.generation(),
            stage = "transport",
            "inbound transport state changed"
        ),
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
