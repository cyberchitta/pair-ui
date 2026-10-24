//! Host one web page that a person and an agent drive together, on
//! 127.0.0.1.
//!
//! The person uses the page in a browser. The agent drives it through the
//! app's own commands, which hand the page opaque JSON (a *tell*) or ask it
//! to show a moniker (a *reveal*), and print its answer. The host never reads
//! a body or a state: what they mean, including what the person has
//! selected, is between the app's commands and its page, and the app's skill
//! explains it.
//!
//! The commands reach the host through a Unix socket that only the owner can
//! open ([`socket_path`]):
//!
//! - `POST /tell`: the project on the first line of the body, the JSON after
//!   it ([`client::tell`] writes one, [`TellRequest`] reads one).
//! - `POST /reveal`: a [`Moniker`], `<scheme>://<project>…`, then an
//!   optional `state: <json>` line ([`client::reveal`]).
//!
//! Either is answered `{"ok","message","result"?}`, or `{"error"}` when no
//! page answered. `GET /page` gives `{"url"}`, the address to open the page
//! at.
//!
//! The page's side is plain HTTP on its own origin, with the token from its
//! URL's `pair` parameter as `Authorization: Bearer <token>`:
//!
//! - `GET /_pair/next`: the next message, `{"pair":1,"tell":{"id","body"}}`
//!   or `{"pair":1,"reveal":{"id","moniker","state"}}`, or 204 after 25 s
//!   with none. Ask again.
//! - `POST /_pair/answer`: `{"id","ok","message","result"?}`, within 10 s
//!   and under 4 MiB ([`MAX_ANSWER`]).
//!   A message the page cannot do in full answers `ok: false` and changes
//!   nothing.
//! - `PUT /_pair/keep`: any JSON under 64 KB, replacing what was kept;
//!   `null` forgets it. `GET /_pair/recall` gives it back, or `null`. It
//!   lasts as long as the host does.
//! - `GET /_pair/project`: `{"project"}`, the name monikers use.
//!
//! A host that puts the page in a frame instead carries the same messages
//! by `postMessage`, every one keyed `pair: 1`. It posts `tell`, `reveal`
//! and `recalled: view` to the page; the page posts `{"pair":1,"ready":true}`
//! once it can answer (the host holds reveals until then), `answer`, `keep`
//! and `recall: true` to its parent. The page client in `page/` speaks both
//! transports, so one build runs under either host. Pages are served under
//! [`Policy`].
//!
//! The `host` feature, on by default, holds the host, `client` and the
//! web stack they need. Without it the crate is the messages, the limits,
//! [`TellRequest`], [`Moniker`] and [`Policy`], for a host that frames the
//! page.

#[cfg(feature = "host")]
pub mod client;
mod message;
mod moniker;
mod policy;
#[cfg(feature = "host")]
mod relay;
#[cfg(feature = "host")]
mod server;

pub use message::{
    ANSWER_WAIT, Answer, BadTell, MAX_ANSWER, MAX_KEEP, MAX_TELL, Message, TellRequest,
};
pub use moniker::{BadMoniker, Moniker};
pub use policy::{BadOrigin, Origin, Policy};
#[cfg(feature = "host")]
pub use server::{Build, Config, Host, HostError, socket_path};
