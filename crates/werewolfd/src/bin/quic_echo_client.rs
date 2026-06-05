use quinn::{ClientConfig, Endpoint};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    DigitallySignedStruct, SignatureScheme,
};
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = CryptoProvider::install_default(rustls::crypto::ring::default_provider());
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;

    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)?;
    let client_config = ClientConfig::new(Arc::new(quic_crypto));

    endpoint.set_default_client_config(client_config);

    let server_addr: SocketAddr = "127.0.0.1:9555".parse()?;
    let connection = endpoint.connect(server_addr, "localhost")?.await?;

    println!("🐺 connected to QUIC server");

    let (mut send, mut recv) = connection.open_bi().await?;

    let msg = b"hello from Werewolf QUIC";
    send.write_all(msg).await?;
    send.finish()?;

    let response = recv.read_to_end(64 * 1024).await?;

    println!("📩 response: {}", String::from_utf8_lossy(&response));

    endpoint.wait_idle().await;

    Ok(())
}
