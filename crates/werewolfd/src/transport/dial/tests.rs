use super::*;
use std::sync::{Arc, Mutex};

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

#[test]
fn addresses_interleave_by_family_from_the_first() {
    let v6a = addr("[fd46::1]:1");
    let v6b = addr("[fd46::2]:1");
    let v4a = addr("192.0.2.1:1");
    let v4b = addr("192.0.2.2:1");
    assert_eq!(
        interleave(vec![v6a, v6b, v4a, v4b]),
        vec![v6a, v4a, v6b, v4b]
    );
    assert_eq!(interleave(vec![v4a, v4b, v6a, v4a]), vec![v4a, v6a, v4b]);
    assert_eq!(interleave(vec![v4a]), vec![v4a]);
    assert!(interleave(vec![]).is_empty());
}

/// How each fake attempt behaves, by the address's last octet.
async fn fake(
    address: SocketAddr,
    log: Arc<Mutex<Vec<(u16, u128)>>>,
    start: Instant,
) -> io::Result<u16> {
    let tag = address.port();
    log.lock().unwrap().push((tag, start.elapsed().as_millis()));
    match tag {
        // Answers after 100 ms.
        1 => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(tag)
        }
        // Never answers.
        2 => std::future::pending().await,
        // Refused at once.
        3 => Err(io::ErrorKind::ConnectionRefused.into()),
        // Reached and refused after 50 ms.
        4 => {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Err(hs::rejected())
        }
        _ => unreachable!(),
    }
}

/// Real time: start times and durations within 80 ms of the expected.
fn near(actual: u128, expected: u128) -> bool {
    actual.abs_diff(expected) < 80
}

fn starts(log: &[(u16, u128)], expected: &[(u16, u128)]) {
    assert_eq!(log.len(), expected.len(), "{log:?}");
    for ((tag, at), (want_tag, want_at)) in log.iter().zip(expected) {
        assert!(
            tag == want_tag && near(*at, *want_at),
            "{log:?} vs {expected:?}"
        );
    }
}

async fn run(ports: &[u16]) -> (io::Result<u16>, Vec<(u16, u128)>, u128) {
    let addresses: Vec<_> = ports
        .iter()
        .map(|port| SocketAddr::from(([127, 0, 0, 1], *port)))
        .collect();
    let log = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();
    let result = {
        let log = log.clone();
        race(&addresses, move |address| fake(address, log.clone(), start)).await
    };
    let elapsed = start.elapsed().as_millis();
    let log = log.lock().unwrap().clone();
    (result, log, elapsed)
}

#[tokio::test]
async fn a_single_address_is_one_attempt() {
    let (result, log, elapsed) = run(&[1]).await;
    assert_eq!(result.unwrap(), 1);
    starts(&log, &[(1, 0)]);
    assert!(near(elapsed, 100), "{elapsed}");
}

#[tokio::test]
async fn a_silent_first_address_costs_the_attempt_delay() {
    let (result, log, elapsed) = run(&[2, 1]).await;
    assert_eq!(result.unwrap(), 1);
    starts(&log, &[(2, 0), (1, 250)]);
    assert!(near(elapsed, 350), "{elapsed}");
}

#[tokio::test]
async fn a_refused_first_address_starts_the_next_at_once() {
    let (result, log, elapsed) = run(&[3, 1]).await;
    assert_eq!(result.unwrap(), 1);
    starts(&log, &[(3, 0), (1, 0)]);
    assert!(near(elapsed, 100), "{elapsed}");
}

#[tokio::test]
async fn the_first_success_wins_and_later_addresses_never_start() {
    let (result, log, _) = run(&[1, 2, 2]).await;
    assert_eq!(result.unwrap(), 1);
    starts(&log, &[(1, 0)]);
}

#[tokio::test]
async fn attempts_are_staggered_while_none_answers() {
    let (result, log, _) = tokio::time::timeout(Duration::from_secs(5), run(&[2, 2, 2, 1]))
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 1);
    starts(&log, &[(2, 0), (2, 250), (2, 500), (1, 750)]);
}

#[tokio::test]
async fn when_every_address_fails_a_reached_peers_refusal_is_reported() {
    let (result, _, _) = run(&[3, 4, 3]).await;
    assert_eq!(
        result.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied,
        "the reached peer's refusal, not the dial errors"
    );
    let (result, _, _) = run(&[3, 3]).await;
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
}

#[tokio::test]
async fn the_callers_window_bounds_a_race_of_silent_addresses() {
    let raced = tokio::time::timeout(Duration::from_millis(600), run(&[2, 2])).await;
    assert!(raced.is_err());
}

#[tokio::test]
async fn names_resolve_to_every_address_and_literals_pass_through() {
    assert_eq!(
        resolve_all("192.0.2.1:9").await.unwrap(),
        vec![addr("192.0.2.1:9")]
    );
    let local = resolve_all("localhost:9").await.unwrap();
    assert!(!local.is_empty() && local.iter().all(|a| a.ip().is_loopback()));
    assert!(resolve_all("werewolf-stage46.invalid:9").await.is_err());

    let resolved = Resolved::names(vec![
        "192.0.2.1:9".into(),
        "localhost:9".into(),
        "werewolf-stage46.invalid:9".into(),
    ])
    .await;
    assert_eq!(
        resolved.addresses("192.0.2.1:9").unwrap(),
        vec![addr("192.0.2.1:9")]
    );
    assert_eq!(resolved.addresses("localhost:9").unwrap(), local);
    assert!(resolved
        .addresses("werewolf-stage46.invalid:9")
        .unwrap_err()
        .starts_with("does not resolve"));
    assert!(resolved
        .addresses("other.invalid:9")
        .unwrap_err()
        .contains("changed while the Fang was opening"));
}

#[tokio::test]
async fn tcp_tries_the_next_address_when_the_first_is_refused() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // Nothing listens on 127.0.0.2 at that port (the listener is bound to
    // 127.0.0.1 only), so the first address is refused at once.
    let addresses = vec![
        addr(&format!("127.0.0.2:{port}")),
        addr(&format!("127.0.0.1:{port}")),
    ];
    let stream = race(&addresses, TcpStream::connect).await.unwrap();
    assert_eq!(stream.peer_addr().unwrap(), addresses[1]);
}

/// Stage47 O1: a single address is awaited directly; its error comes back
/// unchanged, at once.
#[tokio::test]
async fn a_single_address_returns_its_own_error_at_once() {
    let started = Instant::now();
    let (result, log, _) = run(&[3]).await;
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
    starts(&log, &[(3, 0)]);
    assert!(started.elapsed() < Duration::from_millis(50));
    let (result, _, _) = run(&[4]).await;
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}
