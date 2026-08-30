//! Runtime-owned mDNS advertisement, discovery, and QUIC listener tasks.

use std::time::Duration;

use moq_tokio::mdns;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::server;

const INITIAL_SCAN_WINDOW: Duration = Duration::from_secs(3);

pub(crate) struct Event {
    pub(crate) generation: u64,
    pub(crate) kind: EventKind,
}

pub(crate) enum EventKind {
    Found { peer: mdns::Peer, should_dial: bool },
    Lost(String),
    InitialScanFinished,
    DiscoveryStopped,
    ListenerStopped,
    Inbound(moq_tokio::Request),
}

#[derive(Debug, Error)]
pub(crate) enum StartError {
    #[error("listener did not provide a certificate fingerprint")]
    MissingFingerprint,
    #[error(transparent)]
    Native(#[from] moq_tokio::Error),
    #[error(transparent)]
    Mdns(#[from] mdns::Error),
}

pub(crate) struct Services {
    pub(crate) local_id: String,
    pub(crate) credential: String,
    discovery: Option<mdns::Discovery>,
    listener: Option<moq_tokio::Listener>,
    _tasks: JoinSet<()>,
}

impl Services {
    pub(crate) async fn start() -> Result<Self, StartError> {
        let server = server::build()?;
        let port = server.local_addr()?.port();
        let fingerprint = server
            .certificates()
            .fingerprints()
            .into_iter()
            .next()
            .ok_or(StartError::MissingFingerprint)?;
        let discovery = mdns::Config::new(port)
            .with_fingerprint(fingerprint)
            .advertise()
            .await?;
        let local_id = discovery.id().to_owned();
        let credential = discovery.credential().to_owned();
        let listener = server.listen().await?;

        Ok(Self {
            local_id,
            credential,
            discovery: Some(discovery),
            listener: Some(listener),
            _tasks: JoinSet::new(),
        })
    }

    pub(crate) fn activate(&mut self, generation: u64, events: mpsc::Sender<Event>) {
        let discovery = self
            .discovery
            .take()
            .expect("discovery services are activated once");
        let listener = self
            .listener
            .take()
            .expect("discovery services are activated once");
        let discovery_events = events.clone();
        self._tasks
            .spawn(run_discovery(generation, discovery, discovery_events));
        self._tasks
            .spawn(run_listener(generation, listener, events.clone()));
        self._tasks.spawn(async move {
            tokio::time::sleep(INITIAL_SCAN_WINDOW).await;
            let _ = events
                .send(Event {
                    generation,
                    kind: EventKind::InitialScanFinished,
                })
                .await;
        });
    }
}

async fn run_discovery(
    generation: u64,
    mut discovery: mdns::Discovery,
    events: mpsc::Sender<Event>,
) {
    while let Some(event) = discovery.recv().await {
        let kind = match event {
            mdns::Event::Found(peer) => EventKind::Found {
                should_dial: discovery.should_dial(&peer.id),
                peer,
            },
            mdns::Event::Lost(id) => EventKind::Lost(id),
            _ => continue,
        };
        if events.send(Event { generation, kind }).await.is_err() {
            return;
        }
    }
    let _ = events
        .send(Event {
            generation,
            kind: EventKind::DiscoveryStopped,
        })
        .await;
}

async fn run_listener(
    generation: u64,
    mut listener: moq_tokio::Listener,
    events: mpsc::Sender<Event>,
) {
    while let Some(request) = listener.accept().await {
        if events
            .send(Event {
                generation,
                kind: EventKind::Inbound(request),
            })
            .await
            .is_err()
        {
            return;
        }
    }
    let _ = events
        .send(Event {
            generation,
            kind: EventKind::ListenerStopped,
        })
        .await;
}
