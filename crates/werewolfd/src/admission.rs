//! Bounded, nonpersistent v3 admission. No eviction of active replay records.
use crate::handshake::{self, rejected};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Mutex},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const GLOBAL_HANDSHAKES: usize = 256;
const PEER_HANDSHAKES: usize = 16;
// A context is owned before a QUIC handshake completes and remains owned for
// the life of the resulting connection. This separate bound keeps a completed
// transport handshake that never opens a Werewolf stream from consuming the
// entire process admission budget.
const QUIC_CONTEXTS: usize = 128;
const QUIC_OPENS: usize = 8;
const QUIC_RESERVATIONS: usize = 256;
// DNS authorization and target connection attempts begin only after sender
// signature verification. They still need independent global and per-peer
// ceilings: a Pack member is authenticated, not entitled to unlimited work.
const GLOBAL_TARGET_WORK: usize = 64;
const PEER_TARGET_WORK: usize = 8;

pub(super) struct Admission {
    global: Arc<Semaphore>,
    contexts: Arc<Semaphore>,
    target_work: Arc<Semaphore>,
    peers: Mutex<HashMap<[u8; 8], usize>>,
    target_peers: Mutex<HashMap<[u8; 8], usize>>,
    peer_limit: usize,
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            global: Arc::new(Semaphore::new(GLOBAL_HANDSHAKES)),
            contexts: Arc::new(Semaphore::new(QUIC_CONTEXTS)),
            target_work: Arc::new(Semaphore::new(GLOBAL_TARGET_WORK)),
            peers: Mutex::new(HashMap::new()),
            target_peers: Mutex::new(HashMap::new()),
            peer_limit: PEER_HANDSHAKES,
        }
    }
}
impl Admission {
    #[cfg(test)]
    pub(super) fn test_with_handshake_limit(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            global: Arc::new(Semaphore::new(limit)),
            contexts: Arc::new(Semaphore::new(QUIC_CONTEXTS)),
            target_work: Arc::new(Semaphore::new(GLOBAL_TARGET_WORK)),
            peers: Mutex::new(HashMap::new()),
            target_peers: Mutex::new(HashMap::new()),
            peer_limit: PEER_HANDSHAKES,
        })
    }

    #[cfg(test)]
    pub(super) fn available_handshakes(&self) -> usize {
        self.global.available_permits()
    }

    pub(super) fn handshake(self: &Arc<Self>) -> io::Result<HandshakePermit> {
        Ok(HandshakePermit {
            _global: self
                .global
                .clone()
                .try_acquire_owned()
                .map_err(|_| rejected())?,
            owner: self.clone(),
            peer: None,
        })
    }
    pub(super) fn quic_context(self: &Arc<Self>) -> io::Result<OwnedSemaphorePermit> {
        self.contexts
            .clone()
            .try_acquire_owned()
            .map_err(|_| rejected())
    }

    fn target_work(self: &Arc<Self>, peer: [u8; 8]) -> io::Result<TargetWorkPermit> {
        // Acquire the global permit first. If the per-peer check fails, RAII
        // immediately returns it; no request ever waits in an unbounded queue.
        let global = self
            .target_work
            .clone()
            .try_acquire_owned()
            .map_err(|_| rejected())?;
        let mut peers = self.target_peers.lock().map_err(|_| rejected())?;
        let count = peers.get(&peer).copied().unwrap_or(0);
        handshake::require(count < PEER_TARGET_WORK)?;
        peers.insert(peer, count + 1);
        Ok(TargetWorkPermit {
            _global: global,
            owner: self.clone(),
            peer,
        })
    }
}
pub(super) struct HandshakePermit {
    _global: OwnedSemaphorePermit,
    owner: Arc<Admission>,
    peer: Option<[u8; 8]>,
}
impl HandshakePermit {
    // Call only AFTER signature/binding verification. There is no API for a
    // claimed identity to create a peer counter without holding global admission.
    pub(super) fn authenticated(&mut self, fingerprint: &str) -> io::Result<()> {
        handshake::require(self.peer.is_none())?;
        let key = handshake::fingerprint(fingerprint)?;
        let mut peers = self.owner.peers.lock().map_err(|_| rejected())?;
        let count = peers.get(&key).copied().unwrap_or(0);
        handshake::require(count < self.owner.peer_limit)?;
        peers.insert(key, count + 1);
        self.peer = Some(key);
        Ok(())
    }

    /// Acquire bounded DNS/target-connect work only for the identity which
    /// has already completed signature verification on this handshake.
    pub(super) fn target_work(&self, fingerprint: &str) -> io::Result<TargetWorkPermit> {
        let peer = handshake::fingerprint(fingerprint)?;
        handshake::require(self.peer == Some(peer))?;
        self.owner.target_work(peer)
    }
}
impl Drop for HandshakePermit {
    fn drop(&mut self) {
        if let Some(key) = self.peer {
            let mut peers = self.owner.peers.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(count) = peers.get_mut(&key) {
                *count -= 1;
                if *count == 0 {
                    peers.remove(&key);
                }
            }
        }
    }
}

pub(super) struct TargetWorkPermit {
    _global: OwnedSemaphorePermit,
    owner: Arc<Admission>,
    peer: [u8; 8],
}
impl Drop for TargetWorkPermit {
    fn drop(&mut self) {
        let mut peers = self
            .owner
            .target_peers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(count) = peers.get_mut(&self.peer) {
            *count -= 1;
            if *count == 0 {
                peers.remove(&self.peer);
            }
        }
    }
}

