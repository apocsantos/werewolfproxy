use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{
    pkcs8::{EncodePrivateKey, EncodePublicKey},
    SigningKey, VerifyingKey,
};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring, CryptoProvider},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, Error as RustlsError, ServerConfig, SignatureScheme,
};
use std::{error::Error, fmt, sync::Arc};
use werewolf_core::{pack::PeerRecord, pelt::PeltIdentity};

/// Failures are deliberately categorical: no variant carries identity secrets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TlsIdentityError {
    MissingPeerPublicKey,
    InvalidPeerPublicKey,
    InvalidPeltIdentity,
    CertificateGeneration,
    PeltCertificateInvariantMismatch,
    MalformedCertificate,
    SpkiMismatch,
    TlsConfiguration,
}

impl fmt::Display for TlsIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingPeerPublicKey => "selected peer has no full public key",
            Self::InvalidPeerPublicKey => "selected peer public key is invalid",
            Self::InvalidPeltIdentity => "local Pelt identity is invalid",
            Self::CertificateGeneration => "runtime TLS certificate generation failed",
            Self::PeltCertificateInvariantMismatch => {
                "runtime certificate does not contain the local Pelt key"
            }
            Self::MalformedCertificate => "peer certificate is malformed",
            Self::SpkiMismatch => "peer certificate SPKI does not match the selected peer",
            Self::TlsConfiguration => "hardened TLS configuration failed",
        })
    }
}

impl Error for TlsIdentityError {}

/// Ephemeral TLS representation of the daemon's existing Pelt identity.
///
/// This type has no `Debug` implementation so its PKCS#8 bytes cannot be
/// accidentally formatted. Construct it once at daemon startup and retain it
/// only in memory. The Pelt key remains the identity root; no TLS identity is
/// independently generated or persisted.
pub(crate) struct RuntimeTlsIdentity {
    certificate: CertificateDer<'static>,
    private_key: PrivateKeyDer<'static>,
}

impl RuntimeTlsIdentity {
    pub(crate) fn from_pelt(identity: &PeltIdentity) -> Result<Self, TlsIdentityError> {
        werewolf_core::state_validation::identity(identity)
            .map_err(|_| TlsIdentityError::InvalidPeltIdentity)?;

        let secret: [u8; 32] = STANDARD
            .decode(&identity.secret_key_b64)
            .map_err(|_| TlsIdentityError::InvalidPeltIdentity)?
            .try_into()
            .map_err(|_| TlsIdentityError::InvalidPeltIdentity)?;
        let public: [u8; 32] = STANDARD
            .decode(&identity.public_key_b64)
            .map_err(|_| TlsIdentityError::InvalidPeltIdentity)?
            .try_into()
            .map_err(|_| TlsIdentityError::InvalidPeltIdentity)?;
        let signing_key = SigningKey::from_bytes(&secret);
        if signing_key.verifying_key().to_bytes() != public {
            return Err(TlsIdentityError::InvalidPeltIdentity);
        }

        let expected_spki = canonical_spki(&public)?;
        let pkcs8 = signing_key
            .to_pkcs8_der()
            .map_err(|_| TlsIdentityError::CertificateGeneration)?;
        let key_pair_der = PrivatePkcs8KeyDer::from(pkcs8.as_bytes().to_vec());
        let key_pair =
            rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&key_pair_der, &rcgen::PKCS_ED25519)
                .map_err(|_| TlsIdentityError::CertificateGeneration)?;
        if key_pair.public_key_der() != expected_spki {
            return Err(TlsIdentityError::PeltCertificateInvariantMismatch);
        }

        // Empty subject, issuer and SAN form a neutral key-container profile.
        // No fingerprint, peer name, host name, role, or project term enters it.
        let mut parameters = rcgen::CertificateParams::new(Vec::<String>::new())
            .map_err(|_| TlsIdentityError::CertificateGeneration)?;
        parameters.distinguished_name = rcgen::DistinguishedName::new();
        let certificate = parameters
            .self_signed(&key_pair)
            .map_err(|_| TlsIdentityError::CertificateGeneration)?
            .der()
            .clone();

        let actual_spki = leaf_spki(&certificate)?;
        if actual_spki != expected_spki {
            return Err(TlsIdentityError::PeltCertificateInvariantMismatch);
        }

        Ok(Self {
            certificate,
            private_key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.as_bytes().to_vec())),
        })
    }

    pub(crate) fn certificate(&self) -> &CertificateDer<'static> {
        &self.certificate
    }
}

