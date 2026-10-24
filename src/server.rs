//! The two doors: a Unix socket for the app's commands, and loopback HTTP
//! for the page.
//!
//! The socket sits in a directory only the owner can enter, so file
//! permissions guard it and it needs no token. It takes a tell (`POST /tell`,
//! the project on the first line of the body and the JSON after it) and a
//! reveal (`POST /reveal`, the moniker, then an optional `state: <json>`
//! line).
//!
//! The page needs TCP, which any local process and any web page can try. So
//! every HTTP request must name this host as `127.0.0.1:<port>` or
//! `localhost:<port>`, which turns away a page that reaches it through a
//! rebound DNS name, and every relay route needs the token that only the
//! page's URL carries.

use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, UnixListener};

use crate::message::{Answer, BadTell, MAX_ANSWER, MAX_KEEP, MAX_TELL, TellRequest};
use crate::moniker::Moniker;
use crate::policy::Policy;
use crate::relay::{NoAnswer, Relay};

/// Room for the project line in front of a tell.
const MAX_PROJECT_LINE: usize = 1024;
/// How long the page's request for the next tell stays open.
const POLL_WAIT: Duration = Duration::from_secs(25);

type Reply = Response<Full<Bytes>>;

/// What the host serves, and where its commands reach it.
#[derive(Debug, Clone)]
pub struct Config {
    /// The project the page belongs to: one path segment, as a tell names it.
    pub project: String,
    pub build: Build,
    pub policy: Policy,
    /// The commands' socket. [`socket_path`] gives the usual one.
    pub socket: PathBuf,
}

/// The page's build. Either way it holds `index.html`.
#[derive(Debug, Clone)]
pub enum Build {
    /// A folder on disk, read on each request, so a rebuild shows on reload.
    Folder(PathBuf),
    /// Files compiled into the program (`include_bytes!`), as
    /// `(path, contents)` with paths relative and `/`-separated.
    Embedded(&'static [(&'static str, &'static [u8])]),
}

impl Build {
    fn has_index(&self) -> bool {
        match self {
            Build::Folder(root) => root.join("index.html").is_file(),
            Build::Embedded(files) => files.iter().any(|(path, _)| *path == "index.html"),
        }
    }

    /// The file at `relative`, already checked by [`safe_path`].
    async fn read(&self, relative: &Path) -> Option<Bytes> {
        match self {
            Build::Folder(root) => tokio::fs::read(root.join(relative))
                .await
                .ok()
                .map(Bytes::from),
            Build::Embedded(files) => files
                .iter()
                .find(|(path, _)| Path::new(path) == relative)
                .map(|(_, bytes)| Bytes::from_static(bytes)),
        }
    }
}

impl std::fmt::Display for Build {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Build::Folder(root) => write!(f, "{}", root.display()),
            Build::Embedded(_) => f.write_str("the embedded build"),
        }
    }
}

/// `pair-ui/<project>.sock` under `$XDG_RUNTIME_DIR`, else `$TMPDIR` (a
/// per-user directory on macOS), else `/tmp`.
pub fn socket_path(project: &str) -> PathBuf {
    let base = ["XDG_RUNTIME_DIR", "TMPDIR"]
        .iter()
        .find_map(std::env::var_os)
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    base.join("pair-ui").join(format!("{project}.sock"))
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("not a project name (one path segment, no whitespace): {0:?}")]
    BadProject(String),
    #[error("{0} has no index.html")]
    NoIndex(Build),
    #[error("could not listen on 127.0.0.1: {0}")]
    Bind(std::io::Error),
    #[error("could not open the socket {0}: {1}")]
    Socket(PathBuf, std::io::Error),
    #[error("another host is listening on {0}")]
    SocketInUse(PathBuf),
    #[error("no random source for the token: {0}")]
    Random(getrandom::Error),
}

pub struct Host {
    pages: TcpListener,
    commands: UnixListener,
    state: Arc<State>,
}

struct State {
    project: String,
    addr: SocketAddr,
    token: String,
    socket: PathBuf,
    build: Build,
    csp: HeaderValue,
    hosts: [String; 2],
    relay: Relay,
}

