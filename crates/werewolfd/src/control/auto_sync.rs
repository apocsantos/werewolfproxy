//! Stage31 automatic governance sync: the scheduler
//! (`docs/STAGE31_AUTO_SYNC_SPEC.md` sections 3-5). A run is the network
//! phase of manual `governance sync`, then the same internal
//! `governance.sync_apply` through mutation admission. Nothing here filters
//! statement kinds (decision 7a).

use super::{handle_request, limits, MutationTasks};
use crate::auto_sync::{self, LastRun, Phase};
use crate::state::DaemonState;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::{
    sync::{oneshot, Mutex, Notify},
    task::JoinHandle,
    time::Instant,
};
use werewolf_core::protocol::{ControlRequest, ControlResponse};

/// The resolving task is aborted whenever the run is dropped, so stopping
/// the scheduler never leaves a network phase behind.
struct Resolving(JoinHandle<Result<ControlRequest, ControlResponse>>);

impl Drop for Resolving {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The scheduler: one task per control server, one run at a time.
pub(super) async fn run(
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
    mutations: Arc<limits::Mutations>,
    tasks: MutationTasks,
) {
    let poke = state.lock().await.auto_sync.poke.clone();
    let mut was_eligible = false;
    loop {
        let due = {
            let mut st = state.lock().await;
            let eligible = auto_sync::ineligible_reason(&st).is_none();
            if !eligible {
                st.auto_sync.phase = Phase::Ineligible;
                st.auto_sync.next_due = None;
            } else if !was_eligible {
                // Every ineligible -> eligible transition (decision 3a).
                st.auto_sync.phase = Phase::Waiting;
                st.auto_sync.next_due =
                    Some(Instant::now() + auto_sync::first_delay(&st.auto_sync));
            }
            was_eligible = eligible;
            st.auto_sync.next_due
        };
        let fired = match due {
            None => {
                poke.notified().await;
                false
            }
            Some(due) => tokio::select! {
                _ = poke.notified() => false,
                _ = tokio::time::sleep_until(due) => true,
            },
        };
        if !fired || auto_sync::ineligible_reason(&*state.lock().await).is_some() {
            continue;
        }
        let last = one_run(&state, &home, &mutations, &tasks, &poke).await;
        if last.outcome != "nothing_new" {
            println!(
                "[INFO][GOVERNANCE][AUTO_SYNC] run {}{}",
                last.outcome,
                last.stopped
                    .as_ref()
                    .map(|code| format!(" ({code})"))
                    .unwrap_or_default()
            );
        }
        let mut st = state.lock().await;
        st.auto_sync.last_run = Some(last);
        st.auto_sync.resolving = None;
        st.auto_sync.cancelled = false;
        st.auto_sync.phase = Phase::Waiting;
        st.auto_sync.next_due = Some(Instant::now() + auto_sync::interval(&st.auto_sync));
    }
}

fn last(started: Instant, outcome: &'static str, peers: Value) -> LastRun {
    LastRun {
        started,
        outcome,
        applied: json!([]),
        peers,
        stopped: None,
    }
}

async fn one_run(
    state: &Arc<Mutex<DaemonState>>,
    home: &std::path::Path,
    mutations: &Arc<limits::Mutations>,
    tasks: &MutationTasks,
    poke: &Arc<Notify>,
) -> LastRun {
    let started = Instant::now();
    let request = ControlRequest {
        id: "auto-sync".into(),
        cmd: "governance.sync".into(),
        args: json!({}),
    };
    // Phase and abort handle are published under one lock, before the task
    // can take the discovery lock, so an operator can always cancel it.
    let mut resolving = {
        let mut st = state.lock().await;
        st.auto_sync.phase = Phase::Resolving;
        st.auto_sync.cancelled = false;
        let task_state = state.clone();
        let task = tokio::spawn(async move {
            crate::discovery::operator::resolve_sync(request, &task_state, false).await
        });
        st.auto_sync.resolving = Some(task.abort_handle());
        Resolving(task)
    };
    let joined = loop {
        tokio::select! {
            joined = &mut resolving.0 => break joined,
            _ = poke.notified() => {
                // A mutation made the node ineligible: cancel, as an
                // operator command would.
                let mut st = state.lock().await;
                if auto_sync::ineligible_reason(&st).is_some() {
                    st.auto_sync.preempt();
                }
            }
        }
    };
    let apply = {
        let mut st = state.lock().await;
        st.auto_sync.resolving = None;
        // Checked under the same lock that starts the apply: from here on
        // `preempt` finds the phase `applying` and does nothing.
        if std::mem::take(&mut st.auto_sync.cancelled) {
            return last(started, "cancelled", json!([]));
        }
        match joined {
            Err(_) => return last(started, "cancelled", json!([])),
            Ok(Err(refused)) => {
                let code = refused.error.map(|e| e.code).unwrap_or_default();
                if code == "DISCOVER_BUSY" {
                    return last(started, "skipped_busy", json!([]));
                }
                let mut run = last(started, "stopped", json!([]));
                run.stopped = Some(code);
                return run;
            }
            Ok(Ok(apply)) => {
                let nothing = apply.args["statements"]
                    .as_array()
                    .is_none_or(|list| list.is_empty())
                    && apply.args["conflict"].is_null();
                if nothing {
                    return last(started, "nothing_new", apply.args["peers"].clone());
                }
                st.auto_sync.phase = Phase::Applying;
                apply
            }
        }
    };
    let peers = apply.args["peers"].clone();
    let Ok(permit) = mutations.admit().await else {
        let mut run = last(started, "stopped", peers);
        run.stopped = Some("CONTROL_BUSY".into());
        return run;
    };
    let (complete, response) = oneshot::channel();
    {
        let state = state.clone();
        let home = home.to_path_buf();
        tasks.lock().await.spawn(async move {
            let _permit = permit;
            let _ = complete.send(handle_request(apply, state, home).await);
        });
    }
    let Ok(response) = response.await else {
        let mut run = last(started, "stopped", peers);
        run.stopped = Some("CANCELLED".into());
        return run;
    };
    outcome(started, peers, response)
}

/// Reads the `governance.sync_apply` reply into the run's outcome, and logs
/// every statement it applied.
fn outcome(started: Instant, peers: Value, response: ControlResponse) -> LastRun {
    let mut run = last(started, "applied", peers);
    let Some(result) = response.result else {
        run.outcome = "stopped";
        run.stopped = response.error.map(|e| e.code);
        return run;
    };
    run.applied = result["applied"].clone();
    for applied in run.applied.as_array().into_iter().flatten() {
        // The statement's apply result: sequence, hash and its effects.
        println!("[INFO][GOVERNANCE][AUTO_SYNC_APPLIED] {applied}");
    }
    if let Some(code) = result["stopped"]["code"].as_str() {
        run.outcome = if code == "GOVERNANCE_FORK" {
            "fork"
        } else {
            "stopped"
        };
        run.stopped = Some(code.to_owned());
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_sync::tests::eligible_state;
    use std::time::Duration;
    use tokio::task::JoinSet;
    use werewolf_core::state::WolfMode;

    const FIRST: Duration = Duration::from_millis(100);
    const INTERVAL: Duration = Duration::from_secs(30);

    /// The scheduler over `st`, with short fixed delays.
    fn start(mut st: DaemonState) -> (Arc<Mutex<DaemonState>>, JoinHandle<()>) {
        st.auto_sync.test_delays = Some((FIRST, INTERVAL));
        let state = Arc::new(Mutex::new(st));
        let scheduler = tokio::spawn(run(
            state.clone(),
            std::env::temp_dir(),
            Arc::new(limits::Mutations::default()),
            Arc::new(Mutex::new(JoinSet::new())),
        ));
        (state, scheduler)
    }

    async fn change(state: &Arc<Mutex<DaemonState>>, f: impl FnOnce(&mut DaemonState)) {
        let mut st = state.lock().await;
        f(&mut st);
        st.auto_sync.poke.notify_one();
    }

    async fn wait_for(
        state: &Arc<Mutex<DaemonState>>,
        what: impl Fn(&DaemonState) -> bool,
    ) -> Instant {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if what(&*state.lock().await) {
                return Instant::now();
            }
            assert!(Instant::now() < until, "condition not reached");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn due_in(st: &DaemonState) -> Duration {
        st.auto_sync
            .next_due
            .unwrap()
            .saturating_duration_since(Instant::now())
    }

    #[tokio::test]
    async fn every_ineligible_to_eligible_transition_gets_the_first_delay() {
        let mut st = eligible_state();
        st.auto_sync.enabled = false;
        let (state, scheduler) = start(st);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(state.lock().await.auto_sync.phase, Phase::Ineligible);
        assert!(state.lock().await.auto_sync.last_run.is_none());

        // Switched on: one run after the first delay, then the interval.
        let switched = Instant::now();
        change(&state, |st| st.auto_sync.enabled = true).await;
        wait_for(&state, |st| st.auto_sync.last_run.is_some()).await;
        {
            let st = state.lock().await;
            let run = st.auto_sync.last_run.as_ref().unwrap();
            assert_eq!(run.outcome, "nothing_new");
            assert!(run.started.duration_since(switched) >= FIRST);
            assert_eq!(st.auto_sync.phase, Phase::Waiting);
            assert!(due_in(&st) > INTERVAL / 2);
        }

        // Silver makes it ineligible; clearing Silver is a new transition,
        // so the next run comes after the first delay, not the interval.
        change(&state, |st| st.status.mode = WolfMode::Silver).await;
        wait_for(&state, |st| st.auto_sync.phase == Phase::Ineligible).await;
        assert!(state.lock().await.auto_sync.next_due.is_none());
        change(&state, |st| st.status.mode = WolfMode::Human).await;
        wait_for(&state, |st| st.auto_sync.phase == Phase::Waiting).await;
        assert!(due_in(&*state.lock().await) <= FIRST);
        let before = Instant::now();
        wait_for(&state, |st| {
            st.auto_sync
                .last_run
                .as_ref()
                .is_some_and(|run| run.started >= before)
        })
        .await;
        scheduler.abort();
    }

    #[tokio::test]
    async fn a_run_that_finds_discovery_busy_is_skipped_for_a_full_interval() {
        let (state, scheduler) = start(eligible_state());
        let lock = state.lock().await.discovery.clone();
        let held = lock.try_lock_owned().unwrap();
        wait_for(&state, |st| st.auto_sync.last_run.is_some()).await;
        {
            let st = state.lock().await;
            assert_eq!(
                st.auto_sync.last_run.as_ref().unwrap().outcome,
                "skipped_busy"
            );
            assert_eq!(st.auto_sync.phase, Phase::Waiting);
            assert!(due_in(&st) > INTERVAL / 2);
        }
        drop(held);
        scheduler.abort();
    }

    /// A stand-in for a resolving automatic run: it holds the discovery
    /// lock until it is aborted.
    async fn resolving_run(state: &Arc<Mutex<DaemonState>>, phase: Phase) -> JoinHandle<()> {
        let held = state
            .lock()
            .await
            .discovery
            .clone()
            .try_lock_owned()
            .unwrap();
        let task = tokio::spawn(async move {
            let _held = held;
            std::future::pending::<()>().await
        });
        let mut st = state.lock().await;
        st.auto_sync.phase = phase;
        st.auto_sync.resolving = Some(task.abort_handle());
        task
    }

    fn sync() -> ControlRequest {
        ControlRequest {
            id: "1".into(),
            cmd: "governance.sync".into(),
            args: json!({}),
        }
    }

    #[tokio::test]
    async fn an_operator_sync_cancels_a_resolving_run_and_proceeds() {
        let state = Arc::new(Mutex::new(eligible_state()));
        let task = resolving_run(&state, Phase::Resolving).await;
        // Another automatic resolver never cancels anything.
        let refused = crate::discovery::operator::resolve_sync(sync(), &state, false)
            .await
            .unwrap_err();
        assert_eq!(refused.error.unwrap().code, "DISCOVER_BUSY");
        assert!(!task.is_finished());
        // The operator's does, and takes the lock.
        let apply = crate::discovery::operator::resolve_sync(sync(), &state, true)
            .await
            .unwrap();
        assert_eq!(apply.cmd, "governance.sync_apply");
        assert!(state.lock().await.auto_sync.cancelled);
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn an_operator_discover_refresh_cancels_a_resolving_run_too() {
        let state = Arc::new(Mutex::new(eligible_state()));
        let task = resolving_run(&state, Phase::Resolving).await;
        let reply = crate::discovery::operator::handle(
            ControlRequest {
                id: "1".into(),
                cmd: "discover.refresh".into(),
                args: json!({}),
            },
            state.clone(),
        )
        .await;
        assert!(reply.ok, "{:?}", reply.error);
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn a_run_past_resolving_is_never_cancelled() {
        let state = Arc::new(Mutex::new(eligible_state()));
        let task = resolving_run(&state, Phase::Applying).await;
        let refused = crate::discovery::operator::resolve_sync(sync(), &state, true)
            .await
            .unwrap_err();
        assert_eq!(refused.error.unwrap().code, "DISCOVER_BUSY");
        assert!(!state.lock().await.auto_sync.cancelled);
        assert!(!task.is_finished());
        task.abort();
    }
}
