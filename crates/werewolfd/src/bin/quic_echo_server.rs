use quinn::{Endpoint, ServerConfig};
use rcgen::generate_simple_self_signed;
use rustls::crypto::CryptoProvider;
use std::{error::Error, net::SocketAddr};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = CryptoProvider::install_default(rustls::crypto::ring::default_provider());
    let cert = generate_simple_self_signed(vec!["localhost".into()])?;

    let cert_der = cert.cert.der().clone();
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()),
    );

    let server_config = ServerConfig::with_single_cert(vec![cert_der], key_der)?;

    let addr: SocketAddr = "127.0.0.1:9555".parse()?;
    let endpoint = Endpoint::server(server_config, addr)?;

    println!("🐺 QUIC echo server listening on {}", addr);

    while let Some(incoming) = endpoint.accept().await {
        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    println!("🦷 QUIC connection from {}", connection.remote_address());

                    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                        tokio::spawn(async move {
                            match recv.read_to_end(64 * 1024).await {
                                Ok(data) => {
                                    println!("📩 received {} bytes", data.len());
                                    let _ = send.write_all(&data).await;
                                    let _ = send.finish();
                                }
                                Err(e) => eprintln!("recv error: {}", e),
                            }
                        });
                    }
                }
                Err(e) => eprintln!("connection failed: {}", e),
            }
        });
    }

    Ok(())
}
