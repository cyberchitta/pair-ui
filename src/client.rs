//! What an app's commands call to reach a running host, over its socket.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper::header;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::UnixStream;

use crate::message::{Answer, MAX_TELL};
use crate::moniker::Moniker;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("no host answers on {0} (is it running?)")]
    NoHost(PathBuf),
    #[error("the tell is {0} bytes; a host takes at most {MAX_TELL}")]
    TooLarge(usize),
    #[error("the host dropped the request: {0}")]
    Http(#[from] hyper::Error),
    /// The host's own reason: no page took the tell, or it was refused.
    #[error("{0}")]
    Refused(String),
    #[error("the host's reply is not what it should be: {0}")]
    BadReply(serde_json::Error),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostError {
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    url: String,
}

/// Hands `body` to the page of `project` and returns its answer. An answer
/// with `ok: false` is still an answer: the page refused and changed nothing.
pub async fn tell(socket: &Path, project: &str, body: &Value) -> Result<Answer, ClientError> {
    let json = body.to_string();
    if json.len() > MAX_TELL {
        return Err(ClientError::TooLarge(json.len()));
    }
    send(socket, Method::POST, "/tell", format!("{project}\n{json}")).await
}

/// Asks the page to show `moniker`, with `state` handed to it unread, and
/// returns its answer. `ok: false` means the page could not show it and
/// changed nothing. The usual socket is `socket_path(moniker.project())`.
pub async fn reveal(
    socket: &Path,
    moniker: &Moniker,
    state: Option<&Value>,
) -> Result<Answer, ClientError> {
    let body = match state {
        Some(state) => format!("{moniker}\nstate: {state}"),
        None => moniker.to_string(),
    };
    if body.len() > MAX_TELL {
        return Err(ClientError::TooLarge(body.len()));
    }
    send(socket, Method::POST, "/reveal", body).await
}

/// The address to open the page at.
pub async fn page_url(socket: &Path) -> Result<String, ClientError> {
    let page: Page = send(socket, Method::GET, "/page", String::new()).await?;
    Ok(page.url)
}

async fn send<T: DeserializeOwned>(
    socket: &Path,
    method: Method,
    path: &str,
    body: String,
) -> Result<T, ClientError> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|_| ClientError::NoHost(socket.to_path_buf()))?;
    let (mut sender, connection) = http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(connection);
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "pair-ui")
        .body(Full::new(Bytes::from(body)))
        .expect("the request is built from valid parts");
    let response = sender.send_request(request).await?;
    let status = response.status();
    let bytes = response.into_body().collect().await?.to_bytes();
    if status == StatusCode::OK {
        return serde_json::from_slice(&bytes).map_err(ClientError::BadReply);
    }
    let reason = serde_json::from_slice::<HostError>(&bytes)
        .map(|e| e.error)
        .unwrap_or_else(|_| format!("the host answered {status}"));
    Err(ClientError::Refused(reason))
}
