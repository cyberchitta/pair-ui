//! The `pair-ui` binary, run as an app's users would run it: `serve` in the
//! background, `tell` and `reveal` against it, and a page played over HTTP.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_pair-ui");

/// A temp dir standing in for `$TMPDIR`, so each test has its own socket,
/// holding a page build.
fn place(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pair-ui-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(dir.join("build")).expect("dir");
    std::fs::write(dir.join("build/index.html"), "<p>page</p>").expect("write");
    dir
}

fn pair_ui(tmp: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env("TMPDIR", tmp)
        .env_remove("XDG_RUNTIME_DIR");
    command
}

/// Where the page reaches its host: the address and token `serve` printed.
#[derive(Clone)]
struct Door {
    addr: String,
    token: String,
}

/// A running `pair-ui serve`.
struct Served {
    child: Child,
    door: Door,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn serve(tmp: &Path) -> Served {
    let build = tmp.join("build");
    let mut child = pair_ui(tmp, &["serve", "demo", build.to_str().expect("utf-8")])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn serve");
    let mut first = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut first)
        .expect("read the address");
    let url = first
        .trim()
        .strip_prefix("open the page at http://")
        .expect(&first);
    let (addr, token) = url.split_once("/?pair=").expect("address and token");
    let door = Door {
        addr: addr.into(),
        token: token.into(),
    };
    Served { child, door }
}

/// One HTTP/1.1 request to the page's door, as the page makes it.
fn page(door: &Door, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(&door.addr).expect("connect");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        door.addr,
        door.token,
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read");
    let status = response[9..12].parse().expect("status");
    let body = response.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    (status, body.to_string())
}

/// Plays the page once in the background: takes the next message and
/// answers it with `answer`, returning the message.
fn page_answers(served: &Served, answer: Value) -> thread::JoinHandle<Value> {
    let door = served.door.clone();
    thread::spawn(move || {
        let (status, body) = page(&door, "GET", "/_pair/next", "");
        assert_eq!(status, 200, "{body}");
        let message: Value = serde_json::from_str(&body).expect("JSON");
        let kind = if message.get("tell").is_some() {
            "tell"
        } else {
            "reveal"
        };
        let mut answer = answer;
        answer["id"] = message[kind]["id"].clone();
        assert_eq!(
            page(&door, "POST", "/_pair/answer", &answer.to_string()).0,
            204
        );
        message
    })
}

fn run(command: &mut Command) -> Output {
    command.output().expect("run pair-ui")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn tell_prints_the_page_answer_as_shala_does() {
    let tmp = place("tell");
    let served = serve(&tmp);
    let page = page_answers(
        &served,
        json!({ "ok": true, "message": "5 clicks", "result": { "clicks": 5 } }),
    );
    let out = run(&mut pair_ui(&tmp, &["tell", "demo", r#"{"get":"state"}"#]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "told demo\nmessage: 5 clicks\nresult: {\"clicks\":5}\n"
    );
    assert_eq!(
        page.join().expect("page")["tell"]["body"],
        json!({ "get": "state" })
    );
}

#[test]
fn tell_reads_its_json_from_stdin_given_a_dash() {
    let tmp = place("stdin");
    let served = serve(&tmp);
    let page = page_answers(&served, json!({ "ok": true, "message": "set" }));
    let mut child = pair_ui(&tmp, &["tell", "demo", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn tell");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"{\n  \"set\": 5\n}\n")
        .expect("write stdin");
    assert!(child.wait_with_output().expect("tell").status.success());
    assert_eq!(
        page.join().expect("page")["tell"]["body"],
        json!({ "set": 5 })
    );
}

#[test]
fn reveal_hands_the_page_the_moniker_and_state() {
    let tmp = place("reveal");
    let served = serve(&tmp);
    let page = page_answers(&served, json!({ "ok": true, "message": "shown" }));
    let out = run(&mut pair_ui(
        &tmp,
        &["reveal", "demo://demo/clicks/3", "--state", r#"{"n":3}"#],
    ));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "revealed demo://demo/clicks/3\nmessage: shown\n"
    );
    let seen = page.join().expect("page");
    assert_eq!(seen["reveal"]["moniker"], "demo://demo/clicks/3");
    assert_eq!(seen["reveal"]["state"], json!({ "n": 3 }));
}

#[test]
fn a_refusal_exits_1_with_the_page_reason() {
    let tmp = place("refused");
    let served = serve(&tmp);
    let page = page_answers(&served, json!({ "ok": false, "message": "no such part" }));
    let out = run(&mut pair_ui(&tmp, &["reveal", "demo://demo/x"]));
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("no such part"),
        "{}",
        text(&out.stderr)
    );
    assert!(out.stdout.is_empty());
    page.join().expect("page");
}

#[test]
fn no_host_exits_1_and_says_so() {
    let tmp = place("nohost");
    let out = run(&mut pair_ui(&tmp, &["tell", "demo", "{}"]));
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("is it running?"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn a_usage_error_exits_2_with_the_usage() {
    let tmp = place("usage");
    let out = run(&mut pair_ui(&tmp, &["tell", "demo"]));
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("usage: pair-ui serve"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn serve_refuses_a_build_without_index_html() {
    let tmp = place("noindex");
    let empty = tmp.join("empty");
    std::fs::create_dir_all(&empty).expect("dir");
    let out = run(&mut pair_ui(
        &tmp,
        &["serve", "demo", empty.to_str().expect("utf-8")],
    ));
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("has no index.html"),
        "{}",
        text(&out.stderr)
    );
}
