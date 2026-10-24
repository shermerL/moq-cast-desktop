//! Direct MoQ peer-session ownership and lifecycle.

mod peer;
mod server;
mod state;

use std::{collections::BTreeMap, future::Future, net::SocketAddr, pin::Pin};

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
    record: PeerRecord,
    deadline: tokio::time::Instant,
}

struct Replacement {
    record: PeerRecord,
    deadline: tokio::time::Instant,
    cleanup: Pin<Box<dyn Future<Output = Result<(), String>> + Send>>,
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
    replacement: Option<Replacement>,
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
        // Services retains the accepted Found operation across select cancellation.
        // Keep its cleanup future and deadline here, rather than on the recv stack.
        if self.replacement.is_none() {
            let peer_id = &peer.id;
            let record = peer;
            let mut deadline = tokio::time::Instant::now() + peer::RECONNECT_BUDGET;
            if let (Some(outbound), Some(state)) =
                (self.outbound.get(peer_id), self.states.get(peer_id))
            {
                let same_identity = outbound.record.credential == record.credential
                    && outbound.record.fingerprint == record.fingerprint
                    && outbound.record.has_node == record.has_node;
                if same_identity
                    && (outbound.connection.connected()
                        || state.phase() == TransportPhase::Connected)
                {
                    return Ok(TransportUpdate {
                        subject: SessionSubject::Peer(peer_id.to_owned()),
                        state,
                    });
                }
                if same_identity && state.phase() == TransportPhase::Connecting {
                    // Only new addresses for the same advertisement can refresh this attempt.
                    // Repeated answers must not restart its clock or cross authentication identities.
                    let previous = &outbound.record;
                    if previous.has_node
                        || record.has_node
                        || previous.credential != record.credential
                        || previous.fingerprint != record.fingerprint
                        || !record
                            .addrs
                            .iter()
                            .any(|addr| !previous.addrs.contains(addr))
                        || tokio::time::Instant::now() >= outbound.deadline
                    {
                        return Ok(TransportUpdate {
                            subject: SessionSubject::Peer(peer_id.to_owned()),
                            state,
                        });
                    }
                    deadline = outbound.deadline;
                    tracing::debug!(
                        peer_id,
                        previous_addresses = ?previous.addrs,
                        addresses = ?record.addrs,
                        previous_address_count = previous.addrs.len(),
                        address_count = record.addrs.len(),
                        "refreshing pending peer addresses within original connection budget"
                    );
                }
            }
            let outbound = self.outbound.remove(peer_id);
            self.replacement = Some(Replacement {
                record: record.clone(),
                deadline,
                cleanup: Box::pin(async move {
                    match outbound {
                        Some(outbound) => close_outbound(outbound).await,
                        None => Ok(()),
                    }
                }),
            });
        }
        let replacement = self.replacement.as_mut().expect("pending replacement");
        assert!(
            replacement.record == *peer,
            "resume the accepted replacement first"
        );
        self.cleanup
            .result("replacement", replacement.cleanup.as_mut().await);
        let Replacement {
            record, deadline, ..
        } = self.replacement.take().expect("completed cleanup");
        let peer_id = &record.id;
        let record = &record;
        let state = self.states.begin(peer_id);
        let dial = if tokio::time::Instant::now() >= deadline {
            Err(DialError::BudgetExhausted)
        } else {
            peer::dial(record, self.origins.clone())
        };
        let connection = match dial {
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
        tracing::debug!(
            stage = "transport",
            generation,
            peer_id = %peer_id,
            addresses = ?record.addrs,
            address_count = record.addrs.len(),
            ipv4_count = record.addrs.iter().filter(|addr| addr.is_ipv4()).count(),
            ipv6_count = record.addrs.iter().filter(|addr| addr.is_ipv6()).count(),
            remaining_ms = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .as_millis() as u64,
            "starting outbound candidate set"
        );
        let task = tokio::spawn(async move {
            match tokio::time::timeout_at(deadline, observed.clone().established()).await {
                Ok(Ok(connection)) => {
                    tracing::debug!(
                        stage = "transport",
                        generation,
                        peer_id = %observed_peer,
                        "outbound connection established"
                    );
                    let _ = events
                        .send(RuntimeEvent::Outbound {
                            peer: observed_peer.clone(),
                            generation,
                            phase: TransportPhase::Connected,
                        })
                        .await;
                    let reason = connection.closed().await;
                    tracing::debug!(stage = "transport", generation, peer_id = %observed_peer, ?reason, "outbound connection closed");
                    let _ = events
                        .send(RuntimeEvent::Outbound {
                            peer: observed_peer,
                            generation,
                            phase: TransportPhase::Disconnected,
                        })
                        .await;
                }
                result => {
                    tracing::warn!(
                        stage = "transport",
                        generation,
                        peer_id = %observed_peer,
                        error = ?result.as_ref().map(|result| result.as_ref().err()),
                        setup_deadline = result.is_err(),
                        authentication = matches!(&result, Ok(Err(error)) if error.is_auth()),
                        "outbound connection setup failed"
                    );
                    if result.is_err() {
                        observed.abort(moq_tokio::moq_net::Error::Cancel);
                        tracing::warn!("peer connection exhausted its original setup budget");
                    }
                    let _ = events
                        .send(RuntimeEvent::Outbound {
                            peer: observed_peer,
                            generation,
                            phase: if matches!(&result, Ok(Err(error)) if error.is_auth()) {
                                TransportPhase::Rejected
                            } else {
                                TransportPhase::Failed
                            },
                        })
                        .await;
                }
            }
        });
        self.outbound.insert(
            peer_id.clone(),
            Outbound {
                connection,
                task,
                record: record.clone(),
                deadline,
            },
        );
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
                        tracing::debug!(peer_id = %peer, generation, ?phase, "ignored stale outbound connection event");
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
        // Off drains an interrupted cleanup without opening its replacement.
        if let Some(mut replacement) = self.replacement.take() {
            self.cleanup
                .result("replacement", replacement.cleanup.as_mut().await);
        }
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
        self.cleanup
            .result("outbound", close_outbound(outbound).await);
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
            replacement: None,
            cleanup: Default::default(),
        })
    }
}

