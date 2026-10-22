use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper::{HeaderMap, Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::{TcpStream, UnixStream};

use pair_ui::{Answer, Build, Config, Host, HostError, Moniker, Policy, client};

const PROJECT: &str = "demo";

/// A fresh folder holding a page build (`index.html`, a script, a hidden
/// file) and, in a subfolder, the commands' socket.
fn folder(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("pair-ui-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&root).expect("temp dir");
    std::fs::write(root.join("index.html"), "<p>page</p>").expect("write");
    std::fs::write(root.join("page.js"), "1").expect("write");
    std::fs::write(root.join(".secret"), "no").expect("write");
    root
}

fn config(root: &Path) -> Config {
    let connect = vec!["http://localhost:3381".parse().expect("origin")];
    Config {
        project: PROJECT.into(),
        build: Build::Folder(root.to_path_buf()),
        policy: Policy {
            connect,
            wasm: true,
        },
        socket: root.join("run").join("demo.sock"),
    }
}

/// A running host, as the page and the commands find it.
#[derive(Clone)]
struct Running {
    socket: PathBuf,
    addr: SocketAddr,
    token: String,
    url: String,
}

async fn start(name: &str) -> Running {
    let host = Host::bind(config(&folder(name))).await.expect("bind");
    let url = host.page_url();
    let (addr, token) = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split_once("/?pair="))
        .expect("http://<addr>/?pair=<token>");
    let running = Running {
        socket: host.socket().to_path_buf(),
        addr: addr.parse().expect("addr"),
        token: token.into(),
        url: url.clone(),
    };
    tokio::spawn(host.serve());
    running
}

/// One HTTP request to the page's door.
struct Sent<'a> {
    method: Method,
    path: &'a str,
    host: String,
    token: Option<String>,
    body: Option<Value>,
}

impl<'a> Sent<'a> {
    fn new(method: Method, path: &'a str, to: &Running) -> Self {
        let host = to.addr.to_string();
        let token = Some(to.token.clone());
        Sent {
            method,
            path,
            host,
            token,
            body: None,
        }
    }
}

async fn send(to: &Running, sent: Sent<'_>) -> (StatusCode, HeaderMap, Bytes) {
    let stream = TcpStream::connect(to.addr).await.expect("connect");
    let (mut sender, conn) = http1::handshake(TokioIo::new(stream))
        .await
        .expect("handshake");
    tokio::spawn(conn);
    let mut request = Request::builder()
        .method(sent.method)
        .uri(sent.path)
        .header("host", sent.host);
    if let Some(token) = sent.token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = sent.body.map(|b| b.to_string()).unwrap_or_default();
    let response = sender
        .send_request(request.body(Full::new(Bytes::from(body))).expect("request"))
        .await
        .expect("response");
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.expect("body").to_bytes();
    (parts.status, parts.headers, bytes)
}

fn parse(moniker: &str) -> Moniker {
    moniker.parse().expect("a moniker")
}

/// Sends `body` to the socket's reveal route as it is, past the client's
/// typed moniker, and returns the host's reason for refusing it.
async fn raw_reveal(socket: &Path, body: &str) -> String {
    let stream = UnixStream::connect(socket).await.expect("socket");
    let (mut sender, connection) = http1::handshake(TokioIo::new(stream))
        .await
        .expect("handshake");
    tokio::spawn(connection);
    let request = Request::builder()
        .method(Method::POST)
        .uri("/reveal")
        .header(hyper::header::HOST, "pair-ui")
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("request");
    let response = sender.send_request(request).await.expect("reply");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body:?}");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    json_of(&bytes)["error"]
        .as_str()
        .expect("an error")
        .to_string()
}

fn json_of(bytes: &Bytes) -> Value {
    serde_json::from_slice(bytes).expect("JSON")
}

/// Plays the page once: takes the next message, answers it, returns it.
async fn page_answers(to: Running, answer: Value) -> Value {
    let (status, _, message) = send(&to, Sent::new(Method::GET, "/_pair/next", &to)).await;
    assert_eq!(status, StatusCode::OK);
    let message = json_of(&message);
    assert_eq!(message["pair"], 1);
    let kind = ["tell", "reveal"]
        .into_iter()
        .find(|k| message.get(k).is_some())
        .expect("a tell or a reveal");
    let mut answer = answer;
    answer["id"] = message[kind]["id"].clone();
    let sent = Sent {
        body: Some(answer),
        ..Sent::new(Method::POST, "/_pair/answer", &to)
    };
    assert_eq!(send(&to, sent).await.0, StatusCode::NO_CONTENT);
    message
}

