use super::*;
use base64::Engine;
use std::sync::atomic::{AtomicUsize, Ordering};
use werewolf_core::pelt::generate_identity;

fn selected(pelt: &PeltIdentity) -> crate::policy::ExpectedPeerIdentity {
    crate::policy::ExpectedPeerIdentity {
        fingerprint: pelt.fingerprint.clone(),
        public_key_b64: Some(pelt.public_key_b64.clone()),
    }
}

#[tokio::test]
async fn selected_pelt_authenticates_before_application_and_exposes_no_sni_or_alpn() {
    let expected = generate_identity();
    let server_identity = crate::tls_identity::RuntimeTlsIdentity::from_pelt(&expected).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(
        crate::tls_identity::server_config(&server_identity).unwrap(),
    ));
    let received = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut stream = acceptor.accept(AuthorityTcpStream::new(tcp)).await.unwrap();
        assert_eq!(stream.get_ref().1.server_name(), None);
        assert_eq!(stream.get_ref().1.alpn_protocol(), None);
        assert_eq!(
            stream.get_ref().1.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );
        let mut sentinel = [0; 100];
        stream.read_exact(&mut sentinel).await.unwrap();
        sentinel
    });
    let mut client = connect_authenticated_tcp(&address.to_string(), &selected(&expected))
        .await
        .unwrap();
    assert_eq!(
        client.get_ref().1.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
    assert_eq!(client.get_ref().1.alpn_protocol(), None);
    assert_eq!(
        client.get_ref().1.peer_certificates().unwrap()[0].as_ref(),
        server_identity.certificate().as_ref()
    );
    let mut sentinel = [0; 100];
    OsRng.fill_bytes(&mut sentinel);
    client.write_all(&sentinel).await.unwrap();
    assert_eq!(received.await.unwrap(), sentinel);
}

#[tokio::test]
async fn wrong_receiver_and_other_valid_pack_pelt_receive_no_application_data() {
    let expected = generate_identity();
    let wrong = generate_identity();
    let other_pack_peer = generate_identity();
    for actual in [&wrong, &other_pack_peer] {
        let identity = crate::tls_identity::RuntimeTlsIdentity::from_pelt(actual).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(
            crate::tls_identity::server_config(&identity).unwrap(),
        ));
        let app_bytes = Arc::new(AtomicUsize::new(0));
        let observed = app_bytes.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            if let Ok(mut stream) = acceptor.accept(AuthorityTcpStream::new(tcp)).await {
                let mut byte = [0; 1];
                if let Ok(Ok(n)) =
                    tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte)).await
                {
                    observed.fetch_add(n, Ordering::SeqCst);
                }
            }
        });
        assert!(
            connect_authenticated_tcp(&address.to_string(), &selected(&expected))
                .await
                .is_err()
        );
        server.await.unwrap();
        assert_eq!(app_bytes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn production_forwarder_sends_no_open_to_fake_receiver() {
    let sender = generate_identity();
    let selected_peer = generate_identity();
    let fake_receiver = generate_identity();
    let tls_identity = crate::tls_identity::RuntimeTlsIdentity::from_pelt(&fake_receiver).unwrap();
    let fake_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fake_addr = fake_listener.local_addr().unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(
        crate::tls_identity::server_config(&tls_identity).unwrap(),
    ));
    let fake = tokio::spawn(async move {
        let (tcp, _) = fake_listener.accept().await.unwrap();
        if let Ok(mut tls) = acceptor.accept(AuthorityTcpStream::new(tcp)).await {
            let mut buf = [0; 4096];
            match tokio::time::timeout(Duration::from_secs(1), tls.read(&mut buf)).await {
                Ok(Ok(n)) => n,
                _ => 0,
            }
        } else {
            0
        }
    });
    let local = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local.local_addr().unwrap();
    let client = tokio::spawn(async move {
        let (mut inbound, _) = local.accept().await.unwrap();
        pipe_one_fang_connection(
            &mut inbound,
            &fake_addr.to_string(),
            "192.0.2.41:28769",
            sender,
            selected(&selected_peer),
        )
        .await
    });
    let mut local_client = TcpStream::connect(local_addr).await.unwrap();
    local_client
        .write_all(b"OPEN target sender identity authorization")
        .await
        .unwrap();
    assert!(client.await.unwrap().is_err());
    assert_eq!(fake.await.unwrap(), 0);
}

