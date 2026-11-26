use moq_tokio::{Client, Connection, moq_net};
use std::{collections::BTreeMap, future::Future, pin::Pin};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Phase {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub(crate) generation: Generation,
    pub(crate) path: String,
    pub(crate) revision: u64,
}

/// A visible broadcast prefix relative to the scope of the connected relay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishPrefix(String);

impl PublishPrefix {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let prefix = value.strip_suffix('/').unwrap_or(value);
        if prefix.split('/').any(|segment| {
            segment.is_empty()
                || segment.starts_with('.')
                || segment.chars().any(|c| {
                    c.is_control() || c.is_whitespace() || matches!(c, '?' | '#' | '\\' | '%')
                })
        }) {
            return None;
        }
        Some(Self(prefix.to_owned()))
    }

    pub(crate) fn broadcast_path(&self) -> String {
        format!("{}/{}", self.0, uuid::Uuid::new_v4())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub(crate) lifecycle: Lifecycle<Phase>,
    pub(crate) server: String,
    pub(crate) entries: BTreeMap<String, u64>,
    pub(crate) error: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) watch_request: u64,
    pub(crate) publish_request: u64,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            lifecycle: Lifecycle::new(Phase::Disconnected),
            server: String::new(),
            entries: BTreeMap::new(),
            error: None,
            version: None,
            watch_request: 0,
            publish_request: 0,
        }
    }
}

impl Snapshot {
    pub(crate) fn contains(&self, selection: &Selection) -> bool {
        self.lifecycle.phase() == Phase::Connected
            && self.lifecycle.generation() == selection.generation
            && self.entries.get(&selection.path) == Some(&selection.revision)
    }
}

// Never derive Debug: the URL can contain credentials in its query or path.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    url: url::Url,
    fingerprint: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigError {
    Url,
    Scheme,
    Fingerprint,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayConfig")
            .field("server", &self.summary())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    scheme: String,
    host: String,
    port: u16,
}

impl Config {
    pub(crate) fn endpoint(&self) -> Endpoint {
        Endpoint {
            scheme: self.url.scheme().to_owned(),
            host: self
                .url
                .host_str()
                .expect("validated Relay host")
                .to_ascii_lowercase(),
            port: self.url.port().unwrap_or(443),
        }
    }

    pub(crate) fn parse(address: &str, fingerprint: &str) -> Result<Self, ConfigError> {
        let url = url::Url::parse(address.trim()).map_err(|_| ConfigError::Url)?;
        if !matches!(url.scheme(), "https" | "moqt")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(ConfigError::Scheme);
        }
        let fingerprint = fingerprint.trim();
        if !fingerprint.is_empty() {
            moq_tokio::tls::parse_fingerprint(fingerprint).map_err(|_| ConfigError::Fingerprint)?;
        }
        Ok(Self {
            url,
            fingerprint: (!fingerprint.is_empty()).then(|| fingerprint.to_owned()),
        })
    }

    pub(crate) fn summary(&self) -> String {
        // Even an authorization scope in the path may be private.
        format!(
            "{}://{}{}",
            self.url.scheme(),
            self.url.host().expect("validated host"),
            self.url
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default()
        )
    }

    fn client(&self, receive: moq_net::origin::Producer) -> Result<Client, String> {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut config = moq_tokio::client::Config::default();
        if let Some(pin) = &self.fingerprint {
            config.connect.tls.fingerprint = vec![pin.clone()];
        }
        config
            .init()
            .map(|client| client.with_subscriber(receive).with_reconnect(false))
            .map_err(|_| "Could not initialize the relay client or certificate trust".to_owned())
    }
}

pub(crate) enum Event {
    Connected,
    Announcement(moq_net::announce::Event),
    Closed(String),
}

pub(crate) type Request =
    Pin<Box<dyn Future<Output = Result<moq_net::broadcast::Consumer, moq_net::Error>> + Send>>;

pub(crate) struct Owner {
    driver: tokio::task::JoinHandle<()>,
    client: Client,
    connection: Connection,
    origin: moq_net::origin::Producer,
    publish: moq_net::origin::Producer,
    announcements: moq_net::announce::Consumer,
    connected: bool,
    routes: BTreeMap<String, (u64, moq_net::origin::Route)>,
    revision: u64,
}

impl Owner {
    pub(crate) fn start(config: Config) -> Result<Self, String> {
        let (origin, driver) = moq_net::origin::Producer::new(moq_net::origin::Config::default());
        let announcements = origin.consume().announced();
        // Only this origin is exported. Remote announcements never flow back to the relay.
        let (publish, publish_driver) = moq_net::origin::Producer::new(Default::default());
        let client = config
            .client(origin.clone())?
            .with_publisher(publish.consume());
        let driver = tokio::spawn(async move {
            let _ = tokio::join!(
                moq_net::time::run(driver),
                moq_net::time::run(publish_driver)
            );
        });
        let connection = client.connect(config.url);
        Ok(Self {
            driver,
            client,
            connection,
            origin,
            publish,
            announcements,
            connected: false,
            routes: BTreeMap::new(),
            revision: 0,
        })
    }

