//! QUIC listener construction and inbound peer authorization.

use moq_native::moq_net;
use thiserror::Error;

use super::security::authorized;

#[derive(Debug, Error)]
pub(crate) enum AcceptError {
    #[error("LAN peer did not present this listener's credential")]
    Unauthorized,
    #[error(transparent)]
    Native(#[from] moq_native::Error),
}

pub(crate) fn build() -> Result<moq_native::Server, moq_native::Error> {
    let mut config = moq_native::ServerConfig::default();
    config.bind = Some("[::]:0".into());
    config.tls.generate = vec!["moq-cast-desktop".into()];
    config.init()
}

pub(crate) fn authorized_request(request: &moq_native::Request, credential: &str) -> bool {
    authorized(request.path(), credential)
}

pub(crate) fn incoming_peer_id(request: &moq_native::Request) -> String {
    request
        .peer_origin()
        .map(|origin| origin.to_string())
        .unwrap_or_else(|| "incoming-peer".into())
}

pub(crate) async fn accept(
    request: moq_native::Request,
    credential: &str,
    origin: moq_net::origin::Producer,
) -> Result<moq_net::Session, AcceptError> {
    if !authorized_request(&request, credential) {
        request.close(403).await?;
        return Err(AcceptError::Unauthorized);
    }

    Ok(request
        .with_publisher(&origin)
        .with_subscriber(origin)
        .ok()
        .await?)
}