/// Produce the canonical Ed25519 SubjectPublicKeyInfo for a full Pack key.
/// Fingerprints are display/index material and never enter this trust decision.
pub(crate) fn canonical_spki_from_public_key_b64(
    public_key_b64: &str,
) -> Result<Vec<u8>, TlsIdentityError> {
    let decoded = STANDARD
        .decode(public_key_b64)
        .map_err(|_| TlsIdentityError::InvalidPeerPublicKey)?;
    let public: [u8; 32] = decoded
        .try_into()
        .map_err(|_| TlsIdentityError::InvalidPeerPublicKey)?;
    if STANDARD.encode(public) != public_key_b64 {
        return Err(TlsIdentityError::InvalidPeerPublicKey);
    }
    canonical_spki(&public)
}

fn canonical_spki(public: &[u8; 32]) -> Result<Vec<u8>, TlsIdentityError> {
    VerifyingKey::from_bytes(public)
        .map_err(|_| TlsIdentityError::InvalidPeerPublicKey)?
        .to_public_key_der()
        .map_err(|_| TlsIdentityError::InvalidPeerPublicKey)
        .map(|document| document.as_bytes().to_vec())
}

pub(crate) fn expected_spki_for_peer(peer: &PeerRecord) -> Result<Vec<u8>, TlsIdentityError> {
    let public_key = peer
        .public_key_b64
        .as_deref()
        .ok_or(TlsIdentityError::MissingPeerPublicKey)?;
    canonical_spki_from_public_key_b64(public_key)
}

fn leaf_spki(certificate: &CertificateDer<'_>) -> Result<Vec<u8>, TlsIdentityError> {
    let parsed = webpki::EndEntityCert::try_from(certificate)
        .map_err(|_| TlsIdentityError::MalformedCertificate)?;
    Ok(parsed.subject_public_key_info().as_ref().to_vec())
}

fn require_leaf_spki(
    certificate: &CertificateDer<'_>,
    expected_spki: &[u8],
) -> Result<(), TlsIdentityError> {
    if leaf_spki(certificate)? == expected_spki {
        Ok(())
    } else {
        Err(TlsIdentityError::SpkiMismatch)
    }
}

/// Trusts exactly one selected Pack peer's canonical SPKI. It has no roots,
/// hostname policy, TOFU state, or fallback list of Pack identities.
struct ExactPeerSpkiVerifier {
    expected_spki: Vec<u8>,
    provider: Arc<CryptoProvider>,
}

impl fmt::Debug for ExactPeerSpkiVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExactPeerSpkiVerifier")
            .finish_non_exhaustive()
    }
}

