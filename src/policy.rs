//! The page's Content-Security-Policy. A host that frames the page should
//! serve it under the same one, so one build of a page runs in either.

use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

/// An origin the page may fetch from or open a socket to:
/// `scheme://host[:port]`, with scheme `http`, `https`, `ws` or `wss`, the host
/// in lowercase, and no path. Only parsing makes one.
///
/// ```
/// use pair_ui::Origin;
/// assert!("http://localhost:3381".parse::<Origin>().is_ok());
/// assert!("http://[::1]:3381".parse::<Origin>().is_ok());
/// assert!("http://localhost:3381/v1".parse::<Origin>().is_err());
/// assert!("http://LocalHost".parse::<Origin>().is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an origin (scheme://host[:port]; http, https, ws or wss; lowercase; no path): {0:?}")]
pub struct BadOrigin(String);

impl FromStr for Origin {
    type Err = BadOrigin;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || BadOrigin(s.to_string());
        let (scheme, rest) = s.split_once("://").ok_or_else(bad)?;
        let (host, port) = split_port(rest).ok_or_else(bad)?;
        let good = matches!(scheme, "http" | "https" | "ws" | "wss")
            && valid_host(host)
            && port.is_none_or(valid_port);
        good.then(|| Origin(s.to_string())).ok_or_else(bad)
    }
}

impl Origin {
    /// Whether the origin is this machine: `localhost`, a `*.localhost`
    /// name, `[::1]`, or an IPv4 loopback address. Any other is the page
    /// reaching off the machine.
    ///
    /// ```
    /// use pair_ui::Origin;
    /// let local = |text: &str| text.parse::<Origin>().unwrap().is_local();
    /// assert!(local("http://selfcast.localhost:80") && local("http://127.0.0.1:3381"));
    /// assert!(!local("https://localhost.example.com"));
    /// ```
    pub fn is_local(&self) -> bool {
        let (_, rest) = self
            .0
            .split_once("://")
            .expect("a parsed origin has a scheme");
        let (host, _) = split_port(rest).expect("a parsed origin splits");
        host == "localhost"
            || host.ends_with(".localhost")
            || host == "[::1]"
            || host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The host and the port, if any. An IPv6 host keeps its brackets.
fn split_port(rest: &str) -> Option<(&str, Option<&str>)> {
    if rest.starts_with('[') {
        let end = rest.find(']')?;
        let (host, after) = rest.split_at(end + 1);
        return match after {
            "" => Some((host, None)),
            _ => Some((host, Some(after.strip_prefix(':')?))),
        };
    }
    Some(match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    })
}

fn valid_host(host: &str) -> bool {
    let name = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-';
    let v6 = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b':';
    match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Some(inner) => !inner.is_empty() && inner.bytes().all(v6),
        None => !host.is_empty() && host.bytes().all(name),
    }
}

fn valid_port(port: &str) -> bool {
    port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>().is_ok_and(|p| p > 0)
}

/// What the page may reach beyond its own files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Origins added to `connect-src`, and to nothing else.
    pub connect: Vec<Origin>,
    /// Whether the page may compile WebAssembly (`'wasm-unsafe-eval'`).
    pub wasm: bool,
    /// Whether the page may load images from any `https:` origin. Each image
    /// is a request off the machine that tells its host the person's address.
    pub remote_images: bool,
}

impl Policy {
    /// The Content-Security-Policy the page is served under. It refuses
    /// inline scripts, so a page's script is a file beside `index.html`.
    ///
    /// ```
    /// use pair_ui::Policy;
    /// let policy = Policy {
    ///     connect: vec!["http://localhost:3381".parse().unwrap()],
    ///     wasm: true,
    ///     remote_images: true,
    /// };
    /// assert!(policy.csp().contains("connect-src 'self' http://localhost:3381;"));
    /// assert!(policy.csp().contains("'wasm-unsafe-eval'"));
    /// assert!(policy.csp().ends_with("img-src 'self' data: blob: https:"));
    /// assert!(Policy::default().csp().ends_with("connect-src 'self'"));
    /// ```
    pub fn csp(&self) -> String {
        let connect: String = self.connect.iter().map(|o| format!(" {o}")).collect();
        let wasm = if self.wasm {
            "; script-src 'self' data: blob: 'wasm-unsafe-eval'"
        } else {
            ""
        };
        let images = if self.remote_images {
            "; img-src 'self' data: blob: https:"
        } else {
            ""
        };
        format!(
            "default-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; \
             connect-src 'self'{connect}{wasm}{images}"
        )
    }
}
