//! Stage30 death and retirement on the node
//! (`docs/STAGE30_LIFECYCLE_SPEC.md` sections 3, 4 and 6).
//!
//! Points of no return: for death, the commit of the DEAD head; for
//! retirement, the durably committed incomplete retirement Tombstone. In
//! both cases no destruction step begins before a durably committed
//! Tombstone, so a Den whose state is gone always carries one.

use crate::state::DaemonState;
use serde_json::{json, Value};
use std::{ffi::OsStr, io, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use werewolf_core::{
    governance::{Body, Head, Statement},
    local_fs::{CommitOutcome, PrivateDirectory},
    tombstone::{self, Destruction, Kind, Tombstone},
};

/// The daemon's exit code once it has finished, or found, a Tombstone.
pub(crate) const EXIT_CODE: u8 = 67;
/// The Den lock: always present while a process holds the Den, never
/// unlinked (its inode is the lock domain). The I3 allowlist admits it only
/// while it is inert: empty, 0600, one link, this owner.
const DEN_LOCK: &str = ".den.lock";
/// How long a finished terminal transition waits for its reply to be written
/// before the process exits anyway.
const EXIT_GRACE: Duration = Duration::from_secs(2);

pub(crate) fn software() -> String {
    format!("werewolfd {}", env!("CARGO_PKG_VERSION"))
}

fn tombstone_name() -> &'static OsStr {
    OsStr::new(tombstone::FILE_NAME)
}

/// Lab builds only: abort at a named step boundary, as an abrupt crash would
/// (the death gate's crash-convergence check). Absent from release builds.
fn halt(_point: &str) {
    #[cfg(feature = "lab-faults")]
    if std::env::var("WEREWOLF_LAB_TERMINAL_HALT").as_deref() == Ok(_point) {
        std::process::abort();
    }
}

/// Section 4, steps 3 and 4: everything in the Den except the Tombstone and
/// the Den lock, the Pelt first, then the protected security state, then the
/// rest (leftover Tombstone temporaries included), each entry under the I2
/// rule. Then the directory is synced.
fn destroy_den(den: &PrivateDirectory) -> io::Result<()> {
    den.destroy(OsStr::new("pelt.json"))?;
    halt("after_pelt");
    let entries = den.entries()?;
    let keep = |name: &OsStr| name == tombstone_name() || name == OsStr::new(DEN_LOCK);
    for name in entries
        .iter()
        .filter(|n| n.to_string_lossy().starts_with("security_state_"))
    {
        den.destroy(name)?;
    }
    halt("after_security_state");
    for name in den.entries()?.iter().filter(|n| !keep(n)) {
        den.destroy(name)?;
    }
    if !den.lock_is_inert(OsStr::new(DEN_LOCK))? {
        den.scrub_lock(OsStr::new(DEN_LOCK))?;
    }
    den.sync()
}

/// I3: the Den holds `tombstone.json` and, at most, an inert Den lock.
fn den_is_clean(den: &PrivateDirectory) -> io::Result<bool> {
    let entries = den.entries()?;
    let only_allowed = entries
        .iter()
        .all(|name| name == tombstone_name() || name == OsStr::new(DEN_LOCK));
    Ok(only_allowed
        && entries.iter().any(|name| name == tombstone_name())
        && den.lock_is_inert(OsStr::new(DEN_LOCK))?)
}

fn write(den: &PrivateDirectory, tombstone: &Tombstone) -> CommitOutcome {
    match tombstone.encode() {
        Ok(bytes) => den.replace(tombstone_name(), &bytes, false),
        Err(error) => CommitOutcome::NotCommitted(error),
    }
}

/// Section 4, steps 3–5, after a durably committed incomplete Tombstone:
/// destroy, verify the exact allowlist (I3), and only then mark it complete.
fn finish(den: &PrivateDirectory, mut tombstone: Tombstone) -> Destruction {
    tombstone.destruction = Destruction::Incomplete;
    if destroy_den(den).is_err() {
        return Destruction::Incomplete;
    }
    halt("before_complete");
    if !matches!(den_is_clean(den), Ok(true)) {
        return Destruction::Incomplete;
    }
    tombstone.destruction = Destruction::Complete;
    match write(den, &tombstone) {
        CommitOutcome::DurablyCommitted => Destruction::Complete,
        _ => Destruction::Incomplete,
    }
}

fn label(destruction: Destruction) -> &'static str {
    match destruction {
        Destruction::Complete => "complete",
        Destruction::Incomplete => "incomplete",
    }
}

