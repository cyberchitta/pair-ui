//! Carries tells and reveals from the commands to the page and the page's
//! answers back, and holds the one value the page keeps. Nothing here reads a
//! body or a state.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Notify, oneshot};

use crate::message::{ANSWER_WAIT, Answer, Message};

/// Why a tell or reveal has no answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoAnswer {
    #[error("no page took the message within 10 s: is the page open?")]
    NotTaken,
    #[error("the page took the message but did not answer within 10 s")]
    NotAnswered,
}

#[derive(Default)]
pub struct Relay {
    next_id: AtomicU64,
    queue: Mutex<VecDeque<Message>>,
    arrived: Notify,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
    kept: Mutex<Option<Value>>,
}

impl Relay {
    /// Hands `body` to the page and waits for its answer.
    pub async fn tell(&self, body: Value) -> Result<Answer, NoAnswer> {
        self.send(|id| Message::Tell { id, body }).await
    }

    /// Asks the page to show `moniker`, and waits for its answer.
    pub async fn reveal(&self, moniker: String, state: Value) -> Result<Answer, NoAnswer> {
        self.send(|id| Message::Reveal { id, moniker, state }).await
    }

    async fn send(&self, message: impl FnOnce(u64) -> Message) -> Result<Answer, NoAnswer> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (send, answer) = oneshot::channel();
        lock(&self.waiting).insert(id, send);
        lock(&self.queue).push_back(message(id));
        self.arrived.notify_one();
        let outcome = tokio::time::timeout(ANSWER_WAIT, answer).await;
        lock(&self.waiting).remove(&id);
        match outcome {
            Ok(Ok(answer)) => Ok(answer),
            _ => Err(self.withdraw(id)),
        }
    }

    /// Takes back a message no one answered, and says how far it got.
    fn withdraw(&self, id: u64) -> NoAnswer {
        let mut queue = lock(&self.queue);
        let before = queue.len();
        queue.retain(|m| m.id() != id);
        if queue.len() < before {
            NoAnswer::NotTaken
        } else {
            NoAnswer::NotAnswered
        }
    }

    /// The next message for the page, waiting up to `wait` for one.
    pub async fn next(&self, wait: Duration) -> Option<Message> {
        let arrived = self.arrived.notified();
        tokio::pin!(arrived);
        arrived.as_mut().enable();
        if let Some(message) = lock(&self.queue).pop_front() {
            return Some(message);
        }
        tokio::time::timeout(wait, arrived).await.ok()?;
        lock(&self.queue).pop_front()
    }

    /// Delivers the page's answer. False when nothing waits for `id`.
    pub fn answer(&self, id: u64, answer: Answer) -> bool {
        lock(&self.waiting)
            .remove(&id)
            .is_some_and(|send| send.send(answer).is_ok())
    }

    pub fn keep(&self, view: Option<Value>) {
        *lock(&self.kept) = view;
    }

    pub fn recall(&self) -> Option<Value> {
        lock(&self.kept).clone()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().expect("no code panics while holding a relay lock")
}