impl ServerCertVerifier for ExactPeerSpkiVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        match require_leaf_spki(end_entity, &self.expected_spki) {
            Ok(()) => Ok(ServerCertVerified::assertion()),
            Err(TlsIdentityError::MalformedCertificate) => Err(RustlsError::InvalidCertificate(
                rustls::CertificateError::BadEncoding,
            )),
            Err(TlsIdentityError::SpkiMismatch) => Err(RustlsError::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            )),
            Err(_) => Err(RustlsError::General(
                "unexpected exact-SPKI verification failure".into(),
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _certificate: &CertificateDer<'_>,
        _signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Err(RustlsError::General(
            "TLS 1.2 is disabled for Pelt authentication".into(),
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            certificate,
            signed,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

pub(crate) fn client_config_for_peer(peer: &PeerRecord) -> Result<ClientConfig, TlsIdentityError> {
    let expected_spki = expected_spki_for_peer(peer)?;
    client_config_for_spki(expected_spki)
}

fn client_config_for_spki(expected_spki: Vec<u8>) -> Result<ClientConfig, TlsIdentityError> {
    let provider = provider();
    let verifier = Arc::new(ExactPeerSpkiVerifier {
        expected_spki,
        provider: provider.clone(),
    });
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| TlsIdentityError::TlsConfiguration)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.resumption = rustls::client::Resumption::disabled();
    config.enable_early_data = false;
    config.alpn_protocols.clear();
    Ok(config)
}

pub(crate) fn server_config(
    identity: &RuntimeTlsIdentity,
) -> Result<ServerConfig, TlsIdentityError> {
    let mut config = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| TlsIdentityError::TlsConfiguration)?
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.certificate.clone()],
            identity.private_key.clone_key(),
        )
        .map_err(|_| TlsIdentityError::TlsConfiguration)?;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    config.max_early_data_size = 0;
    config.alpn_protocols.clear();
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{
        server::{ClientHello, ResolvesServerCert},
        sign::{CertifiedKey, Signer, SigningKey as RustlsSigningKey},
        ClientConnection, ProtocolVersion, ServerConnection, SignatureAlgorithm,
    };
    use std::{io::Cursor, net::IpAddr, sync::Arc, time::Duration};
    use werewolf_core::{pack::TrustLevel, pelt};

    fn identity() -> (PeltIdentity, RuntimeTlsIdentity) {
        let pelt = pelt::generate_identity();
        let tls = RuntimeTlsIdentity::from_pelt(&pelt).unwrap();
        (pelt, tls)
    }

    fn peer(pelt: &PeltIdentity, public_key_b64: Option<String>) -> PeerRecord {
        PeerRecord {
            name: "selected-peer".into(),
            fingerprint: pelt.fingerprint.clone(),
            address: "quic://127.0.0.1:4433".into(),
            trust: TrustLevel::Packmate,
            public_key_b64,
        }
    }

    fn verifier(public_key_b64: &str) -> ExactPeerSpkiVerifier {
        ExactPeerSpkiVerifier {
            expected_spki: canonical_spki_from_public_key_b64(public_key_b64).unwrap(),
            provider: provider(),
        }
    }

    fn verify_certificate(
        verifier: &ExactPeerSpkiVerifier,
        certificate: &CertificateDer<'_>,
    ) -> Result<ServerCertVerified, RustlsError> {
        verifier.verify_server_cert(
            certificate,
            &[],
            &ServerName::from("127.0.0.1".parse::<IpAddr>().unwrap()),
            &[],
            UnixTime::since_unix_epoch(Duration::ZERO),
        )
    }

    #[test]
    fn a_pelt_certificate_spki_equals_canonical_pelt_spki() {
        let (pelt, tls) = identity();
        assert_eq!(
            leaf_spki(tls.certificate()).unwrap(),
            canonical_spki_from_public_key_b64(&pelt.public_key_b64).unwrap()
        );
    }

    #[test]
    fn b_different_pelts_have_different_spki() {
        let a = pelt::generate_identity();
        let b = pelt::generate_identity();
        assert_ne!(
            canonical_spki_from_public_key_b64(&a.public_key_b64).unwrap(),
            canonical_spki_from_public_key_b64(&b.public_key_b64).unwrap()
        );
    }

    #[test]
    fn c_expected_pelt_certificate_is_accepted() {
        let (pelt, tls) = identity();
        assert!(verify_certificate(&verifier(&pelt.public_key_b64), tls.certificate()).is_ok());
    }

    #[test]
    fn d_different_pelt_certificate_is_rejected() {
        let (expected, _) = identity();
        let (_, actual) = identity();
        assert!(matches!(
            verify_certificate(&verifier(&expected.public_key_b64), actual.certificate()),
            Err(RustlsError::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure
            ))
        ));
    }

    #[test]
    fn e_same_neutral_profile_with_different_spki_is_rejected() {
        let (expected, expected_tls) = identity();
        let (_, other_tls) = identity();
        assert_ne!(expected_tls.certificate(), other_tls.certificate());
        assert_eq!(
            require_leaf_spki(
                other_tls.certificate(),
                &canonical_spki_from_public_key_b64(&expected.public_key_b64).unwrap()
            ),
            Err(TlsIdentityError::SpkiMismatch)
        );
    }

    #[test]
    fn f_malformed_certificate_is_rejected() {
        let (pelt, tls) = identity();
        let mut malformed = tls.certificate().as_ref().to_vec();
        malformed.pop();
        let malformed = CertificateDer::from(malformed);
        assert_eq!(
            require_leaf_spki(
                &malformed,
                &canonical_spki_from_public_key_b64(&pelt.public_key_b64).unwrap()
            ),
            Err(TlsIdentityError::MalformedCertificate)
        );
        assert!(matches!(
            verify_certificate(&verifier(&pelt.public_key_b64), &malformed),
            Err(RustlsError::InvalidCertificate(
                rustls::CertificateError::BadEncoding
            ))
        ));
    }

    #[test]
    fn g_non_ed25519_certificate_is_rejected() {
        let expected = pelt::generate_identity();
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut parameters = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        parameters.distinguished_name = rcgen::DistinguishedName::new();
        let certificate = parameters.self_signed(&key).unwrap().der().clone();
        assert_eq!(
            require_leaf_spki(
                &certificate,
                &canonical_spki_from_public_key_b64(&expected.public_key_b64).unwrap()
            ),
            Err(TlsIdentityError::SpkiMismatch)
        );
    }

    #[derive(Debug)]
    struct InstrumentedKey {
        actual: Arc<dyn RustlsSigningKey>,
        flip_signature: bool,
    }

    impl RustlsSigningKey for InstrumentedKey {
        fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
            self.actual.choose_scheme(offered).map(|inner| {
                Box::new(InstrumentedSigner {
                    inner,
                    flip_signature: self.flip_signature,
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
    }

    impl Signer for InstrumentedSigner {
        fn sign(&self, message: &[u8]) -> Result<Vec<u8>, RustlsError> {
            const CONTEXT: &[u8] = b"TLS 1.3, server CertificateVerify\0";
            assert!(message.starts_with(&[0x20; 64]));
            assert_eq!(message.get(64..64 + CONTEXT.len()), Some(CONTEXT));
            let mut signature = self.inner.sign(message)?;
            if self.flip_signature {
                signature[0] ^= 1;
            }
            Ok(signature)
        }

        fn scheme(&self) -> SignatureScheme {
            self.inner.scheme()
        }
    }

    #[derive(Debug)]
    struct Resolver(Arc<CertifiedKey>);

    impl ResolvesServerCert for Resolver {
        fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            Some(self.0.clone())
        }
    }

    fn server_with_signer(
        certificate: CertificateDer<'static>,
        signer: &RuntimeTlsIdentity,
        flip_signature: bool,
    ) -> ServerConfig {
        server_with_signer_and_versions(
            certificate,
            signer,
            flip_signature,
            &[&rustls::version::TLS13],
        )
    }

    fn server_with_signer_and_versions(
        certificate: CertificateDer<'static>,
        signer: &RuntimeTlsIdentity,
        flip_signature: bool,
        versions: &[&'static rustls::SupportedProtocolVersion],
    ) -> ServerConfig {
        let provider = provider();
        let actual = provider
            .key_provider
            .load_private_key(signer.private_key.clone_key())
            .unwrap();
        let certified_key = Arc::new(CertifiedKey::new(
            vec![certificate],
            Arc::new(InstrumentedKey {
                actual,
                flip_signature,
            }),
        ));
        ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(Resolver(certified_key)))
    }

    fn client_with_versions(
        public_key_b64: &str,
        versions: &[&'static rustls::SupportedProtocolVersion],
    ) -> ClientConfig {
        let provider = provider();
        ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(versions)
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(ExactPeerSpkiVerifier {
                expected_spki: canonical_spki_from_public_key_b64(public_key_b64).unwrap(),
                provider,
            }))
            .with_no_client_auth()
    }

    fn handshake(
        client_config: ClientConfig,
        server_config: ServerConfig,
    ) -> Result<(), RustlsError> {
        let mut client = ClientConnection::new(
            Arc::new(client_config),
            ServerName::from("127.0.0.1".parse::<IpAddr>().unwrap()),
        )
        .unwrap();
        let mut server = ServerConnection::new(Arc::new(server_config)).unwrap();
        for _ in 0..16 {
            let mut flight = Vec::new();
            while client.wants_write() {
                client.write_tls(&mut flight).unwrap();
            }
            if !flight.is_empty() {
                server.read_tls(&mut Cursor::new(flight)).unwrap();
                server.process_new_packets()?;
            }
            let mut flight = Vec::new();
            while server.wants_write() {
                server.write_tls(&mut flight).unwrap();
            }
            if !flight.is_empty() {
                client.read_tls(&mut Cursor::new(flight)).unwrap();
                client.process_new_packets()?;
            }
            if !client.is_handshaking() && !server.is_handshaking() {
                assert_eq!(client.protocol_version(), Some(ProtocolVersion::TLSv1_3));
                assert_eq!(server.protocol_version(), Some(ProtocolVersion::TLSv1_3));
                return Ok(());
            }
        }
        Err(RustlsError::General("handshake did not finish".into()))
    }

    #[test]
    fn h_correct_tls13_certificateverify_is_accepted() {
        let (pelt, tls) = identity();
        let client =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        assert!(handshake(client, server_config(&tls).unwrap()).is_ok());
    }

    #[test]
    fn i_wrong_key_tls13_certificateverify_is_rejected() {
        let (pelt, certificate_identity) = identity();
        let (_, wrong_signer) = identity();
        let client =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        let error = handshake(
            client,
            server_with_signer(
                certificate_identity.certificate.clone(),
                &wrong_signer,
                false,
            ),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RustlsError::InvalidCertificate(rustls::CertificateError::BadSignature)
        ));
    }

    #[test]
    fn j_bit_flipped_tls13_certificateverify_is_rejected() {
        let (pelt, tls) = identity();
        let client =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        let error = handshake(
            client,
            server_with_signer(tls.certificate.clone(), &tls, true),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RustlsError::InvalidCertificate(rustls::CertificateError::BadSignature)
        ));
    }

    #[test]
    fn k_client_config_is_tls13_only() {
        let (pelt, tls) = identity();
        let production =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        assert!(handshake(production, server_config(&tls).unwrap()).is_ok());

        let production =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        let tls12_only = server_with_signer_and_versions(
            tls.certificate.clone(),
            &tls,
            false,
            &[&rustls::version::TLS12],
        );
        assert!(handshake(production, tls12_only).is_err());
    }

    #[test]
    fn l_server_config_is_tls13_only() {
        let (pelt, tls) = identity();
        let production_client =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        assert!(handshake(production_client, server_config(&tls).unwrap()).is_ok());

        let tls12_only_client =
            client_with_versions(&pelt.public_key_b64, &[&rustls::version::TLS12]);
        assert!(handshake(tls12_only_client, server_config(&tls).unwrap()).is_err());
    }

    #[test]
    fn m_client_resumption_is_disabled() {
        let pelt = pelt::generate_identity();
        let config =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        assert!(format!("{:?}", config.resumption).contains("NoClientSessionStorage"));
    }

    #[test]
    fn n_server_resumption_and_tickets_are_disabled() {
        let (_, tls) = identity();
        let config = server_config(&tls).unwrap();
        assert_eq!(config.send_tls13_tickets, 0);
        assert!(!config.ticketer.enabled());
        assert!(!config.session_storage.can_cache());
    }

    #[test]
    fn o_early_data_is_disabled() {
        let (pelt, tls) = identity();
        let client =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        let server = server_config(&tls).unwrap();
        assert!(!client.enable_early_data);
        assert_eq!(server.max_early_data_size, 0);
    }

    #[test]
    fn p_client_alpn_is_empty() {
        let pelt = pelt::generate_identity();
        let config =
            client_config_for_peer(&peer(&pelt, Some(pelt.public_key_b64.clone()))).unwrap();
        assert!(config.alpn_protocols.is_empty());
    }

    #[test]
    fn q_server_alpn_is_empty() {
        let (_, tls) = identity();
        assert!(server_config(&tls).unwrap().alpn_protocols.is_empty());
    }

    #[test]
    fn r_missing_peer_public_key_fails_closed() {
        let pelt = pelt::generate_identity();
        assert_eq!(
            expected_spki_for_peer(&peer(&pelt, None)),
            Err(TlsIdentityError::MissingPeerPublicKey)
        );
        assert!(matches!(
            client_config_for_peer(&peer(&pelt, None)),
            Err(TlsIdentityError::MissingPeerPublicKey)
        ));
    }

    #[test]
    fn s_invalid_peer_public_key_fails_closed() {
        let pelt = pelt::generate_identity();
        let invalid = peer(&pelt, Some("not-canonical-ed25519".into()));
        assert_eq!(
            expected_spki_for_peer(&invalid),
            Err(TlsIdentityError::InvalidPeerPublicKey)
        );
        assert!(matches!(
            client_config_for_peer(&invalid),
            Err(TlsIdentityError::InvalidPeerPublicKey)
        ));
    }

    #[test]
    fn t_certificate_contains_no_project_semantic_metadata() {
        let (pelt, tls) = identity();
        let certificate = tls.certificate().as_ref();
        let lowercase: Vec<u8> = certificate.iter().map(u8::to_ascii_lowercase).collect();
        for forbidden in [
            "werewolf",
            "fang",
            "pelt",
            "pack",
            "wwp1",
            "sender",
            "receiver",
            "target",
            "protocol",
            "daemon",
            "proxy",
            "selected-peer",
            "runtime.invalid",
        ] {
            assert!(!lowercase
                .windows(forbidden.len())
                .any(|bytes| bytes == forbidden.as_bytes()));
        }
        assert!(!certificate
            .windows(pelt.fingerprint.len())
            .any(|bytes| bytes == pelt.fingerprint.as_bytes()));
    }
}