impl Host {
    /// Listens on a free port of 127.0.0.1 and on the config's socket.
    pub async fn bind(config: Config) -> Result<Host, HostError> {
        let project = valid_project(config.project)?;
        if !config.build.has_index() {
            return Err(HostError::NoIndex(config.build));
        }
        let commands = bind_socket(&config.socket)?;
        let pages = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(HostError::Bind)?;
        let addr = pages.local_addr().map_err(HostError::Bind)?;
        let csp = HeaderValue::from_str(&config.policy.csp())
            .expect("a policy is built from checked origins only");
        let hosts = [
            format!("127.0.0.1:{}", addr.port()),
            format!("localhost:{}", addr.port()),
        ];
        let state = State {
            project,
            addr,
            token: token()?,
            socket: config.socket,
            build: config.build,
            csp,
            hosts,
            relay: Relay::default(),
        };
        let state = Arc::new(state);
        Ok(Host {
            pages,
            commands,
            state,
        })
    }

    /// The address to open the page at. It carries the token, so show it to
    /// the person, not to logs.
    pub fn page_url(&self) -> String {
        page_url(&self.state)
    }

    pub fn socket(&self) -> &Path {
        &self.state.socket
    }

    /// Answers both doors until the process ends.
    pub async fn serve(self) -> std::io::Result<()> {
        let Host {
            pages,
            commands,
            state,
        } = self;
        tokio::try_join!(
            serve_pages(pages, state.clone()),
            serve_commands(commands, state)
        )?;
        Ok(())
    }
}

async fn serve_pages(listener: TcpListener, state: Arc<State>) -> std::io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        spawn(stream, state.clone(), page_route);
    }
}

async fn serve_commands(listener: UnixListener, state: Arc<State>) -> std::io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        spawn(stream, state.clone(), command_route);
    }
}

fn spawn<S, F, Fut>(stream: S, state: Arc<State>, route: F)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: Fn(Arc<State>, Request<Incoming>) -> Fut + Send + Copy + 'static,
    Fut: Future<Output = Result<Reply, hyper::Error>> + Send + 'static,
{
    tokio::spawn(async move {
        let service = service_fn(move |req| route(state.clone(), req));
        // A connection dropped mid-request is not our error.
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });
}

fn page_url(state: &State) -> String {
    format!("http://{}/?pair={}", state.addr, state.token)
}

fn valid_project(name: String) -> Result<String, HostError> {
    let good = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(|c: char| c == '/' || c.is_whitespace() || c.is_control());
    if good {
        Ok(name)
    } else {
        Err(HostError::BadProject(name))
    }
}

/// Binds the socket in a directory only the owner can enter, replacing a
/// stale one left by a host that died, but never one that still answers.
fn bind_socket(path: &Path) -> Result<UnixListener, HostError> {
    let fail = |e| HostError::Socket(path.to_path_buf(), e);
    let dir = path
        .parent()
        .ok_or_else(|| fail(std::io::ErrorKind::InvalidInput.into()))?;
    std::fs::create_dir_all(dir).map_err(fail)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(fail)?;
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(HostError::SocketInUse(path.to_path_buf()));
        }
        std::fs::remove_file(path).map_err(fail)?;
    }
    let listener = UnixListener::bind(path).map_err(fail)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(fail)?;
    Ok(listener)
}

fn token() -> Result<String, HostError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(HostError::Random)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The commands' door: `POST /tell`, `POST /reveal`, and `GET /page` for the
/// page's URL.
async fn command_route(state: Arc<State>, req: Request<Incoming>) -> Result<Reply, hyper::Error> {
    Ok(match (req.method().clone(), req.uri().path()) {
        (Method::POST, "/tell") => match read_bytes(req, MAX_TELL + MAX_PROJECT_LINE).await {
            Ok(bytes) => tell(&state, &bytes).await,
            Err((status, why)) => error(status, &why),
        },
        (Method::POST, "/reveal") => match read_bytes(req, MAX_TELL + MAX_PROJECT_LINE).await {
            Ok(bytes) => reveal(&state, &bytes).await,
            Err((status, why)) => error(status, &why),
        },
        (Method::GET, "/page") => reply(StatusCode::OK, &json!({ "url": page_url(&state) })),
        _ => error(StatusCode::NOT_FOUND, "no such route"),
    })
}