    pub(crate) async fn recv(&mut self) -> Event {
        if !self.connected {
            return match self.connection.clone().established().await {
                Ok(_) => {
                    self.connected = true;
                    Event::Connected
                }
                Err(error) => Event::Closed(connection_error(&error)),
            };
        }
        tokio::select! {
            result = self.connection.closed() => Event::Closed(match result {
                Ok(()) => "Relay disconnected".into(),
                Err(error) => connection_error(&error),
            }),
            event = self.announcements.next() => match event {
                Some(event) => Event::Announcement(event),
                None => Event::Closed("Relay announcement stream ended".into()),
            }
        }
    }

    pub(crate) fn publish_origin(&self) -> moq_net::origin::Producer {
        self.publish.clone()
    }

    pub(crate) fn version(&self) -> Option<String> {
        self.connection.version().map(|v| v.to_string())
    }

    pub(crate) fn apply(&mut self, snapshot: &mut Snapshot, event: moq_net::announce::Event) {
        use moq_net::announce::Event;
        match event {
            Event::Start(announce) | Event::Update(announce) => {
                let path = announce.prefix.to_string();
                let revision = match self.routes.get(&path) {
                    Some((revision, route)) if route.epoch == announce.route.epoch => *revision,
                    _ => {
                        self.revision = self
                            .revision
                            .checked_add(1)
                            .expect("relay revision exhausted");
                        self.revision
                    }
                };
                self.routes.insert(path.clone(), (revision, announce.route));
                snapshot.entries.insert(path, revision);
            }
            Event::End(announce) => {
                let path = announce.prefix.to_string();
                if self
                    .routes
                    .get(&path)
                    .is_some_and(|(_, route)| route.epoch == announce.route.epoch)
                {
                    self.routes.remove(&path);
                    snapshot.entries.remove(&path);
                }
            }
        }
    }

    pub(crate) fn request(&self, selection: &Selection) -> Option<Request> {
        let (revision, route) = self.routes.get(&selection.path)?;
        if *revision != selection.revision {
            return None;
        }
        let origin = self.origin.consume();
        let path = selection.path.clone();
        let epoch = route.epoch.clone();
        Some(Box::pin(async move {
            origin.request_broadcast(&path, epoch).await
        }))
    }

    pub(crate) async fn close(self) -> Result<(), String> {
        let Self {
            driver,
            client,
            connection,
            origin,
            publish,
            announcements,
            ..
        } = self;
        drop(announcements);
        let result = connection
            .close()
            .await
            .map_err(|_| "Relay connection cleanup failed".to_owned());
        client.close().await;
        drop(origin);
        drop(publish);
        let mut errors = crate::cleanup::Errors::default();
        errors.result("connection", result);
        errors.join("relay origin", driver.await, false);
        errors.finish()
    }
}

fn connection_error(error: &moq_tokio::Error) -> String {
    if error.is_auth() {
        return "Relay rejected the connection authorization".into();
    }
    match error {
        moq_tokio::Error::Tls(_) => "Relay certificate trust configuration failed",
        moq_tokio::Error::ConnectTimeout(_) => "Relay connection timed out",
        moq_tokio::Error::Connect(_) => "Relay rejected the connection authorization",
        _ => "Relay connection failed; check the server, certificate trust and protocol compatibility",
    }.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_identity_survives_process_restarts() {
        const CHILD: &str = "MOQCAST_TEST_UUID_CHILD";
        if std::env::var_os(CHILD).is_some() {
            println!(
                "fixture-id={}",
                PublishPrefix::parse("test").unwrap().broadcast_path()
            );
            return;
        }
        let generate = || {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "publication_identity_survives_process_restarts",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(output.status.success());
            let output = String::from_utf8(output.stdout).unwrap();
            let (_, suffix) = output.split_once("fixture-id=test/").unwrap();
            let id = uuid::Uuid::parse_str(suffix.lines().next().unwrap()).unwrap();
            assert_eq!(id.get_version_num(), 4);
            id
        };
        assert_ne!(generate(), generate());
    }

    #[test]
    fn explicit_trust_and_private_url_boundaries() {
        for url in [
            "http://localhost/",
            "moql://localhost/",
            "https://name:secret@localhost/",
            "https://localhost/#secret",
        ] {
            assert!(Config::parse(url, "").is_err());
        }
        assert!(Config::parse("moqt://localhost:4443/test/?jwt=secret", "abcd").is_err());
        let c = Config::parse(
            "https://localhost:4443/private/?jwt=secret",
            &"ab".repeat(32),
        )
        .unwrap();
        assert_eq!(c.url.path(), "/private/");
        assert_eq!(c.url.query(), Some("jwt=secret"));
        assert_eq!(c.summary(), "https://localhost:4443");
        assert!(!format!("{c:?}").contains("secret"));
        assert!(!format!("{c:?}").contains("private"));
    }

    #[test]
    fn selection_requires_current_connection_and_exact_announcement() {
        let mut snapshot = Snapshot::default();
        let generation = snapshot.lifecycle.begin(Phase::Connected);
        snapshot.entries.insert("test/one".into(), 4);
        let selection = Selection {
            generation,
            path: "test/one".into(),
            revision: 4,
        };
        assert!(snapshot.contains(&selection));
        snapshot.entries.insert(selection.path.clone(), 5);
        assert!(!snapshot.contains(&selection));
        snapshot.entries.insert(selection.path.clone(), 4);
        snapshot.lifecycle.begin(Phase::Connected);
        assert!(!snapshot.contains(&selection));
    }
}
