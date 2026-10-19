//! Gap 3 of docs/SESSION_END_SIGNALLING_PROPOSAL.md: a direction of an
//! encrypted TCP session ends cleanly only at a frame boundary after TLS
//! `close_notify`. Every other end reaches the application or target as a
//! reset.
use super::authority_tests::{establish, fixture};
use super::*;
use crate::target_policy::TargetPolicy;
use tokio::time::timeout;
use werewolf_core::pelt::generate_identity;

async fn send_frame(session: &mut authority_tests::Session, counter: &mut u64, data: &[u8]) {
    write_encrypted_frame(&mut session.peer, &session.key, 0, counter, data)
        .await
        .unwrap();
    let mut received = vec![0u8; data.len()];
    session.target.read_exact(&mut received).await.unwrap();
    assert_eq!(received, data);
}

#[tokio::test]
async fn close_notify_after_a_frame_ends_the_target_direction_with_fin() {
    let sender = generate_identity();
    let mut session = establish(fixture(&[&sender]), &sender).await;
    let mut counter = 0;
    send_frame(&mut session, &mut counter, b"complete").await;
    session.peer.shutdown().await.unwrap();
    let (rest, reset) = crate::transport::drain_until_closed(&mut session.target).await;
    assert!(rest.is_empty());
    assert!(!reset, "a close_notify end must reach the target as FIN");
}

#[tokio::test]
async fn tcp_close_without_close_notify_reaches_target_as_reset() {
    let sender = generate_identity();
    let mut session = establish(fixture(&[&sender]), &sender).await;
    let mut counter = 0;
    send_frame(&mut session, &mut counter, b"partial").await;
    // Dropping the TLS stream closes TCP with FIN but sends no close_notify,
    // as a crashed or killed peer would.
    drop(session.peer);
    let (_, reset) = crate::transport::drain_until_closed(&mut session.target).await;
    assert!(reset, "a TCP close without close_notify looked clean");
}

#[tokio::test]
async fn tcp_reset_from_peer_reaches_target_as_reset() {
    let sender = generate_identity();
    let mut session = establish(fixture(&[&sender]), &sender).await;
    let mut counter = 0;
    send_frame(&mut session, &mut counter, b"partial").await;
    crate::transport::arm_reset_on_close(session.peer.get_ref().0).unwrap();
    drop(session.peer);
    let (_, reset) = crate::transport::drain_until_closed(&mut session.target).await;
    assert!(reset, "a peer TCP reset looked clean");
}

#[tokio::test]
async fn close_notify_inside_a_length_prefix_reaches_target_as_reset() {
    let sender = generate_identity();
    let mut session = establish(fixture(&[&sender]), &sender).await;
    let mut counter = 0;
    send_frame(&mut session, &mut counter, b"partial").await;
    // Two of the four length bytes, then an orderly TLS close.
    session.peer.write_all(&[0, 0]).await.unwrap();
    session.peer.shutdown().await.unwrap();
    let (_, reset) = crate::transport::drain_until_closed(&mut session.target).await;
    assert!(reset, "a close inside a frame prefix looked clean");
}

/// When the target finishes first, the client ends the whole session to
/// release authority promptly (see
/// `remote_eof_releases_tcp_authority_without_local_write_eof`). It drops
/// the upload direction without close_notify, so the target must see a
/// reset, never a clean FIN that would make the unfinished upload look
/// complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_after_remote_finishes_first_is_reported_as_reset() {
    const UPLOAD: usize = 1 << 20;
    timeout(Duration::from_secs(30), async {
        let sender = generate_identity();
        let state = fixture(&[&sender]);
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target.local_addr().unwrap();
        let (receiver, listen) = {
            let mut st = state.lock().await;
            let TargetPolicy::Grants(grants) = &mut st.target_policy else {
                panic!("fixture grants");
            };
            grants.insert(sender.fingerprint.clone(), [target_address].into());
            let listen = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap();
            (st.pelt.clone().unwrap(), listen)
        };
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let listener_state = state.clone();
        let _listener = authority_tests::Task(tokio::spawn(async move {
            run_fang_listener_with_ready(&listen.to_string(), listener_state, Some(ready_tx)).await
        }));
        ready_rx.await.unwrap().unwrap();
        let local = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let expected = crate::policy::ExpectedPeerIdentity {
            fingerprint: receiver.fingerprint.clone(),
            public_key_b64: Some(receiver.public_key_b64.clone()),
        };
        let (forwarder_tx, forwarder_rx) = tokio::sync::oneshot::channel();
        let _forwarder = authority_tests::Task(tokio::spawn(async move {
            run_local_fang_forwarder(
                "session-end",
                &local.to_string(),
                &listen.to_string(),
                &target_address.to_string(),
                sender,
                crate::fang_registry::FangCancellation::default(),
                forwarder_tx,
                expected,
            )
            .await
        }));
        forwarder_rx.await.unwrap().unwrap();

        let client = TcpStream::connect(local).await.unwrap();
        let (mut target, _) = target.accept().await.unwrap();
        target.write_all(b"ready").await.unwrap();
        target.shutdown().await.unwrap();
        let (mut read, mut write) = client.into_split();
        let mut reply = Vec::new();
        read.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, b"ready");
        // Only now, after the remote direction has ended, try to upload.
        let written = write.write_all(&vec![5u8; UPLOAD]).await;
        let (received, reset) = crate::transport::drain_until_closed(&mut target).await;
        assert!(
            reset,
            "an unfinished upload reached the target as a clean FIN"
        );
        assert!(received.is_empty());
        drop((written, read, write));
    })
    .await
    .unwrap();
}
