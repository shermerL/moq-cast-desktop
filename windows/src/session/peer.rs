//! Fingerprint-pinned, bounded outbound peer connections.

use std::time::Duration;

use moq_native::{Addrs, ClientConfig, mdns, moq_net};
use thiserror::Error;
use url::Url;

use super::security::peer_path;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub(crate) enum DialError {
    #[error("peer did not advertise a certificate fingerprint")]
    MissingFingerprint,
    #[error("peer did not advertise a reachable address")]
    NoAddresses,
    #[error(transparent)]
    Native(#[from] moq_native::Error),
}

#[derive(Clone)]
pub(crate) struct PeerRecord {
    pub(super) urls: Vec<Url>,
    pub(super) has_node: bool,
    pub(super) fingerprint: Option<String>,
    pub(super) credential: String,
}

impl From<&mdns::Peer> for PeerRecord {
    fn from(peer: &mdns::Peer) -> Self {
        Self {
            urls: peer.urls(),
            has_node: peer.node.is_some(),
            fingerprint: peer.fingerprint.clone(),
            credential: peer.credential.clone(),
        }
    }
}

pub(crate) fn dial(peer: &PeerRecord) -> Result<moq_native::Connection, DialError> {
    let fingerprint = peer
        .fingerprint
        .as_ref()
        .ok_or(DialError::MissingFingerprint)?;
    let urls = peer.urls.iter().cloned().map(|mut url| {
        if !peer.has_node {
            url.set_path(&peer_path(&peer.credential));
        }
        url
    });
    let addrs = Addrs::collect(urls).ok_or(DialError::NoAddresses)?;

    let mut config = ClientConfig::default();
    config.bind.set_port(0);
    config.reconnect = Some(false);
    config.timeout = Some(CONNECT_TIMEOUT);
    config.version = config
        .versions()
        .iter()
        .filter(|version| carries_request_path(version))
        .copied()
        .collect();
    config.tls = moq_native::tls::Client::default();
    config.tls.fingerprint = vec![fingerprint.clone()];

    Ok(config.init()?.connect(addrs))
}

fn carries_request_path(version: &moq_net::Version) -> bool {
    !version.is_lite() || version.code() >= 0xff0dad05
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use moq_native::moq_net;

    use super::{PeerRecord, dial};
    use crate::session::server;

    fn peer(addr: SocketAddr, fingerprint: String, credential: &str) -> PeerRecord {
        PeerRecord {
            urls: vec![format!("moqt://{addr}").parse().expect("candidate")],
            has_node: false,
            fingerprint: Some(fingerprint),
            credential: credential.to_owned(),
        }
    }

    async fn listener() -> (moq_native::Listener, SocketAddr, String) {
        let _ = moq_native::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let server = server::bind("127.0.0.1:0".parse().expect("bind")).expect("server");
        let advertised = server.advertisement().clone();
        let listener = server.listen().await.expect("listen");
        (listener, advertised.addr, advertised.fingerprint)
    }

    #[tokio::test]
    async fn connects_with_the_pinned_fingerprint_and_credential() {
        let (mut listener, addr, fingerprint) = listener().await;
        let accept = tokio::spawn(async move {
            let request = listener.accept().await.expect("request");
            server::accept(request, "proof").await
        });

        let connection = dial(&peer(addr, fingerprint, "proof"))
            .expect("dial")
            .established()
            .await
            .expect("established");
        let session = accept.await.expect("accept task").expect("accepted");

        assert!(connection.connected());
        connection.close();
        session.abort(moq_net::Error::Cancel);
    }

    #[tokio::test]
    async fn rejects_a_wrong_credential() {
        let (mut listener, addr, fingerprint) = listener().await;
        let accept = tokio::spawn(async move {
            let request = listener.accept().await.expect("request");
            server::accept(request, "expected").await
        });

        let established = dial(&peer(addr, fingerprint, "wrong"))
            .expect("dial")
            .established()
            .await;
        let accepted = accept.await.expect("accept task");

        assert!(accepted.is_err());
        if let Ok(connection) = established {
            let closed = tokio::time::timeout(Duration::from_secs(3), connection.closed())
                .await
                .expect("rejection must close the client");
            assert!(closed.is_err());
        }
    }

    #[tokio::test]
    async fn rejects_a_mismatched_fingerprint() {
        let (mut listener, addr, _) = listener().await;
        let accept = tokio::spawn(async move { listener.accept().await });

        let connection = dial(&peer(addr, "00".repeat(32), "proof")).expect("dial");
        let result = tokio::time::timeout(Duration::from_secs(3), connection.established())
            .await
            .expect("bounded connection attempt");

        assert!(result.is_err());
        accept.abort();
    }

    #[tokio::test]
    async fn falls_back_to_a_later_candidate() {
        let (mut listener, addr, fingerprint) = listener().await;
        let accept = tokio::spawn(async move {
            let request = listener.accept().await.expect("request");
            server::accept(request, "proof").await
        });

        let mut record = peer(addr, fingerprint, "proof");
        record
            .urls
            .insert(0, "moqt://127.0.0.1:9".parse().expect("candidate"));
        let connection = dial(&record)
            .expect("dial")
            .established()
            .await
            .expect("fallback established");
        let session = accept.await.expect("accept task").expect("accepted");

        assert!(connection.connected());
        connection.close();
        session.abort(moq_net::Error::Cancel);
    }
}
