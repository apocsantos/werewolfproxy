//! Stage32 F1: an initiator retries a failed encrypted-TCP setup once, and
//! only when the failure came before the first byte of the receiver's
//! challenge: TCP connect, TLS at the transport level, or a receiver that
//! closed before it ever spoke. Nothing later is retried, a TLS pinning
//! failure is never retried, and there is never a second retry.
use super::authority_tests::{fixture, Task};
use super::*;
use crate::target_policy::TargetPolicy;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::timeout;
use werewolf_core::pelt::generate_identity;

/// What the fake receiver does with one accepted connection.
#[derive(Clone, Copy)]
enum Conduct {
    /// Close the TCP connection before TLS.
    DropTcp,
    /// Complete TLS, then close without sending the challenge, as the real
    /// receiver does when its TLS-accept window ends first.
    CloseBeforeChallenge,
    /// Complete TLS, send the first bytes of a challenge, then close.
    PartialChallenge,
    /// Serve the connection with the real receiver path.
    Real,
}

struct Receiver {
    address: std::net::SocketAddr,
    target: std::net::SocketAddr,
    expected: crate::policy::ExpectedPeerIdentity,
    accepted: Arc<AtomicUsize>,
    _worker: Task,
    _echo: Task,
}

/// A receiver that treats its n-th connection as `script[n]` (the last
/// entry repeats), presenting `tls_as`'s certificate, or the receiver's own.
async fn receiver(
    sender: &PeltIdentity,
    script: &[Conduct],
    tls_as: Option<&PeltIdentity>,
) -> Receiver {
    let state = fixture(&[sender]);
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = target_listener.local_addr().unwrap();
    {
        let mut state = state.lock().await;
        let TargetPolicy::Grants(grants) = &mut state.target_policy else {
            panic!("fixture grants");
        };
        grants.insert(sender.fingerprint.clone(), [target].into());
    }
    let echo = Task(tokio::spawn(async move {
        loop {
            let (mut stream, _) = target_listener.accept().await?;
            tokio::spawn(async move {
                let (mut r, mut w) = stream.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    }));
    let pelt = state.lock().await.pelt.clone().unwrap();
    let expected = crate::policy::ExpectedPeerIdentity {
        fingerprint: pelt.fingerprint.clone(),
        public_key_b64: Some(pelt.public_key_b64.clone()),
    };
    let identity = match tls_as {
        Some(other) => Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(other).unwrap()),
        None => state.lock().await.runtime_tls_identity.clone().unwrap(),
    };
    let acceptor = TlsAcceptor::from(Arc::new(
        crate::tls_identity::server_config(&identity).unwrap(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    let script = script.to_vec();
    let worker = Task(tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await?;
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let conduct = script[n.min(script.len() - 1)];
            let started = tokio::time::Instant::now();
            let acceptor = acceptor.clone();
            let state = state.clone();
            tokio::spawn(async move {
                if matches!(conduct, Conduct::DropTcp) {
                    return Ok(());
                }
                let mut tls = acceptor.accept(AuthorityTcpStream::new(tcp)).await?;
                match conduct {
                    Conduct::CloseBeforeChallenge => tls.shutdown().await,
                    Conduct::PartialChallenge => {
                        tls.write_all(b"{\"cmd\":").await?;
                        tls.flush().await?;
                        tls.shutdown().await
                    }
                    _ => {
                        let permit = state.lock().await.admission.handshake().unwrap();
                        handle_fang_pipe(tls, state, permit, started).await
                    }
                }
            });
        }
    }));
    Receiver {
        address,
        target,
        expected,
        accepted,
        _worker: worker,
        _echo: echo,
    }
}

async fn setup(sender: &PeltIdentity, receiver: &Receiver) -> io::Result<()> {
    let (mut outbound, key) = connect_tcp_session(
        &receiver.address.to_string(),
        &receiver.target.to_string(),
        sender,
        &receiver.expected,
    )
    .await?;
    // The established session carries application bytes exactly.
    let message = b"stage32 f1 setup retry";
    write_encrypted_frame(&mut outbound, &key, 0, &mut 0, message).await?;
    let echoed = read_encrypted_frame(&mut outbound, &key, 1, &mut 0).await?;
    assert_eq!(echoed, message);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receiver_closing_before_its_challenge_is_retried_once() {
    timeout(Duration::from_secs(30), async {
        let sender = generate_identity();
        for first in [Conduct::DropTcp, Conduct::CloseBeforeChallenge] {
            let receiver = receiver(&sender, &[first, Conduct::Real], None).await;
            setup(&sender, &receiver).await.unwrap();
            assert_eq!(receiver.accepted.load(Ordering::SeqCst), 2);
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn there_is_never_a_second_retry() {
    timeout(Duration::from_secs(30), async {
        let sender = generate_identity();
        for conduct in [Conduct::DropTcp, Conduct::CloseBeforeChallenge] {
            let receiver = receiver(&sender, &[conduct], None).await;
            assert!(setup(&sender, &receiver).await.is_err());
            assert_eq!(receiver.accepted.load(Ordering::SeqCst), 2);
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_started_challenge_is_never_retried() {
    timeout(Duration::from_secs(30), async {
        let sender = generate_identity();
        let receiver = receiver(&sender, &[Conduct::PartialChallenge, Conduct::Real], None).await;
        assert!(setup(&sender, &receiver).await.is_err());
        assert_eq!(receiver.accepted.load(Ordering::SeqCst), 1);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tls_pinning_failure_is_never_retried() {
    timeout(Duration::from_secs(30), async {
        let sender = generate_identity();
        let impostor = generate_identity();
        let receiver = receiver(&sender, &[Conduct::Real], Some(&impostor)).await;
        assert!(setup(&sender, &receiver).await.is_err());
        assert_eq!(receiver.accepted.load(Ordering::SeqCst), 1);
    })
    .await
    .unwrap();
}
