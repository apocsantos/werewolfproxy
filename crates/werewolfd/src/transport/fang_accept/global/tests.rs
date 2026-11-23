use super::*;
use tokio::time::{sleep, timeout};

fn address() -> SocketAddr {
    "127.0.0.1:1234".parse().unwrap()
}

async fn acceptor(global: Arc<Semaphore>, idle: Duration) -> FangAcceptor {
    FangAcceptor::with_limits(
        TcpListener::bind("127.0.0.1:0").await.unwrap(),
        "global-reclaim-test",
        Arc::new(Semaphore::new(64)),
        global,
        Policy {
            idle_evict_after: idle,
            ..Policy::default()
        },
    )
}

async fn held(acceptor: &mut FangAcceptor, established: bool) -> FangSlot {
    let slot = acceptor
        .admit(ClientIdentity::Tcp(address()))
        .await
        .unwrap();
    if established {
        session::scope(slot.session.clone(), async { plain_connected_current() }).await;
    }
    slot
}

async fn running(acceptor: &mut FangAcceptor) -> tokio::task::JoinHandle<bool> {
    let slot = held(acceptor, false).await;
    let (send, ready) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        slot.serve(async {
            plain_connected_current();
            send.send(()).unwrap();
            std::future::pending::<()>().await;
        })
        .await
        .is_none()
    });
    ready.await.unwrap();
    task
}

