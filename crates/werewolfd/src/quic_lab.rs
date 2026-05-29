#![allow(dead_code)]
use quinn::{Endpoint, ServerConfig, ClientConfig};
use rcgen::generate_simple_self_signed;
use rustls::crypto::CryptoProvider;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    DigitallySignedStruct, SignatureScheme,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use std::{error::Error, net::SocketAddr, sync::Arc};

#[derive(Debug)]
struct SkipServerVerification;

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ED25519,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}

pub fn install_crypto_provider() {
    let _ = CryptoProvider::install_default(rustls::crypto::ring::default_provider());
}

pub fn make_server_endpoint(addr: SocketAddr) -> Result<Endpoint, Box<dyn Error + Send + Sync>> {
    install_crypto_provider();

    let cert = generate_simple_self_signed(vec!["localhost".into()])?;
    let cert_der = cert.cert.der().clone();

    let key_der = PrivateKeyDer::Pkcs8(
        PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()),
    );

    let server_config = ServerConfig::with_single_cert(vec![cert_der], key_der)?;
    let endpoint = Endpoint::server(server_config, addr)?;

    Ok(endpoint)
}

pub fn make_client_endpoint() -> Result<Endpoint, Box<dyn Error + Send + Sync>> {
    install_crypto_provider();

    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;

    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)?;
    let client_config = ClientConfig::new(Arc::new(quic_crypto));

    endpoint.set_default_client_config(client_config);

    Ok(endpoint)
}
