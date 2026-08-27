use std::{
    collections::{BTreeMap, BTreeSet, VecDeque, btree_map::Entry},
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use httparse::Status as ParseStatus;
use serde_json::json;

use crate::{
    discovery::DiscoveryView,
    registry::{Lifecycle, Snapshot},
};

const CONNECT_URL: &str = "https://moqtcast.com/connect";
const ALLOWED_ORIGIN: &str = "https://moqtcast.com";
const PRESENCE_PATH: &str = "/v1/presence";
const PRESENCE_SCHEMA: &str = "moqtcast.presence.v1";
const WATCH_DESCRIPTOR_SCHEMA: &str = "moqtcast.watch.v1";
const WATCH_DESCRIPTOR_MODE: &str = "experimental-open-cluster";
const TOKEN_BYTES: usize = 32;
const DEVICE_ID_BYTES: usize = 16;
const DEVICE_ID_CHARS: usize = 22;
const MAX_REQUEST_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_BYTES: usize = 32 * 1024;
const MAX_HEADERS: usize = 32;
const RATE_LIMIT_BURST: usize = 16;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const REQUEST_DEADLINE: Duration = Duration::from_secs(1);

struct Token(String);

impl Token {
    fn generate() -> io::Result<Self> {
        random_urlsafe::<TOKEN_BYTES>().map(Self)
    }

    #[cfg(test)]
    fn from_bytes(bytes: [u8; TOKEN_BYTES]) -> Self {
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

pub(crate) struct Bootstrap {
    #[cfg(test)]
    endpoint: String,
    open_url: String,
}

impl Bootstrap {
    fn new(host: &str, token: &Token) -> Self {
        let endpoint = format!("http://{host}");
        let payload = format!(
            r#"{{"version":1,"endpoint":"{endpoint}","token":"{}"}}"#,
            token.expose()
        );
        let open_url = format!("{CONNECT_URL}#lite={}", URL_SAFE_NO_PAD.encode(payload));
        Self {
            #[cfg(test)]
            endpoint,
            open_url,
        }
    }

    pub(crate) fn open_url(&self) -> &str {
        &self.open_url
    }
}

pub(crate) struct BridgeOwner {
    stop: mpsc::SyncSender<()>,
    wake: SocketAddr,
    worker: Option<thread::JoinHandle<()>>,
}

impl BridgeOwner {
    pub(crate) fn start(view: DiscoveryView) -> io::Result<(Self, Bootstrap)> {
        let token = Token::generate()?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        if address.ip() != Ipv4Addr::LOCALHOST {
            return Err(io::Error::other(
                "loopback listener bound outside 127.0.0.1",
            ));
        }

        let host = address.to_string();
        let bootstrap = Bootstrap::new(&host, &token);
        let (stop, stop_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("moqtcast-loopback".to_string())
            .spawn(move || run(listener, view, ServerState::new(host, token), stop_rx))?;

        Ok((
            Self {
                stop,
                wake: address,
                worker: Some(worker),
            },
            bootstrap,
        ))
    }

    #[cfg(test)]
    fn stop(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let _ = self.stop.try_send(());
        let _ = TcpStream::connect_timeout(&self.wake, IO_TIMEOUT);
        let _ = worker.join();
    }
}

impl Drop for BridgeOwner {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn run(
    listener: TcpListener,
    view: DiscoveryView,
    mut server: ServerState,
    stop: mpsc::Receiver<()>,
) {
    while let Ok((mut stream, peer)) = listener.accept() {
        match stop.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if peer.ip().is_loopback() {
            let _ = handle_connection(&mut stream, &view, &mut server);
        }
    }
}

fn handle_connection(
    stream: &mut TcpStream,
    view: &DiscoveryView,
    server: &mut ServerState,
) -> io::Result<()> {
    let deadline = Instant::now() + REQUEST_DEADLINE;
    let response = match read_request(stream, deadline) {
        Ok(request) => server.handle(&request, &view.latest(), Instant::now()),
        Err(ReadRequestError::TooLarge) => {
            Response::empty(Status::RequestHeaderFieldsTooLarge, false)
        }
        Err(ReadRequestError::Invalid) => Response::empty(Status::BadRequest, false),
    };
    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
        return Ok(());
    };
    if remaining.is_zero() {
        return Ok(());
    }
    stream.set_write_timeout(Some(remaining))?;
    response.write_to(stream)
}

enum ReadRequestError {
    TooLarge,
    Invalid,
}

fn read_request(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, ReadRequestError> {
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(ReadRequestError::Invalid);
        };
        if remaining.is_zero() {
            return Err(ReadRequestError::Invalid);
        }
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| ReadRequestError::Invalid)?;
        let read = stream
            .read(&mut chunk)
            .map_err(|_| ReadRequestError::Invalid)?;
        if read == 0 {
            return Err(ReadRequestError::Invalid);
        }
        request.extend_from_slice(&chunk[..read]);
        if request.len() > MAX_REQUEST_BYTES {
            return Err(ReadRequestError::TooLarge);
        }
        if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            if end + 4 != request.len() {
                return Err(ReadRequestError::Invalid);
            }
            return Ok(request);
        }
    }
}

struct ServerState {
    host: String,
    token: Token,
    device_ids: BTreeMap<String, String>,
    rate_limit: VecDeque<Instant>,
}

#[derive(Clone, Copy)]
enum Route<'a> {
    Presence,
    WatchDescriptor(&'a str),
}

impl<'a> Route<'a> {
    fn parse(path: &'a str) -> Option<Self> {
        if path == PRESENCE_PATH {
            return Some(Self::Presence);
        }
        let id = path
            .strip_prefix("/v1/presence/")?
            .strip_suffix("/watch-descriptor")?;
        if id.len() != DEVICE_ID_CHARS
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return None;
        }
        Some(Self::WatchDescriptor(id))
    }
}

