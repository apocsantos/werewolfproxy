//! Runtime inbound authority, independent of transport replay reservations.
//! The short synchronous mutex is the publication/submission linearization gate.
//! Never acquire daemon state, perform filesystem I/O, or await while holding it.
mod writer;
pub(crate) use writer::AuthorityWriter;

use crate::handshake::{fingerprint, rejected, require};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::sync::{watch, Notify};

pub(crate) const CLEANUP_WINDOW: Duration = Duration::from_secs(5);
const GLOBAL_SESSIONS: usize = 1024;
const PEER_SESSIONS: usize = 64;
const CONNECTION_SESSIONS: usize = 64;
// Same bound as the validated Pack. Removed generations need no tombstones:
// tickets contain a monotonically allocated generation that is never reused.
const PEER_GENERATIONS: usize = 1024;
type Peer = [u8; 8];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PeerAuthority {
    Authorized,
    RuntimeDeniedPendingDurability,
    Revoked,
}
struct Generation {
    number: u64,
    state: PeerAuthority,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transport {
    Tcp,
    Quic(ConnectionId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionId(u64);

// Fields are private: a transport cannot manufacture a valid ticket or lease.
#[derive(Clone, Copy)]
pub(crate) struct AuthorityTicket {
    peer: Peer,
    generation: u64,
    epoch: u64,
}
struct Entry {
    ticket: AuthorityTicket,
    transport: Transport,
    active: bool,
    cancel: watch::Sender<bool>,
}
struct Gate {
    epoch: u64,
    locked: bool,
    next: u64,
    peers: HashMap<Peer, Generation>,
    sessions: HashMap<u64, Entry>,
}
pub(crate) struct Authority {
    gate: Mutex<Gate>,
    changed: Notify,
    silver: watch::Sender<u64>,
}
impl Authority {
    pub(crate) fn new(locked: bool) -> Arc<Self> {
        let (silver, _) = watch::channel(1);
        Arc::new(Self {
            gate: Mutex::new(Gate {
                epoch: 1,
                locked,
                next: 1,
                peers: HashMap::new(),
                sessions: HashMap::new(),
            }),
            changed: Notify::new(),
            silver,
        })
    }
    fn gate(&self) -> io::Result<MutexGuard<'_, Gate>> {
        self.gate.lock().map_err(|_| rejected())
    }
    pub(crate) fn is_locked(&self) -> bool {
        self.gate().map(|g| g.locked).unwrap_or(true)
    }
    pub(crate) fn silver_watch(&self) -> watch::Receiver<u64> {
        self.silver.subscribe()
    }
    pub(crate) fn connection(&self) -> io::Result<ConnectionId> {
        Ok(ConnectionId(self.gate()?.allocate()?))
    }

    /// Caller must hold the daemon-state lock and establish current Pack
    /// membership first. Signature verification follows; reservation must not
    /// occur until it succeeds. This captures the pre-verification generation.
    pub(crate) fn ticket(
        &self,
        authenticated_pack_fingerprint: &str,
    ) -> io::Result<AuthorityTicket> {
        let peer = fingerprint(authenticated_pack_fingerprint)?;
        let mut gate = self.gate()?;
        require(!gate.locked)?;
        if !gate.peers.contains_key(&peer) {
            require(gate.peers.len() < PEER_GENERATIONS)?;
            let number = gate.allocate()?;
            gate.peers.insert(
                peer,
                Generation {
                    number,
                    state: PeerAuthority::Authorized,
                },
            );
        }
        let generation = gate.peers.get(&peer).ok_or_else(rejected)?;
        require(generation.state == PeerAuthority::Authorized)?;
        Ok(AuthorityTicket {
            peer,
            generation: generation.number,
            epoch: gate.epoch,
        })
    }

    /// Called only after signature and connection binding have been verified.
    /// Establishing and active entries share quotas; cancellation never evicts.
    pub(crate) fn reserve(
        self: &Arc<Self>,
        ticket: AuthorityTicket,
        transport: Transport,
    ) -> io::Result<SessionLease> {
        let mut gate = self.gate()?;
        require(gate.current(ticket))?;
        require(gate.sessions.len() < GLOBAL_SESSIONS)?;
        require(
            gate.sessions
                .values()
                .filter(|e| e.ticket.peer == ticket.peer)
                .count()
                < PEER_SESSIONS,
        )?;
        if let Transport::Quic(_) = transport {
            require(
                gate.sessions
                    .values()
                    .filter(|e| e.transport == transport)
                    .count()
                    < CONNECTION_SESSIONS,
            )?;
        }
        let id = gate.allocate()?;
        let (cancel, _) = watch::channel(false);
        gate.sessions.insert(
            id,
            Entry {
                ticket,
                transport,
                active: false,
                cancel,
            },
        );
        Ok(SessionLease(Arc::new(LeaseOwner {
            authority: self.clone(),
            id,
        })))
    }

    /// Runtime-first revocation linearizes at this lock. A failed durable write
    /// must not undo this state. No authority is inferred from a later response.
    pub(crate) fn deny_peer(&self, peer: &str) -> io::Result<()> {
        let peer = fingerprint(peer)?;
        let mut gate = self.gate()?;
        if !gate.peers.contains_key(&peer) {
            require(gate.peers.len() < PEER_GENERATIONS)?;
            let number = gate.allocate()?;
            gate.peers.insert(
                peer,
                Generation {
                    number,
                    state: PeerAuthority::RuntimeDeniedPendingDurability,
                },
            );
        } else if let Some(generation) = gate.peers.get_mut(&peer) {
            generation.state = PeerAuthority::RuntimeDeniedPendingDurability;
        }
        for entry in gate.sessions.values().filter(|e| e.ticket.peer == peer) {
            entry.cancel.send_replace(true);
        }
        Ok(())
    }
    /// Called while publishing durable Pack removal under the daemon-state
    /// lock. Absent peers are REVOKED; future re-add gets a fresh generation.
    pub(crate) fn removed(&self, peer: &str) -> io::Result<()> {
        let peer = fingerprint(peer)?;
        let mut gate = self.gate()?;
        require(
            gate.peers
                .get(&peer)
                .is_some_and(|g| g.state == PeerAuthority::RuntimeDeniedPendingDurability),
        )?;
        gate.peers.remove(&peer);
        Ok(())
    }
    pub(crate) fn peer_state(&self, peer: &str, pack_member: bool) -> io::Result<PeerAuthority> {
        let peer = fingerprint(peer)?;
        Ok(self
            .gate()?
            .peers
            .get(&peer)
            .map(|g| g.state)
            .unwrap_or(if pack_member {
                PeerAuthority::Authorized
            } else {
                PeerAuthority::Revoked
            }))
    }
    /// Returned epoch is the durable transition token. Even repeated ON defeats
    /// an older in-flight OFF; persistence work may be coalesced by its owner.
    pub(crate) fn lock(&self) -> io::Result<u64> {
        let mut gate = self.gate()?;
        gate.locked = true;
        // Overflow is fail closed: locked remains true and no OFF can succeed.
        for entry in gate.sessions.values() {
            entry.cancel.send_replace(true);
        }
        // Exhaustion cannot leave cancelled work asleep or reopen authority.
        let advanced = gate.epoch.checked_add(1);
        gate.epoch = advanced.unwrap_or(u64::MAX);
        self.silver.send_replace(gate.epoch);
        require(advanced.is_some())?;
        Ok(gate.epoch)
    }
    pub(crate) fn epoch(&self) -> io::Result<u64> {
        Ok(self.gate()?.epoch)
    }
    /// Only after durable OPEN and healthy storage. A newer ON wins.
    pub(crate) fn open_after_durable(&self, expected_epoch: u64) -> io::Result<()> {
        let mut gate = self.gate()?;
        require(gate.epoch == expected_epoch)?;
        gate.epoch = gate.epoch.checked_add(1).ok_or_else(rejected)?;
        gate.locked = false;
        Ok(())
    }
    pub(crate) async fn cleanup(&self, peer: Option<&str>) -> io::Result<()> {
        let peer = peer.map(fingerprint).transpose()?;
        tokio::time::timeout(CLEANUP_WINDOW, async {
            loop {
                let notified = self.changed.notified();
                // Register before the predicate to avoid losing final cleanup.
                tokio::pin!(notified);
                notified.as_mut().enable();
                if !self
                    .gate()?
                    .sessions
                    .values()
                    .any(|e| peer.is_none_or(|p| p == e.ticket.peer))
                {
                    return Ok(());
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| rejected())?
    }
}
impl Gate {
    fn allocate(&mut self) -> io::Result<u64> {
        let id = self.next;
        self.next = self.next.checked_add(1).ok_or_else(rejected)?;
        Ok(id)
    }
    fn current(&self, ticket: AuthorityTicket) -> bool {
        !self.locked
            && self.epoch == ticket.epoch
            && self.peers.get(&ticket.peer).is_some_and(|g| {
                g.number == ticket.generation && g.state == PeerAuthority::Authorized
            })
    }
}
#[derive(Clone)]
pub(crate) struct SessionLease(Arc<LeaseOwner>);
struct LeaseOwner {
    authority: Arc<Authority>,
    id: u64,
}
impl SessionLease {
    pub(crate) fn publish(&self) -> io::Result<()> {
        let mut gate = self.0.authority.gate()?;
        let entry = gate.sessions.get(&self.0.id).ok_or_else(rejected)?;
        require(gate.current(entry.ticket))?;
        gate.sessions
            .get_mut(&self.0.id)
            .ok_or_else(rejected)?
            .active = true;
        Ok(())
    }
    /// Closure must perform exactly one nonblocking poll, never await or take
    /// daemon state. Production write callers are sealed raw transport writers.
    fn submit<T>(&self, poll: impl FnOnce() -> T) -> io::Result<T> {
        let gate = self.0.authority.gate()?;
        let entry = gate.sessions.get(&self.0.id).ok_or_else(rejected)?;
        require(gate.current(entry.ticket))?;
        Ok(poll())
    }
    pub(crate) async fn cancelled(&self) {
        let receiver = self
            .0
            .authority
            .gate()
            .ok()
            .and_then(|g| g.sessions.get(&self.0.id).map(|e| e.cancel.subscribe()));
        if let Some(mut receiver) = receiver {
            loop {
                if *receiver.borrow_and_update() {
                    break;
                }
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        }
    }
}
impl Drop for LeaseOwner {
    fn drop(&mut self) {
        // Poison recovery is for releasing resources only, never authorizing.
        self.authority
            .gate
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sessions
            .remove(&self.id);
        self.authority.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests;
