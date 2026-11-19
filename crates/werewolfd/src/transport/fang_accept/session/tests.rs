use super::*;
use crate::{
    policy::ExpectedPeerIdentity,
    transport::peer_quota::{Coordinator, Membership, Reservation},
};

fn identity() -> ExpectedPeerIdentity {
    let pelt = werewolf_core::pelt::generate_identity();
    ExpectedPeerIdentity {
        fingerprint: pelt.fingerprint.clone(),
        public_key_b64: Some(pelt.public_key_b64.clone()),
    }
}

async fn acceptor(limit: usize, idle: Duration, source: bool) -> FangAcceptor {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    FangAcceptor::with_limits(
        listener,
        "peer-reclaim-test",
        Arc::new(Semaphore::new(limit)),
        Arc::new(Semaphore::new(1024)),
        Policy {
            idle_evict_after: idle,
            source_share: 2,
            exempt_loopback: !source,
        },
    )
}

async fn slot(acceptor: &mut FangAcceptor) -> FangSlot {
    acceptor
        .admit("127.0.0.1:1234".parse().unwrap())
        .await
        .unwrap()
}

fn fill(member: &Membership, plan: usize, count: usize) -> Vec<Reservation> {
    (0..count).map(|_| member.reserve(plan).unwrap()).collect()
}

async fn hold(slot: &FangSlot, member: &Membership, plan: usize, committed: bool) -> Reservation {
    scope(slot.session.clone(), async {
        let reservation = member.reserve(plan).unwrap();
        if committed {
            commit_current();
        }
        reservation
    })
    .await
}

async fn serving(slot: FangSlot, member: Membership, plan: usize) -> tokio::task::JoinHandle<bool> {
    let (send, ready) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        slot.serve(async {
            let _reservation = member.reserve(plan).unwrap();
            commit_current();
            send.send(()).unwrap();
            std::future::pending::<()>().await;
        })
        .await
        .is_none()
    });
    ready.await.unwrap();
    task
}

#[tokio::test]
async fn idle_committed_session_releases_real_quota_for_one_newcomer() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let member = coordinator.register(vec![vec![&peer]]).unwrap();
    let _sibling = coordinator.register(vec![vec![&peer]]).unwrap();
    let _held = fill(&member, 0, 31);
    let mut acceptor = acceptor(64, Duration::ZERO, false).await;
    let old = serving(slot(&mut acceptor).await, member.clone(), 0).await;
    let newcomer = slot(&mut acceptor).await;
    let started = Instant::now();
    let replacement = scope(newcomer.session.clone(), async {
        let replacement = member
            .reserve_reclaim(0, started + Duration::from_secs(5))
            .await
            .unwrap();
        // The wait was charged to the first dial and consumed only once.
        assert!(dial_deadline(Duration::from_secs(5)) <= started + Duration::from_secs(5));
        assert!(dial_deadline(Duration::from_secs(5)) > started + Duration::from_secs(5));
        replacement
    })
    .await;
    assert!(old.await.unwrap());
    assert!(member.reserve(0).is_err());
    drop(replacement);
    assert!(member.reserve(0).is_ok());
}

#[tokio::test]
async fn setup_work_and_recent_payload_are_not_peer_victims() {
    for committed in [false, true] {
        let coordinator = Arc::new(Coordinator::default());
        let peer = identity();
        let member = coordinator.register(vec![vec![&peer]]).unwrap();
        let _other = coordinator.register(vec![vec![&peer]]).unwrap();
        let _held = fill(&member, 0, 31);
        let idle = if committed {
            Duration::from_secs(60)
        } else {
            Duration::ZERO
        };
        let mut acceptor = acceptor(64, idle, false).await;
        let old = slot(&mut acceptor).await;
        let _reservation = hold(&old, &member, 0, committed).await;
        old.activity.touch();
        let newcomer = slot(&mut acceptor).await;
        let denied = scope(
            newcomer.session.clone(),
            member.reserve_reclaim(0, Instant::now() + Duration::from_secs(5)),
        )
        .await;
        assert!(denied.is_err());
        assert!(!acceptor.registry.lock().unwrap().entries[&old.id].evicting);
        assert!(!newcomer.session.eviction_used.load(Ordering::Relaxed));
    }
}

#[tokio::test]
async fn actual_plan_not_unused_alternatives_selects_a_victim() {
    let coordinator = Arc::new(Coordinator::default());
    let a = identity();
    let b = identity();
    let member = coordinator.register(vec![vec![&a], vec![&b]]).unwrap();
    let _full_a = fill(&member, 0, 64);
    let mut acceptor = acceptor(64, Duration::ZERO, false).await;
    let old_b = slot(&mut acceptor).await;
    let _reservation_b = hold(&old_b, &member, 1, true).await;
    let newcomer = slot(&mut acceptor).await;
    assert!(scope(
        newcomer.session.clone(),
        member.reserve_reclaim(0, Instant::now() + Duration::from_secs(5))
    )
    .await
    .is_err());
    assert!(!acceptor.registry.lock().unwrap().entries[&old_b.id].evicting);
    // A distinct provider remains immediately usable despite A's denial.
    let fallback = scope(
        newcomer.session.clone(),
        member.reserve_reclaim(1, Instant::now() + Duration::from_secs(5)),
    )
    .await
    .unwrap();
    assert!(!newcomer.session.eviction_used.load(Ordering::Relaxed));
    drop(fallback);
}