impl ServerState {
    fn new(host: String, token: Token) -> Self {
        Self {
            host,
            token,
            device_ids: BTreeMap::new(),
            rate_limit: VecDeque::new(),
        }
    }

    fn handle(&mut self, wire: &[u8], snapshot: &Snapshot, now: Instant) -> Response {
        if wire.len() > MAX_REQUEST_BYTES {
            return Response::empty(Status::RequestHeaderFieldsTooLarge, false);
        }

        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(wire) {
            Ok(ParseStatus::Complete(length)) if length == wire.len() => {}
            Ok(ParseStatus::Complete(_)) | Ok(ParseStatus::Partial) => {
                return Response::empty(Status::BadRequest, false);
            }
            Err(httparse::Error::TooManyHeaders) => {
                return Response::empty(Status::RequestHeaderFieldsTooLarge, false);
            }
            Err(_) => return Response::empty(Status::BadRequest, false),
        }
        if request.version != Some(1) {
            return Response::empty(Status::BadRequest, false);
        }

        let host = match single_header(request.headers, "host") {
            Ok(Some(value)) => value,
            Ok(None) | Err(()) => return Response::empty(Status::BadRequest, false),
        };
        if host != self.host {
            return Response::empty(Status::MisdirectedRequest, false);
        }

        let origin = match single_header(request.headers, "origin") {
            Ok(Some(value)) => value,
            Ok(None) | Err(()) => return Response::empty(Status::Forbidden, false),
        };
        if origin != ALLOWED_ORIGIN {
            return Response::empty(Status::Forbidden, false);
        }
        let cors = true;

        match single_header(request.headers, "transfer-encoding") {
            Ok(None) => {}
            Ok(Some(_)) | Err(()) => return Response::empty(Status::BadRequest, cors),
        }
        match single_header(request.headers, "content-length") {
            Ok(Some("0")) | Ok(None) => {}
            Ok(Some(_)) | Err(()) => return Response::empty(Status::BadRequest, cors),
        }

        let Some(route) = request.path.and_then(Route::parse) else {
            return Response::empty(Status::NotFound, cors);
        };

        match request.method {
            Some("OPTIONS") => self.handle_options(request.headers, now),
            Some("GET") => self.handle_get(route, request.headers, snapshot, now),
            _ => {
                Response::empty(Status::MethodNotAllowed, cors).with_header("Allow", "GET, OPTIONS")
            }
        }
    }

