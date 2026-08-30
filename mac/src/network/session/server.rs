//! QUIC listener ownership, advertised identity, and inbound authorization.

use std::net::SocketAddr;

use thiserror::Error;
use tokio::{sync::mpsc, task::JoinSet};

use super::{RuntimeEvent, SessionOrigins, TransportPhase};
use crate::network::security::authorized;

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
    Native(#[from] moq_tokio::Error),
}

#[derive(Debug, Error)]
pub(crate) enum AcceptError {
    #[error("LAN peer did not present this listener's credential")]
    Unauthorized,
    #[error(transparent)]
    Native(#[from] moq_tokio::Error),
}

pub(crate) struct BoundServer {
    server: moq_tokio::Server,
    advertisement: Advertisement,
}

impl BoundServer {
    pub(crate) fn advertisement(&self) -> &Advertisement {
        &self.advertisement
    }

    pub(super) async fn listen(self) -> Result<moq_tokio::Listener, StartError> {
        Ok(self.server.listen().await?)
    }
}

pub(crate) fn bind(bind: SocketAddr) -> Result<BoundServer, StartError> {
    let mut config = moq_tokio::listen::Config::default();
    config.bind = Some(bind.to_string());
    config.tls.generate = vec!["moq-cast-macos".to_owned()];
    let server = config.init(moq_tokio::quic::Config::default())?;
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

pub(super) async fn accept(
    request: moq_tokio::Request,
    credential: &str,
    origins: SessionOrigins,
) -> Result<moq_tokio::moq_net::Session, AcceptError> {
    if !authorized(request.path(), credential) {
        request.close(403).await?;
        return Err(AcceptError::Unauthorized);
    }

    Ok(request
        .with_publisher(&origins.publish)
        .with_subscriber(origins.receive)
        .ok()
        .await?)
}

pub(super) async fn run_listener(
    mut listener: moq_tokio::Listener,
    credential: String,
    origins: SessionOrigins,
    events: mpsc::Sender<RuntimeEvent>,
) {
    let mut inbound_id = 0_u64;
    let mut sessions = JoinSet::new();

    while let Some(request) = listener.accept().await {
        inbound_id = inbound_id.saturating_add(1);
        let id = inbound_id;
        let events = events.clone();
        let credential = credential.clone();
        let origins = origins.clone();
        sessions.spawn(async move {
            let phase = match accept(request, &credential, origins).await {
                Ok(session) => {
                    let _ = events
                        .send(RuntimeEvent::Inbound {
                            id,
                            phase: TransportPhase::Connected,
                        })
                        .await;
                    let _ = session.closed().await;
                    TransportPhase::Disconnected
                }
                Err(AcceptError::Unauthorized) => TransportPhase::Rejected,
                Err(AcceptError::Native(_)) => TransportPhase::Failed,
            };
            let _ = events.send(RuntimeEvent::Inbound { id, phase }).await;
        });
        while sessions.try_join_next().is_some() {}
    }

    sessions.abort_all();
    while sessions.join_next().await.is_some() {}
    let _ = events.send(RuntimeEvent::ListenerStopped).await;
}
