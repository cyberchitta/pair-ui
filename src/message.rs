//! What crosses between the commands, the host and the page, and how large
//! it may be. This much needs no web stack, so a host that frames the page
//! can take it with the `host` feature off.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The largest tell body.
pub const MAX_TELL: usize = 4 * 1024 * 1024;
/// The largest value the page may keep.
pub const MAX_KEEP: usize = 64 * 1024;
/// The largest answer the page may send: the whole of it, `id`, `ok`,
/// `message` and `result` together.
pub const MAX_ANSWER: usize = MAX_TELL;

/// How long a tell or reveal waits for the page's answer.
pub const ANSWER_WAIT: Duration = Duration::from_secs(10);

/// What the page is handed, the same over HTTP and postMessage:
/// `{"pair":1,"tell":{"id","body"}}` or
/// `{"pair":1,"reveal":{"id","moniker","state"}}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Message {
    Tell {
        id: u64,
        body: Value,
    },
    Reveal {
        id: u64,
        moniker: String,
        state: Value,
    },
}

impl Message {
    pub fn id(&self) -> u64 {
        match self {
            Message::Tell { id, .. } | Message::Reveal { id, .. } => *id,
        }
    }

    /// The message as the page reads it, `pair: 1` included.
    pub fn for_page(&self) -> Value {
        let mut page = serde_json::to_value(self).expect("a message serializes");
        page["pair"] = Value::from(1);
        page
    }
}

/// The page's answer to a tell or reveal. A page that cannot do a body in full answers
/// `ok: false` and changes nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}