    fn handle_options(&mut self, headers: &[httparse::Header<'_>], now: Instant) -> Response {
        if single_header(headers, "access-control-request-method") != Ok(Some("GET"))
            || !requested_headers_are_authorization(headers)
        {
            return Response::empty(Status::Forbidden, true);
        }
        if !self.allow(now) {
            return Response::empty(Status::TooManyRequests, true).with_header("Retry-After", "1");
        }
        Response::empty(Status::NoContent, true)
            .with_header("Access-Control-Allow-Methods", "GET, OPTIONS")
            .with_header("Access-Control-Allow-Headers", "Authorization")
            .with_header("Access-Control-Max-Age", "60")
    }

    fn handle_get(
        &mut self,
        route: Route<'_>,
        headers: &[httparse::Header<'_>],
        snapshot: &Snapshot,
        now: Instant,
    ) -> Response {
        let authorization = match single_header(headers, "authorization") {
            Ok(Some(value)) => value,
            Ok(None) | Err(()) => return Response::empty(Status::Unauthorized, true),
        };
        let Some(candidate) = authorization.strip_prefix("Bearer ") else {
            return Response::empty(Status::Unauthorized, true);
        };
        if !constant_time_eq(candidate.as_bytes(), self.token.expose().as_bytes()) {
            return Response::empty(Status::Unauthorized, true);
        }
        if !self.allow(now) {
            return Response::empty(Status::TooManyRequests, true).with_header("Retry-After", "1");
        }

        let body = match route {
            Route::Presence => self.presence_body(snapshot).map(Some),
            Route::WatchDescriptor(id) => self.watch_descriptor_body(snapshot, id),
        };
        match body {
            Ok(Some(body)) if body.len() <= MAX_RESPONSE_BYTES => Response::json(Status::Ok, body),
            Ok(Some(_)) | Err(_) => Response::empty(Status::InternalServerError, true),
            Ok(None) => Response::empty(Status::NotFound, true),
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        while self
            .rate_limit
            .front()
            .is_some_and(|timestamp| now.saturating_duration_since(*timestamp) >= RATE_LIMIT_WINDOW)
        {
            self.rate_limit.pop_front();
        }
        if self.rate_limit.len() >= RATE_LIMIT_BURST {
            return false;
        }
        self.rate_limit.push_back(now);
        true
    }

    fn presence_body(&mut self, snapshot: &Snapshot) -> io::Result<Vec<u8>> {
        let current_ids: BTreeSet<&str> = snapshot
            .devices
            .iter()
            .map(|presence| presence.stable_id.as_str())
            .collect();
        self.device_ids
            .retain(|stable_id, _| current_ids.contains(stable_id.as_str()));

        let mut devices = Vec::with_capacity(snapshot.devices.len());
        for presence in &snapshot.devices {
            let id = match self.device_ids.entry(presence.stable_id.clone()) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(random_device_id()?),
            };
            devices.push(json!({
                "id": id,
                "display_name": presence.display_name,
                "online": true,
                "watchable": presence.watchable,
            }));
        }

        serde_json::to_vec(&json!({
            "schema": PRESENCE_SCHEMA,
            "revision": snapshot.revision,
            "lifecycle": lifecycle_name(snapshot.lifecycle),
            "devices": devices,
        }))
        .map_err(io::Error::other)
    }

    fn watch_descriptor_body(
        &self,
        snapshot: &Snapshot,
        opaque_id: &str,
    ) -> io::Result<Option<Vec<u8>>> {
        let Some(stable_id) = self.device_ids.iter().find_map(|(stable_id, candidate)| {
            (candidate == opaque_id).then_some(stable_id.as_str())
        }) else {
            return Ok(None);
        };
        let Some(target) = snapshot.watch_target(stable_id) else {
            return Ok(None);
        };
        let endpoints = target
            .addresses
            .iter()
            .map(|address| {
                json!({
                    "url": format!(
                        "https://{address}:{}/.cluster/{}",
                        target.port, target.credential
                    ),
                    "certificateHash": target.fingerprint,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_vec(&json!({
            "schema": WATCH_DESCRIPTOR_SCHEMA,
            "mode": WATCH_DESCRIPTOR_MODE,
            "endpoints": endpoints,
            "broadcast": format!("moqcast.screen/{stable_id}"),
            "catalogFormat": "hang",
        }))
        .map(Some)
        .map_err(io::Error::other)
    }
}

fn random_device_id() -> io::Result<String> {
    random_urlsafe::<DEVICE_ID_BYTES>()
}

fn random_urlsafe<const LENGTH: usize>() -> io::Result<String> {
    let mut bytes = [0_u8; LENGTH];
    getrandom::fill(&mut bytes)
        .map_err(|error| io::Error::other(format!("CSPRNG unavailable: {error}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn lifecycle_name(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Starting => "starting",
        Lifecycle::Browsing => "browsing",
        Lifecycle::Degraded => "degraded",
        Lifecycle::Stopping => "stopping",
        Lifecycle::Stopped => "stopped",
        Lifecycle::Failed => "failed",
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn single_header<'a>(
    headers: &'a [httparse::Header<'a>],
    name: &str,
) -> Result<Option<&'a str>, ()> {
    let mut value = None;
    for header in headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name))
    {
        if value.is_some() {
            return Err(());
        }
        value = Some(std::str::from_utf8(header.value).map_err(|_| ())?.trim());
    }
    Ok(value)
}

fn requested_headers_are_authorization(headers: &[httparse::Header<'_>]) -> bool {
    let Ok(Some(requested)) = single_header(headers, "access-control-request-headers") else {
        return false;
    };
    let mut values = requested.split(',').map(str::trim);
    matches!(values.next(), Some(value) if value.eq_ignore_ascii_case("authorization"))
        && values.next().is_none()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Status {
    Ok,
    NoContent,
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    MisdirectedRequest,
    TooManyRequests,
    RequestHeaderFieldsTooLarge,
    InternalServerError,
}

impl Status {
    fn code_and_reason(self) -> (u16, &'static str) {
        match self {
            Self::Ok => (200, "OK"),
            Self::NoContent => (204, "No Content"),
            Self::BadRequest => (400, "Bad Request"),
            Self::Unauthorized => (401, "Unauthorized"),
            Self::Forbidden => (403, "Forbidden"),
            Self::NotFound => (404, "Not Found"),
            Self::MethodNotAllowed => (405, "Method Not Allowed"),
            Self::MisdirectedRequest => (421, "Misdirected Request"),
            Self::TooManyRequests => (429, "Too Many Requests"),
            Self::RequestHeaderFieldsTooLarge => (431, "Request Header Fields Too Large"),
            Self::InternalServerError => (500, "Internal Server Error"),
        }
    }
}

struct Response {
    status: Status,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Response {
    fn empty(status: Status, cors: bool) -> Self {
        Self::new(status, Vec::new(), cors, None)
    }

    fn json(status: Status, body: Vec<u8>) -> Self {
        Self::new(status, body, true, Some("application/json; charset=utf-8"))
    }

    fn new(status: Status, body: Vec<u8>, cors: bool, content_type: Option<&str>) -> Self {
        let mut response = Self {
            status,
            headers: vec![
                ("Cache-Control", "no-store".to_string()),
                ("X-Content-Type-Options", "nosniff".to_string()),
                ("Connection", "close".to_string()),
            ],
            body,
        };
        if cors {
            response = response
                .with_header("Access-Control-Allow-Origin", ALLOWED_ORIGIN)
                .with_header("Vary", "Origin");
        }
        if let Some(content_type) = content_type {
            response = response.with_header("Content-Type", content_type);
        }
        response
    }

    fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    #[cfg(test)]
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        let (code, reason) = self.status.code_and_reason();
        write!(writer, "HTTP/1.1 {code} {reason}\r\n")?;
        for (name, value) in &self.headers {
            write!(writer, "{name}: {value}\r\n")?;
        }
        write!(writer, "Content-Length: {}\r\n\r\n", self.body.len())?;
        writer.write_all(&self.body)?;
        writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{Ipv4Addr, SocketAddr, TcpStream},
        thread,
        time::{Duration, Instant},
    };

    use crate::{
        discovery::DiscoveryView,
        registry::{Lifecycle, Registry, ResolvedRecord, Snapshot},
    };

    use super::*;

    const HOST: &str = "127.0.0.1:43123";
    const ORIGIN: &str = "https://moqtcast.com";

    fn snapshot() -> Snapshot {
        let now = Instant::now();
        let mut registry = Registry::default();
        registry.resolved(
            "private_peer_123456._moq._udp.local.",
            now,
            Duration::from_secs(30),
        );
        registry.snapshot(7, Lifecycle::Browsing)
    }

    fn watchable_snapshot(addresses: Vec<Ipv4Addr>) -> Snapshot {
        let now = Instant::now();
        let mut registry = Registry::default();
        registry.resolved_record(
            ResolvedRecord {
                fullname: "0123456789abcdef._moq._udp.local.".to_owned(),
                port: 4443,
                fingerprint: Some("a".repeat(64)),
                credential: Some("b".repeat(32)),
                has_shared_secret: false,
                addresses,
            },
            now,
            Duration::from_secs(30),
        );
        registry.snapshot(8, Lifecycle::Browsing)
    }

    fn request(method: &str, path: &str, host: &str, origin: &str, token: &str) -> Vec<u8> {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\nAuthorization: Bearer {token}\r\n\r\n"
        )
        .into_bytes()
    }

    fn fragment_payload(url: &str) -> (&str, String, serde_json::Value) {
        let encoded = url
            .strip_prefix("https://moqtcast.com/connect#lite=")
            .expect("lite fragment");
        let decoded =
            String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).expect("base64url payload"))
                .expect("UTF-8 payload");
        let json = serde_json::from_str(&decoded).expect("JSON payload");
        (encoded, decoded, json)
    }

    fn fragment_token(url: &str) -> String {
        let (_, _, payload) = fragment_payload(url);
        payload["token"]
            .as_str()
            .expect("fragment token")
            .to_owned()
    }

    fn send(address: SocketAddr, wire: &[u8]) -> Vec<u8> {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1))
            .expect("connect loopback bridge");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set read timeout");
        stream.write_all(wire).expect("write request");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).expect("read response");
        response
    }

    #[test]
    fn lite_fragment_roundtrips_with_only_the_frozen_fields() {
        let token = Token::from_bytes([0x5a; TOKEN_BYTES]);
        let bootstrap = Bootstrap::new(HOST, &token);
        let (encoded, decoded, payload) = fragment_payload(bootstrap.open_url());
        let fields = payload.as_object().expect("JSON object");

        assert_eq!(token.expose().len(), 43);
        assert_eq!(
            decoded,
            format!(
                r#"{{"version":1,"endpoint":"http://{HOST}","token":"{}"}}"#,
                token.expose()
            )
        );
        assert_eq!(fields.len(), 3);
        assert!(
            fields
                .keys()
                .all(|field| matches!(field.as_str(), "version" | "endpoint" | "token"))
        );
        assert_eq!(payload["version"], 1);
        assert_eq!(payload["endpoint"], format!("http://{HOST}"));
        assert_eq!(payload["token"], token.expose());
        assert!(!encoded.contains('='));
        assert!(!bootstrap.open_url().contains('?'));
    }

    #[test]
    fn presence_response_exposes_only_sanitized_fields() {
        let token = Token::from_bytes([0x11; TOKEN_BYTES]);
        let token_text = token.expose().to_string();
        let mut server = ServerState::new(HOST.to_string(), token);
        let response = server.handle(
            &request("GET", PRESENCE_PATH, HOST, ORIGIN, &token_text),
            &snapshot(),
            Instant::now(),
        );
        let body: serde_json::Value = serde_json::from_slice(&response.body).expect("JSON body");

        assert_eq!(response.status, Status::Ok);
        assert_eq!(body["schema"], PRESENCE_SCHEMA);
        assert_eq!(body["revision"], 7);
        assert_eq!(body["lifecycle"], "browsing");
        assert_eq!(body["devices"][0]["display_name"], "MoQ device 123456");
        assert_eq!(body["devices"][0]["online"], true);
        assert_eq!(body["devices"][0]["watchable"], false);
        assert!(body["devices"][0]["id"].as_str().is_some());
        for forbidden in [
            "private_peer_123456",
            "_moq._udp.local.",
            "credential",
            "fingerprint",
            "address",
            "stable_id",
            "/.cluster/",
            "moqcast.screen/",
        ] {
            assert!(!String::from_utf8_lossy(&response.body).contains(forbidden));
        }
    }

    #[test]
    fn selected_watchable_device_returns_the_exact_descriptor_shape() {
        let token = Token::from_bytes([0x12; TOKEN_BYTES]);
        let token_text = token.expose().to_owned();
        let snapshot = watchable_snapshot(vec![
            Ipv4Addr::new(192, 168, 1, 21),
            Ipv4Addr::new(192, 168, 1, 20),
        ]);
        let mut server = ServerState::new(HOST.to_owned(), token);
        let now = Instant::now();
        let presence = server.handle(
            &request("GET", PRESENCE_PATH, HOST, ORIGIN, &token_text),
            &snapshot,
            now,
        );
        let presence_body: serde_json::Value =
            serde_json::from_slice(&presence.body).expect("presence JSON");
        let id = presence_body["devices"][0]["id"]
            .as_str()
            .expect("opaque id");
        assert_eq!(presence_body["devices"][0]["watchable"], true);
        let rendered_presence = String::from_utf8_lossy(&presence.body);
        for forbidden in [
            "0123456789abcdef",
            "192.168.1.20",
            "192.168.1.21",
            "4443",
            "/.cluster/",
            "moqcast.screen/",
        ] {
            assert!(!rendered_presence.contains(forbidden));
        }
        assert!(!rendered_presence.contains(&"a".repeat(64)));
        assert!(!rendered_presence.contains(&"b".repeat(32)));

        let path = format!("{PRESENCE_PATH}/{id}/watch-descriptor");
        let response = server.handle(
            &request("GET", &path, HOST, ORIGIN, &token_text),
            &snapshot,
            now,
        );
        let body: serde_json::Value =
            serde_json::from_slice(&response.body).expect("descriptor JSON");
        let fields = body.as_object().expect("descriptor object");

        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.header("Cache-Control"), Some("no-store"));
        assert_eq!(response.header("X-Content-Type-Options"), Some("nosniff"));
        assert_eq!(fields.len(), 5);
        assert!(fields.keys().all(|field| matches!(
            field.as_str(),
            "schema" | "mode" | "endpoints" | "broadcast" | "catalogFormat"
        )));
        assert_eq!(body["schema"], "moqtcast.watch.v1");
        assert_eq!(body["mode"], "experimental-open-cluster");
        assert_eq!(body["broadcast"], "moqcast.screen/0123456789abcdef");
        assert_eq!(body["catalogFormat"], "hang");
        let endpoints = body["endpoints"].as_array().unwrap();
        assert_eq!(endpoints.len(), 2);
        assert!(endpoints.iter().all(|endpoint| {
            let fields = endpoint.as_object().expect("endpoint object");
            fields.len() == 2
                && fields
                    .keys()
                    .all(|field| matches!(field.as_str(), "url" | "certificateHash"))
        }));
        assert_eq!(
            body["endpoints"][0]["url"],
            format!("https://192.168.1.20:4443/.cluster/{}", "b".repeat(32))
        );
        assert_eq!(body["endpoints"][0]["certificateHash"], "a".repeat(64));
        assert_eq!(
            body["endpoints"][1]["url"],
            format!("https://192.168.1.21:4443/.cluster/{}", "b".repeat(32))
        );
    }

