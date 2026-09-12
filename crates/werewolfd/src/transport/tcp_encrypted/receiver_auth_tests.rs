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
        let mut raw = Vec::new();
        let (client_to_server, server_to_client) = tokio::join!(
            async {
                let mut buf = [0; 4096];
                loop {
                    let n = client_r.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    server_w.write_all(&buf[..n]).await.unwrap();
                }
                server_w.shutdown().await.unwrap();
            },
            async {
                io::copy(&mut server_r, &mut client_w).await.unwrap();
                client_w.shutdown().await.unwrap();
            }
        );
        let _ = (client_to_server, server_to_client);
        raw
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