// Domain/context are structural: one QuicReplayV3 value belongs to exactly one
// established v3 QUIC connection. No global string keys or reusable stable_id.
#[derive(Hash, Eq, PartialEq)]
struct QuicReservation {
    sender: [u8; 8],
    nonce: [u8; 16],
}
pub(super) struct QuicReplayV3 {
    pub(super) opens: Arc<Semaphore>,
    reservations: Mutex<HashSet<QuicReservation>>,
    limit: usize,
}
impl Default for QuicReplayV3 {
    fn default() -> Self {
        Self {
            opens: Arc::new(Semaphore::new(QUIC_OPENS)),
            reservations: Mutex::new(HashSet::new()),
            limit: QUIC_RESERVATIONS,
        }
    }
}
impl QuicReplayV3 {
    pub(super) fn reserve(&self, sender: &str, nonce: &str) -> io::Result<()> {
        let key = QuicReservation {
            sender: handshake::fingerprint(sender)?,
            nonce: handshake::unhex(nonce)?,
        };
        let mut entries = self.reservations.lock().map_err(|_| rejected())?;
        handshake::require(entries.len() < self.limit && !entries.contains(&key))?;
        entries.insert(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PEER: &str = "wwp1:01-23-45-67-89-AB-CD-EF";
    #[test]
    fn global_peer_limits_release_without_retained_peer_counters() {
        let a = Arc::new(Admission::default());
        let all: Vec<_> = (0..GLOBAL_HANDSHAKES)
            .map(|_| a.handshake().unwrap())
            .collect();
        assert!(a.handshake().is_err());
        assert!(a.peers.lock().unwrap().is_empty());
        drop(all);
        assert_eq!(a.global.available_permits(), GLOBAL_HANDSHAKES);
        let peer: Vec<_> = (0..PEER_HANDSHAKES)
            .map(|_| {
                let mut p = a.handshake().unwrap();
                p.authenticated(PEER).unwrap();
                p
            })
            .collect();
        let mut extra = a.handshake().unwrap();
        assert!(extra.authenticated(PEER).is_err());
        drop(extra);
        drop(peer);
        assert!(a.peers.lock().unwrap().is_empty());
        assert_eq!(a.global.available_permits(), GLOBAL_HANDSHAKES);
        assert!(a.target_peers.lock().unwrap().is_empty());
    }
    #[test]
    fn connection_limits_and_no_eviction() {
        let a = Arc::new(Admission::default());
        let contexts: Vec<_> = (0..QUIC_CONTEXTS)
            .map(|_| a.quic_context().unwrap())
            .collect();
        assert!(a.quic_context().is_err());
        drop(contexts);
        assert!(a.quic_context().is_ok());
        let replay = QuicReplayV3::default();
        let opens: Vec<_> = (0..QUIC_OPENS)
            .map(|_| replay.opens.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(replay.opens.clone().try_acquire_owned().is_err());
        drop(opens);
        for n in 0..QUIC_RESERVATIONS {
            replay
                .reserve(PEER, &handshake::hex(&(n as u128).to_be_bytes()))
                .unwrap();
        }
        assert!(replay.reserve(PEER, &handshake::hex(&[0xff; 16])).is_err());
        assert!(replay.reserve(PEER, &handshake::hex(&[0; 16])).is_err());
        assert_eq!(replay.reservations.lock().unwrap().len(), QUIC_RESERVATIONS);
    }
    #[test]
    fn authenticated_target_work_is_globally_and_per_peer_bounded_and_released() {
        let admission = Arc::new(Admission::default());
        let handshakes: Vec<_> = (0..PEER_TARGET_WORK)
            .map(|_| {
                let mut permit = admission.handshake().unwrap();
                permit.authenticated(PEER).unwrap();
                permit
            })
            .collect();
        let work: Vec<_> = handshakes
            .iter()
            .map(|permit| permit.target_work(PEER).unwrap())
            .collect();
        assert!(handshakes[0].target_work(PEER).is_err());
        drop(work);
        assert_eq!(
            admission.target_work.available_permits(),
            GLOBAL_TARGET_WORK
        );
        assert!(admission.target_peers.lock().unwrap().is_empty());
        drop(handshakes);
    }
    #[test]
    fn target_work_global_limit_has_no_waiting_queue() {
        let admission = Arc::new(Admission::default());
        let peers: Vec<_> = (0..GLOBAL_TARGET_WORK)
            .map(|n| format!("wwp1:00-00-00-00-00-00-00-{n:02X}"))
            .collect();
        let handshakes: Vec<_> = peers
            .iter()
            .map(|peer| {
                let mut permit = admission.handshake().unwrap();
                permit.authenticated(peer).unwrap();
                permit
            })
            .collect();
        let work: Vec<_> = handshakes
            .iter()
            .zip(&peers)
            .map(|(permit, peer)| permit.target_work(peer).unwrap())
            .collect();
        let overflow_peer = "wwp1:00-00-00-00-00-00-FF-FF";
        let mut overflow = admission.handshake().unwrap();
        overflow.authenticated(overflow_peer).unwrap();
        assert!(overflow.target_work(overflow_peer).is_err());
        drop(work);
        assert_eq!(
            admission.target_work.available_permits(),
            GLOBAL_TARGET_WORK
        );
        assert!(admission.target_peers.lock().unwrap().is_empty());
    }
    #[test]
    fn simultaneous_reservation_has_one_winner() {
        let replay = Arc::new(QuicReplayV3::default());
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let jobs: Vec<_> = (0..8)
            .map(|_| {
                let r = replay.clone();
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    r.reserve(PEER, &handshake::hex(&[1; 16])).is_ok()
                })
            })
            .collect();
        assert_eq!(
            jobs.into_iter()
                .filter_map(|j| j.join().unwrap().then_some(()))
                .count(),
            1
        );
    }
}