    #[test]
    fn descriptor_requires_auth_and_a_known_online_watchable_device() {
        let token = Token::from_bytes([0x13; TOKEN_BYTES]);
        let token_text = token.expose().to_owned();
        let snapshot = watchable_snapshot(vec![Ipv4Addr::new(192, 168, 1, 20)]);
        let mut server = ServerState::new(HOST.to_owned(), token);
        let now = Instant::now();
        let presence = server.handle(
            &request("GET", PRESENCE_PATH, HOST, ORIGIN, &token_text),
            &snapshot,
            now,
        );
        let presence_body: serde_json::Value =
            serde_json::from_slice(&presence.body).expect("presence JSON");
        let id = presence_body["devices"][0]["id"]
            .as_str()
            .expect("opaque id");
        let path = format!("{PRESENCE_PATH}/{id}/watch-descriptor");

        assert_eq!(
            server
                .handle(
                    &request("GET", &path, HOST, ORIGIN, "wrong"),
                    &snapshot,
                    now
                )
                .status,
            Status::Unauthorized
        );
        assert_eq!(
            server
                .handle(
                    &request("GET", &path, "localhost:43123", ORIGIN, &token_text),
                    &snapshot,
                    now,
                )
                .status,
            Status::MisdirectedRequest
        );
        assert_eq!(
            server
                .handle(
                    &request("GET", &path, HOST, "null", &token_text),
                    &snapshot,
                    now,
                )
                .status,
            Status::Forbidden
        );
        assert_eq!(
            server
                .handle(
                    &request(
                        "GET",
                        "/v1/presence/AAAAAAAAAAAAAAAAAAAAAA/watch-descriptor",
                        HOST,
                        ORIGIN,
                        &token_text,
                    ),
                    &snapshot,
                    now,
                )
                .status,
            Status::NotFound
        );
        for invalid_path in [
            format!("{PRESENCE_PATH}/{id}%2Fextra/watch-descriptor"),
            format!("{PRESENCE_PATH}/{id}/watch-descriptor/extra"),
            format!("{PRESENCE_PATH}/{id}/watch-descriptor?leak=1"),
        ] {
            assert_eq!(
                server
                    .handle(
                        &request("GET", &invalid_path, HOST, ORIGIN, &token_text),
                        &snapshot,
                        now,
                    )
                    .status,
                Status::NotFound
            );
        }
        assert_eq!(
            server
                .handle(
                    &request("GET", &path, HOST, ORIGIN, &token_text),
                    &Snapshot::starting(),
                    now,
                )
                .status,
            Status::NotFound
        );
        assert_eq!(
            server
                .handle(
                    &request("POST", &path, HOST, ORIGIN, &token_text),
                    &snapshot,
                    now,
                )
                .status,
            Status::MethodNotAllowed
        );
    }