async fn tell(state: &State, bytes: &[u8]) -> Reply {
    match TellRequest::parse(bytes) {
        Ok(tell) if tell.project() == state.project => {
            answered(state.relay.tell(tell.into_body()).await)
        }
        Ok(tell) => error(
            StatusCode::NOT_FOUND,
            &format!(
                "this host is for {}, not {:?}",
                state.project,
                tell.project()
            ),
        ),
        Err(bad @ BadTell::TooLarge(_)) => error(StatusCode::PAYLOAD_TOO_LARGE, &bad.to_string()),
        Err(bad) => error(StatusCode::BAD_REQUEST, &bad.to_string()),
    }
}

/// A reveal: the moniker on the first line, then an
/// optional `state: <json>` line.
async fn reveal(state: &State, bytes: &[u8]) -> Reply {
    let request = std::str::from_utf8(bytes)
        .map_err(|_| "the reveal is not UTF-8".to_string())
        .and_then(parse_reveal);
    let (moniker, view) = match request {
        Ok(parts) => parts,
        Err(why) => return error(StatusCode::BAD_REQUEST, &why),
    };
    match moniker.parse::<Moniker>() {
        Ok(m) if m.project() == state.project => {
            answered(state.relay.reveal(moniker.to_string(), view).await)
        }
        Ok(m) => error(
            StatusCode::NOT_FOUND,
            &format!("this host is for {}, not {:?}", state.project, m.project()),
        ),
        Err(bad) => error(StatusCode::BAD_REQUEST, &bad.to_string()),
    }
}

fn parse_reveal(text: &str) -> Result<(&str, Value), String> {
    let mut lines = text.lines();
    let moniker = lines
        .next()
        .ok_or("the body is a moniker, then an optional `state: <json>` line")?;
    let view = match (lines.next(), lines.next()) {
        (None, _) => Value::Null,
        (Some(line), None) => {
            let json = line
                .strip_prefix("state: ")
                .ok_or_else(|| format!("unknown reveal option: {line}"))?;
            serde_json::from_str(json).map_err(|e| format!("the state is not JSON: {e}"))?
        }
        (Some(_), Some(extra)) => return Err(format!("unknown reveal option: {extra}")),
    };
    Ok((moniker, view))
}

fn answered(outcome: Result<Answer, NoAnswer>) -> Reply {
    match outcome {
        Ok(answer) => reply(StatusCode::OK, &answer),
        Err(none) => error(StatusCode::GATEWAY_TIMEOUT, &none.to_string()),
    }
}

/// The page's door: its files, and the relay routes under `/_pair/`.
async fn page_route(state: Arc<State>, req: Request<Incoming>) -> Result<Reply, hyper::Error> {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    if !host.is_some_and(|h| state.hosts.iter().any(|ours| ours == h)) {
        return Ok(error(
            StatusCode::MISDIRECTED_REQUEST,
            "this host answers only to 127.0.0.1 or localhost",
        ));
    }
    let path = req.uri().path().to_string();
    Ok(match path.strip_prefix("/_pair/") {
        Some(_) if !authorised(&state, &req) => {
            error(StatusCode::UNAUTHORIZED, "missing or wrong token")
        }
        Some(action) => relay(&state, action, req).await,
        None if req.method() == Method::GET => file(&state, &path).await,
        None => error(StatusCode::METHOD_NOT_ALLOWED, "files are read-only"),
    })
}

fn authorised(state: &State, req: &Request<Incoming>) -> bool {
    let expected = format!("Bearer {}", state.token);
    req.headers()
        .get(header::AUTHORIZATION)
        .is_some_and(|given| same(given.as_bytes(), expected.as_bytes()))
}

