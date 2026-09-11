//! Isolated signed-request probes against production v3 receivers.
use crate::{handshake as hs, state::DaemonState, target_policy::TargetPolicy};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::{timeout, Instant},
};
use werewolf_core::{
    pack::{PeerRecord, TrustLevel},
    pelt::{generate_identity, PeltIdentity},
};

struct Task(tokio::task::JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn reserve(udp: bool) -> SocketAddr {
    if udp {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    } else {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    }
}
fn peer(identity: &PeltIdentity) -> PeerRecord {
    PeerRecord {
        name: identity.fingerprint.clone(),
        fingerprint: identity.fingerprint.clone(),
        public_key_b64: Some(identity.public_key_b64.clone()),
        address: "quic://127.0.0.1:1".into(),
        trust: TrustLevel::Packmate,
    }
}
async fn ready(address: SocketAddr) {
    timeout(Duration::from_secs(5), async {
        loop {
            if TcpStream::connect(address).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
struct Target {
    address: SocketAddr,
    count: Arc<AtomicUsize>,
    _task: Task,
}
impl Target {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let task = Task(tokio::spawn(async move {
            loop {
                let (_stream, _) = listener.accept().await.unwrap();
                c.fetch_add(1, Ordering::SeqCst);
            }
        }));
        Self {
            address,
            count,
            _task: task,
        }
    }
    async fn expect(&self, count: usize) {
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(self.count.load(Ordering::SeqCst), count);
    }
}
struct Fixture {
    state: Arc<Mutex<DaemonState>>,
    sender: PeltIdentity,
    receiver: PeltIdentity,
    target: Target,
    tcp: SocketAddr,
    quic: SocketAddr,
    _tasks: Vec<Task>,
}
impl Fixture {
    async fn new() -> Self {
        let sender = generate_identity();
        let receiver = generate_identity();
        let target = Target::new().await;
        let state = Arc::new(Mutex::new(DaemonState {
            pelt: Some(receiver.clone()),
            peers: vec![peer(&sender)],
            target_policy: TargetPolicy::Grants(HashMap::from([(
                sender.fingerprint.clone(),
                [target.address].into(),
            )])),
            ..Default::default()
        }));
        let tcp = reserve(false);
        let quic = reserve(true);
        let st = state.clone();
        let a = Task(tokio::spawn(async move {
            crate::transport::run_fang_listener(&tcp.to_string(), st)
                .await
                .unwrap();
        }));
        let st = state.clone();
        let b = Task(tokio::spawn(async move {
            crate::transport::run_quic_fang_listener(&quic.to_string(), st)
                .await
                .unwrap();
        }));
        ready(tcp).await;
        Self {
            state,
            sender,
            receiver,
            target,
            tcp,
            quic,
            _tasks: vec![a, b],
        }
    }
    async fn tcp_open(&self) -> (TcpStream, hs::TcpOpen) {
        tcp_open(self.tcp, &self.sender, &self.target.address.to_string()).await
    }
    async fn quic_connection(&self) -> quinn::Connection {
        quic_connection(self.quic).await
    }
}
async fn tcp_open(
    address: SocketAddr,
    sender: &PeltIdentity,
    remote: &str,
) -> (TcpStream, hs::TcpOpen) {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let challenge: hs::TcpChallenge = hs::read(&mut stream, 512, Instant::now() + hs::READ_WINDOW)
        .await
        .unwrap();
    let open = hs::TcpOpen::new(sender, &challenge, remote, &[7; 32]).unwrap();
    (stream, open)
}
async fn tcp_send(stream: &mut TcpStream, open: &hs::TcpOpen) -> Option<hs::TcpAck> {
    hs::write(stream, open, 4096, Instant::now() + hs::READ_WINDOW)
        .await
        .unwrap();
    hs::read(stream, 4096, Instant::now() + hs::READ_WINDOW)
        .await
        .ok()
}
async fn quic_connection(address: SocketAddr) -> quinn::Connection {
    let endpoint = crate::quic_lab::make_client_endpoint().unwrap();
    timeout(
        hs::READ_WINDOW,
        endpoint.connect(address, "localhost").unwrap(),
    )
    .await
    .unwrap()
    .unwrap()
}
async fn quic_send(connection: &quinn::Connection, open: &hs::QuicOpen) -> Option<hs::QuicAck> {
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    hs::write(&mut send, open, 4096, Instant::now() + hs::READ_WINDOW)
        .await
        .unwrap();
    send.finish().unwrap();
    hs::read(&mut recv, 4096, Instant::now() + hs::READ_WINDOW)
        .await
        .ok()
}
fn quic_open(f: &Fixture, c: &quinn::Connection) -> hs::QuicOpen {
    hs::QuicOpen::new(
        &f.sender,
        &f.receiver.fingerprint,
        &f.target.address.to_string(),
        &hs::quic_binding(c).unwrap(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_replay_binding_and_frozen_identity_matrix() {
    let f = Fixture::new().await;
    let (mut stream, open) = f.tcp_open().await;
    // Identity change after challenge emission must not change its ACK signer.
    f.state.lock().await.pelt = Some(generate_identity());
    let ack = tcp_send(&mut stream, &open).await.unwrap();
    assert_eq!(ack.receiver_fingerprint, f.receiver.fingerprint);
    hs::verify(
        &ack.receiver_pubkey,
        &ack.transcript(&open, &open.transcript().unwrap()).unwrap(),
        &ack.signature,
    )
    .unwrap();
    drop(stream);
    f.target.expect(1).await;
    f.state.lock().await.pelt = Some(f.receiver.clone());
    let (mut a, new_a) = f.tcp_open().await;
    let (mut b, new_b) = f.tcp_open().await;
    assert_ne!(open.challenge, new_a.challenge);
    assert_ne!(new_a.challenge, new_b.challenge);
    let (x, y) = tokio::join!(tcp_send(&mut a, &open), tcp_send(&mut b, &open));
    assert!(x.is_none() && y.is_none());
    f.target.expect(1).await;
    for kind in ["challenge", "receiver", "target", "ephemeral", "signature"] {
        let (mut s, mut v) = f.tcp_open().await;
        match kind {
            "challenge" => v.challenge = hs::hex(&[0; 32]),
            "receiver" => v.receiver_fingerprint = f.sender.fingerprint.clone(),
            "target" => v.remote = "127.0.0.1:1".into(),
            "ephemeral" => {
                use base64::Engine;
                v.client_x25519 = base64::engine::general_purpose::STANDARD.encode([8; 32]);
            }
            _ => {
                use base64::Engine;
                v.signature = base64::engine::general_purpose::STANDARD.encode([0; 64]);
            }
        }
        // Challenge and receiver mismatches are tested with valid sender signatures.
        if matches!(kind, "challenge" | "receiver") {
            v.signature = hs::sign(&f.sender, &v.transcript().unwrap()).unwrap();
        }
        assert!(tcp_send(&mut s, &v).await.is_none(), "{kind}");
        f.target.expect(1).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_one_open_and_real_expiry() {
    let f = Fixture::new().await;
    let (mut stream, open) = f.tcp_open().await;
    let bytes = format!(
        "{}\n{}\n",
        serde_json::to_string(&open).unwrap(),
        serde_json::to_string(&open).unwrap()
    );
    stream.write_all(bytes.as_bytes()).await.unwrap();
    let _: hs::TcpAck = hs::read(&mut stream, 4096, Instant::now() + hs::READ_WINDOW)
        .await
        .unwrap();
    f.target.expect(1).await;
    let (mut stale, open) = f.tcp_open().await;
    tokio::time::sleep(Duration::from_millis(5100)).await;
    let bytes = format!("{}\n", serde_json::to_string(&open).unwrap());
    let _ = stale.write_all(bytes.as_bytes()).await;
    assert!(
        hs::read::<hs::TcpAck, _>(&mut stale, 4096, Instant::now() + Duration::from_secs(1))
            .await
            .is_err()
    );
    f.target.expect(1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quic_connection_and_same_context_replay_matrix() {
    let f = Fixture::new().await;
    let c = f.quic_connection().await;
    let other = f.quic_connection().await;
    assert_ne!(
        hs::quic_binding(&c).unwrap(),
        hs::quic_binding(&other).unwrap()
    );
    let open = quic_open(&f, &c);
    let ack = quic_send(&c, &open).await.unwrap();
    hs::verify(
        &ack.receiver_pubkey,
        &ack.transcript(
            &open,
            &open.transcript(&hs::quic_binding(&c).unwrap()).unwrap(),
        )
        .unwrap(),
        &ack.signature,
    )
    .unwrap();
    f.target.expect(1).await;
    assert!(quic_send(&c, &open).await.is_none());
    assert!(quic_send(&other, &open).await.is_none());
    f.target.expect(1).await;
    let duplicate = quic_open(&f, &c);
    let (a, b) = tokio::join!(quic_send(&c, &duplicate), quic_send(&c, &duplicate));
    assert_eq!(usize::from(a.is_some()) + usize::from(b.is_some()), 1);
    f.target.expect(2).await;
    for kind in ["receiver", "target", "signature"] {
        let mut v = quic_open(&f, &c);
        if kind == "receiver" {
            v.receiver_fingerprint = f.sender.fingerprint.clone();
            v.signature = hs::sign(
                &f.sender,
                &v.transcript(&hs::quic_binding(&c).unwrap()).unwrap(),
            )
            .unwrap();
        } else if kind == "target" {
            v.remote = "127.0.0.1:1".into();
        } else {
            use base64::Engine;
            v.signature = base64::engine::general_purpose::STANDARD.encode([0; 64]);
        }
        assert!(quic_send(&c, &v).await.is_none());
        f.target.expect(2).await;
    }
    // Denied reservations must survive policy changes and stream closure.
    f.state.lock().await.target_policy = TargetPolicy::Deny;
    let denied = quic_open(&f, &c);
    assert!(quic_send(&c, &denied).await.is_none());
    f.state.lock().await.target_policy = TargetPolicy::Grants(HashMap::from([(
        f.sender.fingerprint.clone(),
        [f.target.address].into(),
    )]));
    assert!(quic_send(&c, &denied).await.is_none());
    f.target.expect(2).await;
    c.close(0u32.into(), b"");
    other.close(0u32.into(), b"");
}

#[tokio::test]
async fn exporter_equality_separation_and_ack_substitution() {
    let server = crate::quic_lab::make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let client = crate::quic_lab::make_client_endpoint().unwrap();
    let mut previous = None;
    for _ in 0..2 {
        let connecting = client
            .connect(server.local_addr().unwrap(), "localhost")
            .unwrap();
        let (a, b) = tokio::join!(async { connecting.await.unwrap() }, async {
            server.accept().await.unwrap().await.unwrap()
        });
        let binding = hs::quic_binding(&a).unwrap();
        assert_eq!(binding, hs::quic_binding(&b).unwrap());
        if let Some(last) = previous {
            assert_ne!(last, binding);
        }
        previous = Some(binding);
        let export = |connection: &quinn::Connection, label: &[u8], context: &[u8]| {
            let mut value = [0; 32];
            connection
                .export_keying_material(&mut value, label, context)
                .unwrap();
            value
        };
        let label = b"EXPORTER-WerewolfProxy-Fang-QUIC-v3";
        let context = b"werewolfproxy/fang-quic-v3";
        let original = export(&a, label, context);
        assert_eq!(original, export(&b, label, context));
        assert_ne!(original, export(&a, b"stage11a-distinct-label", context));
        assert_ne!(original, export(&a, label, b"stage11a-distinct-context"));
        a.force_key_update();
        let (mut send, _) = a.open_bi().await.unwrap();
        send.write_all(b"key-update").await.unwrap();
        send.finish().unwrap();
        let (_, mut recv) = b.accept_bi().await.unwrap();
        assert_eq!(recv.read_to_end(32).await.unwrap(), b"key-update");
        assert_eq!(original, export(&a, label, context));
        assert_eq!(original, export(&b, label, context));
        assert_eq!(binding, hs::quic_binding(&a).unwrap());
        a.close(0u32.into(), b"");
        b.close(0u32.into(), b"");
    }
    let recreated = crate::quic_lab::make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let connecting = client
        .connect(recreated.local_addr().unwrap(), "localhost")
        .unwrap();
    let (a, b) = tokio::join!(async { connecting.await.unwrap() }, async {
        recreated.accept().await.unwrap().await.unwrap()
    });
    assert_eq!(hs::quic_binding(&a).unwrap(), hs::quic_binding(&b).unwrap());
    assert_ne!(previous.unwrap(), hs::quic_binding(&a).unwrap());
    a.close(0u32.into(), b"");
    b.close(0u32.into(), b"");
    let sender = generate_identity();
    let receiver = generate_identity();
    let addr = server.local_addr().unwrap();
    let receiver_for_server = receiver.clone();
    let fake = tokio::spawn(async move {
        let c = server.accept().await.unwrap().await.unwrap();
        let (mut send, mut recv) = c.accept_bi().await.unwrap();
        let open: hs::QuicOpen = hs::read(&mut recv, 4096, Instant::now() + hs::READ_WINDOW)
            .await
            .unwrap();
        let wrong = hs::QuicAck::new(
            &open,
            &open.transcript(&[0; 32]).unwrap(),
            &receiver_for_server,
        )
        .unwrap();
        hs::write(&mut send, &wrong, 4096, Instant::now() + hs::READ_WINDOW)
            .await
            .unwrap();
        let _ = timeout(Duration::from_secs(2), c.closed()).await;
    });
    assert!(
        crate::quic_fang::connect_v3(addr, "127.0.0.1:1", &sender, &receiver.fingerprint)
            .await
            .is_err()
    );
    fake.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_exhaustion_release_and_spoofed_peer() {
    let f = Fixture::new().await;
    let admission = f.state.lock().await.admission.clone();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let held: Vec<_> = (0..256).map(|_| admission.handshake().unwrap()).collect();
    let mut s = TcpStream::connect(f.tcp).await.unwrap();
    assert!(
        hs::read::<hs::TcpChallenge, _>(&mut s, 512, Instant::now() + Duration::from_secs(1))
            .await
            .is_err()
    );
    drop(held);
    let mut held = Vec::new();
    for _ in 0..16 {
        let mut p = admission.handshake().unwrap();
        p.authenticated(&f.sender.fingerprint).unwrap();
        held.push(p);
    }
    let (mut stream, open) = f.tcp_open().await;
    assert!(tcp_send(&mut stream, &open).await.is_none());
    drop(held);
    f.target.expect(0).await;
    for _ in 0..20 {
        let (mut stream, mut open) = f.tcp_open().await;
        use base64::Engine;
        open.signature = base64::engine::general_purpose::STANDARD.encode([0; 64]);
        assert!(tcp_send(&mut stream, &open).await.is_none());
    }
    let (mut stream, open) = f.tcp_open().await;
    assert!(tcp_send(&mut stream, &open).await.is_some());
    f.target.expect(1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quic_capacity_never_evicts_active_reservations() {
    let f = Fixture::new().await;
    f.state.lock().await.target_policy = TargetPolicy::Deny;
    let c = f.quic_connection().await;
    let first = quic_open(&f, &c);
    assert!(quic_send(&c, &first).await.is_none());
    for _ in 1..256 {
        let open = quic_open(&f, &c);
        assert!(quic_send(&c, &open).await.is_none());
    }
    f.state.lock().await.target_policy = TargetPolicy::Grants(HashMap::from([(
        f.sender.fingerprint.clone(),
        [f.target.address].into(),
    )]));
    assert!(quic_send(&c, &first).await.is_none());
    assert!(quic_send(&c, &quic_open(&f, &c)).await.is_none());
    f.target.expect(0).await;
    let fresh = f.quic_connection().await;
    assert!(quic_send(&fresh, &quic_open(&f, &fresh)).await.is_some());
    f.target.expect(1).await;
    c.close(0u32.into(), b"");
    fresh.close(0u32.into(), b"");
}

fn process_binaries() -> &'static (std::path::PathBuf, std::path::PathBuf) {
    static BINARIES: std::sync::OnceLock<(std::path::PathBuf, std::path::PathBuf)> =
        std::sync::OnceLock::new();
    BINARIES.get_or_init(|| {
        use std::{path::PathBuf, process::Command};
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let fixtures = root.join("target/stage10-legacy-source");
        std::fs::create_dir_all(&fixtures).unwrap();
        let archive = root.join("target/stage10-legacy.tar");
        let result = Command::new("git")
            .current_dir(&root)
            .args(["archive", "--format=tar", "--output"])
            .arg(&archive)
            .arg("23ae82c633990d0d594539f738feaf8569f489fe")
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&fixtures)
            .status()
            .unwrap()
            .success());
        let mut binaries = Vec::new();
        for (manifest, target, label) in [
            (
                root.join("Cargo.toml"),
                root.join("target/stage1-lab"),
                "v3",
            ),
            (
                fixtures.join("Cargo.toml"),
                root.join("target/stage10-audit"),
                "legacy",
            ),
        ] {
            let output = Command::new("cargo")
                .current_dir(&root)
                .args([
                    "build",
                    "--locked",
                    "--offline",
                    "-p",
                    "werewolfd",
                    "--bin",
                    "werewolfd",
                    "--manifest-path",
                ])
                .arg(manifest)
                .arg("--target-dir")
                .arg(&target)
                .output()
                .unwrap();
            std::fs::write(
                root.join(format!("target/stage10-{label}-build.log")),
                &output.stderr,
            )
            .unwrap();
            assert!(
                output.status.success(),
                "{label} fixture build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            binaries.push(target.join("debug/werewolfd"));
        }
        (binaries.remove(0), binaries.remove(0))
    })
}
struct Daemon {
    home: std::path::PathBuf,
    binary: std::path::PathBuf,
    child: Option<std::process::Child>,
    tcp: SocketAddr,
    quic: SocketAddr,
    sender: PeltIdentity,
    receiver: PeltIdentity,
}
impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
impl Daemon {
    async fn new(legacy: bool, target: SocketAddr) -> Self {
        use std::os::unix::fs::PermissionsExt;
        static ID: AtomicUsize = AtomicUsize::new(0);
        let binaries = process_binaries();
        let home = std::env::temp_dir().join(format!(
            "wwp-v3-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sender = generate_identity();
        let receiver = generate_identity();
        werewolf_core::pelt::save_identity(&home.join("pelt.json"), &receiver).unwrap();
        werewolf_core::pack::save_pack(&home.join("pack.json"), &[peer(&sender)]).unwrap();
        std::fs::write(home.join("target_policy.json"),serde_json::json!({"mode":"deny-by-default","peers":{sender.fingerprint.clone():{"targets":[{"address":target.ip().to_string(),"port":target.port()}]}}}).to_string()).unwrap();
        std::fs::set_permissions(
            home.join("target_policy.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mut d = Self {
            home,
            binary: if legacy {
                binaries.1.clone()
            } else {
                binaries.0.clone()
            },
            child: None,
            tcp: reserve(false),
            quic: reserve(true),
            sender,
            receiver,
        };
        d.start().await;
        d
    }
    async fn start(&mut self) {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.home.join("daemon.log"))
            .unwrap();
        self.child = Some(
            std::process::Command::new(&self.binary)
                .arg("--home")
                .arg(&self.home)
                .arg("--socket")
                .arg(self.home.join("control.sock"))
                .arg("--listen")
                .arg(self.tcp.to_string())
                .arg("--quic-listen")
                .arg(self.quic.to_string())
                .env_clear()
                .env("HOME", &self.home)
                .env("TMPDIR", &self.home)
                .env("XDG_CONFIG_HOME", &self.home)
                .env("XDG_RUNTIME_DIR", &self.home)
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        ready(self.tcp).await;
        timeout(hs::READ_WINDOW, async {
            loop {
                if tokio::net::UnixStream::connect(self.home.join("control.sock"))
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(self.child.as_mut().unwrap().try_wait().unwrap().is_none());
    }
    fn stop(&mut self, crash: bool) {
        let mut child = self.child.take().unwrap();
        if crash {
            child.kill().unwrap();
        } else {
            assert!(std::process::Command::new("kill")
                .args(["-TERM", &child.id().to_string()])
                .status()
                .unwrap()
                .success());
        }
        child.wait().unwrap();
    }
    async fn control(&self, cmd: &str) -> serde_json::Value {
        let mut stream = tokio::net::UnixStream::connect(self.home.join("control.sock"))
            .await
            .unwrap();
        let deadline = Instant::now() + hs::READ_WINDOW;
        hs::write(
            &mut stream,
            &serde_json::json!({"id":"stage10","cmd":cmd,"args":{}}),
            4096,
            deadline,
        )
        .await
        .unwrap();
        let response: serde_json::Value = hs::read(&mut stream, 4096, deadline).await.unwrap();
        assert_eq!(response["ok"], true);
        response
    }
}
async fn legacy_request(daemon: &Daemon, target: SocketAddr, quic: bool) -> bool {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let nonce = hs::hex(&hs::random::<16>().unwrap());
    let remote = target.to_string();
    let key = STANDARD.encode([7; 32]);
    let signed = if quic {
        format!(
            "fang.quic.open|fang-quic-v2|{}|{}|{}",
            daemon.sender.fingerprint, nonce, remote
        )
    } else {
        format!(
            "fang.pipe|{}|{}|{}|{}",
            daemon.sender.fingerprint, remote, nonce, key
        )
    };
    let request = serde_json::json!({"cmd":if quic{"fang.quic.open"}else{"fang.pipe"},"protocol":"fang-quic-v2","sender_fingerprint":daemon.sender.fingerprint,"sender_pubkey":daemon.sender.public_key_b64,"nonce":nonce,"remote":remote,"client_x25519":key,"signature":hs::sign(&daemon.sender,signed.as_bytes()).unwrap()});
    let deadline = Instant::now() + hs::READ_WINDOW;
    let ack = if quic {
        let connection = quic_connection(daemon.quic).await;
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        hs::write(&mut send, &request, 4096, deadline)
            .await
            .unwrap();
        send.finish().unwrap();
        let value = hs::read::<serde_json::Value, _>(&mut recv, 4096, deadline)
            .await
            .ok();
        connection.close(0u32.into(), b"");
        value
    } else {
        let mut stream = TcpStream::connect(daemon.tcp).await.unwrap();
        hs::write(&mut stream, &request, 4096, deadline)
            .await
            .unwrap();
        hs::read::<serde_json::Value, _>(&mut stream, 4096, deadline)
            .await
            .ok()
    };
    let Some(ack) = ack.filter(|v| v["ok"] == true) else {
        return false;
    };
    let signed = if quic {
        format!(
            "fang.quic.ack|fang-quic-v2|{}|{}|{}|{}",
            daemon.receiver.fingerprint, daemon.sender.fingerprint, nonce, remote
        )
    } else {
        format!(
            "fang.ack|{}|{}|{}|{}",
            daemon.receiver.fingerprint,
            daemon.sender.fingerprint,
            nonce,
            ack["server_x25519"].as_str().unwrap()
        )
    };
    hs::verify(
        &daemon.receiver.public_key_b64,
        signed.as_bytes(),
        ack["signature"].as_str().unwrap(),
    )
    .unwrap();
    true
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_daemon_restart_crash_and_pelt_regeneration() {
    let target = Target::new().await;
    let mut d = Daemon::new(false, target.address).await;
    let before: [Vec<u8>; 3] = ["pelt.json", "pack.json", "target_policy.json"]
        .map(|name| std::fs::read(d.home.join(name)).unwrap());
    let (mut tcp, open_tcp) = tcp_open(d.tcp, &d.sender, &target.address.to_string()).await;
    assert!(tcp_send(&mut tcp, &open_tcp).await.is_some());
    drop(tcp);
    let quic = quic_connection(d.quic).await;
    let old_binding = hs::quic_binding(&quic).unwrap();
    let open_quic = hs::QuicOpen::new(
        &d.sender,
        &d.receiver.fingerprint,
        &target.address.to_string(),
        &old_binding,
    )
    .unwrap();
    assert!(quic_send(&quic, &open_quic).await.is_some());
    quic.close(0u32.into(), b"");
    target.expect(2).await;
    let mut expected = 2;
    for crash in [false, true] {
        d.stop(crash);
        d.start().await;
        assert_eq!(
            before,
            ["pelt.json", "pack.json", "target_policy.json"]
                .map(|name| std::fs::read(d.home.join(name)).unwrap())
        );
        let (mut tcp, fresh) = tcp_open(d.tcp, &d.sender, &target.address.to_string()).await;
        assert_ne!(open_tcp.challenge, fresh.challenge);
        assert!(tcp_send(&mut tcp, &open_tcp).await.is_none());
        let connection = quic_connection(d.quic).await;
        assert_ne!(old_binding, hs::quic_binding(&connection).unwrap());
        assert!(quic_send(&connection, &open_quic).await.is_none());
        target.expect(expected).await;
        let (mut tcp, fresh) = tcp_open(d.tcp, &d.sender, &target.address.to_string()).await;
        assert!(tcp_send(&mut tcp, &fresh).await.is_some());
        let fresh = hs::QuicOpen::new(
            &d.sender,
            &d.receiver.fingerprint,
            &target.address.to_string(),
            &hs::quic_binding(&connection).unwrap(),
        )
        .unwrap();
        assert!(quic_send(&connection, &fresh).await.is_some());
        connection.close(0u32.into(), b"");
        expected += 2;
        target.expect(expected).await;
        eprintln!("V3 PROCESS restart crash={crash}: captured TCP/QUIC denied with zero target attempts; fresh requests passed");
    }
    let (mut tcp, open) = tcp_open(d.tcp, &d.sender, &target.address.to_string()).await;
    let result = d.control("pelt.init").await;
    assert_ne!(result["result"]["fingerprint"], open.receiver_fingerprint);
    let ack = tcp_send(&mut tcp, &open).await.unwrap();
    assert_eq!(ack.receiver_fingerprint, d.receiver.fingerprint);
    hs::verify(
        &ack.receiver_pubkey,
        &ack.transcript(&open, &open.transcript().unwrap()).unwrap(),
        &ack.signature,
    )
    .unwrap();
    target.expect(expected + 1).await;
    eprintln!("V3 PROCESS pelt.init after challenge: ACK verified under frozen receiver identity");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_interoperability_matrix() {
    let target = Target::new().await;
    let old = Daemon::new(true, target.address).await;
    let new = Daemon::new(false, target.address).await;
    for q in [false, true] {
        assert!(legacy_request(&old, target.address, q).await);
        assert!(!legacy_request(&new, target.address, q).await);
    }
    target.expect(2).await;
    // Exercise the actual v3 initiating TCP forwarder against the old receiver.
    let local = reserve(false);
    let old_address = old.tcp;
    let remote = target.address;
    let sender = old.sender.clone();
    let expected = crate::policy::ExpectedPeerIdentity {
        fingerprint: old.receiver.fingerprint.clone(),
    };
    let cancellation = crate::fang_registry::FangCancellation::default();
    let cancel = cancellation.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _forwarder = Task(tokio::spawn(async move {
        crate::transport::run_local_fang_forwarder(
            "stage10",
            &local.to_string(),
            &old_address.to_string(),
            &remote.to_string(),
            sender,
            cancel,
            tx,
            expected,
        )
        .await
        .unwrap();
    }));
    rx.await.unwrap().unwrap();
    let mut client = TcpStream::connect(local).await.unwrap();
    client.write_all(b"not forwarded").await.unwrap();
    let mut data = [0; 32];
    let result = timeout(Duration::from_secs(7), client.read(&mut data))
        .await
        .expect("new TCP -> old must terminate");
    assert!(matches!(result, Ok(0) | Err(_)));
    cancellation.abort_children();
    assert!(timeout(
        Duration::from_secs(7),
        crate::quic_fang::connect_v3(
            old.quic,
            &target.address.to_string(),
            &old.sender,
            &old.receiver.fingerprint
        )
    )
    .await
    .unwrap()
    .is_err());
    target.expect(2).await;
    let (mut stream, open) = tcp_open(new.tcp, &new.sender, &target.address.to_string()).await;
    assert!(tcp_send(&mut stream, &open).await.is_some());
    let connection = quic_connection(new.quic).await;
    let open = hs::QuicOpen::new(
        &new.sender,
        &new.receiver.fingerprint,
        &target.address.to_string(),
        &hs::quic_binding(&connection).unwrap(),
    )
    .unwrap();
    assert!(quic_send(&connection, &open).await.is_some());
    connection.close(0u32.into(), b"");
    target.expect(4).await;
    eprintln!("V3 INTEROP both transports: old/old passed; old/new denied; new/old denied within bound; new/new passed");
}

fn malformed_open(mut value: serde_json::Value, case: usize) -> Vec<u8> {
    match case {
        0 => value["unknown"] = true.into(),
        1 => value["nonce"] = "AA".repeat(16).into(),
        2 => value["signature"] = "invalid".into(),
        3 => value["remote"] = "x".repeat(513).into(),
        4 => {
            value.as_object_mut().unwrap().remove("nonce");
        }
        5 => value["nonce"] = 12.into(),
        _ => {}
    }
    let mut bytes = serde_json::to_vec(&value).unwrap();
    match case {
        6 => {
            bytes.pop();
            bytes.extend_from_slice(b",\"nonce\":\"duplicate\"}");
        }
        7 => bytes.extend_from_slice(b" {}"),
        8 => bytes = vec![b'x'; 4096],
        _ => {}
    }
    bytes.push(b'\n');
    bytes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_strict_open_parser_matrix() {
    let f = Fixture::new().await;
    for case in 0..9 {
        let (mut stream, open) = f.tcp_open().await;
        stream
            .write_all(&malformed_open(serde_json::to_value(open).unwrap(), case))
            .await
            .unwrap();
        assert!(
            hs::read::<hs::TcpAck, _>(&mut stream, 4096, Instant::now() + hs::READ_WINDOW)
                .await
                .is_err()
        );
        let c = f.quic_connection().await;
        let open = quic_open(&f, &c);
        let (mut send, mut recv) = c.open_bi().await.unwrap();
        send.write_all(&malformed_open(serde_json::to_value(open).unwrap(), case))
            .await
            .unwrap();
        send.finish().unwrap();
        assert!(
            hs::read::<hs::QuicAck, _>(&mut recv, 4096, Instant::now() + hs::READ_WINDOW)
                .await
                .is_err()
        );
        c.close(0u32.into(), b"");
    }
    f.target.expect(0).await;
    // Fragment a valid OPEN into individual writes after all rejection paths.
    let (mut stream, open) = f.tcp_open().await;
    let mut bytes = serde_json::to_vec(&open).unwrap();
    bytes.push(b'\n');
    for byte in bytes {
        stream.write_all(&[byte]).await.unwrap();
        tokio::task::yield_now().await;
    }
    let _: hs::TcpAck = hs::read(&mut stream, 4096, Instant::now() + hs::READ_WINDOW)
        .await
        .unwrap();
    f.target.expect(1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quic_open_limit_and_incomplete_sender_release() {
    let f = Fixture::new().await;
    let c = f.quic_connection().await;
    let mut streams = Vec::new();
    for _ in 0..9 {
        let (mut send, recv) = c.open_bi().await.unwrap();
        send.write_all(b"{").await.unwrap();
        streams.push((send, recv));
    }
    timeout(Duration::from_secs(2), c.closed())
        .await
        .expect("ninth concurrent OPEN must close the connection");
    drop(streams);
    f.target.expect(0).await;
    let c = f.quic_connection().await;
    let (mut send, mut recv) = c.open_bi().await.unwrap();
    send.write_all(b"{").await.unwrap();
    assert!(timeout(Duration::from_secs(6), recv.read_u8())
        .await
        .unwrap()
        .is_err());
    assert!(quic_send(&c, &quic_open(&f, &c)).await.is_some());
    f.target.expect(1).await;
    c.close(0u32.into(), b"");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quic_same_stream_has_only_one_open() {
    let f = Fixture::new().await;
    let c = f.quic_connection().await;
    let open = quic_open(&f, &c);
    let (mut send, mut recv) = c.open_bi().await.unwrap();
    let mut bytes = serde_json::to_vec(&open).unwrap();
    bytes.push(b'\n');
    let duplicate = bytes.clone();
    bytes.extend_from_slice(&duplicate);
    send.write_all(&bytes).await.unwrap();
    send.finish().unwrap();
    let ack: hs::QuicAck = hs::read(&mut recv, 4096, Instant::now() + hs::READ_WINDOW)
        .await
        .unwrap();
    hs::verify(
        &ack.receiver_pubkey,
        &ack.transcript(
            &open,
            &open.transcript(&hs::quic_binding(&c).unwrap()).unwrap(),
        )
        .unwrap(),
        &ack.signature,
    )
    .unwrap();
    f.target.expect(1).await;
    assert!(quic_send(&c, &open).await.is_none());
    f.target.expect(1).await;
    c.close(0u32.into(), b"");
}