    #[test]
    fn shared_secret_presence_is_not_watchable_and_descriptor_is_not_exposed() {
        let now = Instant::now();
        let mut registry = Registry::default();
        registry.resolved_record(
            ResolvedRecord {
                fullname: "0123456789abcdef._moq._udp.local.".to_owned(),
                port: 4443,
                fingerprint: Some("a".repeat(64)),
                credential: Some("b".repeat(32)),
                has_shared_secret: true,
                addresses: vec![Ipv4Addr::new(192, 168, 1, 20)],
            },
            now,
            Duration::from_secs(30),
        );
        let snapshot = registry.snapshot(9, Lifecycle::Browsing);
        let token = Token::from_bytes([0x15; TOKEN_BYTES]);
        let token_text = token.expose().to_owned();
        let mut server = ServerState::new(HOST.to_owned(), token);
        let presence = server.handle(
            &request("GET", PRESENCE_PATH, HOST, ORIGIN, &token_text),
            &snapshot,
            now,
        );
        let body: serde_json::Value =
            serde_json::from_slice(&presence.body).expect("presence JSON");
        let id = body["devices"][0]["id"].as_str().expect("opaque id");
        let path = format!("{PRESENCE_PATH}/{id}/watch-descriptor");

        assert_eq!(body["devices"][0]["watchable"], false);
        assert_eq!(
            server
                .handle(
                    &request("GET", &path, HOST, ORIGIN, &token_text),
                    &snapshot,
                    now,
                )
                .status,
            Status::NotFound
        );
    }