/// Compares without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The page's answer to a tell or reveal.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answered {
    id: u64,
    ok: bool,
    message: String,
    result: Option<Value>,
}

async fn relay(state: &State, action: &str, req: Request<Incoming>) -> Reply {
    let relay = &state.relay;
    match (req.method().clone(), action) {
        (Method::GET, "project") => reply(StatusCode::OK, &json!({ "project": state.project })),
        (Method::GET, "next") => match relay.next(POLL_WAIT).await {
            Some(message) => reply(StatusCode::OK, &message.for_page()),
            None => empty(StatusCode::NO_CONTENT),
        },
        (Method::POST, "answer") => match read::<Answered>(req, MAX_ANSWER).await {
            Ok(answered) => deliver(relay, answered),
            Err((status, why)) => error(status, &why),
        },
        (Method::PUT, "keep") => match read::<Value>(req, MAX_KEEP).await {
            Ok(view) => {
                relay.keep(Some(view).filter(|v| !v.is_null()));
                empty(StatusCode::NO_CONTENT)
            }
            Err((status, why)) => error(status, &why),
        },
        (Method::GET, "recall") => reply(StatusCode::OK, &relay.recall().unwrap_or(Value::Null)),
        _ => error(StatusCode::NOT_FOUND, "no such route"),
    }
}

fn deliver(
    relay: &Relay,
    Answered {
        id,
        ok,
        message,
        result,
    }: Answered,
) -> Reply {
    if relay.answer(
        id,
        Answer {
            ok,
            message,
            result,
        },
    ) {
        empty(StatusCode::NO_CONTENT)
    } else {
        error(
            StatusCode::NOT_FOUND,
            &format!("nothing waits for an answer to {id}"),
        )
    }
}

/// The request's body, or why it is refused.
async fn read_bytes(req: Request<Incoming>, limit: usize) -> Result<Bytes, (StatusCode, String)> {
    Limited::new(req.into_body(), limit)
        .collect()
        .await
        .map(|body| body.to_bytes())
        .map_err(|_| {
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("the body is over {limit} bytes"),
            )
        })
}

/// The request's JSON body, or why it is refused.
async fn read<T: serde::de::DeserializeOwned>(
    req: Request<Incoming>,
    limit: usize,
) -> Result<T, (StatusCode, String)> {
    let bytes = read_bytes(req, limit).await?;
    serde_json::from_slice(&bytes).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("not the JSON expected: {e}"),
        )
    })
}

async fn file(state: &State, path: &str) -> Reply {
    let Some(relative) = safe_path(path) else {
        return error(StatusCode::NOT_FOUND, "no such file");
    };
    match state.build.read(&relative).await {
        Some(bytes) => Response::builder()
            .header(header::CONTENT_TYPE, content_type(&relative))
            .header(header::CONTENT_SECURITY_POLICY, state.csp.clone())
            .header(header::REFERRER_POLICY, "no-referrer")
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Full::new(bytes))
            .expect("static headers are valid"),
        None => error(StatusCode::NOT_FOUND, "no such file"),
    }
}

/// A request path as a file under the root: plain names only, so nothing
/// outside the root, no hidden file, and no escapes to decode.
fn safe_path(path: &str) -> Option<PathBuf> {
    let path = if path == "/" { "/index.html" } else { path };
    let relative = Path::new(path.strip_prefix('/')?);
    let plain = |c: Component| match c {
        Component::Normal(name) => name
            .to_str()
            .is_some_and(|n| !n.starts_with('.') && !n.contains(['%', '\\'])),
        _ => false,
    };
    relative
        .components()
        .all(plain)
        .then(|| relative.to_path_buf())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn reply(status: StatusCode, value: &impl Serialize) -> Reply {
    let body = serde_json::to_vec(value).expect("our replies serialize");
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from(body)))
        .expect("static headers are valid")
}

fn error(status: StatusCode, message: &str) -> Reply {
    reply(status, &json!({ "error": message }))
}

fn empty(status: StatusCode) -> Reply {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .expect("static headers are valid")
}
