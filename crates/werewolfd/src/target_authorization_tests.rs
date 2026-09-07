//! Real receiver requests using fresh Pack identities and disposable loopback listeners.
use crate::{
    state::DaemonState,
    target_policy::{self, TargetPolicy},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::timeout,
};
use werewolf_core::{
    pack::{PeerRecord, TrustLevel},
    pelt::{generate_identity, sign_message, verify_message, PeltIdentity},
};

struct AbortOnDrop(tokio::task::JoinHandle<tokio::io::Result<()>>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn loaded_policy(value: Option<String>, name: &str) -> TargetPolicy {
    let path = std::env::temp_dir().join(format!("wwp-stage9-{}-{}", std::process::id(), name));
    if let Some(value) = value {
        std::fs::write(&path, value).unwrap();
    }
    let policy = target_policy::load(&path);
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
    policy
}

fn grant(fp: &str, address: std::net::SocketAddr) -> String {
    json!({"mode":"deny-by-default", "peers":{fp:{"targets":[{"address":address.ip().to_string(),"port":address.port()}]}}}).to_string()
}

async fn tcp_request(
    address: std::net::SocketAddr,
    sender: &PeltIdentity,
    remote: &str,
    nonce: &str,
) -> Vec<u8> {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let key = STANDARD.encode([7u8; 32]);
    let signed = format!(
        "fang.pipe|{}|{}|{}|{}",
        sender.fingerprint, remote, nonce, key
    );
    let request = json!({"cmd":"fang.pipe", "remote":remote, "sender_pubkey":sender.public_key_b64, "sender_fingerprint":sender.fingerprint, "client_x25519":key, "nonce":nonce, "signature":sign_message(sender, signed.as_bytes()).unwrap()});
    stream
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).await.unwrap();
    line.into_bytes()
}

async fn quic_request(
    connection: &quinn::Connection,
    sender: &PeltIdentity,
    remote: &str,
    nonce: &str,
) -> Vec<u8> {
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let signed = format!(
        "fang.quic.open|fang-quic-v2|{}|{}|{}",
        sender.fingerprint, nonce, remote
    );
    let request = json!({"cmd":"fang.quic.open", "protocol":"fang-quic-v2", "remote":remote, "sender_fingerprint":sender.fingerprint, "nonce":nonce, "signature":sign_message(sender, signed.as_bytes()).unwrap()});
    send.write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    send.finish().unwrap();
    let mut line = Vec::new();
    while let Ok(Some(chunk)) = recv.read_chunk(1, true).await {
        line.extend_from_slice(&chunk.bytes);
        assert!(line.len() < 4096);
        if line.ends_with(b"\n") {
            break;
        }
    }
    line
}