    #[test]
    fn descriptor_preflight_and_response_budget_use_existing_boundaries() {
        let token = Token::from_bytes([0x14; TOKEN_BYTES]);
        let token_text = token.expose().to_owned();
        let snapshot = watchable_snapshot(
            (1..=8)
                .map(|octet| Ipv4Addr::new(192, 168, 1, octet))
                .collect(),
        );
        let mut server = ServerState::new(HOST.to_owned(), token);
        let now = Instant::now();
        let presence = server.handle(
            &request("GET", PRESENCE_PATH, HOST, ORIGIN, &token_text),
            &snapshot,
            now,
        );
        let presence_body: serde_json::Value =
            serde_json::from_slice(&presence.body).expect("presence JSON");
        let id = presence_body["devices"][0]["id"]
            .as_str()
            .expect("opaque id");
        let path = format!("{PRESENCE_PATH}/{id}/watch-descriptor");
        let preflight = format!(
            "OPTIONS {path} HTTP/1.1\r\nHost: {HOST}\r\nOrigin: {ORIGIN}\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: Authorization\r\n\r\n"
        );

        assert_eq!(
            server.handle(preflight.as_bytes(), &snapshot, now).status,
            Status::NoContent
        );
        let response = server.handle(
            &request("GET", &path, HOST, ORIGIN, &token_text),
            &snapshot,
            now,
        );
        assert_eq!(response.status, Status::Ok);
        assert!(response.body.len() <= MAX_RESPONSE_BYTES);
        let body: serde_json::Value =
            serde_json::from_slice(&response.body).expect("descriptor JSON");
        assert_eq!(body["endpoints"].as_array().unwrap().len(), 8);
    }

