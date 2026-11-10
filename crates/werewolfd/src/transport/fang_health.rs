//! What an operator can see of a Fang's failing sessions (Stage45,
//! `docs/STAGE45_FANG_FAILURE_VISIBILITY_SURVEY.md`).
//!
//! Before Stage45 a session that failed to set up was silent on every Fang
//! kind but plain TCP, which logged one line per failure without bound, and
//! `fang list` showed a Fang whose every session failed as `active`. Each
//! Fang now holds one [`FangHealth`]:
//!
//! - every **setup** failure is logged at a bounded rate (S45-D1, the
//!   Stage42 limiter: the first, then at most one line per 10 s per class,
//!   with the count suppressed in between), with its class;
//! - `sessions_failed` and `last_failure` appear in `fang list` (S45-D4).
//!
//! Classes: `dial` (no connection was made: refused locally, refused, no
//! route, no answer), `handshake` (the peer was reached but no session was
//! established; the receiver answers every refusal the same way, so the
//! initiator cannot say why, by design) and, for plain TCP, `target`.
//! Errors after a session is established are normal application behaviour:
//! not counted (plain TCP still logs them, bounded, S45-D2). Only this
//! node's own log and control output change; nothing is sent on the wire.

use super::fang_accept::RateLimitedLog;
use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureClass {
    Dial,
    Handshake,
    Target,
}

impl FailureClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Dial => "dial",
            Self::Handshake => "handshake",
            Self::Target => "target",
        }
    }

    /// The class of a failed session setup towards a peer: `dial` for the
    /// error kinds that mean no connection was made (the same set discovery
    /// reports `unreachable`), `handshake` for everything else.
    pub(crate) fn of_setup(error: &io::Error) -> Self {
        use io::ErrorKind::*;
        match error.kind() {
            ConnectionRefused | HostUnreachable | NetworkUnreachable | AddrNotAvailable
            | TimedOut => Self::Dial,
            _ => Self::Handshake,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

struct Logs {
    setup: [RateLimitedLog; 3],
    pipe: RateLimitedLog,
}

pub(crate) struct FangHealth {
    /// The label of every line: the Fang id and its peer.
    label: String,
    failed: AtomicU64,
    last: Mutex<Option<(FailureClass, Instant)>>,
    logs: Mutex<Logs>,
}

impl Default for FangHealth {
    fn default() -> Self {
        Self::new("fang", "")
    }
}

impl FangHealth {
    pub(crate) fn new(fang_id: &str, peer: &str) -> Self {
        let label = if peer.is_empty() {
            fang_id.to_string()
        } else {
            format!("{fang_id} to {peer}")
        };
        Self {
            label,
            failed: AtomicU64::new(0),
            last: Mutex::new(None),
            logs: Mutex::new(Logs {
                setup: [
                    RateLimitedLog::new("session setup failed (dial)"),
                    RateLimitedLog::new("session setup failed (handshake)"),
                    RateLimitedLog::new("session setup failed (target)"),
                ],
                pipe: RateLimitedLog::new("plain session ended with an error"),
            }),
        }
    }

    /// A session to the peer failed before it was established.
    pub(crate) fn setup_failed(&self, error: &io::Error) {
        self.record(FailureClass::of_setup(error), error);
    }

    /// Records a failed session setup of `class`, and logs it at a bounded
    /// rate.
    pub(crate) fn record(&self, class: FailureClass, detail: &dyn std::fmt::Display) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        *lock(&self.last) = Some((class, Instant::now()));
        lock(&self.logs).setup[class.index()].event(&self.label, detail);
    }

    /// A plain-TCP session failed after it was established: logged at a
    /// bounded rate, not counted.
    pub(crate) fn pipe_error(&self, detail: &dyn std::fmt::Display) {
        lock(&self.logs).pipe.event(&self.label, detail);
    }

    /// Setup failures since the Fang opened.
    pub(crate) fn sessions_failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }

    /// `fang list`'s `last_failure`: `{"class", "seconds_ago"}` or `null`.
    pub(crate) fn last_failure(&self) -> serde_json::Value {
        match *lock(&self.last) {
            None => serde_json::Value::Null,
            Some((class, at)) => serde_json::json!({
                "class": class.as_str(),
                "seconds_ago": at.elapsed().as_secs(),
            }),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