#[tokio::test]
async fn one_routed_victim_must_satisfy_both_receiver_constraints() {
    for routed_victim in [false, true] {
        let coordinator = Arc::new(Coordinator::default());
        let a = identity();
        let b = identity();
        let member = coordinator
            .register(vec![vec![&a], vec![&b], vec![&a, &b]])
            .unwrap();
        let _other = coordinator.register(vec![vec![&a, &b]]).unwrap();
        let _a = fill(&member, 0, 31);
        let _b = fill(&member, 1, if routed_victim { 31 } else { 32 });
        let mut acceptor = acceptor(64, Duration::ZERO, false).await;
        let victim = slot(&mut acceptor).await;
        let newcomer = slot(&mut acceptor).await;
        if routed_victim {
            let old = serving(victim, member.clone(), 2).await;
            let _replacement = scope(
                newcomer.session.clone(),
                member.reserve_reclaim(2, Instant::now() + Duration::from_secs(5)),
            )
            .await
            .unwrap();
            assert!(old.await.unwrap());
            assert!(member.reserve(0).is_err() && member.reserve(1).is_err());
        } else {
            let _old = hold(&victim, &member, 0, true).await;
            assert!(scope(
                newcomer.session.clone(),
                member.reserve_reclaim(2, Instant::now() + Duration::from_secs(5))
            )
            .await
            .is_err());
            assert!(!acceptor.registry.lock().unwrap().entries[&victim.id].evicting);
        }
    }
}

#[tokio::test]
async fn grandfathered_over_share_is_not_forced_down_by_idle_evictions() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let member = coordinator.register(vec![vec![&peer]]).unwrap();
    let _held = fill(&member, 0, 63);
    let mut acceptor = acceptor(64, Duration::ZERO, false).await;
    let old = slot(&mut acceptor).await;
    let _reservation = hold(&old, &member, 0, true).await;
    let _late = coordinator.register(vec![vec![&peer]]).unwrap();
    let newcomer = slot(&mut acceptor).await;
    assert!(scope(
        newcomer.session.clone(),
        member.reserve_reclaim(0, Instant::now() + Duration::from_secs(5))
    )
    .await
    .is_err());
    assert!(!acceptor.registry.lock().unwrap().entries[&old.id].evicting);
}

#[tokio::test]
async fn deadline_and_unavailable_plan_do_not_select_idle_sessions() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let member = coordinator.register(vec![vec![&peer]]).unwrap();
    let _held = fill(&member, 0, 63);
    let mut acceptor = acceptor(64, Duration::ZERO, false).await;
    let old = slot(&mut acceptor).await;
    let _reservation = hold(&old, &member, 0, true).await;
    let newcomer = slot(&mut acceptor).await;
    for (plan, deadline) in [
        (0, Instant::now()),
        (99, Instant::now() + Duration::from_secs(5)),
    ] {
        assert!(scope(
            newcomer.session.clone(),
            member.reserve_reclaim(plan, deadline)
        )
        .await
        .is_err());
        assert!(!acceptor.registry.lock().unwrap().entries[&old.id].evicting);
    }
}

#[tokio::test]
async fn slow_cleanup_keeps_capacity_counted_and_one_victim_budget() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let member = coordinator.register(vec![vec![&peer]]).unwrap();
    let _other = coordinator.register(vec![vec![&peer]]).unwrap();
    let _held = fill(&member, 0, 30);
    let mut acceptor = acceptor(64, Duration::ZERO, false).await;
    let first = slot(&mut acceptor).await;
    let second = slot(&mut acceptor).await;
    let first_reservation = hold(&first, &member, 0, true).await;
    let _second_reservation = hold(&second, &member, 0, true).await;
    // Identical timestamps: the smaller stable session id wins.
    first.activity.0.store(0, Ordering::Relaxed);
    second.activity.0.store(0, Ordering::Relaxed);
    let newcomer = slot(&mut acceptor).await;
    let before = Instant::now();
    assert!(scope(
        newcomer.session.clone(),
        member.reserve_reclaim(0, before + Duration::from_millis(30))
    )
    .await
    .is_err());
    assert!(before.elapsed() < Duration::from_secs(1));
    assert!(
        member.reserve(0).is_err(),
        "notification must not credit a still-owned reservation"
    );
    assert!(acceptor.registry.lock().unwrap().entries[&first.id].evicting);
    assert!(!acceptor.registry.lock().unwrap().entries[&second.id].evicting);
    // A subsequent provider attempt on this newcomer gets no second victim.
    assert!(scope(
        newcomer.session.clone(),
        member.reserve_reclaim(0, Instant::now() + Duration::from_secs(5))
    )
    .await
    .is_err());
    assert!(!acceptor.registry.lock().unwrap().entries[&second.id].evicting);
    drop(first_reservation);
    assert!(member.reserve(0).is_ok());
}