/// Section 6, step 1: before any security state is loaded. `Ok(None)` means
/// no Tombstone; `Ok(Some(message))` means this process must exit 67; an
/// error (a malformed Tombstone is never treated as absent) exits 65.
pub(crate) fn at_startup(den: &PrivateDirectory) -> io::Result<Option<String>> {
    let Some(bytes) = den.read(tombstone_name(), tombstone::MAX_LEN + 1)? else {
        return Ok(None);
    };
    let found = Tombstone::parse(&bytes)?;
    let kind = match found.kind() {
        Kind::Death => "death",
        Kind::Retirement => "retirement",
    };
    if found.destruction == Destruction::Complete && den_is_clean(den)? {
        return Ok(Some(format!("{kind} tombstone, destruction complete")));
    }
    // I3: a complete Tombstone next to anything else is first durably
    // rewritten as incomplete; destruction then runs again.
    let mut incomplete = found.clone();
    incomplete.destruction = Destruction::Incomplete;
    if found.destruction == Destruction::Complete
        && !matches!(write(den, &incomplete), CommitOutcome::DurablyCommitted)
    {
        return Ok(Some(format!(
            "{kind} tombstone found next to other entries and could not be marked incomplete"
        )));
    }
    let destruction = finish(den, incomplete);
    Ok(Some(format!(
        "{kind} tombstone, destruction {}",
        label(destruction)
    )))
}

/// The death statement recorded for a DEAD head: its archive entry, written
/// before the head was committed (G3), checked against the head.
fn archived_death(den: &PrivateDirectory, head: &Head) -> io::Result<Statement> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "death statement unavailable");
    let name = crate::discovery::governance_archive_name(head.sequence);
    let bytes = den
        .read(
            OsStr::new(&name),
            werewolf_core::governance::MAX_STATEMENT_LEN,
        )?
        .ok_or_else(invalid)?;
    let statement = Statement::decode(&bytes).map_err(|_| invalid())?;
    if statement.core.hash() != head.hash || !matches!(statement.core.body, Body::Death) {
        return Err(invalid());
    }
    Ok(statement)
}

/// Section 6, step 2: a committed DEAD head without a Tombstone (a crash
/// between the commit and the durable Tombstone). Runs section 4 from the
/// Tombstone write; returns the message for the exit-67 log.
pub(crate) fn dead_head_at_startup(den: &PrivateDirectory, head: &Head) -> io::Result<String> {
    let statement = archived_death(den, head)?;
    let tombstone = Tombstone::death(statement, Destruction::Incomplete, &software())?;
    if !matches!(write(den, &tombstone), CommitOutcome::DurablyCommitted) {
        return Ok("death committed; tombstone not yet durable, retried at next start".into());
    }
    halt("after_tombstone");
    Ok(format!(
        "death tombstone, destruction {}",
        label(finish(den, tombstone))
    ))
}

/// Section 4, step 1: lock everything at once. Inbound sessions and RPCs
/// are refused (the Silver lock), every Fang is closed, and the node is
/// DYING, so every control command but `status` and `governance status` is
/// refused. Listeners end with the process.
fn lock_node(live: &mut DaemonState) {
    live.dying = true;
    let _ = live.inbound_authority.lock();
    live.fang_registry.abort_all();
    live.fang_registry.clear_started();
    live.fang_registry.clear_records();
    live.status.active_fangs = 0;
}

/// Exit 67 once the reply is written: the control loop notifies after
/// writing; this grace covers a client that went away.
fn request_exit(live: &mut DaemonState) {
    live.terminal_exit_requested = true;
    let exit = live.terminal_exit.clone();
    tokio::spawn(async move {
        tokio::time::sleep(EXIT_GRACE).await;
        exit.notify_one();
    });
}

async fn den_of(state: &Arc<Mutex<DaemonState>>) -> io::Result<Arc<PrivateDirectory>> {
    state
        .lock()
        .await
        .den
        .clone()
        .ok_or_else(|| io::Error::other("Den not initialized"))
}

