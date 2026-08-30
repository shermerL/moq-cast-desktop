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
    publish_origin: &moq_net::origin::Producer,
    receive_origin: moq_net::origin::Producer,
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
        .with_publisher(publish_origin)
        .with_subscriber(receive_origin);
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

    fn origin_pair() -> (moq_net::origin::Producer, moq_net::origin::Producer) {
        (
            moq_tokio::origin::spawn(moq_net::Origin::random()),
            moq_tokio::origin::spawn(moq_net::Origin::random()),
        )
    }

    fn publish_test_screen(
        origin: &moq_net::origin::Producer,
        path: &str,
        payload: &'static [u8],
    ) -> (moq_net::broadcast::Producer, moq_net::track::Producer) {
        let mut broadcast = origin
            .create_broadcast(path, moq_net::broadcast::Route::new().with_announce(true))
            .unwrap();
        let mut track = broadcast.create_track("video", None).unwrap();
        let mut group = track.append_group().unwrap();
        group
            .write_frame(moq_net::Timestamp::ZERO, payload)
            .unwrap();
        group.finish().unwrap();
        (broadcast, track)
    }

    async fn find_available(
        announcements: &mut moq_net::announce::Consumer,
        path: &str,
    ) -> Option<moq_net::broadcast::Consumer> {
        while let Some(update) = announcements.next().await {
            if update.path.as_str() == path && update.broadcast.is_some() {
                return update.broadcast;
            }
        }
        None
    }

    async fn expect_available(
        announcements: &mut moq_net::announce::Consumer,
        path: &str,
    ) -> moq_net::broadcast::Consumer {
        tokio::time::timeout(Duration::from_secs(3), find_available(announcements, path))
            .await
            .expect("announcement stayed bounded")
            .expect("origin closed before the expected announcement")
    }

    async fn read_test_frame(broadcast: &moq_net::broadcast::Consumer) -> Vec<u8> {
        let mut track = broadcast
            .track("video")
            .unwrap()
            .subscribe(None)
            .await
            .unwrap();
        let mut group = tokio::time::timeout(Duration::from_secs(3), track.recv_group())
            .await
            .expect("group read stayed bounded")
            .unwrap()
            .expect("track ended before the test group");
        let frame = tokio::time::timeout(Duration::from_secs(3), group.read_frame())
            .await
            .expect("frame read stayed bounded")
            .unwrap()
            .expect("group ended before the test frame");
        frame.payload.to_vec()
    }

    #[tokio::test]
    async fn routes_only_local_publications_across_a_three_peer_mesh() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (server, addr, fingerprint) = listener().await;
        let mut listener = server.listen().await.unwrap();

        let a_publish = moq_tokio::origin::spawn(moq_net::Origin::random());
        let a_receive = moq_tokio::origin::spawn(moq_net::Origin::random());
        let b_publish = moq_tokio::origin::spawn(moq_net::Origin::random());
        let b_receive = moq_tokio::origin::spawn(moq_net::Origin::random());
        let c_publish = moq_tokio::origin::spawn(moq_net::Origin::random());
        let c_receive = moq_tokio::origin::spawn(moq_net::Origin::random());

        let mut a_announcements = a_receive.consume().announced();
        let mut b_announcements = b_receive.consume().announced();
        let mut c_b_announcements = c_receive.consume().announced();
        let mut c_no_relay_announcements = c_receive.consume().announced();
        let (_a_screen, _a_track) = publish_test_screen(&a_publish, "moqcast.screen/a", b"from-a");
        let (_b_screen, _b_track) = publish_test_screen(&b_publish, "moqcast.screen/b", b"from-b");

        let a_pending = dial(
            &peer(addr, fingerprint.clone(), "proof"),
            &a_publish,
            a_receive.clone(),
        )
        .unwrap();
        let request = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .expect("A to B accept stayed bounded")
            .unwrap();
        let (a_connection, b_a_session) = tokio::join!(
            a_pending.established(),
            server::accept(request, "proof", &b_publish, b_receive.clone())
        );
        let a_connection = a_connection.unwrap();
        let b_a_session = b_a_session.unwrap();

        let a_on_b = expect_available(&mut b_announcements, "moqcast.screen/a").await;
        assert_eq!(read_test_frame(&a_on_b).await, b"from-a");
        let b_on_a = expect_available(&mut a_announcements, "moqcast.screen/b").await;
        assert_eq!(read_test_frame(&b_on_a).await, b"from-b");

        let c_pending = dial(
            &peer(addr, fingerprint, "proof"),
            &c_publish,
            c_receive.clone(),
        )
        .unwrap();
        let request = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .expect("C to B accept stayed bounded")
            .unwrap();
        let (c_connection, b_c_session) = tokio::join!(
            c_pending.established(),
            server::accept(request, "proof", &b_publish, b_receive.clone())
        );
        let c_connection = c_connection.unwrap();
        let b_c_session = b_c_session.unwrap();

        let b_on_c = expect_available(&mut c_b_announcements, "moqcast.screen/b").await;
        assert_eq!(read_test_frame(&b_on_c).await, b"from-b");

        let relayed_to_c = tokio::time::timeout(
            Duration::from_secs(1),
            find_available(&mut c_no_relay_announcements, "moqcast.screen/a"),
        )
        .await
        .ok()
        .flatten();
        assert!(
            relayed_to_c.is_none(),
            "B must not republish A's remote screen to C"
        );

        a_connection.close();
        c_connection.close();
        b_a_session.abort(moq_net::Error::Cancel);
        b_c_session.abort(moq_net::Error::Cancel);
    }

    #[tokio::test]
    async fn connects_only_with_the_pinned_fingerprint_and_credential() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (server, addr, fingerprint) = listener().await;
        let (server_publish_origin, server_receive_origin) = origin_pair();
        let accept = tokio::spawn(async move {
            let mut listener = server.listen().await.unwrap();
            let request = listener.accept().await.unwrap();
            server::accept(
                request,
                "proof",
                &server_publish_origin,
                server_receive_origin,
            )
            .await
        });

        let (client_publish_origin, client_receive_origin) = origin_pair();
        let connection = dial(
            &peer(addr, fingerprint, "proof"),
            &client_publish_origin,
            client_receive_origin,
        )
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
        let (server_publish_origin, server_receive_origin) = origin_pair();
        let accept = tokio::spawn(async move {
            let mut listener = server.listen().await.unwrap();
            let request = listener.accept().await.unwrap();
            server::accept(
                request,
                "expected",
                &server_publish_origin,
                server_receive_origin,
            )
            .await
        });

        let (client_publish_origin, client_receive_origin) = origin_pair();
        let connection = dial(
            &peer(addr, fingerprint, "wrong"),
            &client_publish_origin,
            client_receive_origin,
        )
        .unwrap();
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

        let (client_publish_origin, client_receive_origin) = origin_pair();
        let fingerprint = "00".repeat(32);
        let connection = dial(
            &peer(addr, fingerprint, "proof"),
            &client_publish_origin,
            client_receive_origin,
        )
        .unwrap();
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
        let (server_publish_origin, server_receive_origin) = origin_pair();
        let accept = tokio::spawn(async move {
            let mut listener = server.listen().await.unwrap();
            let request = listener.accept().await.unwrap();
            server::accept(
                request,
                "proof",
                &server_publish_origin,
                server_receive_origin,
            )
            .await
        });

        let mut record = peer(addr, fingerprint, "proof");
        record.urls.insert(0, "moqt://127.0.0.1:9".parse().unwrap());
        let (client_publish_origin, client_receive_origin) = origin_pair();
        let connection = dial(&record, &client_publish_origin, client_receive_origin)
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
