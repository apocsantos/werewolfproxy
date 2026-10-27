//! Stage31 automatic governance sync (`docs/STAGE31_AUTO_SYNC_SPEC.md`): the
//! local switch and its Den file, eligibility, the delays, and the runtime
//! state that `governance status` reports. The scheduler that runs the sync
//! lives with the control server (`control/auto_sync.rs`), which owns
//! mutation admission.

use crate::state::DaemonState;
use rand_core::{OsRng, RngCore};
use serde_json::{json, Value};
use std::{io, time::Duration};
use tokio::{task::AbortHandle, time::Instant};
use werewolf_core::state::WolfMode;

/// Independent Den state (ARCHITECTURE §8.2, spec decision S1).
pub(crate) const FILE: &str = "auto_sync.json";

/// The exact bytes of the switch file: both fields, in this order, always
/// written explicitly.
pub(crate) fn encode(enabled: bool) -> Vec<u8> {
    format!("{{\"version\":1,\"enabled\":{enabled}}}").into_bytes()
}

/// Strict: only the two canonical encodings are accepted.
pub(crate) fn parse(bytes: &[u8]) -> io::Result<bool> {
    [true, false]
        .into_iter()
        .find(|enabled| bytes == encode(*enabled).as_slice())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid auto_sync.json"))
}

/// The run phases of section 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Ineligible,
    Waiting,
    Resolving,
    Applying,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Ineligible => "ineligible",
            Phase::Waiting => "waiting",
            Phase::Resolving => "resolving",
            Phase::Applying => "applying",
        }
    }
}

/// What the last run did (section 6).
#[derive(Clone, Debug)]
pub(crate) struct LastRun {
    pub(crate) started: Instant,
    pub(crate) outcome: &'static str,
    pub(crate) applied: Value,
    pub(crate) peers: Value,
    pub(crate) stopped: Option<String>,
}

/// Runtime only; of all this, only `enabled` is persisted.
pub(crate) struct Runtime {
    pub(crate) enabled: bool,
    pub(crate) phase: Phase,
    pub(crate) next_due: Option<Instant>,
    pub(crate) last_run: Option<LastRun>,
    /// The resolving run's network task, while it may still be cancelled.
    pub(crate) resolving: Option<AbortHandle>,
    /// An operator command cancelled the current run before its apply.
    pub(crate) cancelled: bool,
    /// Wakes the scheduler to re-evaluate eligibility; `notify_one` keeps
    /// a permit, so a change while it is busy is never lost.
    pub(crate) poke: std::sync::Arc<tokio::sync::Notify>,
    /// Unit tests only: fixed (first, interval) delays.
    #[cfg(test)]
    pub(crate) test_delays: Option<(Duration, Duration)>,
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime {
            // Absent file: on (decision 2a).
            enabled: true,
            phase: Phase::Ineligible,
            next_due: None,
            last_run: None,
            resolving: None,
            cancelled: false,
            poke: Default::default(),
            #[cfg(test)]
            test_delays: None,
        }
    }
}

impl Runtime {
    /// Operator precedence (section 4): cancel a run that is still
    /// resolving. Returns whether one was cancelled; an applying run is
    /// never touched.
    pub(crate) fn preempt(&mut self) -> bool {
        if self.phase != Phase::Resolving {
            return false;
        }
        self.cancelled = true;
        if let Some(task) = self.resolving.take() {
            task.abort();
        }
        true
    }
}

/// The first failing condition of section 1, or `None` when eligible.
pub(crate) fn ineligible_reason(st: &DaemonState) -> Option<&'static str> {
    if st.pelt.is_none() {
        return Some("no_pelt");
    }
    if !st.status.pelt_ready {
        return Some("not_ready");
    }
    let Some(head) = &st.governance else {
        return Some("no_genesis");
    };
    if head.activated_at.is_none() {
        return Some("not_activated");
    }
    if head.dead_at.is_some() {
        return Some("dead");
    }
    if head.forked {
        return Some("forked");
    }
    if crate::governance::unadmitted(st) {
        return Some("unadmitted");
    }
    if !st.auto_sync.enabled {
        return Some("switched_off");
    }
    if matches!(st.status.mode, WolfMode::Silver) {
        return Some("silver");
    }
    if st.dying {
        return Some("dying");
    }
    if st.storage_degraded {
        return Some("storage_degraded");
    }
    None
}

/// U(a, b) in milliseconds, from the OS RNG.
fn uniform(low_ms: u64, high_ms: u64) -> Duration {
    Duration::from_millis(low_ms + OsRng.next_u64() % (high_ms - low_ms + 1))
}

/// Lab builds only: an exact delay from the environment (section 7).
fn lab(_name: &str) -> Option<Duration> {
    #[cfg(feature = "lab-faults")]
    if let Some(ms) = std::env::var(_name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return Some(Duration::from_millis(ms));
    }
    None
}

/// The delay before the run after an ineligible → eligible transition.
pub(crate) fn first_delay(_runtime: &Runtime) -> Duration {
    #[cfg(test)]
    if let Some((first, _)) = _runtime.test_delays {
        return first;
    }
    lab("WEREWOLF_LAB_AUTO_SYNC_FIRST_MS").unwrap_or_else(|| uniform(5_000, 30_000))
}

/// The delay after every run: 15 minutes ±50%, drawn afresh each time.
pub(crate) fn interval(_runtime: &Runtime) -> Duration {
    #[cfg(test)]
    if let Some((_, interval)) = _runtime.test_delays {
        return interval;
    }
    lab("WEREWOLF_LAB_AUTO_SYNC_INTERVAL_MS").unwrap_or_else(|| uniform(450_000, 1_350_000))
}

