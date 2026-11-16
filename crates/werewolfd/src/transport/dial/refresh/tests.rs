use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Condvar,
};
use std::time::Duration;
use tokio::time::{sleep, timeout};

fn address() -> SocketAddr {
    "127.0.0.1:9560".parse().unwrap()
}

async fn until(mut check: impl FnMut() -> bool) {
    timeout(Duration::from_secs(3), async {
        while !check() {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("test condition did not become true");
}

/// A test failure must also release blocking work, or runtime shutdown
/// would wait forever. Closures capture the state, not this release guard.
struct Blocked {
    state: Arc<(Mutex<bool>, Condvar)>,
    calls: Arc<AtomicUsize>,
}

impl Blocked {
    fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(false), Condvar::new())),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn resolver(&self, slots: Arc<Semaphore>) -> Arc<Refresh> {
        let state = self.state.clone();
        let calls = self.calls.clone();
        Arc::new(Refresh::with_lookup(
            "blocked.test:9560",
            slots,
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                let (lock, changed) = &*state;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                Ok(vec![address()])
            },
        ))
    }

    fn release(&self) {
        let (lock, changed) = &*self.state;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}

impl Drop for Blocked {
    fn drop(&mut self) {
        self.release();
    }
}

fn start(refresh: Arc<Refresh>) -> tokio::task::JoinHandle<io::Result<Arc<Vec<SocketAddr>>>> {
    tokio::spawn(async move { refresh.resolve().await })
}

fn waiters(refresh: &Refresh) -> usize {
    refresh
        .flight
        .lock()
        .unwrap()
        .as_ref()
        .map_or(0, watch::Sender::receiver_count)
}

#[tokio::test]
async fn independent_dials_refresh_answers_and_recover_from_resolution_errors() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let v6: SocketAddr = "[::1]:9560".parse().unwrap();
    let refresh = Refresh::with_lookup(
        "changes.test:9560",
        Arc::new(Semaphore::new(1)),
        move |_| match seen.fetch_add(1, Ordering::SeqCst) {
            0 => Ok(vec![address()]),
            1 => Err(io::ErrorKind::NotFound.into()),
            2 => Ok(vec![v6, v6, address()]),
            3 => Ok(vec![]),
            _ => Ok(vec![address()]),
        },
    );
    assert_eq!(*refresh.resolve().await.unwrap(), vec![address()]);
    assert_eq!(
        refresh.resolve().await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(*refresh.resolve().await.unwrap(), vec![v6, address()]);
    assert_eq!(
        refresh.resolve().await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(*refresh.resolve().await.unwrap(), vec![address()]);
    assert_eq!(calls.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn concurrent_waiters_share_only_the_in_flight_lookup() {
    let blocked = Blocked::new();
    let refresh = blocked.resolver(Arc::new(Semaphore::new(1)));
    let first = start(refresh.clone());
    until(|| blocked.calls.load(Ordering::SeqCst) == 1).await;
    let second = start(refresh.clone());
    until(|| waiters(&refresh) == 2).await;
    blocked.release();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    refresh.resolve().await.unwrap();
    assert_eq!(blocked.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancelled_waiters_do_not_release_started_work_or_start_a_duplicate() {
    let slots = Arc::new(Semaphore::new(1));
    let blocked = Blocked::new();
    let refresh = blocked.resolver(slots.clone());
    let first = start(refresh.clone());
    until(|| blocked.calls.load(Ordering::SeqCst) == 1).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert_eq!(slots.available_permits(), 0);
    let second = start(refresh.clone());
    until(|| waiters(&refresh) == 1).await;
    assert_eq!(blocked.calls.load(Ordering::SeqCst), 1);
    blocked.release();
    assert_eq!(*second.await.unwrap().unwrap(), vec![address()]);
    until(|| slots.available_permits() == 1).await;
}

#[tokio::test]
async fn dropping_the_peer_keeps_capacity_owned_by_its_blocking_lookup() {
    let slots = Arc::new(Semaphore::new(1));
    let blocked = Blocked::new();
    let refresh = blocked.resolver(slots.clone());
    let first = start(refresh.clone());
    until(|| blocked.calls.load(Ordering::SeqCst) == 1).await;
    first.abort();
    let _ = first.await;
    drop(refresh);
    assert_eq!(slots.available_permits(), 0);
    let other = blocked.resolver(slots.clone());
    assert!(timeout(Duration::from_millis(30), other.resolve())
        .await
        .is_err());
    assert_eq!(blocked.calls.load(Ordering::SeqCst), 1);
    blocked.release();
    until(|| slots.available_permits() == 1).await;
    assert_eq!(*other.resolve().await.unwrap(), vec![address()]);
    assert_eq!(blocked.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn all_cancelled_capacity_waiters_submit_no_work_and_can_try_again() {
    let slots = Arc::new(Semaphore::new(0));
    let blocked = Blocked::new();
    let refresh = blocked.resolver(slots.clone());
    let first = start(refresh.clone());
    until(|| waiters(&refresh) == 1).await;
    first.abort();
    let _ = first.await;
    until(|| refresh.flight.lock().unwrap().is_none()).await;
    assert_eq!(blocked.calls.load(Ordering::SeqCst), 0);
    slots.add_permits(1);
    blocked.release();
    refresh.resolve().await.unwrap();
    assert_eq!(blocked.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sixteen_jobs_stay_bounded_after_all_peers_and_waiters_are_dropped() {
    let slots = Arc::new(Semaphore::new(MAX_REFRESH_JOBS));
    let blocked = Blocked::new();
    let mut tasks = Vec::new();
    for _ in 0..MAX_REFRESH_JOBS + 1 {
        tasks.push(start(blocked.resolver(slots.clone())));
    }
    until(|| blocked.calls.load(Ordering::SeqCst) == MAX_REFRESH_JOBS).await;
    assert_eq!(slots.available_permits(), 0);
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        let _ = task.await;
    }
    assert_eq!(slots.available_permits(), 0);
    blocked.release();
    until(|| slots.available_permits() == MAX_REFRESH_JOBS).await;
    // Let the queued coordinator observe that all of its receivers left.
    tokio::task::yield_now().await;
    assert_eq!(blocked.calls.load(Ordering::SeqCst), MAX_REFRESH_JOBS);
}

#[tokio::test]
async fn a_panicking_resolver_returns_capacity_and_does_not_poison_future_dials() {
    let slots = Arc::new(Semaphore::new(1));
    let calls = Arc::new(AtomicUsize::new(0));
    let refresh = Refresh::with_lookup("panic.test:9560", slots.clone(), move |_| {
        assert_ne!(
            calls.fetch_add(1, Ordering::SeqCst),
            0,
            "fake resolver panic"
        );
        Ok(vec![address()])
    });
    assert_eq!(
        refresh.resolve().await.unwrap_err().kind(),
        io::ErrorKind::Other
    );
    assert_eq!(slots.available_permits(), 1);
    assert_eq!(*refresh.resolve().await.unwrap(), vec![address()]);
}

#[tokio::test]
async fn a_completed_answer_is_released_after_its_last_waiter() {
    let refresh = Refresh::with_lookup("once.test:9560", Arc::new(Semaphore::new(1)), |_| {
        Ok(vec![address()])
    });
    let answer = refresh.resolve().await.unwrap();
    let weak = Arc::downgrade(&answer);
    drop(answer);
    until(|| weak.upgrade().is_none()).await;
    assert!(refresh.flight.lock().unwrap().is_none());
}

#[tokio::test]
async fn a_closed_capacity_pool_returns_an_error_instead_of_retrying_forever() {
    let slots = Arc::new(Semaphore::new(0));
    slots.close();
    let refresh = Refresh::with_lookup("closed.test:9560", slots, |_| {
        panic!("closed capacity must not submit work")
    });
    assert!(timeout(Duration::from_secs(1), refresh.resolve())
        .await
        .unwrap()
        .is_err());
}