    #[test]
    fn host_origin_auth_method_and_path_are_strict() {
        let token = Token::from_bytes([0x22; TOKEN_BYTES]);
        let token_text = token.expose().to_string();
        let snapshot = snapshot();
        let now = Instant::now();

        let cases = [
            (
                request("GET", PRESENCE_PATH, "localhost:43123", ORIGIN, &token_text),
                Status::MisdirectedRequest,
            ),
            (
                request("GET", PRESENCE_PATH, HOST, "null", &token_text),
                Status::Forbidden,
            ),
            (
                request("GET", PRESENCE_PATH, HOST, ORIGIN, "wrong"),
                Status::Unauthorized,
            ),
            (
                request("POST", PRESENCE_PATH, HOST, ORIGIN, &token_text),
                Status::MethodNotAllowed,
            ),
            (
                request("GET", "/v1/other", HOST, ORIGIN, &token_text),
                Status::NotFound,
            ),
            (
                request("GET", "/v1/presence?token=bad", HOST, ORIGIN, &token_text),
                Status::NotFound,
            ),
        ];

        for (wire, expected) in cases {
            let mut server =
                ServerState::new(HOST.to_string(), Token::from_bytes([0x22; TOKEN_BYTES]));
            assert_eq!(server.handle(&wire, &snapshot, now).status, expected);
        }
    }

    #[test]
    fn duplicate_security_headers_and_request_bodies_are_rejected() {
        let token = Token::from_bytes([0x2a; TOKEN_BYTES]);
        let token_text = token.expose().to_string();
        let snapshot = snapshot();
        let now = Instant::now();
        let duplicate_host = format!(
            "GET {PRESENCE_PATH} HTTP/1.1\r\nHost: {HOST}\r\nHost: {HOST}\r\nOrigin: {ORIGIN}\r\nAuthorization: Bearer {token_text}\r\n\r\n"
        );
        let request_body = format!(
            "GET {PRESENCE_PATH} HTTP/1.1\r\nHost: {HOST}\r\nOrigin: {ORIGIN}\r\nAuthorization: Bearer {token_text}\r\nContent-Length: 1\r\n\r\n"
        );
        let duplicate_transfer_encoding = format!(
            "GET {PRESENCE_PATH} HTTP/1.1\r\nHost: {HOST}\r\nOrigin: {ORIGIN}\r\nAuthorization: Bearer {token_text}\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n\r\n"
        );

        for wire in [duplicate_host, request_body, duplicate_transfer_encoding] {
            let mut server =
                ServerState::new(HOST.to_string(), Token::from_bytes([0x2a; TOKEN_BYTES]));
            assert_eq!(
                server.handle(wire.as_bytes(), &snapshot, now).status,
                Status::BadRequest
            );
        }
    }

    #[test]
    fn options_allows_only_the_expected_preflight() {
        let token = Token::from_bytes([0x33; TOKEN_BYTES]);
        let mut server = ServerState::new(HOST.to_string(), token);
        let wire = format!(
            "OPTIONS {PRESENCE_PATH} HTTP/1.1\r\nHost: {HOST}\r\nOrigin: {ORIGIN}\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: authorization\r\n\r\n"
        );
        let response = server.handle(wire.as_bytes(), &snapshot(), Instant::now());

        assert_eq!(response.status, Status::NoContent);
        assert!(response.body.is_empty());
        assert_eq!(response.header("access-control-allow-origin"), Some(ORIGIN));
        assert_eq!(response.header("cache-control"), Some("no-store"));
        assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    }

    #[test]
    fn request_and_rate_limits_fail_closed() {
        let token = Token::from_bytes([0x44; TOKEN_BYTES]);
        let token_text = token.expose().to_string();
        let wire = request("GET", PRESENCE_PATH, HOST, ORIGIN, &token_text);
        let mut server = ServerState::new(HOST.to_string(), token);
        let now = Instant::now();

        assert_eq!(
            server
                .handle(&vec![b'a'; MAX_REQUEST_BYTES + 1], &snapshot(), now)
                .status,
            Status::RequestHeaderFieldsTooLarge
        );
        for _ in 0..RATE_LIMIT_BURST {
            assert_eq!(server.handle(&wire, &snapshot(), now).status, Status::Ok);
        }
        assert_eq!(
            server.handle(&wire, &snapshot(), now).status,
            Status::TooManyRequests
        );
    }

