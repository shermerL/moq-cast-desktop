//! Direct MoQ peer-session ownership and lifecycle.

mod peer;
mod server;
mod state;

use std::{collections::BTreeMap, net::SocketAddr};

use tokio::{sync::mpsc, task::JoinHandle};

pub(crate) use peer::DialError;
pub(crate) use server::{BoundServer, StartError};
pub(crate) use state::{TransportDirection, TransportPhase, TransportState};

use self::state::PeerStates;
use crate::network::discovery::PeerRecord;

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

pub(crate) enum SessionEvent {
    Transport(TransportUpdate),
    ListenerStopped,
}

enum RuntimeEvent {
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

#[derive(Clone)]
struct SessionOrigins {
    publish: moq_tokio::moq_net::origin::Producer,
    receive: moq_tokio::moq_net::origin::Producer,
}

struct OriginDrivers {
    publish: JoinHandle<()>,
    receive: JoinHandle<()>,
}

pub(crate) struct SessionFoundation {
    origins: SessionOrigins,
    origin_drivers: OriginDrivers,
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

    pub(crate) fn receive_origin(&self) -> moq_tokio::moq_net::origin::Producer {
        self.origins.receive.clone()
    }

    pub(crate) fn publish_origin(&self) -> moq_tokio::moq_net::origin::Producer {
        self.origins.publish.clone()
    }

    pub(crate) async fn connect(
        &mut self,
        peer: &PeerRecord,
    ) -> Result<TransportUpdate, DialError> {
        self.stop_outbound(&peer.id).await;
        let state = self.states.begin(&peer.id);
        let connection = match peer::dial(peer, self.origins.clone()) {
            Ok(connection) => connection,
            Err(error) => {
                self.states
                    .transition(&peer.id, state.generation(), TransportPhase::Failed);
                return Err(error);
            }
        };
        let observed = connection.clone();
        let events = self.events.clone();
        let peer_id = peer.id.clone();
        let observed_peer = peer_id.clone();
        let generation = state.generation();
        let task = tokio::spawn(async move {
            let phase = match observed.established().await {
                Ok(connection) => {
                    let _ = events
                        .send(RuntimeEvent::Outbound {
                            peer: observed_peer.clone(),
                            generation,
                            phase: TransportPhase::Connected,
                        })
                        .await;
                    let _ = connection.closed().await;
                    TransportPhase::Disconnected
                }
                Err(error) if error.is_auth() => TransportPhase::Rejected,
                Err(_) => TransportPhase::Failed,
            };
            let _ = events
                .send(RuntimeEvent::Outbound {
                    peer: observed_peer,
                    generation,
                    phase,
                })
                .await;
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

    pub(crate) async fn recv(&mut self) -> Option<SessionEvent> {
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
                    return Some(SessionEvent::Transport(TransportUpdate {
                        subject: SessionSubject::Peer(peer),
                        state,
                    }));
                }
                RuntimeEvent::Inbound { id, phase } => {
                    return Some(SessionEvent::Transport(TransportUpdate {
                        subject: SessionSubject::Inbound(id),
                        state: TransportState::inbound(id, phase),
                    }));
                }
                RuntimeEvent::ListenerStopped => return Some(SessionEvent::ListenerStopped),
            }
        }
    }

    pub(crate) async fn shutdown(mut self) {
        let peers = self.outbound.keys().cloned().collect::<Vec<_>>();
        for peer in peers {
            self.stop_outbound(&peer).await;
            self.states.disconnect(&peer);
        }
        if let Some(task) = self.listener_task.take() {
            task.abort();
            let _ = task.await;
        }
        drop(self.origins);
        let _ = self.origin_drivers.publish.await;
        let _ = self.origin_drivers.receive.await;
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
        let listener = self.listen().await?;
        let (publish, publish_driver) = spawn_origin();
        let (receive, receive_driver) = spawn_origin();
        let origins = SessionOrigins { publish, receive };
        let (events, recv) = mpsc::channel(EVENT_CAPACITY);
        let listener_task = tokio::spawn(server::run_listener(
            listener,
            credential,
            origins.clone(),
            events.clone(),
        ));
        Ok(SessionFoundation {
            origins,
            origin_drivers: OriginDrivers {
                publish: publish_driver,
                receive: receive_driver,
            },
            events,
            recv,
            listener_task: Some(listener_task),
            outbound: BTreeMap::new(),
            states: PeerStates::default(),
        })
    }
}

fn spawn_origin() -> (moq_tokio::moq_net::origin::Producer, JoinHandle<()>) {
    let (origin, driver) = moq_tokio::moq_net::origin::Producer::new(
        moq_tokio::moq_net::origin::Info::new(moq_tokio::moq_net::Origin::random()),
    );
    (origin, tokio::spawn(driver))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{SessionEvent, SessionFoundation, SessionSubject, TransportPhase};
    use crate::network::discovery::PeerRecord;

    #[tokio::test]
    async fn disconnect_closes_current_generation() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let bound = SessionFoundation::bind("127.0.0.1:0".parse().expect("bind")).expect("bound");
        let advertised = bound.advertisement().clone();
        let mut foundation = bound.start("proof".to_owned()).await.expect("started");
        let peer = PeerRecord {
            id: "remote".to_owned(),
            urls: vec![
                format!("moqt://{}", advertised.addr)
                    .parse()
                    .expect("candidate"),
            ],
            has_node: false,
            fingerprint: Some(advertised.fingerprint),
            credential: "proof".to_owned(),
        };
        let connecting = foundation.connect(&peer).await.expect("dial");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = foundation.recv().await.expect("runtime");
                let SessionEvent::Transport(update) = event else {
                    panic!("listener stopped");
                };
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
        assert_eq!(foundation.state("remote"), None);
        foundation.shutdown().await;
    }
}
