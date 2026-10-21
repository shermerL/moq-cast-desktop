//! Direct MoQ peer-session ownership and lifecycle.

mod peer;
mod server;
mod state;

use std::{collections::BTreeMap, net::SocketAddr};

use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

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
    listener_shutdown: Option<oneshot::Sender<()>>,
    listener_task: Option<JoinHandle<Result<(), String>>>,
    outbound: BTreeMap<String, Outbound>,
    states: PeerStates,
    cleanup: crate::cleanup::Errors,
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

    #[cfg(test)]
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

    pub(crate) async fn shutdown(mut self) -> Result<(), String> {
        let peers = self.outbound.keys().cloned().collect::<Vec<_>>();
        for peer in peers {
            self.stop_outbound(&peer).await;
            self.states.disconnect(&peer);
        }
        if let Some(task) = self.listener_task.take() {
            if let Some(shutdown) = self.listener_shutdown.take() {
                let _ = shutdown.send(());
            }
            match task.await {
                Ok(result) => self.cleanup.result("listener", result),
                Err(error) => self.cleanup.join("listener", Err(error), false),
            }
        }
        drop(self.origins);
        self.cleanup
            .join("publish origin", self.origin_drivers.publish.await, false);
        self.cleanup
            .join("receive origin", self.origin_drivers.receive.await, false);
        self.cleanup.finish()
    }

    async fn stop_outbound(&mut self, peer: &str) {
        let Some(outbound) = self.outbound.remove(peer) else {
            return;
        };
        outbound.connection.abort(moq_tokio::moq_net::Error::Cancel);
        outbound.task.abort();
        self.cleanup
            .join("outbound observer", outbound.task.await, true);
        // Abort requests shutdown; closed observes the connection loop actually exiting.
        let _ = outbound.connection.closed().await;
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
        let (listener_shutdown, shutdown) = oneshot::channel();
        let listener_task = tokio::spawn(server::run_listener(
            listener,
            credential,
            origins.clone(),
            events.clone(),
            shutdown,
        ));
        Ok(SessionFoundation {
            origins,
            origin_drivers: OriginDrivers {
                publish: publish_driver,
                receive: receive_driver,
            },
            events,
            recv,
            listener_shutdown: Some(listener_shutdown),
            listener_task: Some(listener_task),
            outbound: BTreeMap::new(),
            states: PeerStates::default(),
            cleanup: Default::default(),
        })
    }
}

fn spawn_origin() -> (moq_tokio::moq_net::origin::Producer, JoinHandle<()>) {
    let (origin, driver) =
        moq_tokio::moq_net::origin::Producer::new(moq_tokio::moq_net::origin::Config::default());
    (
        origin,
        tokio::spawn(async move {
            let _ = moq_tokio::moq_net::time::run(driver).await;
        }),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        EVENT_CAPACITY, RuntimeEvent, SessionEvent, SessionFoundation, SessionSubject,
        TransportPhase,
    };
    use crate::network::discovery::PeerRecord;

    #[tokio::test]
    async fn stopped_outbound_releases_its_local_udp_socket() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut foundation = SessionFoundation::bind("127.0.0.1:0".parse().unwrap())
            .unwrap()
            .start("proof".into())
            .await
            .unwrap();
        let reserved = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = reserved.local_addr().unwrap();
        drop(reserved);
        let mut config = moq_tokio::connect::Config::default();
        config.bind = Some(addr);
        let client = config.init(Default::default()).unwrap();
        let connection = client.connect("moqt://127.0.0.1:9".parse::<url::Url>().unwrap());
        drop(client);
        foundation.outbound.insert(
            "test".into(),
            super::Outbound {
                connection,
                task: tokio::spawn(std::future::pending()),
            },
        );
        foundation.stop_outbound("test").await;
        let rebound = std::net::UdpSocket::bind(addr).expect("outbound socket released");
        drop(rebound);
        foundation.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_collects_failures_and_still_joins_every_owner() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut foundation = SessionFoundation::bind("127.0.0.1:0".parse().unwrap())
            .unwrap()
            .start("proof".into())
            .await
            .unwrap();
        // Replace the workers only after stopping the real listener and its origins.
        foundation
            .listener_shutdown
            .take()
            .unwrap()
            .send(())
            .unwrap();
        foundation
            .listener_task
            .take()
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        foundation.origin_drivers.publish.abort();
        foundation.origin_drivers.receive.abort();
        let _ = (&mut foundation.origin_drivers.publish).await;
        let _ = (&mut foundation.origin_drivers.receive).await;
        let completed = Arc::new(AtomicBool::new(false));
        let done = completed.clone();
        foundation.listener_task = Some(tokio::spawn(async {
            panic!("listener panic");
        }));
        foundation.origin_drivers.publish = tokio::spawn(async {
            panic!("origin panic");
        });
        foundation.origin_drivers.receive = tokio::spawn(async move {
            done.store(true, Ordering::SeqCst);
        });
        let error = foundation.shutdown().await.unwrap_err();
        assert!(error.contains("listener"));
        assert!(error.contains("publish origin"));
        assert!(completed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn shutdown_waits_for_connection_loop_exit() {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut foundation = SessionFoundation::bind("127.0.0.1:0".parse().unwrap())
            .unwrap()
            .start("proof".into())
            .await
            .unwrap();
        let (done, finished) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = done.send(());
        });
        finished.await.unwrap();
        assert!(task.is_finished());
        let client = moq_tokio::connect::Config::default()
            .init(Default::default())
            .unwrap();
        let connection = client.connect("moqt://127.0.0.1:9".parse::<url::Url>().unwrap());
        let observed = connection.clone();
        foundation
            .outbound
            .insert("test".into(), super::Outbound { connection, task });
        foundation.stop_outbound("test").await;
        let mut closed = std::pin::pin!(observed.closed());
        assert!(
            matches!(
                closed
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(_)
            ),
            "stop returned before the connection loop exited"
        );
        foundation.shutdown().await.unwrap();
    }

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
            addrs: vec![advertised.addr],
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
        foundation.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_releases_the_listener_socket() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let bound = SessionFoundation::bind("127.0.0.1:0".parse().expect("bind")).expect("bound");
        let addr = bound.advertisement().addr;
        let foundation = bound.start("proof".to_owned()).await.expect("started");

        foundation.shutdown().await.unwrap();

        let rebound = SessionFoundation::bind(addr).expect("listener socket released");
        rebound
            .start("proof".to_owned())
            .await
            .expect("listener restarted")
            .shutdown()
            .await
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_completes_when_the_event_queue_is_full() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let bound = SessionFoundation::bind("127.0.0.1:0".parse().expect("bind")).expect("bound");
        let foundation = bound.start("proof".to_owned()).await.expect("started");
        for _ in 0..EVENT_CAPACITY {
            foundation
                .events
                .try_send(RuntimeEvent::ListenerStopped)
                .expect("event queue slot");
        }

        tokio::time::timeout(Duration::from_secs(1), foundation.shutdown())
            .await
            .expect("shutdown must not wait for event capacity")
            .unwrap();
    }
}
