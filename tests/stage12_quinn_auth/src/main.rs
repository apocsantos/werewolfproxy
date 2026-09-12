//! Stage 12C.1: isolated real-UDP Quinn receiver-authentication proof.
mod passive;
mod verifier;

use base64::{engine::general_purpose::STANDARD, Engine};
use quinn::crypto::rustls::{HandshakeData, QuicClientConfig, QuicServerConfig};
use ring::rand::{SecureRandom, SystemRandom};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::UdpSocket, sync::oneshot, time::timeout};
use werewolf_core::{
    pack::{PeerRecord, TrustLevel},
    pelt,
};

const WINDOW: Duration = Duration::from_secs(8);
const EXPORTER_LABEL: &[u8] = b"EXPORTER-WerewolfProxy-Fang-QUIC-v3";
const EXPORTER_CONTEXT: &[u8] = b"werewolfproxy/fang-quic-v3";
const SPKI_PREFIX: &[u8] = &[
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0,
];
const PKCS8_PREFIX: &[u8] = &[
    0x30, 0x2e, 0x02, 0x01, 0, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

struct Identity {
    pelt: pelt::PeltIdentity,
    raw32: Vec<u8>,
    spki: Vec<u8>,
    pkcs8: Vec<u8>,
    cert: CertificateDer<'static>,
}

impl Identity {
    fn generate() -> Self {
        let pelt = pelt::generate_identity();
        werewolf_core::state_validation::identity(&pelt).unwrap();
        let seed = STANDARD.decode(&pelt.secret_key_b64).unwrap();
        let raw32 = STANDARD.decode(&pelt.public_key_b64).unwrap();
        assert_eq!(seed.len(), 32);
        assert_eq!(raw32.len(), 32);
        let pkcs8 = [PKCS8_PREFIX, &seed].concat();
        let key = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
            &PrivatePkcs8KeyDer::from(pkcs8.clone()),
            &rcgen::PKCS_ED25519,
        )
        .unwrap();
        let spki = [SPKI_PREFIX, &raw32].concat();
        assert_eq!(key.public_key_der(), spki);
        let cert = rcgen::CertificateParams::new(vec!["runtime.invalid".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap()
            .der()
            .clone();
        let parsed = webpki::EndEntityCert::try_from(&cert).unwrap();
        assert_eq!(parsed.subject_public_key_info().as_ref(), spki);
        let challenge = b"stage12c-disposable-pack-identity-possession";
        let signature = pelt::sign_message(&pelt, challenge).unwrap();
        pelt::verify_message(&pelt.public_key_b64, challenge, &signature).unwrap();
        Self {
            pelt,
            raw32,
            spki,
            pkcs8,
            cert,
        }
    }

    fn public_evidence(&self) -> Value {
        json!({"fingerprint": self.pelt.fingerprint, "raw32_hex": hex(&self.raw32),
            "canonical_spki_hex": hex(&self.spki), "certificate_der_hex": hex(&self.cert),
            "pelt_possession_selfcheck": "PASS", "certificate_spki_matches_pelt": true})
    }
}

struct Sentinels {
    open: Vec<u8>,
    target: Vec<u8>,
    sender: Vec<u8>,
    payload: Vec<u8>,
}

impl Sentinels {
    fn new() -> Self {
        let mut random = [0; 32];
        SystemRandom::new().fill(&mut random).unwrap();
        let token = hex(&random);
        let open = format!("OPEN-{token}").into_bytes();
        let target = format!("target-{token}.invalid:443").into_bytes();
        let sender = format!("sender-identity-{token}").into_bytes();
        let payload = [&open[..], b"\n", &target, b"\n", &sender, b"\n"].concat();
        Self {
            open,
            target,
            sender,
            payload,
        }
    }
}

#[derive(Clone)]
pub struct Datagram {
    pub direction: &'static str,
    pub bytes: Vec<u8>,
}

async fn relay(
    socket: UdpSocket,
    server: SocketAddr,
    mut stop: oneshot::Receiver<()>,
) -> Vec<Datagram> {
    let mut client = None;
    let mut capture = Vec::new();
    let mut buffer = vec![0; 65536];
    loop {
        tokio::select! {
            _ = &mut stop => break,
            received = socket.recv_from(&mut buffer) => {
                let (len, source) = received.unwrap();
                let (destination, direction) = if source == server {
                    (client.expect("client must send first"), "server_to_client")
                } else {
                    if let Some(existing) = client { assert_eq!(existing, source); }
                    client = Some(source);
                    (server, "client_to_server")
                };
                capture.push(Datagram { direction, bytes: buffer[..len].to_vec() });
                assert_eq!(socket.send_to(&buffer[..len], destination).await.unwrap(), len);
            }
        }
    }
    capture
}

struct ServerObservation {
    error: Option<String>,
    streams: usize,
    payloads: Vec<Vec<u8>>,
    exporter: Option<[u8; 32]>,
    sni: Option<String>,
    alpn: Option<Vec<u8>>,
}

async fn observe_server(endpoint: quinn::Endpoint) -> ServerObservation {
    let incoming = timeout(WINDOW, endpoint.accept()).await.unwrap().unwrap();
    let mut observation = ServerObservation {
        error: None,
        streams: 0,
        payloads: Vec::new(),
        exporter: None,
        sni: None,
        alpn: None,
    };
    // Normal handshake await only: never accept an early connection/0-RTT.
    let connection = match timeout(WINDOW, incoming).await.unwrap() {
        Ok(connection) => connection,
        Err(error) => {
            observation.error = Some(format!("{error:?}"));
            return observation;
        }
    };
    let metadata = connection
        .handshake_data()
        .unwrap()
        .downcast::<HandshakeData>()
        .unwrap();
    observation.sni = metadata.server_name;
    observation.alpn = metadata.protocol;
    let mut exporter = [0; 32];
    connection
        .export_keying_material(&mut exporter, EXPORTER_LABEL, EXPORTER_CONTEXT)
        .unwrap();
    observation.exporter = Some(exporter);
    loop {
        match timeout(WINDOW, connection.accept_bi()).await.unwrap() {
            Ok((mut send, mut recv)) => {
                observation.streams += 1;
                let payload = timeout(WINDOW, recv.read_to_end(8192))
                    .await
                    .unwrap()
                    .unwrap();
                observation.payloads.push(payload.clone());
                timeout(WINDOW, send.write_all(&payload))
                    .await
                    .unwrap()
                    .unwrap();
                send.finish().unwrap();
            }
            Err(error) => {
                observation.error = Some(format!("{error:?}"));
                break;
            }
        }
    }
    observation
}

fn server_config(identity: &Identity) -> quinn::ServerConfig {
    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![identity.cert.clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.pkcs8.clone())),
    )
    .unwrap();
    tls.alpn_protocols = Vec::new();
    tls.max_early_data_size = 0;
    tls.send_tls13_tickets = 0;
    tls.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    assert!(!tls.ticketer.enabled());
    assert!(!tls.session_storage.can_cache());
    assert_eq!(tls.max_early_data_size, 0);
    assert_eq!(tls.send_tls13_tickets, 0);
    quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls).unwrap()))
}

