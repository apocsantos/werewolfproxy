//! Stage53: reserved shares of one daemon's outgoing peer-session budget.
//! No remote capacity is inferred or negotiated. The receiver stays authoritative.
use crate::{handshake as hs, policy::ExpectedPeerIdentity};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt, io,
    sync::{Arc, Mutex},
};

const BUDGET: usize = 64;
const MEMBERS: usize = 64;
type Key = [u8; 32];

#[derive(Debug)]
pub(crate) enum RegistrationError {
    Capacity,
    Identity,
    Unavailable,
}

impl RegistrationError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Capacity => "FANG_PEER_CAPACITY",
            _ => "FANG_ADMISSION_FAILED",
        }
    }
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Capacity => "a receiver already has 64 registered Fangs",
            Self::Identity => "peer admission requires a valid captured Pack public key",
            Self::Unavailable => "peer admission coordinator unavailable",
        })
    }
}
impl std::error::Error for RegistrationError {}

#[derive(Default)]
struct Peer {
    // Registration serial gives stable remainder order, independent of key/name.
    members: BTreeMap<u64, usize>,
    used: usize,
}

#[derive(Default)]
struct State {
    next: u64,
    peers: HashMap<Key, Peer>,
}

#[derive(Default)]
pub(crate) struct Coordinator(Mutex<State>);

#[derive(Clone)]
pub(crate) struct Membership(Arc<Member>);

struct Member {
    coordinator: Arc<Coordinator>,
    serial: u64,
    keys: BTreeSet<Key>,
    plans: Vec<BTreeSet<Key>>,
}

pub(crate) struct Reservation {
    membership: Membership,
    plan: usize,
    session: Option<super::fang_accept::Registration>,
}

/// Actual live attempt keys, never the union of unused alternatives.
pub(crate) struct Demand {
    membership: Membership,
    plan: usize,
}

#[derive(Debug)]
struct CapacityDenied;
impl fmt::Display for CapacityDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("dial failed locally: local peer-session share exhausted")
    }
}
impl std::error::Error for CapacityDenied {}

impl Coordinator {
    /// Register the union of all potential paths atomically. Plain TCP does
    /// not call this. No endpoint, address or transport contributes a key.
    pub(crate) fn register(
        self: &Arc<Self>,
        paths: Vec<Vec<&ExpectedPeerIdentity>>,
    ) -> io::Result<Membership> {
        let plans: Vec<BTreeSet<Key>> = paths
            .into_iter()
            .map(|path| {
                path.into_iter()
                    .map(|expected| {
                        let public = expected
                            .public_key_b64
                            .as_deref()
                            .ok_or_else(identity_error)?;
                        let bytes = hs::binary(public).map_err(|_| identity_error())?;
                        let fingerprint =
                            werewolf_core::pelt::fingerprint_from_public_key_b64(public)
                                .map_err(|_| identity_error())?;
                        if fingerprint != expected.fingerprint {
                            return Err(identity_error());
                        }
                        Ok(bytes)
                    })
                    .collect()
            })
            .collect::<io::Result<_>>()?;
        if plans.is_empty() || plans.iter().any(BTreeSet::is_empty) {
            return Err(identity_error());
        }
        let keys: BTreeSet<Key> = plans.iter().flatten().copied().collect();
        let mut state = self.0.lock().map_err(|_| unavailable())?;
        if keys.iter().any(|key| {
            state
                .peers
                .get(key)
                .is_some_and(|peer| peer.members.len() >= MEMBERS)
        }) {
            return Err(io::Error::other(RegistrationError::Capacity));
        }
        let serial = state.next.checked_add(1).ok_or_else(unavailable)?;
        state.next = serial;
        for key in &keys {
            state
                .peers
                .entry(*key)
                .or_default()
                .members
                .insert(serial, 0);
        }
        Ok(Membership(Arc::new(Member {
            coordinator: self.clone(),
            serial,
            keys,
            plans,
        })))
    }
}

fn identity_error() -> io::Error {
    io::Error::other(RegistrationError::Identity)
}
fn unavailable() -> io::Error {
    io::Error::other(RegistrationError::Unavailable)
}
fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::AddrNotAvailable, CapacityDenied)
}

