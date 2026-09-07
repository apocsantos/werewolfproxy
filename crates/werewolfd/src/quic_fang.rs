#![allow(dead_code)]

use crate::quic_lab::make_client_endpoint;
use werewolf_core::pelt::{
    fingerprint_from_public_key_b64, sign_message, verify_message, PeltIdentity,
};

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
    _expected_peer: crate::policy::ExpectedPeerIdentity,
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

                let (mut send, mut recv) = match connection.open_bi().await {
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
                    "fang.quic.open|fang-quic-v2|{}|{}|{}",
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
                    "protocol": "fang-quic-v2",
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

                let ack = match read_quic_ack(&mut recv).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("QUIC receiver authentication failed: {}", e);
                        return;
                    }
                };

                let receiver_pubkey = match ack["receiver_pubkey"].as_str() {
                    Some(v) => v,
                    None => {
                        eprintln!("QUIC ACK missing receiver public key");
                        return;
                    }
                };
                let receiver_fingerprint = match ack["receiver_fingerprint"].as_str() {
                    Some(v) => v,
                    None => {
                        eprintln!("QUIC ACK missing receiver fingerprint");
                        return;
                    }
                };
                if fingerprint_from_public_key_b64(receiver_pubkey)
                    .ok()
                    .as_deref()
                    != Some(receiver_fingerprint)
                {
                    eprintln!("QUIC ACK receiver fingerprint mismatch");
                    return;
                }
                let ack_sender = ack["sender_fingerprint"].as_str();
                let ack_nonce = ack["nonce"].as_str();
                let ack_remote = ack["remote"].as_str();
                let ack_signature = match ack["signature"].as_str() {
                    Some(v) => v,
                    None => {
                        eprintln!("QUIC ACK missing signature");
                        return;
                    }
                };
                if ack["ok"].as_bool() != Some(true)
                    || ack["protocol"].as_str() != Some("fang-quic-v2")
                    || ack_sender != Some(sender_fingerprint.as_str())
                    || ack_nonce != Some(nonce.as_str())
                    || ack_remote != Some(remote_addr.as_str())
                {
                    eprintln!("QUIC ACK context mismatch");
                    return;
                }
                let ack_payload = format!(
                    "fang.quic.ack|fang-quic-v2|{}|{}|{}|{}",
                    receiver_fingerprint, sender_fingerprint, nonce, remote_addr
                );
                if let Err(e) =
                    verify_message(receiver_pubkey, ack_payload.as_bytes(), ack_signature)
                {
                    eprintln!("QUIC ACK signature verification failed: {}", e);
                    return;
                }
                let _ = proxy_streams(tcp, send, recv).await;
            });
            cancellation.track(&handle);
        }
    });

    Ok(handle)
}

async fn read_quic_ack(
    recv: &mut RecvStream,
) -> Result<serde_json::Value, Box<dyn Error + Send + Sync>> {
    let mut buf = Vec::new();
    loop {
        match recv.read_chunk(1, true).await? {
            Some(chunk) => {
                buf.extend_from_slice(&chunk.bytes);
                if buf.ends_with(b"\n") {
                    break;
                }
                if buf.len() > 4096 {
                    return Err("QUIC ACK too long".into());
                }
            }
            None => return Err("QUIC ACK truncated".into()),
        }
    }
    let line = std::str::from_utf8(&buf)?.trim();
    Ok(serde_json::from_str(line)?)
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