#[tokio::test]
async fn a_tell_reaches_the_page_untouched_and_its_answer_comes_back() {
    let to = start("roundtrip").await;
    let answer = json!({ "ok": true, "message": "shown", "result": { "n": 3 } });
    let page = tokio::spawn(page_answers(to.clone(), answer));
    let body = json!({ "app": 1, "open": "37:0xabc", "nested": [1, { "x": null }] });
    let answer = client::tell(&to.socket, PROJECT, &body)
        .await
        .expect("answer");
    let expected = Answer {
        ok: true,
        message: "shown".into(),
        result: Some(json!({ "n": 3 })),
    };
    assert_eq!(answer, expected);
    let seen = page.await.expect("page");
    assert_eq!(seen["tell"]["body"], body);
}

#[tokio::test]
async fn a_reveal_reaches_the_page_with_its_state_and_its_answer_comes_back() {
    let to = start("reveal").await;
    let page = tokio::spawn(page_answers(
        to.clone(),
        json!({ "ok": true, "message": "thread shown" }),
    ));
    let moniker = "demo://demo/thread/37:0xabc#cast=1:0x2";
    let state = json!({ "node": "http://localhost:3381" });
    let answer = client::reveal(&to.socket, &parse(moniker), Some(&state))
        .await
        .expect("answer");
    assert!(answer.ok);
    assert_eq!(answer.message, "thread shown");
    let seen = page.await.expect("page");
    assert_eq!(seen["reveal"]["moniker"], moniker);
    assert_eq!(seen["reveal"]["state"], state);
    assert!(seen.get("tell").is_none());
}

#[tokio::test]
async fn a_reveal_without_state_hands_the_page_null() {
    let to = start("nostate").await;
    let page = tokio::spawn(page_answers(
        to.clone(),
        json!({ "ok": true, "message": "" }),
    ));
    client::reveal(&to.socket, &parse("demo://demo"), None)
        .await
        .expect("answer");
    assert_eq!(page.await.expect("page")["reveal"]["state"], Value::Null);
}

#[tokio::test]
async fn a_reveal_is_refused_for_another_project_or_a_bad_request() {
    let to = start("badreveal").await;
    let other = client::reveal(&to.socket, &parse("demo://other/thread"), None)
        .await
        .expect_err("refused")
        .to_string();
    assert!(other.contains("this host is for demo"), "{other}");
    for bad in [
        "demo",
        "Demo://demo",
        "demo://",
        "demo://../x",
        "demo://de mo",
    ] {
        let why = raw_reveal(&to.socket, bad).await;
        assert!(why.contains("not a moniker"), "{bad}: {why}");
    }
    let extra = raw_reveal(&to.socket, "demo://demo\nzoom: 2").await;
    assert!(extra.contains("unknown reveal option"), "{extra}");
}

#[tokio::test]
async fn the_page_learns_its_project_from_the_host() {
    let to = start("project-route").await;
    let (status, _, body) = send(&to, Sent::new(Method::GET, "/_pair/project", &to)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_of(&body), json!({ "project": PROJECT }));
}

#[tokio::test]
async fn a_refusal_is_an_answer_not_an_error() {
    let to = start("refusal").await;
    tokio::spawn(page_answers(
        to.clone(),
        json!({ "ok": false, "message": "no such cast" }),
    ));
    let answer = client::tell(&to.socket, PROJECT, &json!({}))
        .await
        .expect("answer");
    assert!(!answer.ok);
    assert_eq!(answer.message, "no such cast");
}

#[tokio::test]
async fn a_tell_with_no_page_says_so() {
    let to = start("nopage").await;
    let error = client::tell(&to.socket, PROJECT, &json!({}))
        .await
        .expect_err("no page");
    assert!(error.to_string().contains("is the page open?"), "{error}");
}

#[tokio::test]
async fn a_tell_for_another_project_is_refused() {
    let to = start("project").await;
    let error = client::tell(&to.socket, "other", &json!({}))
        .await
        .expect_err("refused");
    assert!(
        error.to_string().contains("this host is for demo"),
        "{error}"
    );
}

#[tokio::test]
async fn the_socket_gives_the_page_url_and_only_the_owner_can_open_it() {
    let to = start("socket").await;
    assert_eq!(client::page_url(&to.socket).await.expect("url"), to.url);
    let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode(&to.socket), 0o600);
    assert_eq!(mode(to.socket.parent().expect("dir")), 0o700);
}

#[tokio::test]
async fn a_second_host_cannot_take_a_live_socket_but_replaces_a_stale_one() {
    let root = folder("second");
    let first = Host::bind(config(&root)).await.expect("bind");
    tokio::spawn(first.serve());
    let refused = Host::bind(config(&root)).await.err().expect("in use");
    assert!(matches!(refused, HostError::SocketInUse(_)), "{refused}");

    let stale = folder("stale");
    let path = config(&stale).socket;
    std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
    drop(std::os::unix::net::UnixListener::bind(&path).expect("stale socket"));
    assert!(Host::bind(config(&stale)).await.is_ok());
}