async fn run_case(
    name: &str,
    expected: &Identity,
    actual: &Identity,
    should_accept: bool,
) -> Value {
    let sentinel = Sentinels::new();
    let (verifier, probe) = verifier::make(expected.spki.clone());
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .dangerous()
    .with_custom_certificate_verifier(verifier)
    .with_no_client_auth();
    tls.alpn_protocols = Vec::new();
    tls.enable_early_data = false;
    tls.resumption = rustls::client::Resumption::disabled();
    assert!(!tls.enable_early_data);
    let client_config =
        quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls).unwrap()));

    let server = network(quinn::Endpoint::server(
        server_config(actual),
        "127.0.0.1:0".parse().unwrap(),
    ));
    let relay_socket = network(UdpSocket::bind("127.0.0.1:0").await);
    let relay_address = relay_socket.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let relay_task = tokio::spawn(relay(relay_socket, server.local_addr().unwrap(), stopped));
    let observer_task = tokio::spawn(observe_server(server.clone()));
    let mut client = network(quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()));
    client.set_default_client_config(client_config);

    let mut events = vec!["normal_connecting_await_started"];
    let mut application_stream_open_count = 0;
    let mut client_error = None;
    let mut client_exporter = None;
    let mut recovered = Vec::new();
    let mut client_alpn = None;
    // No into_0rtt call. The only application stream creation is below this await.
    match timeout(WINDOW, client.connect(relay_address, "127.0.0.1").unwrap())
        .await
        .unwrap()
    {
        Ok(connection) => {
            events.push("normal_connecting_await_authenticated");
            assert!(probe.authenticated());
            let metadata = connection
                .handshake_data()
                .unwrap()
                .downcast::<HandshakeData>()
                .unwrap();
            client_alpn = metadata.protocol;
            let mut exporter = [0; 32];
            connection
                .export_keying_material(&mut exporter, EXPORTER_LABEL, EXPORTER_CONTEXT)
                .unwrap();
            client_exporter = Some(exporter);
            events.push("application_stream_open_attempt");
            application_stream_open_count += 1;
            let (mut send, mut recv) = timeout(WINDOW, connection.open_bi())
                .await
                .unwrap()
                .unwrap();
            events.push("application_stream_opened");
            timeout(WINDOW, send.write_all(&sentinel.payload))
                .await
                .unwrap()
                .unwrap();
            events.push("application_payload_submitted");
            send.finish().unwrap();
            recovered = timeout(WINDOW, recv.read_to_end(8192))
                .await
                .unwrap()
                .unwrap();
            connection.close(0u32.into(), b"");
        }
        Err(error) => {
            events.push("normal_connecting_await_rejected");
            client_error = Some(format!("{error:?}"));
        }
    }
    let observed = timeout(WINDOW, observer_task).await.unwrap().unwrap();
    client.close(0u32.into(), b"");
    server.close(0u32.into(), b"");
    timeout(WINDOW, client.wait_idle()).await.unwrap();
    timeout(WINDOW, server.wait_idle()).await.unwrap();
    stop.send(()).unwrap();
    let capture = relay_task.await.unwrap();

    let evidence = probe.evidence();
    let exporter_match = client_exporter.is_some() && client_exporter == observed.exporter;
    let count = |needle: &[u8]| {
        observed
            .payloads
            .iter()
            .map(|p| occurrences(p, needle))
            .sum::<usize>()
    };
    if should_accept {
        assert!(client_error.is_none());
        assert_eq!(application_stream_open_count, 1);
        assert_eq!(observed.streams, 1);
        assert_eq!(observed.payloads, vec![sentinel.payload.clone()]);
        assert_eq!(recovered, sentinel.payload);
        assert!(exporter_match);
        assert_eq!(
            events,
            vec![
                "normal_connecting_await_started",
                "normal_connecting_await_authenticated",
                "application_stream_open_attempt",
                "application_stream_opened",
                "application_payload_submitted"
            ]
        );
    } else {
        assert!(client_error.is_some());
        assert_eq!(evidence["certificate_calls"], 1);
        assert_eq!(evidence["certificate_result"], "pin_mismatch");
        assert_eq!(evidence["pin_accepted"], false);
        assert_eq!(evidence["certificate_verify_calls"], 0);
        assert_eq!(application_stream_open_count, 0);
        assert_eq!(observed.streams, 0);
        assert!(observed.payloads.is_empty());
        assert_eq!(count(&sentinel.open), 0);
        assert_eq!(count(&sentinel.target), 0);
        assert_eq!(count(&sentinel.sender), 0);
        assert_eq!(
            events,
            vec![
                "normal_connecting_await_started",
                "normal_connecting_await_rejected"
            ]
        );
    }
    assert_eq!(client_alpn, None);
    assert_eq!(observed.alpn, None);
    assert_eq!(observed.sni, None);
    assert_eq!(evidence["tls12_calls"], 0);
    let passive = passive::inspect(&capture);
    assert_eq!(passive["client_hello"]["sni"], Value::Null);
    assert_eq!(passive["client_hello"]["alpn_hex"], json!([]));
    assert_eq!(passive["client_hello"]["pre_shared_key_present"], false);
    assert_eq!(passive["client_hello"]["early_data_present"], false);
    json!({
        "case": name, "result": "PASS", "client_error": client_error,
        "server_terminal_error": observed.error, "verifier": evidence,
        "application_stream_open_count": application_stream_open_count,
        "server_application_stream_count": observed.streams,
        "server_application_bytes": observed.payloads.iter().map(Vec::len).sum::<usize>(),
        "server_application_payloads_hex": observed.payloads.iter().map(|p| hex(p)).collect::<Vec<_>>(),
        "open_sentinel_count": count(&sentinel.open), "target_sentinel_count": count(&sentinel.target),
        "sender_id_sentinel_count": count(&sentinel.sender),
        "client_events": events,
        "exact_sentinel_recovered": should_accept && recovered == sentinel.payload,
        "sentinels": {"open_hex": hex(&sentinel.open), "target_hex": hex(&sentinel.target),
            "sender_id_hex": hex(&sentinel.sender), "application_hex": hex(&sentinel.payload)},
        "authorization_data_in_0rtt": "NO", "resumed_path_used": false,
        "client_resumption": "DISABLED", "server_tickets": "DISABLED",
        "server_session_storage": "NoServerSessionStorage", "server_ticketer_enabled": false,
        "client_enable_early_data": false, "server_max_early_data_size": 0,
        "normal_connecting_await_only": true,
        "client_alpn": client_alpn, "server_alpn": observed.alpn, "server_sni": observed.sni,
        "exporter_client_server_match": if should_accept { json!(exporter_match) } else { Value::Null },
        "raw_datagram_byte_scan": scan(&capture, actual, expected, &sentinel),
        "passive_initial_decryption": passive,
        "datagrams": capture.iter().map(|d| json!({"direction": d.direction, "hex": hex(&d.bytes)})).collect::<Vec<_>>(),
    })
}

fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn scan(
    capture: &[Datagram],
    identity: &Identity,
    expected: &Identity,
    sentinel: &Sentinels,
) -> Value {
    let search = |needle: &[u8], insensitive: bool| {
        let mut hits = Vec::new();
        for (datagram, item) in capture.iter().enumerate() {
            for (offset, window) in item.bytes.windows(needle.len()).enumerate() {
                if if insensitive {
                    window.eq_ignore_ascii_case(needle)
                } else {
                    window == needle
                } {
                    hits.push(json!({"datagram": datagram, "direction": item.direction, "offset": offset}));
                }
            }
        }
        json!({"result": if hits.is_empty() {"NOT_FOUND"} else {"FOUND"}, "hits": hits})
    };
    let mut semantic = serde_json::Map::new();
    for word in [
        "werewolf", "fang", "pelt", "pack", "wwp1", "target", "sender", "receiver", "ack",
        "protocol",
    ] {
        semantic.insert(word.into(), search(word.as_bytes(), true));
    }
    let mut exact = serde_json::Map::new();
    for (name, bytes) in [
        ("test_fingerprint", identity.pelt.fingerprint.as_bytes()),
        ("target", &sentinel.target),
        ("application_sentinel", &sentinel.payload),
        ("open_sentinel", &sentinel.open),
        ("sender_id_sentinel", &sentinel.sender),
        ("pelt_raw32", &identity.raw32),
        ("certificate_der", identity.cert.as_ref()),
        ("spki_der", &identity.spki),
        (
            "expected_receiver_fingerprint",
            expected.pelt.fingerprint.as_bytes(),
        ),
        ("expected_receiver_raw32", &expected.raw32),
        ("expected_receiver_spki_der", &expected.spki),
        ("expected_receiver_certificate_der", expected.cert.as_ref()),
        ("exporter_label", EXPORTER_LABEL),
        ("exporter_context", EXPORTER_CONTEXT),
    ] {
        exact.insert(name.into(), search(bytes, false));
    }
    json!({"semantic_case_insensitive": semantic, "exact": exact, "datagram_count": capture.len(),
        "total_udp_payload_bytes": capture.iter().map(|d| d.bytes.len()).sum::<usize>()})
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn network<T>(result: std::io::Result<T>) -> T {
    match result {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            println!(
                "{}",
                json!({"stage": "12C.1", "status": "BLOCKED",
                "blocker": "QUINN_NETWORK_ENVIRONMENT_BLOCKED", "error": error.to_string()})
            );
            std::process::exit(2);
        }
        Err(error) => panic!("network error: {error}"),
    }
}

