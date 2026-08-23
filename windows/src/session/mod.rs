//! Private orchestration and lifecycle for direct MoQ peer sessions.

mod peer;
mod security;
mod server;
mod state;

use std::{collections::BTreeMap, net::SocketAddr};

use moq_tokio::mdns;
use tokio::{sync::mpsc, task::JoinHandle};

pub(crate) use peer::DialError;
pub(crate) use server::{Advertisement, BoundServer, StartError};
pub(crate) use state::{TransportDirection, TransportPhase, TransportState};

use state::PeerStates;

const EVENT_CAPACITY: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionSubject {
    Peer(String),
    Inbound(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TransportUpdate {
    pub(crate) subject: SessionSubject,
    pub(crate) state: TransportState,
}

pub(super) enum RuntimeEvent {
    Outbound {
        peer: String,
        generation: u64,
        phase: TransportPhase,
    },
    Inbound {
        id: u64,
        phase: TransportPhase,
    },
    ListenerStopped,
}

struct Outbound {
    connection: moq_tokio::Connection,
    task: JoinHandle<()>,
}

pub(crate) struct SessionFoundation {
    advertisement: Advertisement,
    origin: moq_tokio::moq_net::origin::Producer,
    origin_driver: JoinHandle<()>,
    events: mpsc::Sender<RuntimeEvent>,
    recv: mpsc::Receiver<RuntimeEvent>,
    listener_task: Option<JoinHandle<()>>,
    outbound: BTreeMap<String, Outbound>,
    states: PeerStates,
}

impl SessionFoundation {
    pub(crate) fn bind(bind: SocketAddr) -> Result<BoundServer, StartError> {
        server::bind(bind)
    }

    pub(crate) fn advertisement(&self) -> &Advertisement {
        &self.advertisement
    }

    pub(crate) fn origin(&self) -> &moq_tokio::moq_net::origin::Producer {
        &self.origin
    }

    pub(crate) async fn connect(
        &mut self,
        peer: &mdns::Peer,
    ) -> Result<TransportUpdate, DialError> {
        self.connect_record(&peer.id, &peer::PeerRecord::from(peer))
            .await
    }

    async fn connect_record(
        &mut self,
        peer_id: &str,
        record: &peer::PeerRecord,
    ) -> Result<TransportUpdate, DialError> {
        self.stop_outbound(peer_id).await;
        let state = self.states.begin(peer_id);
        let connection = match peer::dial(record, self.origin.clone()) {
            Ok(connection) => connection,
            Err(error) => {
                self.states
                    .transition(peer_id, state.generation(), TransportPhase::Failed);
                return Err(error);
            }
        };
        let observed = connection.clone();
        let events = self.events.clone();
        let peer_id = peer_id.to_owned();
        let observed_peer = peer_id.clone();
        let generation = state.generation();
        let task = tokio::spawn(async move {
            match observed.established().await {
                Ok(connection) => {
                    let _ = events
                        .send(RuntimeEvent::Outbound {
                            peer: observed_peer.clone(),
                            generation,
                            phase: TransportPhase::Connected,
                        })
                        .await;
                    let _ = connection.closed().await;
                    let _ = events
                        .send(RuntimeEvent::Outbound {
                            peer: observed_peer,
                            generation,
                            phase: TransportPhase::Disconnected,
                        })
                        .await;
                }
                Err(_) => {
                    let _ = events
                        .send(RuntimeEvent::Outbound {
                            peer: observed_peer,
                            generation,
                            phase: TransportPhase::Failed,
                        })
                        .await;
                }
            }
        });
        self.outbound
            .insert(peer_id.clone(), Outbound { connection, task });
        Ok(TransportUpdate {
            subject: SessionSubject::Peer(peer_id),
            state,
        })
    }

    pub(crate) async fn disconnect(&mut self, peer: &str) -> Option<TransportUpdate> {
        self.stop_outbound(peer).await;
        let state = self.states.disconnect(peer)?;
        Some(TransportUpdate {
            subject: SessionSubject::Peer(peer.to_owned()),
            state,
        })
    }

    pub(crate) async fn recv(&mut self) -> Option<TransportUpdate> {
        loop {
            match self.recv.recv().await? {
                RuntimeEvent::Outbound {
                    peer,
                    generation,
                    phase,
                } => {
                    let Some(state) = self.states.transition(&peer, generation, phase) else {
                        continue;
                    };
                    return Some(TransportUpdate {
                        subject: SessionSubject::Peer(peer),
                        state,
                    });
                }
                RuntimeEvent::Inbound { id, phase } => {
                    return Some(TransportUpdate {
                        subject: SessionSubject::Inbound(id),
                        state: TransportState::inbound(id, phase),
                    });
                }
                RuntimeEvent::ListenerStopped => return None,
            }
        }
    }

    pub(crate) async fn shutdown(mut self) {
        let peers: Vec<_> = self.outbound.keys().cloned().collect();
        for peer in peers {
            self.stop_outbound(&peer).await;
            self.states.disconnect(&peer);
        }
        if let Some(task) = self.listener_task.take() {
            task.abort();
            let _ = task.await;
        }
        drop(self.origin);
        let _ = self.origin_driver.await;
    }

    async fn stop_outbound(&mut self, peer: &str) {
        let Some(outbound) = self.outbound.remove(peer) else {
            return;
        };
        outbound.connection.close();
        let _ = outbound.task.await;
    }

    #[cfg(test)]
    fn state(&self, peer: &str) -> Option<TransportState> {
        self.states.get(peer)
    }
}

impl BoundServer {
    pub(crate) async fn start(self, credential: String) -> Result<SessionFoundation, StartError> {
        let advertisement = self.advertisement().clone();
        let listener = self.listen().await?;
        let (origin, origin_driver) = moq_tokio::moq_net::origin::Producer::new(
            moq_tokio::moq_net::origin::Info::new(moq_tokio::moq_net::Origin::random()),
        );
        let origin_driver = tokio::spawn(origin_driver);
        let (events, recv) = mpsc::channel(EVENT_CAPACITY);
        let listener_task = tokio::spawn(server::run_listener(
            listener,
            credential,
            origin.clone(),
            events.clone(),
        ));
        Ok(SessionFoundation {
            advertisement,
            origin,
            origin_driver,
            events,
            recv,
            listener_task: Some(listener_task),
            outbound: BTreeMap::new(),
            states: PeerStates::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{SessionFoundation, SessionSubject, TransportPhase, peer::PeerRecord};

    #[tokio::test]
    async fn disconnect_closes_current_generation() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let bound = SessionFoundation::bind("127.0.0.1:0".parse().expect("bind")).expect("bound");
        let advertised = bound.advertisement().clone();
        let mut foundation = bound.start("proof".to_owned()).await.expect("started");
        let peer = PeerRecord {
            urls: vec![
                format!("moqt://{}", advertised.addr)
                    .parse()
                    .expect("candidate"),
            ],
            has_node: false,
            fingerprint: Some(advertised.fingerprint),
            credential: "proof".to_owned(),
        };
        let connecting = foundation
            .connect_record("remote", &peer)
            .await
            .expect("dial");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let update = foundation.recv().await.expect("runtime");
                if matches!(&update.subject, SessionSubject::Peer(id) if id == "remote")
                    && update.state.phase() == TransportPhase::Connected
                {
                    break;
                }
            }
        })
        .await
        .expect("connected");

        let disconnected = foundation.disconnect("remote").await.expect("known peer");
        assert_eq!(disconnected.state.phase(), TransportPhase::Disconnected);
        assert!(disconnected.state.generation() > connecting.state.generation());
        assert_eq!(foundation.state("remote"), Some(disconnected.state));
        foundation.shutdown().await;
    }
}
