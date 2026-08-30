//! Fingerprint-pinned, bounded outbound peer connections.

use std::time::Duration;

use moq_tokio::{Addrs, moq_net};
use thiserror::Error;

use super::SessionOrigins;
use crate::{contract::cluster_path, network::discovery::PeerRecord};

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

pub(super) fn dial(
    peer: &PeerRecord,
    origins: SessionOrigins,
) -> Result<moq_tokio::Connection, DialError> {
    let fingerprint = peer
        .fingerprint
        .as_ref()
        .ok_or(DialError::MissingFingerprint)?;
    let urls = peer.urls.iter().cloned().map(|mut url| {
        if !peer.has_node {
            url.set_path(&cluster_path(&peer.credential));
        }
        url
    });
    let addrs = Addrs::collect(urls).ok_or(DialError::NoAddresses)?;

    let config = direct_config(fingerprint.clone());
    let client = config
        .init(moq_tokio::quic::Config::default())?
        .with_publisher(&origins.publish)
        .with_subscriber(origins.receive);
    Ok(client.connect(addrs))
}

fn direct_config(fingerprint: String) -> moq_tokio::connect::Config {
    let mut config = moq_tokio::connect::Config::default();
    config.bind = Some("[::]:0".parse().expect("valid ephemeral bind"));
    config.once = Some(false);
    config.backoff.timeout = Some(RECONNECT_BUDGET);
    config.timeout = Some(CONNECT_TIMEOUT);
    config.goaway.redirect = Some(moq_tokio::Redirect::Ignore);
    config.version = config
        .versions()
        .iter()
        .filter(|version| carries_request_path(version))
        .copied()
        .collect();
    config.tls = moq_tokio::tls::Connect::default();
    config.tls.fingerprint = vec![fingerprint];
    config
}

fn carries_request_path(version: &moq_net::Version) -> bool {
    !version.is_lite() || version.code() >= 0xff0dad05
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use moq_tokio::moq_net;

    use super::{dial, direct_config};
    use crate::network::{
        discovery::PeerRecord,
        session::{SessionOrigins, server},
    };

    fn peer(addr: SocketAddr, fingerprint: String, credential: &str) -> PeerRecord {
        PeerRecord {
            id: "peer".to_owned(),
            urls: vec![format!("moqt://{addr}").parse().expect("candidate")],
            has_node: false,
            fingerprint: Some(fingerprint),
            credential: credential.to_owned(),
        }
    }

    async fn listener() -> (moq_tokio::Listener, SocketAddr, String) {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let server = server::bind("127.0.0.1:0".parse().expect("bind")).expect("server");
        let advertised = server.advertisement().clone();
        let listener = server.listen().await.expect("listen");
        (listener, advertised.addr, advertised.fingerprint)
    }

    fn origins() -> SessionOrigins {
        SessionOrigins {
            publish: moq_tokio::origin::spawn(moq_net::Origin::random()),
            receive: moq_tokio::origin::spawn(moq_net::Origin::random()),
        }
    }

    #[test]
    fn direct_only_connections_ignore_goaway_redirects() {
        let config = direct_config("00".repeat(32));

        assert_eq!(config.goaway.redirect, Some(moq_tokio::Redirect::Ignore));
    }

    #[tokio::test]
    async fn connects_with_the_pinned_fingerprint_and_credential() {
        let (mut listener, addr, fingerprint) = listener().await;
        let accept = tokio::spawn(async move {
            let request = listener.accept().await.expect("request");
            server::accept(request, "proof", origins()).await
        });

        let connection = dial(&peer(addr, fingerprint, "proof"), origins())
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
            server::accept(request, "expected", origins()).await
        });

        let connection = dial(&peer(addr, fingerprint, "wrong"), origins()).expect("dial");
        let established = tokio::time::timeout(Duration::from_secs(4), connection.established())
            .await
            .expect("credential rejection stays bounded");
        let accepted = accept.await.expect("accept task");

        assert!(accepted.is_err());
        match established {
            Err(error) => assert!(error.is_auth()),
            Ok(connection) => {
                let closed = tokio::time::timeout(Duration::from_secs(3), connection.closed())
                    .await
                    .expect("rejection must close the client");
                assert!(closed.is_err());
            }
        }
    }

    #[tokio::test]
    async fn rejects_a_mismatched_fingerprint() {
        let (mut listener, addr, _) = listener().await;
        let accept = tokio::spawn(async move { listener.accept().await });

        let connection = dial(&peer(addr, "00".repeat(32), "proof"), origins()).expect("dial");
        let result = tokio::time::timeout(Duration::from_secs(4), connection.established())
            .await
            .expect("bounded connection attempt");

        assert!(result.is_err());
        accept.abort();
    }

    #[tokio::test]
    async fn three_peer_mesh_does_not_forward_remote_broadcasts() {
        let (mut listener, addr, fingerprint) = listener().await;
        let a = origins();
        let b = origins();
        let c = origins();
        let a_publish = a.publish.clone();
        let b_publish = b.publish.clone();
        let mut a_announcements = a.receive.consume().announced();
        let mut b_announcements = b.receive.consume().announced();
        let mut c_announcements = c.receive.consume().announced();
        let accepted_origins = b.clone();
        let accept = tokio::spawn(async move {
            let request = listener.accept().await.expect("A request");
            let a_session = server::accept(request, "proof", accepted_origins.clone())
                .await
                .expect("A accepted");
            let request = listener.accept().await.expect("C request");
            let c_session = server::accept(request, "proof", accepted_origins)
                .await
                .expect("C accepted");
            (a_session, c_session)
        });

        let a_connection = dial(&peer(addr, fingerprint.clone(), "proof"), a)
            .expect("A dial")
            .established()
            .await
            .expect("A established");
        let c_connection = dial(&peer(addr, fingerprint, "proof"), c)
            .expect("C dial")
            .established()
            .await
            .expect("C established");
        let (a_session, c_session) = tokio::time::timeout(Duration::from_secs(5), accept)
            .await
            .expect("both sessions accepted")
            .expect("accept task");

        let _from_a = a_publish
            .create_broadcast(
                "moqcast.screen/a",
                moq_net::broadcast::Route::new().with_announce(true),
            )
            .expect("A broadcast");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let update = b_announcements.next().await.expect("B receive origin");
                if update.path.as_str() == "moqcast.screen/a" {
                    break;
                }
            }
        })
        .await
        .expect("A broadcast reached B locally");

        let _from_b = b_publish
            .create_broadcast(
                "moqcast.screen/b",
                moq_net::broadcast::Route::new().with_announce(true),
            )
            .expect("B broadcast");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let update = a_announcements.next().await.expect("A receive origin");
                if update.path.as_str() == "moqcast.screen/b" {
                    break;
                }
            }
        })
        .await
        .expect("B broadcast reached A");

        let c_saw_a_before_b = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let update = c_announcements.next().await.expect("C receive origin");
                match update.path.as_str() {
                    "moqcast.screen/a" => break true,
                    "moqcast.screen/b" => break false,
                    _ => {}
                }
            }
        })
        .await
        .expect("B broadcast reached C");
        assert!(!c_saw_a_before_b, "C received A through B");

        let forwarded = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let update = c_announcements.next().await.expect("C receive origin");
                if update.path.as_str() == "moqcast.screen/a" {
                    break;
                }
            }
        })
        .await;
        assert!(forwarded.is_err(), "C received A through B");

        a_connection.close();
        c_connection.close();
        a_session.abort(moq_net::Error::Cancel);
        c_session.abort(moq_net::Error::Cancel);
    }
}