fn udp_probe() -> std::io::Result<()> {
    let receiver = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let sender = std::net::UdpSocket::bind("127.0.0.1:0")?;
    receiver.set_read_timeout(Some(Duration::from_secs(2)))?;
    sender.send_to(b"stage12c-udp-probe", receiver.local_addr()?)?;
    let mut data = [0; 64];
    let (len, _) = receiver.recv_from(&mut data)?;
    assert_eq!(&data[..len], b"stage12c-udp-probe");
    Ok(())
}

#[tokio::main]
async fn main() {
    network(udp_probe());
    let a = Identity::generate();
    let b = Identity::generate();
    let c = Identity::generate();
    assert_ne!(a.spki, b.spki);
    assert_ne!(a.spki, c.spki);
    assert_ne!(b.spki, c.spki);
    // Both A and C are valid independently authenticated Pelt/Pack records.
    // Selection of A supplies exactly A's SPKI to the verifier, never this list.
    let pack: Vec<_> = [(&a, "A"), (&c, "C")]
        .into_iter()
        .map(|(identity, name)| PeerRecord {
            name: name.into(),
            fingerprint: identity.pelt.fingerprint.clone(),
            address: "quic://127.0.0.1:4433".into(),
            trust: TrustLevel::Packmate,
            public_key_b64: Some(identity.pelt.public_key_b64.clone()),
        })
        .collect();
    werewolf_core::state_validation::pack(&pack).unwrap();
    let results = vec![
        run_case("A_correct_receiver", &a, &a, true).await,
        run_case("B_wrong_receiver", &a, &b, false).await,
        run_case("C_other_pack_peer", &a, &c, false).await,
    ];
    println!("{}", serde_json::to_string_pretty(&json!({
        "stage": "12C.1", "status": "COMPLETE", "udp_environment": "PASS",
        "quinn": "0.11.9", "rustls": "0.23.40", "quinn_proto": "0.11.15", "quinn_udp": "0.5.14",
        "quic_no_alpn": "PASS", "fallback_temporary_alpn": null,
        "exporter_label": String::from_utf8_lossy(EXPORTER_LABEL),
        "exporter_context": String::from_utf8_lossy(EXPORTER_CONTEXT),
        "identities": {"A": a.public_evidence(), "B": b.public_evidence(), "C": c.public_evidence()},
        "pack_records_validated": pack, "cases": results,
    })).unwrap());
}