async fn marked(registry: &Arc<Mutex<Registry>>, id: u64) {
    timeout(Duration::from_secs(1), async {
        loop {
            if registry.lock().unwrap().entries[&id].evicting {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn global_setup_and_recent_sessions_are_protected() {
    for established in [false, true] {
        let global = Arc::new(Semaphore::new(1));
        let mut a = acceptor(
            global,
            if established {
                Duration::from_secs(3600)
            } else {
                Duration::ZERO
            },
        )
        .await;
        let old = held(&mut a, established).await;
        assert!(a.admit(ClientIdentity::Tcp(address())).await.is_none());
        assert!(!a.registry.lock().unwrap().entries[&old.id].evicting);
    }
}

#[tokio::test]
async fn absent_peer_reservation_does_not_establish_plain_tcp() {
    let global = Arc::new(Semaphore::new(1));
    let mut a = acceptor(global.clone(), Duration::ZERO).await;
    let old = held(&mut a, false).await;
    session::scope(old.session.clone(), async { commit_current() }).await;
    assert!(!a.registry.lock().unwrap().entries[&old.id].established);
    assert!(a.admit(ClientIdentity::Tcp(address())).await.is_none());
    session::scope(old.session.clone(), async { plain_connected_current() }).await;
    let released = a
        .registry
        .lock()
        .unwrap()
        .global_victim(Duration::ZERO, Instant::now() + EVICT_WAIT, || true)
        .unwrap()
        .1;
    assert_eq!(global.available_permits(), 0);
    drop(old);
    timeout(Duration::from_secs(1), released.notified())
        .await
        .unwrap();
    assert_eq!(global.available_permits(), 1);
}

#[tokio::test]
async fn empty_fang_never_steals_a_siblings_global_slot() {
    let global = Arc::new(Semaphore::new(1));
    let mut a = acceptor(global.clone(), Duration::ZERO).await;
    let mut b = acceptor(global.clone(), Duration::ZERO).await;
    let old = running(&mut a).await;
    assert!(b.admit(ClientIdentity::Tcp(address())).await.is_none());
    assert!(!old.is_finished());
    old.abort();
    assert!(old.await.unwrap_err().is_cancelled());
    assert_eq!(global.available_permits(), 1);
}

#[tokio::test]
async fn busy_peer_gate_does_not_start_global_reclamation() {
    let global = Arc::new(Semaphore::new(1));
    let mut a = acceptor(global, Duration::ZERO).await;
    let old = running(&mut a).await;
    let gate = a.peer_reclaim.clone().acquire_owned().await.unwrap();
    assert!(a.admit(ClientIdentity::Tcp(address())).await.is_none());
    assert!(!old.is_finished());
    assert!(!a.registry.lock().unwrap().entries[&0].evicting);
    drop(gate);
    let new = a.admit(ClientIdentity::Tcp(address())).await.unwrap();
    assert!(old.await.unwrap());
    drop(new);
}

#[tokio::test]
async fn prior_local_or_source_victim_prevents_another_global_victim() {
    let global = Arc::new(Semaphore::new(1));
    let mut a = acceptor(global, Duration::ZERO).await;
    let old = held(&mut a, true).await;
    assert!(a.acquire_global(true).await.is_none());
    assert!(!a.registry.lock().unwrap().entries[&old.id].evicting);
}

#[tokio::test]
async fn slow_global_cleanup_never_credits_quota_or_selects_two_victims() {
    let global = Arc::new(Semaphore::new(2));
    let mut a = acceptor(global.clone(), Duration::ZERO).await;
    let first = held(&mut a, true).await;
    let second = held(&mut a, true).await;
    first.activity.0.store(0, Ordering::Relaxed);
    second.activity.0.store(0, Ordering::Relaxed);
    assert!(timeout(
        Duration::from_millis(30),
        a.admit(ClientIdentity::Tcp(address()))
    )
    .await
    .is_err());
    assert_eq!(global.available_permits(), 0);
    let registry = a.registry.lock().unwrap();
    assert!(registry.entries[&first.id].evicting);
    assert!(!registry.entries[&second.id].evicting);
    drop(registry);
    assert_eq!(a.peer_reclaim.available_permits(), 1);
    drop(first);
    assert_eq!(global.available_permits(), 1);
    drop(second);
    assert_eq!(global.available_permits(), 2);
}

#[tokio::test]
async fn global_pressure_and_deadline_are_rechecked_before_marking() {
    use std::sync::atomic::AtomicUsize;
    let global = Arc::new(Semaphore::new(1));
    let mut a = acceptor(global, Duration::ZERO).await;
    let old = held(&mut a, true).await;
    let calls = AtomicUsize::new(0);
    assert!(a
        .registry
        .lock()
        .unwrap()
        .global_victim(Duration::ZERO, Instant::now() + EVICT_WAIT, || calls
            .fetch_add(1, Ordering::Relaxed)
            == 0,)
        .is_none());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(a
        .registry
        .lock()
        .unwrap()
        .global_victim(Duration::ZERO, Instant::now(), || true)
        .is_none());
    assert!(!a.registry.lock().unwrap().entries[&old.id].evicting);
}

#[tokio::test]
async fn freed_or_closed_global_capacity_does_not_mark_a_victim() {
    let global = Arc::new(Semaphore::new(2));
    let mut a = acceptor(global.clone(), Duration::ZERO).await;
    let first = held(&mut a, true).await;
    let second = held(&mut a, true).await;
    assert!(global.clone().try_acquire_owned().is_err());
    drop(second);
    assert!(a
        .registry
        .lock()
        .unwrap()
        .global_victim(Duration::ZERO, Instant::now() + EVICT_WAIT, || global
            .available_permits()
            == 0,)
        .is_none());
    global.close();
    assert!(a.acquire_global(false).await.is_none());
    assert!(!a.registry.lock().unwrap().entries[&first.id].evicting);
}

#[tokio::test]
async fn global_candidate_is_longest_idle_not_first_registered() {
    let global = Arc::new(Semaphore::new(2));
    let mut a = acceptor(global, Duration::ZERO).await;
    let first = held(&mut a, true).await;
    let second = held(&mut a, true).await;
    sleep(Duration::from_millis(5)).await;
    first.activity.touch();
    second.activity.0.store(0, Ordering::Relaxed);
    a.registry
        .lock()
        .unwrap()
        .global_victim(Duration::ZERO, Instant::now() + EVICT_WAIT, || true)
        .unwrap();
    let registry = a.registry.lock().unwrap();
    assert!(!registry.entries[&first.id].evicting);
    assert!(registry.entries[&second.id].evicting);
}

#[tokio::test]
async fn global_wait_charges_first_dial_and_selection_but_not_the_tcp_retry() {
    let global = Arc::new(Semaphore::new(1));
    let mut a = acceptor(global, Duration::ZERO).await;
    let old = held(&mut a, true).await;
    let registry = a.registry.clone();
    let request = tokio::spawn(async move { a.admit(ClientIdentity::Tcp(address())).await });
    marked(&registry, old.id).await;
    sleep(Duration::from_millis(30)).await;
    let earlier = Instant::now() - Duration::from_millis(20);
    drop(old);
    let new = request.await.unwrap().unwrap();
    session::scope(new.session.clone(), async {
        Session::current().unwrap().clear_first_dial().unwrap();
        let first = dial_deadline(Duration::from_secs(5));
        assert!(first <= earlier + Duration::from_secs(5));
        assert!(selection_deadline(Duration::from_secs(15)) <= earlier + Duration::from_secs(15));
        assert!(dial_deadline(Duration::from_secs(5)) > first);
    })
    .await;
}

#[tokio::test]
async fn competitor_can_win_released_global_capacity_without_a_queue_or_second_victim() {
    let global = Arc::new(Semaphore::new(2));
    let mut a = acceptor(global.clone(), Duration::ZERO).await;
    let first = held(&mut a, true).await;
    let second = held(&mut a, true).await;
    first.activity.0.store(0, Ordering::Relaxed);
    second.activity.0.store(0, Ordering::Relaxed);
    let registry = a.registry.clone();
    let gate = a.peer_reclaim.clone();
    let request = tokio::spawn(async move { a.admit(ClientIdentity::Tcp(address())).await });
    marked(&registry, first.id).await;
    drop(first);
    let competitor = global.clone().try_acquire_owned().unwrap();
    assert!(timeout(Duration::from_millis(200), request)
        .await
        .unwrap()
        .unwrap()
        .is_none());
    assert!(!registry.lock().unwrap().entries[&second.id].evicting);
    assert_eq!(gate.available_permits(), 1);
    drop(competitor);
    drop(second);
    assert_eq!(global.available_permits(), 2);
}

#[tokio::test]
async fn cancelling_global_wait_releases_gate_without_speculative_capacity() {
    let global = Arc::new(Semaphore::new(1));
    let mut a = acceptor(global.clone(), Duration::ZERO).await;
    let old = held(&mut a, true).await;
    let registry = a.registry.clone();
    let gate = a.peer_reclaim.clone();
    let request = tokio::spawn(async move { a.admit(ClientIdentity::Tcp(address())).await });
    marked(&registry, old.id).await;
    request.abort();
    assert!(request.await.err().unwrap().is_cancelled());
    assert_eq!(gate.available_permits(), 1);
    assert_eq!(global.available_permits(), 0);
    drop(old);
    assert_eq!(global.available_permits(), 1);
}

#[tokio::test]
async fn global_cleanup_wait_is_bounded_by_the_existing_one_second_window() {
    let global = Arc::new(Semaphore::new(1));
    let mut a = acceptor(global.clone(), Duration::ZERO).await;
    let old = held(&mut a, true).await;
    let began = Instant::now();
    assert!(timeout(
        EVICT_WAIT + Duration::from_secs(1),
        a.admit(ClientIdentity::Tcp(address()))
    )
    .await
    .unwrap()
    .is_none());
    assert!(began.elapsed() >= EVICT_WAIT);
    assert_eq!(global.available_permits(), 0);
    assert_eq!(a.peer_reclaim.available_permits(), 1);
    drop(old);
    assert_eq!(global.available_permits(), 1);
}