impl Membership {
    /// Short synchronous all-or-nothing check. No queue, borrowing or eviction.
    pub(crate) fn reserve(&self, plan: usize) -> io::Result<Reservation> {
        let member = &self.0;
        let keys = member.plans.get(plan).ok_or_else(unavailable)?;
        let mut state = member.coordinator.0.lock().map_err(|_| unavailable())?;
        for key in keys {
            let peer = state.peers.get(key).ok_or_else(unavailable)?;
            let position = peer
                .members
                .keys()
                .position(|id| *id == member.serial)
                .ok_or_else(unavailable)?;
            let share =
                BUDGET / peer.members.len() + usize::from(position < BUDGET % peer.members.len());
            if peer.used >= BUDGET || peer.members[&member.serial] >= share {
                return Err(denied());
            }
        }
        for key in keys {
            let peer = state.peers.get_mut(key).expect("checked peer");
            peer.used += 1;
            *peer
                .members
                .get_mut(&member.serial)
                .expect("checked member") += 1;
        }
        // Never acquire a Fang registry while holding the coordinator:
        // reclamation and committed release use registry -> coordinator.
        drop(state);
        let mut reservation = Reservation {
            membership: self.clone(),
            plan,
            session: None,
        };
        if let Some(session) = super::fang_accept::Session::current() {
            reservation.session = Some(session.attach(Demand {
                membership: self.clone(),
                plan,
            })?);
        }
        Ok(reservation)
    }

    /// One useful idle victim, at most once per newcomer. A real release
    /// wakes this waiter; capacity is never credited before cleanup.
    pub(crate) async fn reserve_reclaim(
        &self,
        plan: usize,
        deadline: tokio::time::Instant,
    ) -> io::Result<Reservation> {
        use super::fang_accept::{Session, EVICT_WAIT};
        use tokio::time::Instant;
        let began = Instant::now();
        if began >= deadline {
            return Err(hs::no_answer());
        }
        let session = Session::current();
        if let Some(session) = &session {
            session.clear_first_dial()?;
        }
        let error = match self.reserve(plan) {
            Ok(reservation) => return Ok(reservation),
            Err(error) => error,
        };
        if !error
            .get_ref()
            .is_some_and(|error| error.is::<CapacityDenied>())
        {
            return Err(error);
        }
        let Some(session) = session else {
            return Err(error);
        };
        let dial_deadline = deadline.min(began + hs::READ_WINDOW);
        let release_deadline = dial_deadline.min(began + EVICT_WAIT);
        let Some(victim) = session.reclaim(
            |held| held.permits_replacement(self, plan),
            release_deadline,
        ) else {
            return Err(error);
        };
        if !victim.wait(release_deadline).await || Instant::now() >= dial_deadline {
            return Err(error);
        }
        session.charge_first_dial(dial_deadline)?;
        self.reserve(plan)
    }
}

impl Demand {
    fn permits_replacement(&self, requested: &Membership, plan: usize) -> bool {
        // A Fang registry is already held. This is the only nested lock
        // direction; a reservation removes its metadata before releasing.
        if !Arc::ptr_eq(&self.membership.0, &requested.0) {
            return false;
        }
        let member = &requested.0;
        let Some(keys) = member.plans.get(plan) else {
            return false;
        };
        let Some(held) = member.plans.get(self.plan) else {
            return false;
        };
        let Ok(state) = member.coordinator.0.lock() else {
            return false;
        };
        let mut pressure = false;
        let fits_after_release = keys.iter().all(|key| {
            let Some(peer) = state.peers.get(key) else {
                return false;
            };
            let Some(position) = peer.members.keys().position(|id| *id == member.serial) else {
                return false;
            };
            let share =
                BUDGET / peer.members.len() + usize::from(position < BUDGET % peer.members.len());
            pressure |= peer.used >= BUDGET || peer.members[&member.serial] >= share;
            let release = usize::from(held.contains(key));
            match (
                peer.used.checked_sub(release),
                peer.members[&member.serial].checked_sub(release),
            ) {
                (Some(total), Some(own)) => total < BUDGET && own < share,
                _ => false,
            }
        });
        // Another session can release capacity after the original denial.
        // An already-admissible plan has no useful eviction victim, even
        // if subtracting an unrelated reservation would also leave it free.
        pressure && fits_after_release
    }
}

impl Reservation {
    fn release_usage(&self) {
        let member = &self.membership.0;
        // Poisoning fails future admission closed. Do not grant capacity by
        // guessing at possibly damaged state during unwinding.
        if let Ok(mut state) = member.coordinator.0.lock() {
            for key in &member.plans[self.plan] {
                let peer = state.peers.get_mut(key).expect("live reservation peer");
                peer.used -= 1;
                *peer
                    .members
                    .get_mut(&member.serial)
                    .expect("live reservation member") -= 1;
            }
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            session.release(|| self.release_usage());
        } else {
            self.release_usage();
        }
    }
}

impl Drop for Member {
    fn drop(&mut self) {
        // Reservations retain Member, so this runs only after every local
        // attempt/session and listener reference has really been dropped.
        if let Ok(mut state) = self.coordinator.0.lock() {
            for key in &self.keys {
                let peer = state.peers.get_mut(key).expect("registered peer");
                let usage = peer.members.remove(&self.serial);
                debug_assert_eq!(usage, Some(0));
            }
            state.peers.retain(|_, peer| !peer.members.is_empty());
        }
    }
}

#[cfg(test)]
mod tests;
