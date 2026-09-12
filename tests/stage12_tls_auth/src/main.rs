//! Disposable TLS authentication experiment. Nothing here is a production verifier.
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring, verify_tls13_signature, CryptoProvider},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::{ClientHello, ResolvesServerCert},
    sign::{CertifiedKey, Signer, SigningKey},
    ClientConfig, ClientConnection, DigitallySignedStruct, Error, ServerConfig, ServerConnection,
    SignatureAlgorithm, SignatureScheme,
};
use serde_json::json;
use std::{
    io::{Cursor, Read, Write},
    sync::{Arc, Mutex},
};

// RFC 8410 Ed25519 SPKI: absent parameters, zero unused bits, exactly 32 key bytes.
const ED25519_SPKI_PREFIX: &[u8] = &[
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
const CV_CONTEXT: &[u8] = b"TLS 1.3, server CertificateVerify\0";
const SENTINEL: [u8; 100] = [0x57; 100];

#[derive(Debug, Default)]
struct Audit {
    cert_calls: usize,
    cert_result: Option<String>,
    pin_accepted: bool,
    tls12_calls: usize,
    cv_calls: usize,
    cv_context_valid: bool,
    cv_result: Option<String>,
    cv_message: Vec<u8>,
    cv_signature: Vec<u8>,
    signer_calls: usize,
    signer_context_valid: bool,
    signer_message: Vec<u8>,
    signature_before: Vec<u8>,
    signature_after: Vec<u8>,
}

#[derive(Debug)]
struct ExactPeltVerifier {
    expected_spki: Vec<u8>,
    provider: Arc<CryptoProvider>,
    audit: Arc<Mutex<Audit>>,
}

fn valid_cv_context(message: &[u8]) -> bool {
    message.starts_with(&[0x20; 64])
        && message.get(64..64 + CV_CONTEXT.len()) == Some(CV_CONTEXT)
        && matches!(message.len() - (64 + CV_CONTEXT.len()), 32 | 48)
}

impl ServerCertVerifier for ExactPeltVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let mut audit = self.audit.lock().unwrap();
        audit.cert_calls += 1;
        // Parse the actual certificate delivered by rustls. Never search arbitrary DER
        // for a key substring, and never use the fixture name to choose an outcome.
        let cert = match webpki::EndEntityCert::try_from(cert) {
            Ok(cert) => cert,
            Err(error) => {
                audit.cert_result = Some(format!("parse_rejected:{error:?}"));
                return Err(Error::InvalidCertificate(
                    rustls::CertificateError::BadEncoding,
                ));
            }
        };
        let spki = cert.subject_public_key_info();
        let spki = spki.as_ref();
        if spki.len() != 44 || !spki.starts_with(ED25519_SPKI_PREFIX) {
            audit.cert_result = Some("noncanonical_or_non_ed25519_spki".into());
            return Err(Error::InvalidCertificate(
                rustls::CertificateError::BadEncoding,
            ));
        }
        if spki != self.expected_spki {
            audit.cert_result = Some("pin_mismatch".into());
            return Err(Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        audit.cert_result = Some("exact_spki_accepted".into());
        audit.pin_accepted = true;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.audit.lock().unwrap().tls12_calls += 1;
        Err(Error::General(
            "TLS 1.2 is forbidden in this experiment".into(),
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        // This is the real cryptographic verifier, including on negative cases.
        let result = verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        );
        let mut audit = self.audit.lock().unwrap();
        audit.cv_calls += 1;
        audit.cv_context_valid = valid_cv_context(message);
        audit.cv_message = message.to_vec();
        audit.cv_signature = dss.signature().to_vec();
        audit.cv_result = Some(match &result {
            Ok(_) => "verified".into(),
            Err(error) => format!("{error:?}"),
        });
        result
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

#[derive(Debug)]
struct InstrumentedKey {
    actual: Arc<dyn SigningKey>,
    flip_signature: bool,
    audit: Arc<Mutex<Audit>>,
}

impl SigningKey for InstrumentedKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        self.actual.choose_scheme(offered).map(|inner| {
            Box::new(InstrumentedSigner {
                inner,
                flip_signature: self.flip_signature,
                audit: self.audit.clone(),
            }) as Box<dyn Signer>
        })
    }
    fn algorithm(&self) -> SignatureAlgorithm {
        self.actual.algorithm()
    }
}

#[derive(Debug)]
struct InstrumentedSigner {
    inner: Box<dyn Signer>,
    flip_signature: bool,
    audit: Arc<Mutex<Audit>>,
}

impl Signer for InstrumentedSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, Error> {
        let mut signature = self.inner.sign(message)?;
        assert!(
            valid_cv_context(message),
            "fault injection must target CertificateVerify"
        );
        assert_eq!(signature.len(), 64);
        let mut audit = self.audit.lock().unwrap();
        audit.signer_calls += 1;
        audit.signer_context_valid = true;
        audit.signer_message = message.to_vec();
        audit.signature_before = signature.clone();
        if self.flip_signature {
            signature[0] ^= 1;
        }
        audit.signature_after = signature.clone();
        Ok(signature)
    }
    fn scheme(&self) -> SignatureScheme {
        self.inner.scheme()
    }
}

#[derive(Debug)]
struct TestResolver(Arc<CertifiedKey>);
impl ResolvesServerCert for TestResolver {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        // Intentionally permits malicious fixtures a normal server builder rejects.
        Some(self.0.clone())
    }
}