    #[test]
    fn maximum_registry_snapshot_fits_the_response_budget() {
        let now = Instant::now();
        let mut registry = Registry::default();
        for index in 0..128 {
            registry.resolved(
                &format!("peer_{index:03}._moq._udp.local."),
                now,
                Duration::from_secs(30),
            );
        }
        let snapshot = registry.snapshot(128, Lifecycle::Browsing);
        let token = Token::from_bytes([0x4a; TOKEN_BYTES]);
        let token_text = token.expose().to_string();
        let mut server = ServerState::new(HOST.to_string(), token);
        let response = server.handle(
            &request("GET", PRESENCE_PATH, HOST, ORIGIN, &token_text),
            &snapshot,
            now,
        );

        assert_eq!(response.status, Status::Ok);
        assert!(response.body.len() <= MAX_RESPONSE_BYTES);
        let body: serde_json::Value = serde_json::from_slice(&response.body).expect("JSON body");
        assert_eq!(body["devices"].as_array().expect("devices").len(), 128);
    }

    #[test]
    fn a_new_process_rejects_the_previous_token() {
        let old_token = Token::from_bytes([0x55; TOKEN_BYTES]);
        let new_token = Token::from_bytes([0x66; TOKEN_BYTES]);
        let wire = request("GET", PRESENCE_PATH, HOST, ORIGIN, old_token.expose());
        let mut restarted = ServerState::new(HOST.to_string(), new_token);

        assert_eq!(
            restarted.handle(&wire, &snapshot(), Instant::now()).status,
            Status::Unauthorized
        );
    }

    #[test]
    fn listener_is_loopback_only_and_restart_rejects_the_old_token() {
        let view = DiscoveryView::from_snapshot(snapshot());
        let (first, first_bootstrap) = BridgeOwner::start(view.clone()).expect("start bridge");
        let first_address: SocketAddr = first_bootstrap
            .endpoint
            .strip_prefix("http://")
            .expect("HTTP endpoint")
            .parse()
            .expect("socket address");
        let old_token = fragment_token(first_bootstrap.open_url()).to_string();

        assert_eq!(first_address.ip(), Ipv4Addr::LOCALHOST);
        first.stop();
        assert!(TcpStream::connect_timeout(&first_address, Duration::from_millis(100)).is_err());

        let (second, second_bootstrap) = BridgeOwner::start(view).expect("restart bridge");
        let second_address: SocketAddr = second_bootstrap
            .endpoint
            .strip_prefix("http://")
            .expect("HTTP endpoint")
            .parse()
            .expect("socket address");
        let second_host = second_address.to_string();
        let new_token = fragment_token(second_bootstrap.open_url());
        assert_ne!(old_token, new_token);

        let response = send(
            second_address,
            &request("GET", PRESENCE_PATH, &second_host, ORIGIN, &old_token),
        );
        assert!(
            response.starts_with(b"HTTP/1.1 401 Unauthorized\r\n"),
            "unexpected response status: {}",
            String::from_utf8_lossy(&response)
                .lines()
                .next()
                .unwrap_or("empty response")
        );
        second.stop();
    }

    #[test]
    fn repeated_shutdown_does_not_open_another_wake_connection() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind listener");
        listener.set_nonblocking(true).expect("set nonblocking");
        let (stop, stop_rx) = mpsc::sync_channel(1);
        let mut bridge = BridgeOwner {
            stop,
            wake: listener.local_addr().expect("listener address"),
            worker: None,
        };

        bridge.stop_and_join();

        assert!(matches!(stop_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        match listener.accept() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("stopped bridge opened another wake connection"),
            Err(error) => panic!("unexpected accept error: {error}"),
        }
    }

    #[test]
    fn slow_drip_cannot_extend_request_deadline_or_block_shutdown() {
        let view = DiscoveryView::from_snapshot(snapshot());
        let (bridge, bootstrap) = BridgeOwner::start(view).expect("start bridge");
        let address: SocketAddr = bootstrap
            .endpoint
            .strip_prefix("http://")
            .expect("HTTP endpoint")
            .parse()
            .expect("socket address");
        let mut stream = TcpStream::connect(address).expect("connect to bridge");
        let dripper = thread::spawn(move || {
            for _ in 0..30 {
                if stream.write_all(b"G").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });

        thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        bridge.stop();
        let elapsed = started.elapsed();
        dripper.join().expect("join slow dripper");

        assert!(
            elapsed < REQUEST_DEADLINE + Duration::from_millis(500),
            "shutdown exceeded the absolute request deadline: {elapsed:?}"
        );
    }
}
