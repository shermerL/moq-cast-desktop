//! QUIC server ownership, advertisement identity, and inbound authorization.

use std::net::SocketAddr;

use thiserror::Error;
use tokio::{sync::mpsc, task::JoinSet};

use super::{RuntimeEvent, TransportPhase, security::authorized};

#[derive(Clone, Debug)]
pub(crate) struct Advertisement {
    pub(crate) addr: SocketAddr,
    pub(crate) fingerprint: String,
}

#[derive(Debug, Error)]
pub(crate) enum StartError {
    #[error("listener did not provide a certificate fingerprint")]
    MissingFingerprint,
    #[error(transparent)]
    Native(#[from] moq_native::Error),
}

#[derive(Debug, Error)]
pub(crate) enum AcceptError {
    #[error("LAN peer did not present this listener's credential")]
    Unauthorized,
    #[error(transparent)]
    Native(#[from] moq_native::Error),
}

pub(crate) struct BoundServer {
    server: moq_native::Server,
    advertisement: Advertisement,
}

impl BoundServer {
    pub(crate) fn advertisement(&self) -> &Advertisement {
        &self.advertisement
    }

    pub(super) async fn listen(self) -> Result<moq_native::Listener, StartError> {
        Ok(self.server.listen().await?)
    }
}

pub(crate) fn bind(bind: SocketAddr) -> Result<BoundServer, StartError> {
    let mut config = moq_native::ServerConfig::default();
    config.bind = Some(bind.to_string());
    config.tls.generate = vec!["moq-cast-windows".to_owned()];
    let server = config.init()?;
    let addr = server.local_addr()?;
    let fingerprint = server
        .certificates()
        .fingerprints()
        .into_iter()
        .next()
        .ok_or(StartError::MissingFingerprint)?;
    Ok(BoundServer {
        server,
        advertisement: Advertisement { addr, fingerprint },
    })
}

pub(crate) async fn accept(
    request: moq_native::Request,
    credential: &str,
) -> Result<moq_native::moq_net::Session, AcceptError> {
    if !authorized(request.path(), credential) {
        request.close(403).await?;
        return Err(AcceptError::Unauthorized);
    }
    Ok(request.ok().await?)
}

pub(super) async fn run_listener(
    mut listener: moq_native::Listener,
    credential: String,
    events: mpsc::Sender<RuntimeEvent>,
) {
    let mut inbound_id = 0_u64;
    let mut sessions = JoinSet::new();
    while let Some(request) = listener.accept().await {
        inbound_id = inbound_id.saturating_add(1);
        let id = inbound_id;
        let events = events.clone();
        let credential = credential.clone();
        sessions.spawn(async move {
            match accept(request, &credential).await {
                Ok(session) => {
                    let _ = events
                        .send(RuntimeEvent::Inbound {
                            id,
                            phase: TransportPhase::Connected,
                        })
                        .await;
                    let _ = session.closed().await;
                    let _ = events
                        .send(RuntimeEvent::Inbound {
                            id,
                            phase: TransportPhase::Disconnected,
                        })
                        .await;
                }
                Err(AcceptError::Unauthorized) => {
                    let _ = events
                        .send(RuntimeEvent::Inbound {
                            id,
                            phase: TransportPhase::Rejected,
                        })
                        .await;
                }
                Err(AcceptError::Native(_)) => {
                    let _ = events
                        .send(RuntimeEvent::Inbound {
                            id,
                            phase: TransportPhase::Failed,
                        })
                        .await;
                }
            }
        });
        while sessions.try_join_next().is_some() {}
    }
    sessions.abort_all();
    while sessions.join_next().await.is_some() {}
    let _ = events.send(RuntimeEvent::ListenerStopped).await;
}
