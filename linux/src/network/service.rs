//! Runtime-owned mDNS advertisement, discovery, and QUIC listener tasks.

use std::time::Duration;

use moq_tokio::mdns;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};

use super::{discovery::PeerRecord, server};

const INITIAL_SCAN_WINDOW: Duration = Duration::from_secs(3);

pub(crate) struct Event {
    pub(crate) generation: u64,
    pub(crate) kind: EventKind,
}

pub(crate) enum EventKind {
    Found { peer: PeerRecord, should_dial: bool },
    Lost(String),
    InitialScanFinished,
    DiscoveryStopped,
    ListenerStopped,
    Inbound(moq_tokio::server::Request),
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
    listener_task: Option<JoinHandle<()>>,
    listener_stop: Option<oneshot::Sender<()>>,
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
        let app = "moqcast".parse::<mdns::App>().expect("valid LAN app name");
        let discovery = mdns::Config::new(app, port)
            .with_fingerprint(fingerprint)
            .advertise()
            .await?;
        let local_id = discovery.id().to_owned();
        let credential = discovery.credential().to_owned();
        let listener = server.listen().await?;
        tracing::info!(
            stage = "discovery",
            listen_port = port,
            "Nearby listener and discovery ready"
        );

        Ok(Self {
            local_id,
            credential,
            discovery: Some(discovery),
            listener: Some(listener),
            _tasks: JoinSet::new(),
            listener_task: None,
            listener_stop: None,
        })
    }

    pub(crate) async fn shutdown(mut self) -> Result<(), String> {
        let mut errors = crate::cleanup::Errors::default();
        if let Some(stop) = self.listener_stop.take() {
            let _ = stop.send(());
        }
        self._tasks.abort_all();
        while let Some(result) = self._tasks.join_next().await {
            errors.join("discovery worker", result, true);
        }
        if let Some(task) = self.listener_task.take() {
            errors.join("listener", task.await, false);
        }
        if let Some(listener) = self.listener.take() {
            listener.close().await;
        }
        // Discovery Drop requests daemon shutdown; it has no awaitable completion API.
        self.discovery = None;
        errors.finish()
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
        let (stop, stopped) = oneshot::channel();
        self.listener_stop = Some(stop);
        self.listener_task = Some(tokio::spawn(run_listener(
            generation,
            listener,
            events.clone(),
            stopped,
        )));
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
                peer: PeerRecord::from_mdns(peer),
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
    mut stopped: oneshot::Receiver<()>,
) {
    let unexpected = loop {
        let request = tokio::select! {
            biased;
            _ = &mut stopped => break false,
            request = listener.accept() => match request {
                Some(request) => request,
                None => break true,
            },
        };
        tokio::select! {
            biased;
            _ = &mut stopped => break false,
            result = events.send(Event { generation, kind: EventKind::Inbound(request) }) => {
                if result.is_err() { break false; }
            }
        }
    };
    // Dropping Listener only requests close. Keep its owner alive through socket release.
    listener.close().await;
    if unexpected {
        tokio::select! {
            biased;
            _ = &mut stopped => {},
            _ = events.send(Event { generation, kind: EventKind::ListenerStopped }) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_collects_worker_failures_and_releases_socket() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut config = moq_tokio::listen::Config::default();
        config.bind = Some(moq_tokio::listen::Bind::Addr(
            "127.0.0.1:0".parse().unwrap(),
        ));
        config.tls.generate = vec!["test".into()];
        let server = config.clone().init(Default::default()).unwrap();
        let addr = server.local_addr().unwrap();
        let mut services = Services {
            local_id: "test".into(),
            credential: "test".into(),
            discovery: None,
            listener: Some(server.listen().await.unwrap()),
            _tasks: JoinSet::new(),
            listener_task: Some(tokio::spawn(async {
                panic!("listener failure");
            })),
            listener_stop: None,
        };
        let (ready, done) = oneshot::channel();
        services._tasks.spawn(async move {
            let _ = ready.send(());
            panic!("discovery failure");
        });
        done.await.unwrap();
        let error = services.shutdown().await.unwrap_err();
        assert!(error.contains("discovery worker"));
        assert!(error.contains("listener"));
        config.bind = Some(moq_tokio::listen::Bind::Addr(addr));
        config
            .init(Default::default())
            .unwrap()
            .listen()
            .await
            .unwrap()
            .close()
            .await;
    }

    #[tokio::test]
    async fn nearby_audit_shutdown_releases_listener_socket() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut config = moq_tokio::listen::Config::default();
        config.bind = Some(moq_tokio::listen::Bind::Addr(
            "127.0.0.1:0".parse().unwrap(),
        ));
        config.tls.generate = vec!["test".into()];
        for active in [false, true] {
            let server = config.clone().init(Default::default()).unwrap();
            let addr = server.local_addr().unwrap();
            let mut services = Services {
                local_id: "test".into(),
                credential: "test".into(),
                discovery: None,
                listener: Some(server.listen().await.unwrap()),
                _tasks: JoinSet::new(),
                listener_task: None,
                listener_stop: None,
            };
            let (events, _receiver) = mpsc::channel(1);
            events
                .try_send(Event {
                    generation: 1,
                    kind: EventKind::InitialScanFinished,
                })
                .unwrap();
            if active {
                let (stop, stopped) = oneshot::channel();
                services.listener_stop = Some(stop);
                services.listener_task = Some(tokio::spawn(run_listener(
                    1,
                    services.listener.take().unwrap(),
                    events,
                    stopped,
                )));
            }
            services.shutdown().await.unwrap();
            config.bind = Some(moq_tokio::listen::Bind::Addr(addr));
            let rebound = config
                .clone()
                .init(Default::default())
                .expect("shutdown must release the listener socket");
            rebound.listen().await.unwrap().close().await;
        }
    }
}