async fn receiver_matrix(quic: bool) {
    let sender = generate_identity();
    let other = generate_identity();
    let receiver = generate_identity();
    let state = Arc::new(Mutex::new(DaemonState {
        pelt: Some(receiver.clone()),
        peers: [&sender, &other]
            .into_iter()
            .map(|p| PeerRecord {
                name: "same-alias".into(),
                fingerprint: p.fingerprint.clone(),
                address: "tcp://192.0.2.1:1".into(),
                trust: TrustLevel::Packmate,
                public_key_b64: Some(p.public_key_b64.clone()),
            })
            .collect(),
        ..Default::default()
    }));
    let address = if quic {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    } else {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    };
    let server_state = state.clone();
    let server = AbortOnDrop(tokio::spawn(async move {
        if quic {
            crate::transport::run_quic_fang_listener(&address.to_string(), server_state).await
        } else {
            crate::transport::run_fang_listener(&address.to_string(), server_state).await
        }
    }));
    let endpoint = crate::quic_lab::make_client_endpoint().unwrap();
    let connection = if quic {
        Some(
            timeout(
                Duration::from_secs(5),
                endpoint.connect(address, "localhost").unwrap(),
            )
            .await
            .unwrap()
            .unwrap(),
        )
    } else {
        timeout(Duration::from_secs(5), async {
            loop {
                assert!(!server.0.is_finished(), "receiver exited during startup");
                if TcpStream::connect(address).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        None
    };
    for bind in ["127.0.0.1:0", "[::1]:0"] {
        let target = match TcpListener::bind(bind).await {
            Ok(listener) => listener,
            Err(error)
                if bind.starts_with('[')
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::Unsupported
                    ) =>
            {
                eprintln!("Stage 9 IPv6 live target unavailable: {error}");
                continue;
            }
            Err(error) => panic!("target bind: {error}"),
        };
        let target_address = target.local_addr().unwrap();
        let remote = target_address.to_string();
        let mut wrong_port = target_address;
        wrong_port.set_port(if target_address.port() == 65535 {
            65534
        } else {
            target_address.port() + 1
        });
        let wrong_ip = if target_address.is_ipv4() {
            "127.0.0.2"
        } else {
            "::2"
        }
        .parse()
        .unwrap();
        let mut wrong_address = target_address;
        wrong_address.set_ip(wrong_ip);
        let cases = [
            (
                "explicit-grant",
                Some(grant(&sender.fingerprint, target_address)),
                &sender,
                true,
            ),
            ("missing", None, &sender, false),
            (
                "empty-grants",
                Some(json!({"mode":"deny-by-default","peers":{}}).to_string()),
                &sender,
                false,
            ),
            ("malformed", Some("{".into()), &sender, false),
            (
                "unknown-mode",
                Some(json!({"mode":"unknown"}).to_string()),
                &sender,
                false,
            ),
            (
                "wrong-port",
                Some(grant(&sender.fingerprint, wrong_port)),
                &sender,
                false,
            ),
            (
                "wrong-ip",
                Some(grant(&sender.fingerprint, wrong_address)),
                &sender,
                false,
            ),
            (
                "other-peer",
                Some(grant(&sender.fingerprint, target_address)),
                &other,
                false,
            ),
            (
                "legacy",
                Some(json!({"mode":"legacy-allow"}).to_string()),
                &other,
                true,
            ),
        ];
        for (name, policy, identity, allowed) in cases {
            let nonce = format!("stage9-{quic}-{bind}-{name}");
            state.lock().await.target_policy = loaded_policy(policy, &nonce);
            let line = timeout(Duration::from_secs(5), async {
                match &connection {
                    Some(connection) => quic_request(connection, identity, &remote, &nonce).await,
                    None => tcp_request(address, identity, &remote, &nonce).await,
                }
            })
            .await
            .expect("request outcome deadline");
            let replay_key = if quic {
                format!("quic|{}|{}", identity.fingerprint, nonce)
            } else {
                format!("{}|{}", identity.fingerprint, nonce)
            };
            assert!(
                state.lock().await.seen_nonces.contains_key(&replay_key),
                "{nonce}: request must have passed authentication and replay validation"
            );
            if allowed {
                let ack: serde_json::Value =
                    serde_json::from_slice(&line).expect("successful ACK JSON");
                assert_eq!(ack["ok"], true, "{nonce}");
                let signed = if quic {
                    format!(
                        "fang.quic.ack|fang-quic-v2|{}|{}|{}|{}",
                        receiver.fingerprint, identity.fingerprint, nonce, remote
                    )
                } else {
                    format!(
                        "fang.ack|{}|{}|{}|{}",
                        receiver.fingerprint,
                        identity.fingerprint,
                        nonce,
                        ack["server_x25519"].as_str().unwrap()
                    )
                };
                verify_message(
                    &receiver.public_key_b64,
                    signed.as_bytes(),
                    ack["signature"].as_str().unwrap(),
                )
                .unwrap();
                timeout(Duration::from_secs(1), target.accept())
                    .await
                    .expect("authorized target connection")
                    .unwrap();
            } else {
                assert!(
                    line.is_empty(),
                    "{nonce}: denied request must not receive a successful ACK or policy details"
                );
                assert!(
                    timeout(Duration::from_millis(100), target.accept())
                        .await
                        .is_err(),
                    "{nonce}: denied target saw a connection"
                );
            }
            eprintln!(
                "Stage 9 live {nonce}: PASS; allowed={allowed}; target_connections={}",
                usize::from(allowed)
            );
        }
    }
    if let Some(connection) = connection {
        connection.close(0u32.into(), b"test complete");
    }
    endpoint.close(0u32.into(), b"test complete");
}

#[tokio::test]
async fn encrypted_tcp_target_authorization_live_matrix() {
    receiver_matrix(false).await;
}

#[tokio::test]
async fn quic_target_authorization_live_matrix() {
    receiver_matrix(true).await;
}
