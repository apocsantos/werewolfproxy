use super::*;

fn identity() -> ExpectedPeerIdentity {
    let pelt = werewolf_core::pelt::generate_identity();
    ExpectedPeerIdentity {
        fingerprint: pelt.fingerprint.clone(),
        public_key_b64: Some(pelt.public_key_b64.clone()),
    }
}

fn fill(member: &Membership, plan: usize, count: usize) -> Vec<Reservation> {
    (0..count).map(|_| member.reserve(plan).unwrap()).collect()
}

#[test]
fn one_fang_gets_64_and_denial_is_a_local_dial_failure() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let member = coordinator.register(vec![vec![&peer]]).unwrap();
    let slots = fill(&member, 0, 64);
    let error = member.reserve(0).err().unwrap();
    assert!(hs::no_connection_made(&error));
    assert!(error.to_string().contains("local peer-session share"));
    drop(slots);
    assert!(member.reserve(0).is_ok());
    drop(member);
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn three_fangs_get_stable_remainders_and_unused_shares_are_not_lent() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let a = coordinator.register(vec![vec![&peer]]).unwrap();
    let b = coordinator.register(vec![vec![&peer]]).unwrap();
    let c = coordinator.register(vec![vec![&peer]]).unwrap();
    let a_slots = fill(&a, 0, 22);
    assert!(a.reserve(0).is_err()); // Other two shares are still unused.
    let b_slots = fill(&b, 0, 21);
    let c_slots = fill(&c, 0, 21);
    assert!(b.reserve(0).is_err() && c.reserve(0).is_err());
    drop(a_slots);
    drop(a);
    let b_more = fill(&b, 0, 11);
    let c_more = fill(&c, 0, 11);
    assert!(b.reserve(0).is_err() && c.reserve(0).is_err());
    drop((b_slots, c_slots, b_more, c_more, b, c));
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn later_registration_grandfathers_sessions_but_cannot_overcommit_total() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let old = coordinator.register(vec![vec![&peer]]).unwrap();
    let mut old_slots = fill(&old, 0, 64);
    let new = coordinator.register(vec![vec![&peer]]).unwrap();
    assert!(old.reserve(0).is_err() && new.reserve(0).is_err());
    assert_eq!(old_slots.len(), 64); // No preemption.
    old_slots.pop();
    let new_slot = new.reserve(0).unwrap();
    assert!(old.reserve(0).is_err() && new.reserve(0).is_err());
    old_slots.truncate(31);
    assert!(old.reserve(0).is_ok());
    let new_slots = fill(&new, 0, 31);
    assert!(new.reserve(0).is_err());
    drop((old_slots, new_slot, new_slots, old, new));
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn cloned_identity_groups_paths_and_repeated_receiver_is_counted_once() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let same = peer.clone();
    let first = coordinator
        .register(vec![vec![&peer, &same], vec![&same]])
        .unwrap();
    let second = coordinator.register(vec![vec![&same]]).unwrap();
    let mut first_slots = fill(&first, 0, 31);
    first_slots.push(first.reserve(1).unwrap());
    assert!(first.reserve(0).is_err() && first.reserve(1).is_err());
    let second_slots = fill(&second, 0, 32);
    assert_eq!(coordinator.0.lock().unwrap().peers.len(), 1);
    drop((first_slots, second_slots, first, second));
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn multi_receiver_admission_is_atomic_and_provider_paths_release_separately() {
    let coordinator = Arc::new(Coordinator::default());
    let a = identity();
    let b = identity();
    let member = coordinator.register(vec![vec![&a, &b], vec![&b]]).unwrap();
    let mut b_slots = fill(&member, 1, 64);
    assert!(member.reserve(0).is_err());
    let a_key = hs::binary::<32>(a.public_key_b64.as_deref().unwrap()).unwrap();
    assert_eq!(coordinator.0.lock().unwrap().peers[&a_key].used, 0);
    b_slots.pop();
    let routed = member.reserve(0).unwrap();
    assert!(member.reserve(1).is_err());
    drop(routed); // Failed/finished routed attempt releases both receivers.
    assert_eq!(coordinator.0.lock().unwrap().peers[&a_key].used, 0);
    assert!(member.reserve(1).is_ok());
    assert!(member.reserve(2).is_err());
    drop((b_slots, member));
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn sixty_fifth_registration_rolls_back_every_receiver() {
    let coordinator = Arc::new(Coordinator::default());
    let a = identity();
    let b = identity();
    let members: Vec<_> = (0..64)
        .map(|_| coordinator.register(vec![vec![&a]]).unwrap())
        .collect();
    let error = coordinator
        .register(vec![vec![&b], vec![&a]])
        .err()
        .unwrap();
    assert_eq!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<RegistrationError>()
            .unwrap()
            .code(),
        "FANG_PEER_CAPACITY"
    );
    assert_eq!(coordinator.0.lock().unwrap().peers.len(), 1);
    let slots: Vec<_> = members.iter().map(|m| m.reserve(0).unwrap()).collect();
    assert!(members.iter().all(|m| m.reserve(0).is_err()));
    drop((slots, members));
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn bad_identity_or_empty_demand_never_registers_partial_membership() {
    let coordinator = Arc::new(Coordinator::default());
    let good = identity();
    let mut bad = good.clone();
    bad.fingerprint = identity().fingerprint;
    assert!(coordinator.register(vec![vec![&good], vec![&bad]]).is_err());
    bad.public_key_b64 = None;
    assert!(coordinator.register(vec![vec![&bad]]).is_err());
    assert!(coordinator.register(vec![]).is_err());
    assert!(coordinator.register(vec![vec![]]).is_err());
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn reservation_retains_membership_until_cleanup_and_repeated_drop_leaves_no_keys() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    for _ in 0..100 {
        let member = coordinator.register(vec![vec![&peer]]).unwrap();
        let reservation = member.reserve(0).unwrap();
        drop(member);
        assert_eq!(coordinator.0.lock().unwrap().peers.len(), 1);
        drop(reservation);
        assert!(coordinator.0.lock().unwrap().peers.is_empty());
    }
}

#[tokio::test]
async fn aborting_a_session_returns_capacity_and_its_membership() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let member = coordinator.register(vec![vec![&peer]]).unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let child = tokio::spawn(async move {
        let _reservation = member.reserve(0).unwrap();
        let _ = started.send(());
        std::future::pending::<()>().await;
    });
    ready.await.unwrap();
    assert_eq!(coordinator.0.lock().unwrap().peers.len(), 1);
    child.abort();
    assert!(child.await.unwrap_err().is_cancelled());
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn independent_coordinators_and_receivers_do_not_share_capacity() {
    let first = Arc::new(Coordinator::default());
    let second = Arc::new(Coordinator::default());
    let a = identity();
    let b = identity();
    let one = first.register(vec![vec![&a]]).unwrap();
    let two = first.register(vec![vec![&b]]).unwrap();
    let other_process = second.register(vec![vec![&a]]).unwrap();
    let slots = fill(&one, 0, 64);
    let other_slots = fill(&two, 0, 64);
    let process_slots = fill(&other_process, 0, 64);
    assert!(one.reserve(0).is_err());
    drop((slots, other_slots, process_slots, one, two, other_process));
    assert!(first.0.lock().unwrap().peers.is_empty());
    assert!(second.0.lock().unwrap().peers.is_empty());
}

#[test]
fn concurrent_attempts_cannot_exceed_either_share_or_total() {
    let coordinator = Arc::new(Coordinator::default());
    let peer = identity();
    let members: Vec<_> = (0..2)
        .map(|_| coordinator.register(vec![vec![&peer]]).unwrap())
        .collect();
    let barrier = Arc::new(std::sync::Barrier::new(17));
    let (send, recv) = std::sync::mpsc::channel();
    let workers: Vec<_> = (0..16)
        .map(|i| {
            let member = members[i % 2].clone();
            let barrier = barrier.clone();
            let send = send.clone();
            std::thread::spawn(move || {
                let slots: Vec<_> = (0..8).filter_map(|_| member.reserve(0).ok()).collect();
                send.send((i % 2, slots.len())).unwrap();
                barrier.wait();
                barrier.wait();
                drop(slots);
            })
        })
        .collect();
    barrier.wait();
    let mut counts = [0, 0];
    for _ in 0..16 {
        let (member, count) = recv.recv().unwrap();
        counts[member] += count;
    }
    assert_eq!(counts, [32, 32]);
    assert_eq!(
        coordinator
            .0
            .lock()
            .unwrap()
            .peers
            .values()
            .next()
            .unwrap()
            .used,
        64
    );
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    drop(members);
    assert!(coordinator.0.lock().unwrap().peers.is_empty());
}

#[test]
fn freed_capacity_does_not_make_an_idle_victim_useful() {
    for limit in [32, 64] {
        let coordinator = Arc::new(Coordinator::default());
        let a = identity();
        let b = identity();
        let member = coordinator.register(vec![vec![&a], vec![&b]]).unwrap();
        let _sibling = (limit == 32).then(|| coordinator.register(vec![vec![&a]]).unwrap());
        let mut held_a = fill(&member, 0, limit);
        let _held_b = member.reserve(1).unwrap();
        let idle_a = Demand {
            membership: member.clone(),
            plan: 0,
        };
        let idle_b = Demand {
            membership: member.clone(),
            plan: 1,
        };
        assert!(member.reserve(0).is_err());
        assert!(idle_a.permits_replacement(&member, 0));
        assert!(!idle_b.permits_replacement(&member, 0));

        // Another connection releases A between the initial denial and
        // candidate selection. At that snapshot the attempt already fits:
        // neither its own A session nor an unrelated idle B is useful.
        held_a.pop();
        assert!(member.reserve(0).is_ok());
        assert!(
            !idle_b.permits_replacement(&member, 0),
            "already-free capacity must not select an unrelated provider victim"
        );
        assert!(
            !idle_a.permits_replacement(&member, 0),
            "already-free capacity must not select even a same-provider victim"
        );
    }
}