/// Death after the DEAD head is durably committed and published (the point
/// of no return): lock, durable incomplete Tombstone, destroy, verify,
/// complete. The reply reports the destruction; the process then exits 67.
pub(crate) async fn die(state: &Arc<Mutex<DaemonState>>, statement: Statement) -> Value {
    halt("after_commit");
    let reply = |destruction: Destruction| {
        json!({
            "status": "applied",
            "sequence": statement.core.sequence,
            "hash": werewolf_core::governance::tool::hex(&statement.core.hash()),
            "lifecycle": "dead",
            "destruction": label(destruction),
        })
    };
    lock_node(&mut *state.lock().await);
    let destruction = match den_of(state).await {
        Ok(den) => {
            let statement = statement.clone();
            tokio::task::spawn_blocking(move || {
                let tombstone =
                    Tombstone::death(statement, Destruction::Incomplete, &software()).ok()?;
                if !matches!(write(&den, &tombstone), CommitOutcome::DurablyCommitted) {
                    // DEAD is committed: the next start retries (section 6).
                    return None;
                }
                halt("after_tombstone");
                Some(finish(&den, tombstone))
            })
            .await
            .ok()
            .flatten()
            .unwrap_or(Destruction::Incomplete)
        }
        Err(_) => Destruction::Incomplete,
    };
    request_exit(&mut *state.lock().await);
    reply(destruction)
}

/// `den retire --confirm FINGERPRINT` (section 3, "Retiring a node").
pub(crate) async fn retire(
    state: &Arc<Mutex<DaemonState>>,
    confirm: &str,
) -> Result<Value, (&'static str, String)> {
    let den = {
        let mut live = state.lock().await;
        let Some(pelt) = live.pelt.as_ref() else {
            return Err((
                "NO_PELT",
                "a Den without a Pelt holds no identity; remove it directly".into(),
            ));
        };
        if pelt.fingerprint != confirm {
            return Err((
                "RETIRE_CONFIRM_MISMATCH",
                "--confirm must be this node's own Pelt fingerprint".into(),
            ));
        }
        let den = live
            .den
            .clone()
            .ok_or(("RETIRE_SAVE_FAILED", "Den not initialized".to_string()))?;
        // Step 1, in memory only: refuse every other command and all inbound
        // work. A crash here leaves the node exactly as before.
        live.dying = true;
        let _ = live.inbound_authority.lock();
        den
    };
    let tombstone = Tombstone::retirement(Destruction::Incomplete, &software())
        .map_err(|_| ("RETIRE_SAVE_FAILED", "cannot encode the tombstone".into()))?;
    let written = {
        let den = den.clone();
        let tombstone = tombstone.clone();
        tokio::task::spawn_blocking(move || write(&den, &tombstone))
            .await
            .unwrap_or_else(|_| {
                CommitOutcome::IndeterminateAfterRename(io::Error::other("storage worker failed"))
            })
    };
    match written {
        // Step 2 succeeded: the point of no return.
        CommitOutcome::DurablyCommitted => {}
        CommitOutcome::NotCommitted(_) => {
            // Nothing durable changed: the node unlocks, unless Silver or
            // local admission holds the lock on their own.
            let mut live = state.lock().await;
            live.dying = false;
            let silver = live.status.silver == "active";
            if !silver && !crate::governance::unadmitted(&live) {
                if let Ok(epoch) = live.inbound_authority.epoch() {
                    let _ = live.inbound_authority.open_after_durable(epoch);
                }
            }
            return Err((
                "RETIRE_SAVE_FAILED",
                "the retirement tombstone was not written; nothing happened".into(),
            ));
        }
        CommitOutcome::IndeterminateAfterRename(_) => {
            // No destruction without a confirmed Tombstone; the next start
            // decides from what is on disk.
            state.lock().await.storage_degraded = true;
            return Err((
                "RETIRE_SAVE_FAILED",
                "the retirement tombstone write is indeterminate; the node stays locked and \
                 retirement resumes at the next start if the tombstone is present"
                    .into(),
            ));
        }
    }
    halt("after_tombstone");
    lock_node(&mut *state.lock().await);
    let destruction = tokio::task::spawn_blocking(move || finish(&den, tombstone))
        .await
        .unwrap_or(Destruction::Incomplete);
    request_exit(&mut *state.lock().await);
    Ok(json!({"status": "retired", "destruction": label(destruction)}))
}

/// `lifecycle` for `status` and `governance status` (section 3).
pub(crate) fn lifecycle(live: &DaemonState) -> Value {
    let chain = match &live.governance {
        None => "none",
        Some(head) if head.dead_at.is_some() => "dead",
        Some(head) if head.activated_at.is_some() => "active",
        Some(_) => "ceremony",
    };
    json!({"chain": chain, "node": if live.dying { "dying" } else { "normal" }})
}

#[cfg(test)]
mod tests;