/// The `auto_sync` object of `governance status`.
pub(crate) fn status(st: &DaemonState) -> Value {
    let now = Instant::now();
    let runtime = &st.auto_sync;
    let reason = ineligible_reason(st);
    let next_due = (runtime.phase == Phase::Waiting)
        .then_some(runtime.next_due)
        .flatten()
        .map(|due| due.saturating_duration_since(now).as_secs());
    let last_run = runtime.last_run.as_ref().map(|run| {
        let mut value = json!({
            "started_secs_ago": now.saturating_duration_since(run.started).as_secs(),
            "outcome": run.outcome,
            "applied": run.applied,
            "peers": run.peers,
        });
        if let Some(code) = &run.stopped {
            value["stopped"] = json!(code);
        }
        value
    });
    json!({
        "enabled": runtime.enabled,
        "eligible": reason.is_none(),
        "ineligible_reason": reason,
        "phase": runtime.phase.name(),
        "next_due_in_secs": next_due,
        "last_run": last_run,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use werewolf_core::governance::{apply, sign, Body, Core, Outcome, Statement};

    fn signed(core: Core) -> Statement {
        let key = |i: u8| SigningKey::from_bytes(&[i + 1; 32]);
        Statement {
            signatures: (0..7u8).map(|i| (i, sign(&core, &key(i)))).collect(),
            core,
        }
    }

    /// A node that is eligible: a Pelt with listeners configured, on an
    /// active network that admits it.
    pub(crate) fn eligible_state() -> DaemonState {
        let pelt = werewolf_core::pelt::generate_identity();
        let own = werewolf_core::governance::tool::public_key(&pelt.public_key_b64).unwrap();
        let key = |i: u8| SigningKey::from_bytes(&[i + 1; 32]);
        let mut head = None;
        let mut previous = [0; 32];
        for (sequence, body) in [
            Body::Genesis {
                threshold: 7,
                keys: (0..10).map(|i| key(i).verifying_key().to_bytes()).collect(),
            },
            Body::Admission(vec![own]),
            Body::Activation,
        ]
        .into_iter()
        .enumerate()
        {
            let statement = signed(Core {
                network_id: [0x31; 32],
                sequence: sequence as u64,
                previous,
                body,
            });
            let expect = (sequence == 0).then(|| statement.core.hash());
            let Ok(Outcome::Applied(next)) = apply(head.as_ref(), &statement, expect.as_ref())
            else {
                panic!("chain");
            };
            previous = next.hash;
            head = Some(next);
        }
        let mut st = DaemonState {
            pelt: Some(pelt),
            governance: head,
            ..DaemonState::default()
        };
        st.status.pelt_ready = true;
        st
    }

    #[test]
    fn each_condition_alone_makes_the_node_ineligible() {
        assert_eq!(ineligible_reason(&eligible_state()), None);
        type Change = fn(&mut DaemonState);
        let cases: [(&str, Change); 11] = [
            ("no_pelt", |st| st.pelt = None),
            ("not_ready", |st| st.status.pelt_ready = false),
            ("no_genesis", |st| st.governance = None),
            ("not_activated", |st| {
                st.governance.as_mut().unwrap().activated_at = None
            }),
            ("dead", |st| {
                st.governance.as_mut().unwrap().dead_at = Some(2)
            }),
            ("forked", |st| st.governance.as_mut().unwrap().forked = true),
            ("unadmitted", |st| {
                st.governance.as_mut().unwrap().roster.clear()
            }),
            ("switched_off", |st| st.auto_sync.enabled = false),
            ("silver", |st| st.status.mode = WolfMode::Silver),
            ("dying", |st| st.dying = true),
            ("storage_degraded", |st| st.storage_degraded = true),
        ];
        for (reason, change) in cases {
            let mut st = eligible_state();
            change(&mut st);
            assert_eq!(ineligible_reason(&st), Some(reason));
            let status = status(&st);
            assert_eq!(status["eligible"], false);
            assert_eq!(status["ineligible_reason"], reason);
        }
    }

    #[test]
    fn the_switch_file_is_exactly_canonical() {
        assert_eq!(encode(false), br#"{"version":1,"enabled":false}"#);
        assert!(parse(&encode(true)).unwrap());
        assert!(!parse(&encode(false)).unwrap());
        for bad in [
            &br#"{"enabled":false,"version":1}"#[..],
            br#"{"version":1,"enabled":false}
"#,
            br#"{"version":2,"enabled":false}"#,
            br#"{"version":1,"enabled":"false"}"#,
            br#"{"version":1,"enabled":false,"x":0}"#,
            br#"{ "version":1,"enabled":false}"#,
            b"",
        ] {
            assert!(parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn delays_stay_within_their_bounds() {
        for _ in 0..1000 {
            let first = uniform(5_000, 30_000);
            assert!((Duration::from_secs(5)..=Duration::from_secs(30)).contains(&first));
            let next = uniform(450_000, 1_350_000);
            assert!((Duration::from_secs(450)..=Duration::from_secs(1350)).contains(&next));
        }
    }

    #[test]
    fn preempt_cancels_only_a_resolving_run() {
        let mut runtime = Runtime::default();
        for phase in [Phase::Ineligible, Phase::Waiting, Phase::Applying] {
            runtime.phase = phase;
            assert!(!runtime.preempt());
            assert!(!runtime.cancelled);
        }
        runtime.phase = Phase::Resolving;
        assert!(runtime.preempt());
        assert!(runtime.cancelled);
    }
}