#[tokio::test]
async fn missing_selected_pack_key_fails_before_tcp_connect() {
    let expected = generate_identity();
    let mut peer = selected(&expected);
    peer.public_key_b64 = None;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(
        connect_authenticated_tcp(&listener.local_addr().unwrap().to_string(), &peer)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn invalid_selected_pack_key_fails_before_tcp_connect() {
    let expected = generate_identity();
    let mut peer = selected(&expected);
    peer.public_key_b64 = Some("not a canonical Ed25519 key".into());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(
        connect_authenticated_tcp(&listener.local_addr().unwrap().to_string(), &peer)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn listener_waits_for_same_process_pelt_publication() {
    let receiver = generate_identity();
    let runtime = Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(&receiver).unwrap());
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let state_for_task = state.clone();
    let task =
        tokio::spawn(async move { run_fang_listener(&address.to_string(), state_for_task).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(TcpStream::connect(address).await.is_err());
    {
        let mut live = state.lock().await;
        live.pelt = Some(receiver.clone());
        live.runtime_tls_identity = Some(runtime);
        live.tls_identity_ready.publish();
    }
    let expected = selected(&receiver);
    let _client = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(client) = connect_authenticated_tcp(&address.to_string(), &expected).await {
                break client;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
}

#[tokio::test]
async fn authenticated_tcp_wire_hides_application_and_pelt_certificate_bytes() {
    let expected = generate_identity();
    let identity = crate::tls_identity::RuntimeTlsIdentity::from_pelt(&expected).unwrap();
    let cert = identity.certificate().as_ref().to_vec();
    let expected_spki =
        crate::tls_identity::canonical_spki_from_public_key_b64(&expected.public_key_b64).unwrap();
    let raw_key = base64::engine::general_purpose::STANDARD
        .decode(&expected.public_key_b64)
        .unwrap();
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_address = relay.local_addr().unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(
        crate::tls_identity::server_config(&identity).unwrap(),
    ));
    let mut sentinel = [0; 100];
    OsRng.fill_bytes(&mut sentinel);
    let target = "192.0.2.153:28769";
    let mut payload = format!(
        "fang pelt pack wwp1 target sender receiver ack protocol {} {target}\n",
        expected.fingerprint
    )
    .into_bytes();
    payload.extend_from_slice(&sentinel);
    let sent = payload.clone();
    let server = tokio::spawn(async move {
        let (tcp, _) = upstream.accept().await.unwrap();
        let mut tls = acceptor.accept(AuthorityTcpStream::new(tcp)).await.unwrap();
        assert_eq!(tls.get_ref().1.server_name(), None);
        assert_eq!(tls.get_ref().1.alpn_protocol(), None);
        let mut received = vec![0; sent.len()];
        tls.read_exact(&mut received).await.unwrap();
        assert_eq!(received, sent);
        tls.shutdown().await.unwrap();
    });
    let relay_task = tokio::spawn(async move {
        let (from_client, _) = relay.accept().await.unwrap();
        let to_server = TcpStream::connect(upstream_address).await.unwrap();
        let (mut client_r, mut client_w) = from_client.into_split();
        let (mut server_r, mut server_w) = to_server.into_split();
        let (mut client_to_server, server_to_client) = tokio::join!(
            async {
                let mut captured = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let n = client_r.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    captured.extend_from_slice(&buf[..n]);
                    server_w.write_all(&buf[..n]).await.unwrap();
                }
                server_w.shutdown().await.unwrap();
                captured
            },
            async {
                let mut captured = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let n = server_r.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    captured.extend_from_slice(&buf[..n]);
                    client_w.write_all(&buf[..n]).await.unwrap();
                }
                client_w.shutdown().await.unwrap();
                captured
            }
        );
        assert!(!client_to_server.is_empty() && !server_to_client.is_empty());
        client_to_server.extend_from_slice(&server_to_client);
        client_to_server
    });
    let mut client = connect_authenticated_tcp(&relay_address.to_string(), &selected(&expected))
        .await
        .unwrap();
    client.write_all(&payload).await.unwrap();
    client.shutdown().await.unwrap();
    server.await.unwrap();
    let raw = tokio::time::timeout(Duration::from_secs(5), relay_task)
        .await
        .unwrap()
        .unwrap();
    for secret in [
        sentinel.as_slice(),
        cert.as_slice(),
        expected_spki.as_slice(),
        raw_key.as_slice(),
        target.as_bytes(),
        expected.fingerprint.as_bytes(),
    ] {
        assert!(!raw.windows(secret.len()).any(|window| window == secret));
    }
    let lower = String::from_utf8_lossy(&raw).to_ascii_lowercase();
    for semantic in [
        "werewolf", "fang", "pelt", "pack", "wwp1", "target", "sender", "receiver", "ack",
        "protocol",
    ] {
        assert!(!lower.contains(semantic), "raw TLS contained {semantic}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_production_open_ack_is_absent_from_passive_tcp_wire() {
    use crate::{handshake::test_observer, target_policy::TargetPolicy};
    use ed25519_dalek::{pkcs8::EncodePrivateKey, SigningKey};
    use std::{
        collections::HashMap,
        io::Write,
        process::{Command, Stdio},
    };
    use werewolf_core::pack::{PeerRecord, TrustLevel};

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty() && haystack.windows(needle.len()).any(|part| part == needle)
    }

    fn frame(events: &[test_observer::Event], cmd: &str, phase: test_observer::Phase) -> Vec<u8> {
        let matches: Vec<_> = events
            .iter()
            .filter(|event| {
                event.phase == phase
                    && serde_json::from_slice::<serde_json::Value>(&event.serialized)
                        .unwrap()
                        .get("cmd")
                        .and_then(serde_json::Value::as_str)
                        == Some(cmd)
            })
            .collect();
        assert_eq!(matches.len(), 1, "expected one production {cmd} {phase:?}");
        matches[0].serialized.clone()
    }

    async fn copy_and_capture<R, W>(mut read: R, mut write: W) -> io::Result<Vec<u8>>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut captured = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let count = read.read(&mut buffer).await?;
            if count == 0 {
                write.shutdown().await?;
                return Ok(captured);
            }
            captured.extend_from_slice(&buffer[..count]);
            write.write_all(&buffer[..count]).await?;
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        let mut process = Command::new("sha256sum")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        process.stdin.take().unwrap().write_all(bytes).unwrap();
        let output = process.wait_with_output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned()
    }

    tokio::time::timeout(Duration::from_secs(20), async {
        let sender = generate_identity();
        let receiver = generate_identity();
        let identity =
            Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(&receiver).unwrap());
        let receiver_cert = identity.certificate().as_ref().to_vec();
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target.local_addr().unwrap();
        let target_text = target_addr.to_string();
        let state = Arc::new(Mutex::new(DaemonState {
            pelt: Some(receiver.clone()),
            runtime_tls_identity: Some(identity),
            peers: vec![PeerRecord {
                name: "sender".into(),
                fingerprint: sender.fingerprint.clone(),
                public_key_b64: Some(sender.public_key_b64.clone()),
                address: "tcp://127.0.0.1:1".into(),
                trust: TrustLevel::Packmate,
            }],
            target_policy: TargetPolicy::Grants(HashMap::from([(
                sender.fingerprint.clone(),
                [target_addr].into(),
            )])),
            ..Default::default()
        }));

        let server_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let server_state = state.clone();
        let server =
            tokio::spawn(
                async move { run_fang_listener(&server_port.to_string(), server_state).await },
            );
        // A separate raw connection only establishes listener readiness. It
        // is not the captured application session and carries no OPEN.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if TcpStream::connect(server_port).await.is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = relay.local_addr().unwrap();
        let relay_task = tokio::spawn(async move {
            let (client, _) = relay.accept().await.unwrap();
            let upstream = TcpStream::connect(server_port).await.unwrap();
            let (client_read, client_write) = client.into_split();
            let (server_read, server_write) = upstream.into_split();
            tokio::try_join!(
                copy_and_capture(client_read, server_write),
                copy_and_capture(server_read, client_write),
            )
            .unwrap()
        });

        let (observer, mut events) = test_observer::observe(&receiver.fingerprint);
        let local = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = local.local_addr().unwrap();
        let client_sender = sender.clone();
        let expected = selected(&receiver);
        let client_task = tokio::spawn(async move {
            let (mut inbound, _) = local.accept().await.unwrap();
            pipe_one_fang_connection(
                &mut inbound,
                &relay_addr.to_string(),
                &target_text,
                client_sender,
                expected,
            )
            .await
        });

        let mut sentinel = [0u8; 128];
        OsRng.fill_bytes(&mut sentinel);
        let expected_sentinel = sentinel;
        let target_task = tokio::spawn(async move {
            let (mut stream, _) = target.accept().await.unwrap();
            let mut received = [0u8; 128];
            stream.read_exact(&mut received).await.unwrap();
            stream.write_all(&received).await.unwrap();
            stream.shutdown().await.unwrap();
            received
        });
        let mut app = TcpStream::connect(local_addr).await.unwrap();
        app.write_all(&sentinel).await.unwrap();
        let mut echo = [0u8; 128];
        app.read_exact(&mut echo).await.unwrap();
        assert_eq!(echo, expected_sentinel);
        assert_eq!(target_task.await.unwrap(), expected_sentinel);
        app.shutdown().await.unwrap();
        drop(app);
        assert!(client_task.await.unwrap().is_ok());
        let (client_to_server, server_to_client) = relay_task.await.unwrap();
        server.abort();
        drop(observer);

        let recorded: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        let challenge = frame(&recorded, "fang.tcp.challenge", test_observer::Phase::Write);
        let open = frame(&recorded, "fang.pipe", test_observer::Phase::Write);
        let ack = frame(&recorded, "fang.tcp.ack", test_observer::Phase::Write);
        let challenge_equal =
            challenge == frame(&recorded, "fang.tcp.challenge", test_observer::Phase::Read);
        let open_equal = open == frame(&recorded, "fang.pipe", test_observer::Phase::Read);
        let ack_equal = ack == frame(&recorded, "fang.tcp.ack", test_observer::Phase::Read);
        assert!(challenge_equal && open_equal && ack_equal);
        let parsed_challenge: hs::TcpChallenge = serde_json::from_slice(&challenge).unwrap();
        let parsed_open: hs::TcpOpen = serde_json::from_slice(&open).unwrap();
        let parsed_ack: hs::TcpAck = serde_json::from_slice(&ack).unwrap();
        assert_eq!(parsed_challenge.receiver_fingerprint, receiver.fingerprint);
        assert_eq!(parsed_challenge.challenge, parsed_open.challenge);
        assert_eq!(parsed_challenge.challenge, parsed_ack.challenge);
        assert_eq!(parsed_open.sender_fingerprint, sender.fingerprint);
        assert_eq!(parsed_open.receiver_fingerprint, receiver.fingerprint);
        assert_eq!(parsed_open.remote, target_addr.to_string());
        assert_eq!(parsed_ack.sender_fingerprint, sender.fingerprint);
        assert_eq!(parsed_ack.receiver_fingerprint, receiver.fingerprint);
        assert_eq!(parsed_open.nonce, parsed_ack.nonce);
        assert_eq!(parsed_ack.remote, target_addr.to_string());

        let sender_key = STANDARD.decode(&sender.public_key_b64).unwrap();
        let receiver_key = STANDARD.decode(&receiver.public_key_b64).unwrap();
        let sender_spki =
            crate::tls_identity::canonical_spki_from_public_key_b64(&sender.public_key_b64)
                .unwrap();
        let receiver_spki =
            crate::tls_identity::canonical_spki_from_public_key_b64(&receiver.public_key_b64)
                .unwrap();
        let mut combined = client_to_server.clone();
        combined.extend_from_slice(&server_to_client);
        // The relay stores only ciphertext. Check the two runtime Pelt seeds
        // and their private PKCS#8 encodings in memory without printing them.
        let sender_seed: [u8; 32] = STANDARD
            .decode(&sender.secret_key_b64)
            .unwrap()
            .try_into()
            .unwrap();
        let receiver_seed: [u8; 32] = STANDARD
            .decode(&receiver.secret_key_b64)
            .unwrap()
            .try_into()
            .unwrap();
        let sender_pkcs8 = SigningKey::from_bytes(&sender_seed).to_pkcs8_der().unwrap();
        let receiver_pkcs8 = SigningKey::from_bytes(&receiver_seed)
            .to_pkcs8_der()
            .unwrap();
        let capture_secret_audit = [
            sender_seed.as_slice(),
            receiver_seed.as_slice(),
            sender_pkcs8.as_bytes(),
            receiver_pkcs8.as_bytes(),
            b"CLIENT_TRAFFIC_SECRET".as_slice(),
            b"SERVER_TRAFFIC_SECRET".as_slice(),
        ]
        .into_iter()
        .all(|secret| !contains(&combined, secret));
        assert!(capture_secret_audit);
        let target_search = target_addr.to_string();
        let exact = [
            ("sender_fingerprint", sender.fingerprint.as_bytes()),
            ("receiver_fingerprint", receiver.fingerprint.as_bytes()),
            ("target", target_search.as_bytes()),
            ("sentinel", sentinel.as_slice()),
            ("challenge", challenge.as_slice()),
            ("open", open.as_slice()),
            ("ack", ack.as_slice()),
            ("sender_raw_pelt", sender_key.as_slice()),
            ("receiver_raw_pelt", receiver_key.as_slice()),
            ("sender_spki", sender_spki.as_slice()),
            ("receiver_spki", receiver_spki.as_slice()),
            ("receiver_certificate", receiver_cert.as_slice()),
        ];
        let mut exact_scan = serde_json::Map::new();
        for (name, pattern) in exact {
            let c2s = contains(&client_to_server, pattern);
            let s2c = contains(&server_to_client, pattern);
            let both = contains(&combined, pattern);
            assert!(!c2s && !s2c && !both, "raw TLS contained {name}");
            exact_scan.insert(
                name.into(),
                serde_json::json!({"c2s":c2s,"s2c":s2c,"combined":both}),
            );
        }
        let c2s_lower = client_to_server.to_ascii_lowercase();
        let s2c_lower = server_to_client.to_ascii_lowercase();
        let combined_lower = combined.to_ascii_lowercase();
        let mut semantic_scan = serde_json::Map::new();
        for semantic in [
            "werewolf",
            "fang",
            "pelt",
            "pack",
            "wwp1",
            "sender",
            "receiver",
            "target",
            "protocol",
            "challenge",
            "ack",
            "fang.tcp.challenge",
            "fang.pipe",
            "fang.tcp.ack",
            "fang-tcp-v3",
            "fang-v3-secure",
        ] {
            let pattern = semantic.as_bytes();
            let c2s = contains(&c2s_lower, pattern);
            let s2c = contains(&s2c_lower, pattern);
            let both = contains(&combined_lower, pattern);
            if both && semantic != "ack" {
                panic!("raw TLS contained semantic token {semantic}");
            }
            semantic_scan.insert(
                semantic.into(),
                serde_json::json!({"c2s":c2s,"s2c":s2c,"combined":both}),
            );
        }
        assert!(!client_to_server.is_empty() && !server_to_client.is_empty());
        println!(
            "STAGE12E2_EVIDENCE_JSON={}",
            serde_json::json!({
                "full_production_tcp_open_ack_path":"EXECUTED",
                "transport_used":"TCP_ENCRYPTED_TLS_STAGE10",
                "client_to_server_raw_bytes":client_to_server.len(),
                "server_to_client_raw_bytes":server_to_client.len(),
                "combined_sha256":sha256(&combined),
                "sender_fingerprint":sender.fingerprint,
                "receiver_fingerprint":receiver.fingerprint,
                "target":target_addr.to_string(),
                "sentinel_bytes":sentinel.len(),
                "sentinel_blake3":blake3::hash(&sentinel).to_hex().to_string(),
                "challenge_write_read_equal":challenge_equal,
                "open_write_read_equal":open_equal,
                "ack_write_read_equal":ack_equal,
                "challenge_serialized_bytes":challenge.len(),
                "open_serialized_bytes":open.len(),
                "ack_serialized_bytes":ack.len(),
                "challenge_sha256":sha256(&challenge),
                "open_sha256":sha256(&open),
                "ack_sha256":sha256(&ack),
                "sentinel_round_trip_equal":echo==sentinel,
                "exact_scan":exact_scan,
                "semantic_scan":semantic_scan,
                "capture_secret_audit":if capture_secret_audit {"PASS"} else {"FAIL"},
            })
        );
    })
    .await
    .unwrap();
}
