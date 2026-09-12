use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Barrier,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const A: &str = "wwp1:01-23-45-67-89-AB-CD-EF";
const B: &str = "wwp1:11-23-45-67-89-AB-CD-EF";

fn lease(authority: &Arc<Authority>, peer: &str, transport: Transport) -> SessionLease {
    authority
        .reserve(authority.ticket(peer).unwrap(), transport)
        .unwrap()
}

#[test]
fn stale_ticket_cannot_reserve_publish_or_submit_after_revoke_and_readd() {
    let authority = Authority::new(false);
    let ticket = authority.ticket(A).unwrap();
    let session = authority.reserve(ticket, Transport::Tcp).unwrap();
    authority.deny_peer(A).unwrap();
    authority.deny_peer(A).unwrap();
    assert_eq!(
        authority.peer_state(A, true).unwrap(),
        PeerAuthority::RuntimeDeniedPendingDurability
    );
    assert!(authority.reserve(ticket, Transport::Tcp).is_err());
    assert!(session.publish().is_err());
    assert!(session.submit(|| panic!("stale submission")).is_err());
    authority.removed(A).unwrap();
    assert_eq!(
        authority.peer_state(A, false).unwrap(),
        PeerAuthority::Revoked
    );
    let fresh = lease(&authority, A, Transport::Tcp);
    fresh.publish().unwrap();
    assert!(authority.reserve(ticket, Transport::Tcp).is_err());
    assert!(session.submit(|| ()).is_err());
}

#[tokio::test]
async fn cancellation_preserves_other_identity_on_shared_quic_connection() {
    let authority = Authority::new(false);
    let transport = Transport::Quic(authority.connection().unwrap());
    let a = lease(&authority, A, transport);
    let b = lease(&authority, B, transport);
    a.publish().unwrap();
    b.publish().unwrap();
    authority.deny_peer(A).unwrap();
    tokio::time::timeout(Duration::from_secs(1), a.cancelled())
        .await
        .unwrap();
    b.submit(|| ()).unwrap();
    assert_eq!(authority.gate().unwrap().sessions.len(), 2);
    drop(a);
    authority.cleanup(Some(A)).await.unwrap();
    assert_eq!(authority.gate().unwrap().sessions.len(), 1);
    drop(b);
    authority.cleanup(None).await.unwrap();
}

#[test]
fn quotas_count_cancelled_entries_and_all_lease_owners_until_cleanup() {
    let authority = Authority::new(false);
    let sessions: Vec<_> = (0..PEER_SESSIONS)
        .map(|_| lease(&authority, A, Transport::Tcp))
        .collect();
    assert!(authority
        .reserve(authority.ticket(A).unwrap(), Transport::Tcp)
        .is_err());
    let retained = sessions[0].clone();
    authority.deny_peer(A).unwrap();
    assert_eq!(authority.gate().unwrap().sessions.len(), PEER_SESSIONS);
    drop(sessions);
    assert_eq!(authority.gate().unwrap().sessions.len(), 1);
    drop(retained);
    assert!(authority.gate().unwrap().sessions.is_empty());
}

#[test]
fn connection_and_global_limits_are_independent_and_never_evict() {
    let authority = Authority::new(false);
    let connection = Transport::Quic(authority.connection().unwrap());
    let mut sessions = Vec::new();
    for n in 0..CONNECTION_SESSIONS {
        sessions.push(lease(
            &authority,
            if n % 2 == 0 { A } else { B },
            connection,
        ));
    }
    assert!(authority
        .reserve(authority.ticket(A).unwrap(), connection)
        .is_err());
    let other = Transport::Quic(authority.connection().unwrap());
    sessions.push(lease(&authority, A, other));
    while sessions.len() < GLOBAL_SESSIONS {
        let n = sessions.len();
        let peer = format!("wwp1:22-33-44-55-66-77-{:02X}-{:02X}", n / 256, n % 256);
        sessions.push(lease(&authority, &peer, Transport::Tcp));
    }
    assert!(authority
        .reserve(authority.ticket(B).unwrap(), Transport::Tcp)
        .is_err());
    assert_eq!(authority.gate().unwrap().sessions.len(), GLOBAL_SESSIONS);
    drop(sessions.pop());
    sessions.push(lease(&authority, B, Transport::Tcp));
    drop(sessions);
    assert!(authority.gate().unwrap().sessions.is_empty());
}

#[test]
fn silver_newer_on_defeats_older_off_and_old_sessions_never_resume() {
    let authority = Authority::new(true);
    assert!(authority.ticket(A).is_err());
    let first = authority.epoch().unwrap();
    authority.open_after_durable(first).unwrap();
    let session = lease(&authority, A, Transport::Tcp);
    let old_off = authority.lock().unwrap();
    let new_on = authority.lock().unwrap();
    assert!(authority.open_after_durable(old_off).is_err());
    assert!(authority.is_locked());
    authority.open_after_durable(new_on).unwrap();
    assert!(!authority.is_locked());
    assert!(session.publish().is_err());
    assert!(session.submit(|| ()).is_err());
    lease(&authority, A, Transport::Tcp).publish().unwrap();
}

#[test]
fn synchronous_submission_and_revoke_have_one_linearization_order() {
    let authority = Authority::new(false);
    let session = lease(&authority, A, Transport::Tcp);
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let submissions = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        let child_entered = entered.clone();
        let child_release = release.clone();
        let submissions = submissions.clone();
        let session = session.clone();
        scope.spawn(move || {
            session
                .submit(|| {
                    child_entered.wait();
                    child_release.wait();
                    submissions.fetch_add(1, Ordering::SeqCst);
                })
                .unwrap();
        });
        // This artificial blocking poll is test-only. Production polls never
        // block. It makes the serialization ordering deterministic.
        entered.wait();
        assert!(authority.gate.try_lock().is_err());
        release.wait();
    });
    authority.deny_peer(A).unwrap();
    assert!(session
        .submit(|| submissions.fetch_add(1, Ordering::SeqCst))
        .is_err());
    assert_eq!(submissions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn raw_tcp_writer_refuses_new_bytes_after_publication_then_revocation() {
    let authority = Authority::new(false);
    let lease = lease(&authority, A, Transport::Tcp);
    lease.publish().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut read, _) = listener.accept().await.unwrap();
        let mut writer = AuthorityWriter::new(stream, lease);
        writer.write_all(b"before").await.unwrap();
        authority.deny_peer(A).unwrap();
        assert!(writer.write_all(b"after").await.is_err());
        drop(writer);
        let mut bytes = Vec::new();
        read.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"before");
        authority.cleanup(None).await.unwrap();
    })
    .await
    .unwrap();
}

#[test]
fn counter_exhaustion_and_poison_fail_closed() {
    let authority = Authority::new(false);
    let session = lease(&authority, A, Transport::Tcp);
    authority.gate().unwrap().epoch = u64::MAX;
    assert!(authority.lock().is_err());
    assert!(authority.is_locked());
    assert!(authority.open_after_durable(u64::MAX).is_err());
    assert!(session.submit(|| ()).is_err());
    let copy = authority.clone();
    let _ = std::thread::spawn(move || {
        let _guard = copy.gate.lock().unwrap();
        panic!("injected poison");
    })
    .join();
    assert!(authority.is_locked());
    assert!(authority.ticket(A).is_err());
    drop(session); // Cleanup may recover poison, but never grants authority.
}
