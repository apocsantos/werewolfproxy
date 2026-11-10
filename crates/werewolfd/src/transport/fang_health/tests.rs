use super::*;
use crate::handshake as hs;

#[test]
fn setup_failures_are_classed_dial_or_handshake() {
    use io::ErrorKind::*;
    for kind in [
        ConnectionRefused,
        HostUnreachable,
        NetworkUnreachable,
        AddrNotAvailable,
        TimedOut,
    ] {
        assert_eq!(
            FailureClass::of_setup(&io::Error::from(kind)),
            FailureClass::Dial,
            "{kind:?}"
        );
    }
    assert_eq!(FailureClass::of_setup(&hs::no_answer()), FailureClass::Dial);
    assert_eq!(
        FailureClass::of_setup(&hs::local_dial_failed("invalid remote address")),
        FailureClass::Dial
    );
    // A peer that was reached and refused, whatever its reason.
    for error in [
        hs::rejected(),
        io::Error::from(ConnectionReset),
        io::Error::from(UnexpectedEof),
        io::Error::from(InvalidData),
    ] {
        assert_eq!(FailureClass::of_setup(&error), FailureClass::Handshake);
    }
}

#[tokio::test]
async fn failures_are_counted_and_logged_at_a_bounded_rate_per_class() {
    let health = FangHealth::new("fang_test", "wolf-b");
    assert_eq!(health.sessions_failed(), 0);
    assert!(health.last_failure().is_null());

    for _ in 0..1000 {
        health.setup_failed(&hs::no_answer());
    }
    // One line, then 999 suppressed within the interval.
    assert_eq!(lock(&health.logs).setup[0].suppressed(), 999);
    // Another class has its own limiter: its first event is logged.
    health.setup_failed(&hs::rejected());
    assert_eq!(lock(&health.logs).setup[1].suppressed(), 0);
    health.record(FailureClass::Target, &"127.0.0.1:9: refused");
    assert_eq!(lock(&health.logs).setup[2].suppressed(), 0);

    assert_eq!(health.sessions_failed(), 1002);
    assert_eq!(health.last_failure()["class"], "target");
    assert_eq!(health.last_failure()["seconds_ago"], 0);
}

#[test]
fn established_session_errors_are_logged_but_not_counted() {
    let health = FangHealth::new("fang_test", "");
    health.pipe_error(&"client 127.0.0.1:5: reset");
    health.pipe_error(&"client 127.0.0.1:5: reset");
    assert_eq!(lock(&health.logs).pipe.suppressed(), 1);
    assert_eq!(health.sessions_failed(), 0);
    assert!(health.last_failure().is_null());
}
