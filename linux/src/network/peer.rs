//! Fingerprint-pinned outbound peer sessions.

use std::time::Duration;

use moq_tokio::moq_net;
use thiserror::Error;

use super::discovery::PeerRecord;
use super::security::peer_path;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RECONNECT_BUDGET: Duration = Duration::from_secs(3);

#[derive(Debug, Error)]
pub(crate) enum DialError {
    #[error("peer did not advertise a certificate fingerprint")]
    MissingFingerprint,
    #[error("peer did not advertise a reachable address")]
    NoAddresses,
    #[error(transparent)]
    Native(#[from] moq_tokio::Error),
}

pub(crate) fn dial(
    peer: &PeerRecord,
    origin: moq_net::origin::Producer,
) -> Result<moq_tokio::Connection, DialError> {
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
    let addrs = moq_tokio::Addrs::collect(urls).ok_or(DialError::NoAddresses)?;

    let mut config = moq_tokio::connect::Config::default();
    config.bind = Some("[::]:0".parse().expect("valid ephemeral bind"));
    config.once = Some(false);
    config.backoff.timeout = Some(RECONNECT_BUDGET);
    config.timeout = Some(CONNECT_TIMEOUT);
    config.version = config
        .versions()
        .iter()
        .filter(|version| carries_request_path(version))
        .copied()
        .collect();
    config.tls = moq_tokio::tls::Connect::default();
    config.tls.fingerprint = vec![fingerprint.clone()];

    let client = config
        .init(moq_tokio::quic::Config::default())?
        .with_publisher(&origin)
        .with_subscriber(origin);
    Ok(client.connect(addrs))
}

fn carries_request_path(version: &moq_net::Version) -> bool {
    !version.is_lite() || version.code() >= 0xff0dad05
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use moq_tokio::moq_net;
    use url::Url;

    use super::dial;
    use crate::network::discovery::PeerRecord;
    use crate::network::server;

    fn peer(addr: SocketAddr, fingerprint: String, credential: &str) -> PeerRecord {
        PeerRecord {
            id: "android-test".into(),
            addrs: vec![addr],
            urls: vec![format!("moqt://{addr}").parse::<Url>().unwrap()],
            fingerprint: Some(fingerprint),
            has_node: false,
            credential: credential.into(),
        }
    }

    async fn listener() -> (moq_tokio::Server, SocketAddr, String) {
        let server = server::build().unwrap();
        let port = server.local_addr().unwrap().port();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let fingerprint = server
            .certificates()
            .fingerprints()
            .into_iter()
            .next()
            .unwrap();
        (server, addr, fingerprint)
    }

    #[tokio::test]
    async fn connects_only_with_the_pinned_fingerprint_and_credential() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (server, addr, fingerprint) = listener().await;
        let server_origin = moq_tokio::origin::spawn(moq_net::Origin::random());
        let accept = tokio::spawn(async move {
            let mut listener = server.listen().await.unwrap();
            let request = listener.accept().await.unwrap();
            server::accept(request, "proof", server_origin).await
        });

        let client_origin = moq_tokio::origin::spawn(moq_net::Origin::random());
        let connection = dial(&peer(addr, fingerprint, "proof"), client_origin)
            .unwrap()
            .established()
            .await
            .unwrap();
        let session = accept.await.unwrap().unwrap();

        assert!(connection.connected());
        connection.close();
        session.abort(moq_net::Error::Cancel);
    }

    #[tokio::test]
    async fn rejects_a_wrong_credential() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (server, addr, fingerprint) = listener().await;
        let server_origin = moq_tokio::origin::spawn(moq_net::Origin::random());
        let accept = tokio::spawn(async move {
            let mut listener = server.listen().await.unwrap();
            let request = listener.accept().await.unwrap();
            server::accept(request, "expected", server_origin).await
        });

        let client_origin = moq_tokio::origin::spawn(moq_net::Origin::random());
        let connection = dial(&peer(addr, fingerprint, "wrong"), client_origin).unwrap();
        let established = tokio::time::timeout(Duration::from_secs(4), connection.established())
            .await
            .expect("credential rejection stays bounded");
        let accepted = accept.await.unwrap();

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
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (server, addr, _) = listener().await;
        let accept = tokio::spawn(async move {
            let mut listener = server.listen().await.unwrap();
            listener.accept().await
        });

        let client_origin = moq_tokio::origin::spawn(moq_net::Origin::random());
        let fingerprint = "00".repeat(32);
        let connection = dial(&peer(addr, fingerprint, "proof"), client_origin).unwrap();
        let result =
            tokio::time::timeout(Duration::from_secs(3), connection.clone().established()).await;

        assert!(
            result.is_err(),
            "fingerprint failures remain in bounded backoff"
        );
        connection.close();
        accept.abort();
    }

    #[tokio::test]
    async fn falls_back_to_a_later_candidate_address() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (server, addr, fingerprint) = listener().await;
        let server_origin = moq_tokio::origin::spawn(moq_net::Origin::random());
        let accept = tokio::spawn(async move {
            let mut listener = server.listen().await.unwrap();
            let request = listener.accept().await.unwrap();
            server::accept(request, "proof", server_origin).await
        });

        let mut record = peer(addr, fingerprint, "proof");
        record.urls.insert(0, "moqt://127.0.0.1:9".parse().unwrap());
        let client_origin = moq_tokio::origin::spawn(moq_net::Origin::random());
        let connection = dial(&record, client_origin)
            .unwrap()
            .established()
            .await
            .unwrap();
        let session = accept.await.unwrap().unwrap();

        assert!(connection.connected());
        connection.close();
        session.abort(moq_net::Error::Cancel);
    }
}