#[tokio::test]
async fn cancellation_and_busy_gate_never_select_a_second_victim() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let member = coordinator.register(vec![vec![&peer]]).unwrap();
    let _other = coordinator.register(vec![vec![&peer]]).unwrap();
    let _held = fill(&member, 0, 31);
    let mut acceptor = acceptor(64, Duration::ZERO, false).await;
    let old = slot(&mut acceptor).await;
    let reservation = hold(&old, &member, 0, true).await;
    let first = slot(&mut acceptor).await;
    let first_session = first.session.clone();
    let request_member = member.clone();
    let task = tokio::spawn(async move {
        scope(
            first_session,
            request_member.reserve_reclaim(0, Instant::now() + Duration::from_secs(5)),
        )
        .await
        .is_ok()
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while acceptor.peer_reclaim.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let second = slot(&mut acceptor).await;
    assert!(scope(
        second.session.clone(),
        member.reserve_reclaim(0, Instant::now() + Duration::from_secs(5))
    )
    .await
    .is_err());
    assert!(!second.session.eviction_used.load(Ordering::Relaxed));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(acceptor.peer_reclaim.available_permits(), 1);
    assert!(member.reserve(0).is_err());
    drop(reservation);
    assert!(member.reserve(0).is_ok());
}

#[tokio::test]
async fn local_or_source_eviction_already_consumed_the_newcomers_budget() {
    for source in [false, true] {
        let coordinator = Arc::new(Coordinator::default());
        let a = identity();
        let b = identity();
        let member = coordinator.register(vec![vec![&a], vec![&b]]).unwrap();
        let _sibling = coordinator.register(vec![vec![&a, &b]]).unwrap();
        let _a = fill(&member, 0, 31);
        let _b = fill(&member, 1, 31);
        let mut acceptor = acceptor(if source { 64 } else { 2 }, Duration::ZERO, source).await;
        let old_a_slot = slot(&mut acceptor).await;
        let old_a = serving(old_a_slot, member.clone(), 0).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let old_b = serving(slot(&mut acceptor).await, member.clone(), 1).await;
        let newcomer = slot(&mut acceptor).await;
        assert!(old_a.await.unwrap());
        assert!(newcomer.session.eviction_used.load(Ordering::Relaxed));
        assert!(scope(
            newcomer.session.clone(),
            member.reserve_reclaim(1, Instant::now() + Duration::from_secs(5))
        )
        .await
        .is_err());
        assert!(!old_b.is_finished());
        old_b.abort();
        assert!(old_b.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn a_distinct_fangs_membership_is_never_an_idle_candidate() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let one = coordinator.register(vec![vec![&peer]]).unwrap();
    let two = coordinator.register(vec![vec![&peer]]).unwrap();
    let _one_full = fill(&one, 0, 32);
    let mut acceptor = acceptor(64, Duration::ZERO, false).await;
    let other = slot(&mut acceptor).await;
    let _other = hold(&other, &two, 0, true).await;
    let newcomer = slot(&mut acceptor).await;
    assert!(scope(
        newcomer.session.clone(),
        one.reserve_reclaim(0, Instant::now() + Duration::from_secs(5))
    )
    .await
    .is_err());
    assert!(!acceptor.registry.lock().unwrap().entries[&other.id].evicting);
}

#[tokio::test]
async fn membership_change_during_cleanup_rechecks_actual_shares() {
    for grows in [false, true] {
        let coordinator = Arc::new(Coordinator::default());
        let peer = identity();
        let member = coordinator.register(vec![vec![&peer]]).unwrap();
        let sibling = coordinator.register(vec![vec![&peer]]).unwrap();
        let _held = fill(&member, 0, 31);
        let mut acceptor = acceptor(64, Duration::ZERO, false).await;
        let old = slot(&mut acceptor).await;
        let reservation = hold(&old, &member, 0, true).await;
        let newcomer = slot(&mut acceptor).await;
        let context = newcomer.session.clone();
        let requested = member.clone();
        let task = tokio::spawn(async move {
            scope(
                context,
                requested.reserve_reclaim(0, Instant::now() + Duration::from_secs(5)),
            )
            .await
            .is_ok()
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while acceptor.peer_reclaim.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let late = if grows {
            Some(coordinator.register(vec![vec![&peer]]).unwrap())
        } else {
            None
        };
        if !grows {
            drop(sibling);
        }
        drop(reservation);
        assert_eq!(task.await.unwrap(), !grows);
        assert_eq!(acceptor.peer_reclaim.available_permits(), 1);
        drop(late);
    }
}
