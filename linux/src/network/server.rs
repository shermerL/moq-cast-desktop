//! QUIC listener construction and inbound peer authorization.

use moq_tokio::moq_net;
use thiserror::Error;

use super::security::authorized;

#[derive(Debug, Error)]
pub(crate) enum AcceptError {
    #[error("LAN peer did not present this listener's credential")]
    Unauthorized,
    #[error(transparent)]
    Native(#[from] moq_tokio::Error),
}

pub(crate) fn build() -> Result<moq_tokio::Server, moq_tokio::Error> {
    let mut config = moq_tokio::listen::Config::default();
    config.bind = Some("[::]:0".into());
    config.tls.generate = vec!["moq-cast-desktop".into()];
    config.init(moq_tokio::quic::Config::default())
}

pub(crate) fn authorized_request(request: &moq_tokio::Request, credential: &str) -> bool {
    authorized(request.path(), credential)
}

pub(crate) async fn accept(
    request: moq_tokio::Request,
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
