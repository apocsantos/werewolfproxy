//! Bounded, nonpersistent v3 admission. No eviction of active replay records.
#![allow(dead_code)] // Introduced before transport migration.
use crate::handshake::{self, rejected};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Mutex},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const GLOBAL_HANDSHAKES: usize = 256;
const PEER_HANDSHAKES: usize = 16;
const QUIC_CONTEXTS: usize = 1024;
const QUIC_OPENS: usize = 8;
const QUIC_RESERVATIONS: usize = 256;

pub(super) struct Admission {
    global: Arc<Semaphore>,
    contexts: Arc<Semaphore>,
    peers: Mutex<HashMap<[u8; 8], usize>>,
    peer_limit: usize,
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            global: Arc::new(Semaphore::new(GLOBAL_HANDSHAKES)),
            contexts: Arc::new(Semaphore::new(QUIC_CONTEXTS)),
            peers: Mutex::new(HashMap::new()),
            peer_limit: PEER_HANDSHAKES,
        }
    }
}
impl Admission {
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