fn certificate(key: &rcgen::KeyPair, name: &str) -> CertificateDer<'static> {
    rcgen::CertificateParams::new(vec![name.to_owned()])
        .unwrap()
        .self_signed(key)
        .unwrap()
        .der()
        .clone()
}

#[derive(Clone, Copy, Debug)]
enum Expected {
    Success,
    PinMismatch,
    BadCertificate,
    BadSpki,
    BadSignature,
}

fn run_case(
    name: &str,
    cert: CertificateDer<'static>,
    signing_key: &rcgen::KeyPair,
    pin: Vec<u8>,
    flip: bool,
    expected: Expected,
) -> serde_json::Value {
    let expected_spki = pin.clone();
    let certificate_sha256 = ::ring::digest::digest(&::ring::digest::SHA256, cert.as_ref());
    let audit = Arc::new(Mutex::new(Audit::default()));
    let provider = Arc::new(ring::default_provider());
    let mut client_config = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ExactPeltVerifier {
            expected_spki: pin,
            provider: provider.clone(),
            audit: audit.clone(),
        }))
        .with_no_client_auth();
    client_config.enable_early_data = false;
    client_config.resumption = rustls::client::Resumption::disabled();
    assert!(client_config.alpn_protocols.is_empty());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
    let actual = provider.key_provider.load_private_key(key_der).unwrap();
    let key = Arc::new(InstrumentedKey {
        actual,
        flip_signature: flip,
        audit: audit.clone(),
    });
    let certified = Arc::new(CertifiedKey::new(vec![cert], key));
    let mut server_config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(TestResolver(certified)));
    server_config.send_tls13_tickets = 0;
    assert!(server_config.alpn_protocols.is_empty());
    let mut client = ClientConnection::new(
        Arc::new(client_config),
        ServerName::from("127.0.0.1".parse::<std::net::IpAddr>().unwrap()),
    )
    .unwrap();
    let mut server = ServerConnection::new(Arc::new(server_config)).unwrap();
    let mut client_error = None;
    let mut server_error = None;
    let mut client_wire = Vec::new();
    let mut server_wire = Vec::new();
    // Carry complete serialized TLS records through byte buffers. No handshake
    // bytes, ciphertext, or records are edited by the transport pump.
    for _ in 0..16 {
        let mut flight = Vec::new();
        while client.wants_write() {
            client.write_tls(&mut flight).unwrap();
        }
        if !flight.is_empty() {
            client_wire.extend_from_slice(&flight);
            server.read_tls(&mut Cursor::new(flight)).unwrap();
            if let Err(error) = server.process_new_packets() {
                server_error = Some(error);
                break;
            }
        }
        let mut flight = Vec::new();
        while server.wants_write() {
            server.write_tls(&mut flight).unwrap();
        }
        if !flight.is_empty() {
            server_wire.extend_from_slice(&flight);
            client.read_tls(&mut Cursor::new(flight)).unwrap();
            if let Err(error) = client.process_new_packets() {
                client_error = Some(error);
                // Deliver the actual fatal alert to the peer as additional evidence.
                let mut alert = Vec::new();
                while client.wants_write() {
                    client.write_tls(&mut alert).unwrap();
                }
                assert!(!alert.is_empty());
                client_wire.extend_from_slice(&alert);
                server.read_tls(&mut Cursor::new(alert)).unwrap();
                server_error = server.process_new_packets().err();
                break;
            }
        }
        if !client.is_handshaking() && !server.is_handshaking() {
            break;
        }
    }
    let client_complete = !client.is_handshaking();
    let server_complete = !server.is_handshaking();
    let mut application_submitted = 0;
    let mut recovered = Vec::new();
    if client_error.is_none() && server_error.is_none() && client_complete && server_complete {
        let evidence = audit.lock().unwrap();
        assert!(evidence.pin_accepted);
        assert_eq!(evidence.cv_result.as_deref(), Some("verified"));
        drop(evidence);
        client.writer().write_all(&SENTINEL).unwrap();
        application_submitted += SENTINEL.len();
        let mut flight = Vec::new();
        while client.wants_write() {
            client.write_tls(&mut flight).unwrap();
        }
        client_wire.extend_from_slice(&flight);
        server.read_tls(&mut Cursor::new(flight)).unwrap();
        server.process_new_packets().unwrap();
    }
    let mut plaintext = [0; 1024];
    loop {
        match server.reader().read(&mut plaintext) {
            Ok(0) => break,
            Ok(n) => recovered.extend_from_slice(&plaintext[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) if server_error.is_some() => break,
            Err(e) => panic!("unexpected plaintext read: {e}"),
        }
    }
    let audit = audit.lock().unwrap();
    assert_eq!(
        audit.cert_calls, 1,
        "{name}: certificate must reach the verifier"
    );
    assert_eq!(audit.tls12_calls, 0);
    assert_eq!(audit.signer_calls, 1);
    assert!(audit.signer_context_valid);
    assert_eq!(server.server_name(), None);
    assert_eq!(client.alpn_protocol(), None);
    assert_eq!(server.alpn_protocol(), None);
    assert_eq!(
        client.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
    // Cross-check generated signatures outside rustls's verifier callback. This
    // also establishes that case B has a valid signature from its own key and
    // that the C2 pre-mutation signature was valid for the pinned identity.
    let signer_public = signing_key.public_key_raw();
    let original_valid_for_signer =
        ::ring::signature::UnparsedPublicKey::new(&::ring::signature::ED25519, signer_public)
            .verify(&audit.signer_message, &audit.signature_before)
            .is_ok();
    assert!(original_valid_for_signer);
    let emitted_valid_for_pin = ::ring::signature::UnparsedPublicKey::new(
        &::ring::signature::ED25519,
        &expected_spki[12..],
    )
    .verify(&audit.signer_message, &audit.signature_after)
    .is_ok();
    let original_valid_for_pin = ::ring::signature::UnparsedPublicKey::new(
        &::ring::signature::ED25519,
        &expected_spki[12..],
    )
    .verify(&audit.signer_message, &audit.signature_before)
    .is_ok();
    if audit.cv_calls > 0 {
        assert_eq!(audit.cv_message, audit.signer_message);
        assert_eq!(audit.cv_signature, audit.signature_after);
    }
    match expected {
        Expected::Success => {
            assert!(original_valid_for_pin && emitted_valid_for_pin);
            assert!(client_error.is_none() && server_error.is_none());
            assert!(client_complete && server_complete);
            assert!(audit.pin_accepted && audit.cv_context_valid);
            assert_eq!(audit.cv_calls, 1);
            assert_eq!(audit.cv_result.as_deref(), Some("verified"));
            assert_eq!(application_submitted, 100);
            assert_eq!(recovered, SENTINEL);
        }
        _ => {
            assert!(client_error.is_some(), "{name}: client must reject");
            assert!(
                matches!(server_error, Some(Error::AlertReceived(_))),
                "{name}: peer must receive fatal alert: {server_error:?}"
            );
            assert!(!client_complete && !server_complete);
            assert_eq!(application_submitted, 0);
            assert!(recovered.is_empty());
            match expected {
                Expected::BadSignature => {
                    assert!(!emitted_valid_for_pin);
                    assert_eq!(original_valid_for_pin, flip);
                    assert!(audit.pin_accepted && audit.cv_context_valid);
                    assert_eq!(audit.cv_calls, 1);
                    assert_eq!(
                        audit.cv_result.as_deref(),
                        Some("InvalidCertificate(BadSignature)")
                    );
                    assert!(matches!(
                        client_error,
                        Some(Error::InvalidCertificate(
                            rustls::CertificateError::BadSignature
                        ))
                    ));
                    assert_eq!(audit.cv_signature, audit.signature_after);
                    if flip {
                        assert_eq!(
                            audit
                                .signature_before
                                .iter()
                                .zip(&audit.signature_after)
                                .filter(|(a, b)| a != b)
                                .count(),
                            1
                        );
                        assert_eq!(audit.signature_before[0] ^ audit.signature_after[0], 1);
                    }
                }
                _ => {
                    assert!(!audit.pin_accepted);
                    assert_eq!(audit.cv_calls, 0);
                    match expected {
                        Expected::PinMismatch => {
                            assert_eq!(audit.cert_result.as_deref(), Some("pin_mismatch"))
                        }
                        Expected::BadCertificate => assert!(audit
                            .cert_result
                            .as_deref()
                            .unwrap()
                            .starts_with("parse_rejected:")),
                        Expected::BadSpki => assert_eq!(
                            audit.cert_result.as_deref(),
                            Some("noncanonical_or_non_ed25519_spki")
                        ),
                        _ => unreachable!(),
                    }
                }
            }
        }
    }
    json!({
        "case": name, "result": "PASS", "expected": format!("{expected:?}"),
        "certificate_verifier_calls": audit.cert_calls, "certificate_result": audit.cert_result,
        "pin_accepted": audit.pin_accepted, "tls12_verifier_calls": audit.tls12_calls,
        "certificate_verify_calls": audit.cv_calls, "certificate_verify_context_valid": audit.cv_context_valid,
        "certificate_verify_result": audit.cv_result,
        "signer_calls": audit.signer_calls, "signer_context_valid": audit.signer_context_valid,
        "signature_bytes_changed": audit.signature_before.iter().zip(&audit.signature_after).filter(|(a,b)| a != b).count(),
        "original_signature_valid_for_signer": original_valid_for_signer,
        "original_signature_valid_for_pin": original_valid_for_pin,
        "emitted_signature_valid_for_pin": emitted_valid_for_pin,
        "received_message_equals_signed": audit.cv_calls > 0 && audit.cv_message == audit.signer_message,
        "received_signature_equals_emitted": audit.cv_calls > 0 && audit.cv_signature == audit.signature_after,
        "certificate_sha256": hex(certificate_sha256.as_ref()),
        "expected_spki_hex": hex(&expected_spki),
        "signer_public_key_hex": hex(signer_public),
        "client_error": client_error.map(|e| format!("{e:?}")),
        "server_error": server_error.map(|e| format!("{e:?}")),
        "client_handshake_complete": client_complete, "server_handshake_complete": server_complete,
        "application_bytes_submitted": application_submitted, "server_plaintext_bytes": recovered.len(),
        "client_wire_bytes": client_wire.len(), "server_wire_bytes": server_wire.len(),
        "tls_version": "TLSv1_3", "sni": server.server_name(), "alpn": client.alpn_protocol(),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn main() {
    let a = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let b = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let ec = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert_a = certificate(&a, "receiver.invalid");
    let cert_b = certificate(&b, "receiver.invalid");
    let pin_a = a.public_key_der();
    assert_ne!(pin_a, b.public_key_der());
    let mut malformed = cert_a.as_ref().to_vec();
    malformed.pop(); // Actual malformed DER, still sent inside a valid TLS Certificate message.
    let cert_ec = certificate(&ec, "receiver.invalid");
    assert!(webpki::EndEntityCert::try_from(&cert_ec).is_ok());
    let mut results = Vec::new();
    for (name, cert, signer, flip, expected) in [
        (
            "A_correct_receiver_correct_key",
            cert_a.clone(),
            &a,
            false,
            Expected::Success,
        ),
        (
            "A2_same_pelt_new_certificate",
            certificate(&a, "different.invalid"),
            &a,
            false,
            Expected::Success,
        ),
        (
            "B_wrong_receiver_valid_signature",
            cert_b,
            &b,
            false,
            Expected::PinMismatch,
        ),
        (
            "C_correct_certificate_wrong_signing_key",
            cert_a.clone(),
            &b,
            false,
            Expected::BadSignature,
        ),
        (
            "C2_certificate_verify_signature_bit_flip",
            cert_a.clone(),
            &a,
            true,
            Expected::BadSignature,
        ),
        (
            "D_malformed_certificate_der",
            CertificateDer::from(malformed),
            &a,
            false,
            Expected::BadCertificate,
        ),
        (
            "E_non_ed25519_certificate",
            cert_ec,
            &a,
            false,
            Expected::BadSpki,
        ),
    ] {
        results.push(run_case(name, cert, signer, pin_a.clone(), flip, expected));
    }
    // Fresh positive after all negatives also checks there is no global poisoned state.
    results.push(run_case(
        "A3_positive_after_negatives",
        cert_a,
        &a,
        pin_a,
        false,
        Expected::Success,
    ));
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "stage": "12B.11", "status": "PASS", "scope": "isolated TLS 1.3 authentication harness",
            "transport": "serialized TLS records over in-memory byte buffers",
            "rustls": "0.23.40", "webpki": "0.103.13", "cases": results,
        }))
        .unwrap()
    );
}