async fn close_outbound(outbound: Outbound) -> Result<(), String> {
    outbound.connection.abort(moq_tokio::moq_net::Error::Cancel);
    outbound.task.abort();
    let mut errors = crate::cleanup::Errors::default();
    errors.join("observer", outbound.task.await, true);
    // Abort requests shutdown; closed observes the connection loop actually exiting.
    let _ = outbound.connection.closed().await;
    errors.finish()
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

    fn assert_closed(connection: &moq_tokio::Connection) {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        let mut closed = std::pin::pin!(connection.closed());
        assert!(
            matches!(
                closed
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(_)
            ),
            "owner returned before its connection loop closed"
        );
    }

    async fn pending_peer() -> (
        SessionFoundation,
        PeerRecord,
        tokio::net::UdpSocket,
        std::net::SocketAddr,
    ) {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let bound = SessionFoundation::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let reachable = bound.advertisement().addr;
        let fingerprint = bound.advertisement().fingerprint.clone();
        let foundation = bound.start("proof".into()).await.unwrap();
        let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = silent.local_addr().unwrap();
        let record = PeerRecord {
            id: "remote".into(),
            urls: vec![format!("moqt://{addr}").parse().unwrap()],
            addrs: vec![addr],
            has_node: false,
            fingerprint: Some(fingerprint),
            credential: "proof".into(),
        };
        (foundation, record, silent, reachable)
    }

    #[tokio::test]
    async fn late_addresses_connect_without_extending_budget_or_accepting_stale_failure() {
        let (mut foundation, mut record, silent, reachable) = pending_peer().await;
        let first = foundation.connect(&record).await.unwrap();
        let deadline = foundation.outbound["remote"].deadline;
        let old_connection = foundation.outbound["remote"].connection.clone();
        // Observe a real QUIC packet before discovering the usable candidate.
        tokio::time::timeout_at(deadline, silent.recv(&mut [0; 2048]))
            .await
            .unwrap()
            .unwrap();
        record.addrs.push(reachable);
        let refreshed = foundation.connect(&record).await.unwrap();
        assert!(refreshed.state.generation() > first.state.generation());
        assert_eq!(foundation.outbound["remote"].deadline, deadline);
        assert_closed(&old_connection);
        let current = foundation.outbound["remote"].connection.clone();
        let _established = tokio::time::timeout_at(deadline, current.clone().established())
            .await
            .unwrap()
            .unwrap();

        // The handshake has won even if its Connected event has not reached the UI.
        record.addrs.push("127.0.0.1:9".parse().unwrap());
        let kept = foundation.connect(&record).await.unwrap();
        assert_eq!(kept.state.generation(), refreshed.state.generation());
        assert!(current.connected());
        foundation
            .events
            .send(super::RuntimeEvent::Outbound {
                peer: "remote".into(),
                generation: first.state.generation(),
                phase: TransportPhase::Failed,
            })
            .await
            .unwrap();
        let connected = tokio::time::timeout_at(deadline, async {
            loop {
                let SessionEvent::Transport(update) = foundation.recv().await.unwrap() else {
                    panic!("listener stopped");
                };
                if matches!(&update.subject, SessionSubject::Peer(id) if id == "remote") {
                    assert_eq!(update.state.generation(), refreshed.state.generation());
                    assert_eq!(update.state.phase(), TransportPhase::Connected);
                    break update;
                }
            }
        })
        .await
        .unwrap();
        let kept = foundation.connect(&record).await.unwrap();
        assert_eq!(kept, connected);
        let disconnected = foundation.disconnect("remote").await.unwrap();
        assert!(disconnected.state.generation() > refreshed.state.generation());
        assert_closed(&current);
        // Drain the queued old failure without waiting for a new network event.
        foundation
            .events
            .send(super::RuntimeEvent::ListenerStopped)
            .await
            .unwrap();
        while let Some(SessionEvent::Transport(update)) = foundation.recv().await {
            assert!(
                !matches!(&update.subject, SessionSubject::Peer(id) if id == "remote"),
                "old outbound callbacks survived disconnect"
            );
        }
        assert_eq!(foundation.states.get("remote"), None);
        foundation.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_found_survives_receive_cancellation() {
        use crate::network::{DialRole, PeerSession, PeerStatus, PendingConnection};
        for identity_change in [false, true] {
            let (mut foundation, mut record, _silent, reachable) = pending_peer().await;
            let first = foundation.connect(&record).await.unwrap();
            let old = foundation.outbound["remote"].connection.clone();
            record.addrs.push(reachable);
            if identity_change {
                record.credential = "new-proof".into();
            }
            let mut accepted = PendingConnection {
                record: record.clone(),
                status: PeerStatus {
                    id: record.id.clone(),
                    ordinal: 1,
                    discovered: true,
                    role: DialRole::Active,
                    session: if identity_change {
                        PeerSession::Connected
                    } else {
                        PeerSession::Connecting
                    },
                    transport_generation: Some(first.state.generation()),
                },
            };
            // Poll exactly to the old observer's join, then discard only the recv future.
            {
                let mut recv = std::pin::pin!(accepted.complete(&mut foundation));
                std::future::poll_fn(|cx| {
                    assert!(std::future::Future::poll(recv.as_mut(), cx).is_pending());
                    std::task::Poll::Ready(())
                })
                .await;
            }
            let deadline = foundation.replacement.as_ref().unwrap().deadline;
            tokio::time::advance(Duration::from_secs(1)).await;
            let status = accepted.complete(&mut foundation).await;
            assert_eq!(status.session, PeerSession::Connecting);
            assert!(status.transport_generation.unwrap() > first.state.generation());
            assert_eq!(foundation.outbound["remote"].record, record);
            assert_eq!(foundation.outbound["remote"].deadline, deadline);
            assert!(foundation.replacement.is_none());
            assert_closed(&old);
            let current = foundation.connect(&record).await.unwrap();
            assert_eq!(
                Some(current.state.generation()),
                status.transport_generation
            );
            foundation.shutdown().await.unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_replacement_cannot_revive_an_exhausted_budget() {
        let (mut foundation, mut record, _silent, reachable) = pending_peer().await;
        foundation.connect(&record).await.unwrap();
        let old = foundation.outbound["remote"].connection.clone();
        record.addrs.push(reachable);
        {
            let mut replacing = std::pin::pin!(foundation.connect(&record));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(replacing.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        }
        tokio::time::advance(super::peer::RECONNECT_BUDGET + Duration::from_secs(1)).await;
        assert!(matches!(
            foundation.connect(&record).await,
            Err(super::DialError::BudgetExhausted)
        ));
        assert!(foundation.replacement.is_none());
        assert!(foundation.outbound.is_empty());
        assert_eq!(
            foundation.state("remote").unwrap().phase(),
            TransportPhase::Failed
        );
        assert_closed(&old);
        foundation.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_drains_interrupted_replacement_without_redial() {
        let (mut foundation, mut record, _silent, reachable) = pending_peer().await;
        foundation.connect(&record).await.unwrap();
        let old = foundation.outbound["remote"].connection.clone();
        record.addrs.push(reachable);
        {
            let mut replacing = std::pin::pin!(foundation.connect(&record));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(replacing.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        }
        assert!(foundation.outbound.is_empty());
        // Hold cleanup at an explicit barrier, so shutdown must join the owned work.
        let replacement = foundation.replacement.as_mut().unwrap();
        let cleanup = std::mem::replace(&mut replacement.cleanup, Box::pin(async { Ok(()) }));
        let (release, wait) = tokio::sync::oneshot::channel();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (completed, done) = tokio::sync::oneshot::channel();
        replacement.cleanup = Box::pin(async move {
            entered.send(()).unwrap();
            wait.await.unwrap();
            let result = cleanup.await;
            completed.send(()).unwrap();
            result
        });
        let shutdown = tokio::spawn(foundation.shutdown());
        started.await.unwrap();
        assert!(!shutdown.is_finished());
        release.send(()).unwrap();
        shutdown.await.unwrap().unwrap();
        done.await.unwrap();
        assert_closed(&old);
        // A fresh network owner can start after the interrupted owner has closed.
        SessionFoundation::bind("127.0.0.1:0".parse().unwrap())
            .unwrap()
            .start("fresh".into())
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_address_replacement_keeps_the_original_deadline() {
        let (mut foundation, mut record, _silent, reachable) = pending_peer().await;
        foundation.connect(&record).await.unwrap();
        let deadline = foundation.outbound["remote"].deadline;
        record.addrs.push(reachable);
        {
            let mut replacing = std::pin::pin!(foundation.connect(&record));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(replacing.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        foundation.connect(&record).await.unwrap();
        assert_eq!(foundation.outbound["remote"].deadline, deadline);
        foundation.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn repeated_candidates_keep_attempt_but_new_identity_replaces_it() {
        let (mut foundation, mut record, _silent, reachable) = pending_peer().await;
        record.addrs.push("127.0.0.1:9".parse().unwrap());
        let first = foundation.connect(&record).await.unwrap();
        let deadline = foundation.outbound["remote"].deadline;
        record.addrs.reverse();
        record.addrs.push(record.addrs[0]);
        assert_eq!(foundation.connect(&record).await.unwrap(), first);
        assert_eq!(foundation.outbound["remote"].deadline, deadline);
        // Identity replacement is an existing explicit operation, not an address refresh.
        record.credential = "replacement".into();
        record.addrs.push(reachable);
        let replaced = foundation.connect(&record).await.unwrap();
        assert!(replaced.state.generation() > first.state.generation());
        assert_eq!(
            foundation.outbound["remote"].record.credential,
            "replacement"
        );
        foundation.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn expired_address_refresh_does_not_start_another_budget() {
        let (mut foundation, mut record, _silent, reachable) = pending_peer().await;
        let first = foundation.connect(&record).await.unwrap();
        // Inject an expired deadline; no wall-clock sleep is needed to test admission.
        let expired = tokio::time::Instant::now() - Duration::from_secs(1);
        foundation.outbound.get_mut("remote").unwrap().deadline = expired;
        record.addrs.push(reachable);
        assert_eq!(foundation.connect(&record).await.unwrap(), first);
        assert_eq!(foundation.outbound["remote"].deadline, expired);
        foundation.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_closes_refreshed_pending_connection() {
        let (mut foundation, mut record, _silent, _reachable) = pending_peer().await;
        foundation.connect(&record).await.unwrap();
        let old = foundation.outbound["remote"].connection.clone();
        let second = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        record.addrs.push(second.local_addr().unwrap());
        foundation.connect(&record).await.unwrap();
        let current = foundation.outbound["remote"].connection.clone();
        foundation.shutdown().await.unwrap();
        assert_closed(&old);
        assert_closed(&current);
    }

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
                record: PeerRecord {
                    id: "test".into(),
                    urls: vec![],
                    addrs: vec![],
                    has_node: false,
                    fingerprint: None,
                    credential: String::new(),
                },
                deadline: tokio::time::Instant::now(),
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
        foundation.outbound.insert(
            "test".into(),
            super::Outbound {
                connection,
                task,
                record: PeerRecord {
                    id: "test".into(),
                    urls: vec![],
                    addrs: vec![],
                    has_node: false,
                    fingerprint: None,
                    credential: String::new(),
                },
                deadline: tokio::time::Instant::now(),
            },
        );
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
