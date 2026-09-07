#![allow(dead_code)]

use crate::quic_lab::make_client_endpoint;
use werewolf_core::pelt::{sign_message, PeltIdentity};

use quinn::{RecvStream, SendStream};
use rand_core::{OsRng, RngCore};
use std::{error::Error, net::SocketAddr};
use tokio::{
    io::copy,
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

pub async fn open_quic_fang(
    local_addr: String,
    quic_server: String,
    remote_addr: String,
    identity: PeltIdentity,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<std::io::Result<()>>,
) -> Result<JoinHandle<()>, Box<dyn Error + Send + Sync>> {
    let handle = tokio::spawn(async move {
        let listener = match TcpListener::bind(&local_addr).await {
            Ok(v) => v,
            Err(e) => {
                let _ = ready.send(Err(std::io::Error::new(e.kind(), e.to_string())));
                eprintln!("quic fang bind failed: {}", e);
                return;
            }
        };
        let _ = ready.send(Ok(()));

        println!(
            "[INFO][QUIC][FANG_OPEN] 🐺 QUIC Fang listening on {} → {} target {}",
            local_addr, quic_server, remote_addr
        );

        loop {
            let (tcp, _) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("accept failed: {}", e);
                    continue;
                }
            };

            let quic_server = quic_server.clone();
            let remote_addr = remote_addr.clone();
            let identity = identity.clone();

            let handle = tokio::spawn(async move {
                let server_addr: SocketAddr = match quic_server.parse() {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("bad server addr: {}", e);
                        return;
                    }
                };

                let mut retry_delay = 1u64;

                let connection = loop {
                    let endpoint = match make_client_endpoint() {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("endpoint failed: {}", e);
                            tokio::time::sleep(std::time::Duration::from_secs(retry_delay)).await;

                            retry_delay = (retry_delay * 2).min(15);
                            continue;
                        }
                    };

                    let connecting = match endpoint.connect(server_addr, "localhost") {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("connect setup failed: {}", e);

                            tokio::time::sleep(std::time::Duration::from_secs(retry_delay)).await;

                            retry_delay = (retry_delay * 2).min(15);
                            continue;
                        }
                    };

                    match tokio::time::timeout(std::time::Duration::from_secs(5), connecting).await
                    {
                        Ok(Ok(v)) => {
                            if retry_delay > 1 {
                                println!("✅ QUIC reconnect succeeded");
                            }
                            break v;
                        }
                        Ok(Err(e)) => {
                            eprintln!("⚠️ QUIC connect failed: {} (retry {}s)", e, retry_delay);
                        }
                        Err(_) => {
                            eprintln!("⚠️ QUIC timeout (retry {}s)", retry_delay);
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(retry_delay)).await;

                    retry_delay = (retry_delay * 2).min(15);
                };

                let (mut send, recv) = match connection.open_bi().await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("open_bi failed: {}", e);
                        return;
                    }
                };

                println!("🦷 QUIC Fang sending target request to {}", remote_addr);

                let mut nonce_bytes = [0u8; 16];
                OsRng.fill_bytes(&mut nonce_bytes);

                let nonce = nonce_bytes
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>();

                let sender_fingerprint = identity.fingerprint.clone();

                let signed_payload = format!(
                    "fang.quic.open|{}|{}|{}",
                    sender_fingerprint, nonce, remote_addr
                );

                let signature = match sign_message(&identity, signed_payload.as_bytes()) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("sign QUIC request failed: {}", e);
                        return;
                    }
                };

                let request = serde_json::json!({
                    "cmd": "fang.quic.open",
                    "sender_fingerprint":
                        sender_fingerprint,
                    "nonce": nonce,
                    "remote": remote_addr,
                    "signature": signature,
                });

                if let Err(e) = send.write_all(format!("{}\n", request).as_bytes()).await {
                    eprintln!("send target failed: {}", e);
                    return;
                }

                let _ = proxy_streams(tcp, send, recv).await;
            });
            cancellation.track(&handle);
        }
    });

    Ok(handle)
}

async fn proxy_streams(
    tcp: TcpStream,
    mut send: SendStream,
    mut recv: RecvStream,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (mut rd, mut wr) = tcp.into_split();

    let up = copy(&mut rd, &mut send);
    let down = copy(&mut recv, &mut wr);

    let _ = tokio::join!(up, down);

    Ok(())
}
