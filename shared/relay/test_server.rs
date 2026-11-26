//! Isolated pinned SDK listener for native transport/owner tests.
pub(crate) struct Server {
    listener: tokio::task::JoinHandle<()>,
    disconnect: tokio::sync::watch::Sender<u64>,
}

impl Server {
    pub(crate) fn close_connections(&self) {
        self.disconnect.send_modify(|epoch| *epoch += 1);
    }
    pub(crate) fn abort(self) {
        self.listener.abort();
    }
}

pub(crate) async fn listen() -> (std::net::SocketAddr, String, Server) {
    let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut config = moq_tokio::listen::Config::default();
    config.bind = Some(moq_tokio::listen::Bind::Addr(
        "127.0.0.1:0".parse().unwrap(),
    ));
    config.tls.generate = vec!["localhost".into()];
    let server = config.init(moq_tokio::quic::Config::default()).unwrap();
    let address = server.local_addr().unwrap();
    let pin = server.certificates().fingerprints()[0].clone();
    let mut listener = server.listen().await.unwrap();
    let origin = moq_tokio::origin::spawn();
    let (disconnect, _) = tokio::sync::watch::channel(0);
    let close = disconnect.clone();
    let server = tokio::spawn(async move {
        let mut sessions = tokio::task::JoinSet::new();
        while let Some(request) = listener.accept().await {
            let origin = origin.clone();
            let mut ended = close.subscribe();
            sessions.spawn(async move {
                if let Ok(session) = request
                    .with_publisher(&origin)
                    .with_subscriber(origin.clone())
                    .ok()
                    .await
                {
                    tokio::select! {
                        _ = session.closed() => {}
                        _ = ended.changed() => { let _ = session.close().await; }
                    }
                }
            });
        }
    });
    (
        address,
        pin,
        Server {
            listener: server,
            disconnect,
        },
    )
}
