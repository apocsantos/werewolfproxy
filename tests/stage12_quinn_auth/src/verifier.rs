// Verifier implementation copied verbatim from the certified Stage 12B harness.
// Unused signer audit fields remain to preserve the certified block byte-for-byte.
#![allow(dead_code)]
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring, verify_tls13_signature, CryptoProvider},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, Error, SignatureScheme,
};
use std::sync::{Arc, Mutex};
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

pub struct Probe(Arc<Mutex<Audit>>);

impl Probe {
    pub fn evidence(&self) -> serde_json::Value {
        let a = self.0.lock().unwrap();
        serde_json::json!({
            "certificate_calls": a.cert_calls, "certificate_result": a.cert_result,
            "pin_accepted": a.pin_accepted, "tls12_calls": a.tls12_calls,
            "certificate_verify_calls": a.cv_calls,
            "certificate_verify_context_valid": a.cv_context_valid,
            "certificate_verify_result": a.cv_result,
        })
    }
    pub fn authenticated(&self) -> bool {
        let a = self.0.lock().unwrap();
        a.cert_calls == 1
            && a.pin_accepted
            && a.cv_calls == 1
            && a.cv_context_valid
            && a.cv_result.as_deref() == Some("verified")
            && a.tls12_calls == 0
    }
}

pub fn make(pin: Vec<u8>) -> (Arc<dyn ServerCertVerifier>, Probe) {
    let audit = Arc::new(Mutex::new(Audit::default()));
    let verifier = Arc::new(ExactPeltVerifier {
        expected_spki: pin,
        provider: Arc::new(ring::default_provider()),
        audit: audit.clone(),
    });
    (verifier, Probe(audit))
}
