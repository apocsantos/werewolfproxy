use super::*;
use crate::{authority::Authority, target_policy::TargetPolicy};
use std::collections::HashMap;
use werewolf_core::{
    pack::{PeerRecord, TrustLevel},
    pelt::generate_identity,
};

struct Task(tokio::task::JoinHandle<io::Result<()>>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Session {
    peer: TcpStream,
    target: TcpStream,
    key: [u8; 32],
    worker: Task,
}
async fn establish(state: Arc<Mutex<DaemonState>>, sender: &PeltIdentity) -> Session {
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_address = target_listener.local_addr().unwrap();
    {
        let mut state = state.lock().await;
        let TargetPolicy::Grants(grants) = &mut state.target_policy else {
            panic!("fixture grants");
        };
        grants.insert(sender.fingerprint.clone(), [target_address].into());
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (accepted, _) = listener.accept().await.unwrap();
    let permit = state.lock().await.admission.handshake().unwrap();
    let started = tokio::time::Instant::now();
    let worker = Task(tokio::spawn(handle_fang_pipe(
        accepted, state, permit, started,
    )));
    let deadline = started + Duration::from_secs(5);
    let challenge: hs::TcpChallenge = hs::read(&mut peer, hs::CHALLENGE_LIMIT, deadline)
        .await
        .unwrap();
    let secret = StaticSecret::from(hs::random::<32>().unwrap());
    let public = X25519PublicKey::from(&secret);
    let open = hs::TcpOpen::new(
        sender,
        &challenge,
        &target_address.to_string(),
        public.as_bytes(),
    )
    .unwrap();
    hs::write(&mut peer, &open, hs::MESSAGE_LIMIT, deadline)
        .await
        .unwrap();
    let ack: hs::TcpAck = hs::read(&mut peer, hs::MESSAGE_LIMIT, deadline)
        .await
        .unwrap();
    let retained = open.transcript().unwrap();
    hs::verify(
        &ack.receiver_pubkey,
        &ack.transcript(&open, &retained).unwrap(),
        &ack.signature,
    )
    .unwrap();
    let key = derive_shared_key(&secret, &ack.server_x25519).unwrap();
    let (target, _) = target_listener.accept().await.unwrap();
    Session {
        peer,
        target,
        key,
        worker,
    }
}
fn fixture(senders: &[&PeltIdentity]) -> Arc<Mutex<DaemonState>> {
    Arc::new(Mutex::new(DaemonState {
        inbound_authority: Authority::new(false),
        pelt: Some(generate_identity()),
        peers: senders
            .iter()
            .map(|sender| PeerRecord {
                name: sender.fingerprint.clone(),
                fingerprint: sender.fingerprint.clone(),
                public_key_b64: Some(sender.public_key_b64.clone()),
                address: "tcp://127.0.0.1:1".into(),
                trust: TrustLevel::Packmate,
            })
            .collect(),
        target_policy: TargetPolicy::Grants(HashMap::new()),
        ..Default::default()
    }))
}

#[tokio::test]
async fn revoke_joins_both_tcp_directions_without_affecting_other_peer() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let a = generate_identity();
        let b = generate_identity();
        let state = fixture(&[&a, &b]);
        let authority = state.lock().await.inbound_authority.clone();
        let mut first = establish(state.clone(), &a).await;
        let mut second = establish(state, &b).await;
        write_encrypted_frame(&mut first.peer, &first.key, 0, &mut 0, b"before")
            .await
            .unwrap();
        let mut before = [0u8; 6];
        first.target.read_exact(&mut before).await.unwrap();
        assert_eq!(&before, b"before");
        // Receipt proves publication and a live forwarding session. There is
        // no sleep used to guess whether revocation raced with establishment.
        authority.deny_peer(&a.fingerprint).unwrap();
        assert!((&mut first.worker.0).await.unwrap().is_err());
        authority.cleanup(Some(&a.fingerprint)).await.unwrap();
        let mut rest = Vec::new();
        first.target.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
        write_encrypted_frame(&mut second.peer, &second.key, 0, &mut 0, b"other")
            .await
            .unwrap();
        let mut other = [0u8; 5];
        second.target.read_exact(&mut other).await.unwrap();
        assert_eq!(&other, b"other");
        authority.lock().unwrap();
        assert!((&mut second.worker.0).await.unwrap().is_err());
        authority.cleanup(None).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn silver_closes_established_tcp_even_with_both_readers_stalled() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let sender = generate_identity();
        let state = fixture(&[&sender]);
        let authority = state.lock().await.inbound_authority.clone();
        let mut session = establish(state, &sender).await;
        authority.lock().unwrap();
        assert!((&mut session.worker.0).await.unwrap().is_err());
        authority.cleanup(None).await.unwrap();
        let mut rest = Vec::new();
        session.target.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
    })
    .await
    .unwrap();
}