#[tokio::test]
async fn the_page_door_carries_no_tells_in() {
    let to = start("nodoor").await;
    let sent = Sent {
        body: Some(json!({})),
        ..Sent::new(Method::POST, "/_pair/tell", &to)
    };
    assert_eq!(send(&to, sent).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn relay_routes_need_the_token_and_every_route_a_loopback_host() {
    let to = start("gates").await;
    let no_token = Sent {
        token: None,
        ..Sent::new(Method::GET, "/_pair/recall", &to)
    };
    assert_eq!(send(&to, no_token).await.0, StatusCode::UNAUTHORIZED);
    let wrong = Sent {
        token: Some("0".repeat(64)),
        ..Sent::new(Method::GET, "/_pair/recall", &to)
    };
    assert_eq!(send(&to, wrong).await.0, StatusCode::UNAUTHORIZED);
    let rebound = Sent {
        host: format!("evil.example:{}", to.addr.port()),
        ..Sent::new(Method::GET, "/", &to)
    };
    assert_eq!(send(&to, rebound).await.0, StatusCode::MISDIRECTED_REQUEST);
    let named = Sent {
        host: format!("localhost:{}", to.addr.port()),
        ..Sent::new(Method::GET, "/", &to)
    };
    assert_eq!(send(&to, named).await.0, StatusCode::OK);
}

#[tokio::test]
async fn files_are_served_under_the_policy_and_only_from_the_build() {
    let to = start("files").await;
    let (status, headers, body) = send(&to, Sent::new(Method::GET, "/", &to)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<p>page</p>");
    let csp = headers["content-security-policy"].to_str().expect("ascii");
    assert!(
        csp.contains("connect-src 'self' http://localhost:3381;"),
        "{csp}"
    );
    assert!(csp.contains("'wasm-unsafe-eval'"), "{csp}");
    let (_, headers, _) = send(&to, Sent::new(Method::GET, "/page.js", &to)).await;
    assert!(
        headers["content-type"]
            .to_str()
            .expect("ascii")
            .starts_with("text/javascript")
    );
    for path in [
        "/.secret",
        "/../Cargo.toml",
        "/%2e%2e/x",
        "/missing.js",
        "/run/demo.sock",
    ] {
        let (status, _, _) = send(&to, Sent::new(Method::GET, path, &to)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn the_page_keeps_one_value_and_null_forgets_it() {
    let to = start("keep").await;
    let recall = || send(&to, Sent::new(Method::GET, "/_pair/recall", &to));
    assert_eq!(json_of(&recall().await.2), Value::Null);
    let keep = |v: Value| Sent {
        body: Some(v),
        ..Sent::new(Method::PUT, "/_pair/keep", &to)
    };
    assert_eq!(
        send(&to, keep(json!({ "cast": "1:0x2" }))).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(json_of(&recall().await.2), json!({ "cast": "1:0x2" }));
    let big = json!("x".repeat(pair_ui::MAX_KEEP));
    assert_eq!(send(&to, keep(big)).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(send(&to, keep(Value::Null)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(json_of(&recall().await.2), Value::Null);
}

#[tokio::test]
async fn an_answer_with_fields_we_do_not_know_is_refused() {
    let to = start("strict").await;
    let odd = json!({ "id": 0, "ok": true, "message": "", "extra": 1 });
    let sent = Sent {
        body: Some(odd),
        ..Sent::new(Method::POST, "/_pair/answer", &to)
    };
    assert_eq!(send(&to, sent).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_embedded_build_is_served_like_a_folder() {
    static FILES: &[(&str, &[u8])] = &[("index.html", b"<p>embedded</p>"), ("js/page.js", b"2")];
    let root = folder("embedded");
    let host = Host::bind(Config {
        build: Build::Embedded(FILES),
        ..config(&root)
    })
    .await
    .expect("bind");
    let url = host.page_url();
    let (addr, token) = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split_once("/?pair="))
        .expect("url");
    let to = Running {
        socket: host.socket().to_path_buf(),
        addr: addr.parse().expect("addr"),
        token: token.into(),
        url: url.clone(),
    };
    tokio::spawn(host.serve());
    let (status, headers, body) = send(&to, Sent::new(Method::GET, "/", &to)).await;
    assert_eq!(
        (status, body.as_ref()),
        (StatusCode::OK, b"<p>embedded</p>".as_ref())
    );
    assert!(headers.contains_key("content-security-policy"));
    assert_eq!(
        send(&to, Sent::new(Method::GET, "/js/page.js", &to))
            .await
            .2,
        "2"
    );
    for path in ["/page.js", "/../index.html", "/js/../index.html"] {
        assert_eq!(
            send(&to, Sent::new(Method::GET, path, &to)).await.0,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

#[tokio::test]
async fn an_embedded_build_without_index_html_is_refused() {
    static FILES: &[(&str, &[u8])] = &[("page.js", b"1")];
    let refused = Host::bind(Config {
        build: Build::Embedded(FILES),
        ..config(&folder("noindex"))
    })
    .await
    .err()
    .expect("refused");
    assert!(matches!(refused, HostError::NoIndex(_)), "{refused}");
}
