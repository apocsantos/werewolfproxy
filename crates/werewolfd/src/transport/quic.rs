use crate::{quic_lab::make_server_endpoint, state::DaemonState};
use std::{sync::Arc, time::Duration};
use tokio::{io, sync::Mutex};
use werewolf_core::pelt::{sign_message, verify_message};

pub(super) async fn run_quic_fang_listener(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let addr: std::net::SocketAddr = listen_addr.parse().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bad QUIC listen addr: {}", e),
        )
    })?;

    let endpoint = make_server_endpoint(addr)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    println!(
        "[INFO][QUIC][NATIVE_LISTENER] ⚡ Native QUIC Fang listening on {}",
        listen_addr
    );

    while let Some(incoming) = endpoint.accept().await {
        let state = state.clone();

        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    println!(
                        "⚡ Native QUIC Fang connection from {}",
                        connection.remote_address()
                    );

                    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                        let state = state.clone();

                        println!("⚡ Native QUIC Fang stream accepted");

                        let mut line_buf = Vec::new();

                        loop {
                            match recv.read_chunk(1, true).await {
                                Ok(Some(chunk)) => {
                                    line_buf.extend_from_slice(&chunk.bytes);

                                    if line_buf.ends_with(b"\n") {
                                        break;
                                    }

                                    if line_buf.len() > 2048 {
                                        eprintln!("QUIC Fang request line too long");
                                        break;
                                    }
                                }
                                Ok(None) => {
                                    eprintln!("stream closed before request line");
                                    break;
                                }
                                Err(e) => {
                                    eprintln!("QUIC request read failed: {}", e);
                                    break;
                                }
                            }
                        }

                        let request_line = String::from_utf8_lossy(&line_buf).trim().to_string();

                        let request_value: serde_json::Value =
                            match serde_json::from_str(&request_line) {
                                Ok(v) => v,
                                Err(e) => {
                                    eprintln!("bad QUIC Fang request JSON: {}", e);
                                    eprintln!("raw request line: {}", request_line);
                                    continue;
                                }
                            };

                        if request_value["cmd"].as_str() != Some("fang.quic.open") {
                            eprintln!("bad QUIC Fang request cmd");
                            continue;
                        }

                        if request_value["protocol"].as_str() != Some("fang-quic-v2") {
                            eprintln!("bad QUIC Fang request protocol");
                            continue;
                        }

                        let sender_fp = match request_value["sender_fingerprint"].as_str() {
                            Some(v) => v,
                            None => {
                                eprintln!("missing sender fingerprint");
                                continue;
                            }
                        };

                        let target = match request_value["remote"].as_str() {
                            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
                            _ => {
                                eprintln!("missing QUIC Fang remote target");
                                continue;
                            }
                        };

                        println!("🐾 QUIC sender fingerprint: {}", sender_fp);

                        let sender_public_key = {
                            let st = state.lock().await;

                            match st.peers.iter().find(|p| p.fingerprint == sender_fp) {
                                Some(peer) => match &peer.public_key_b64 {
                                    Some(pk) => pk.clone(),
                                    None => {
                                        eprintln!(
                                            "❌ QUIC sender has no public key in Pack: {}",
                                            sender_fp
                                        );
                                        continue;
                                    }
                                },
                                None => {
                                    eprintln!("❌ QUIC sender not in Pack: {}", sender_fp);
                                    continue;
                                }
                            }
                        };

                        let nonce = match request_value["nonce"].as_str() {
                            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
                            _ => {
                                eprintln!("missing QUIC nonce");
                                continue;
                            }
                        };

                        let signature = match request_value["signature"].as_str() {
                            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
                            _ => {
                                eprintln!("❌ missing QUIC signature");
                                continue;
                            }
                        };

                        let signed_payload = format!(
                            "fang.quic.open|fang-quic-v2|{}|{}|{}",
                            sender_fp, nonce, target
                        );

                        if let Err(e) = verify_message(
                            &sender_public_key,
                            signed_payload.as_bytes(),
                            &signature,
                        ) {
                            eprintln!("❌ QUIC signature verification failed: {}", e);
                            continue;
                        }

                        {
                            let mut st = state.lock().await;

                            let replay_key = format!("quic|{}|{}", sender_fp, nonce);

                            if st.seen_nonces.contains_key(&replay_key) {
                                eprintln!("❌ QUIC replay detected: {}", replay_key);
                                continue;
                            }

                            st.seen_nonces.insert(replay_key, std::time::Instant::now());
                        }

                        println!("✅ QUIC sender trusted by Pack");
                        println!("🔐 QUIC signature verified");
                        println!("🧠 QUIC nonce accepted");
                        println!("🦷 Native QUIC target request: {}", target);

                        let authorized_targets = {
                            let policy = state.lock().await.target_policy.clone();
                            match crate::target_policy::authorize(&policy, sender_fp, &target).await
                            {
                                Ok(v) => v,
                                Err(_) => {
                                    eprintln!("❌ QUIC target authorization denied");
                                    continue;
                                }
                            }
                        };

                        match tokio::time::timeout(
                            Duration::from_secs(5),
                            tokio::net::TcpStream::connect(authorized_targets.as_slice()),
                        )
                        .await
                        {
                            Ok(Ok(mut target_stream)) => {
                                println!("✅ target connected: {}", target);

                                let receiver = {
                                    let st = state.lock().await;
                                    match st.pelt.clone() {
                                        Some(identity) => identity,
                                        None => {
                                            eprintln!(
                                                "❌ QUIC receiver has no local Pelt identity"
                                            );
                                            continue;
                                        }
                                    }
                                };
                                let ack_payload = format!(
                                    "fang.quic.ack|fang-quic-v2|{}|{}|{}|{}",
                                    receiver.fingerprint, sender_fp, nonce, target
                                );
                                let ack_signature =
                                    match sign_message(&receiver, ack_payload.as_bytes()) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            eprintln!("❌ QUIC ACK signing failed: {}", e);
                                            continue;
                                        }
                                    };
                                let ack = serde_json::json!({
                                    "ok": true,
                                    "protocol": "fang-quic-v2",
                                    "receiver_pubkey": receiver.public_key_b64,
                                    "receiver_fingerprint": receiver.fingerprint,
                                    "sender_fingerprint": sender_fp,
                                    "nonce": nonce,
                                    "remote": target,
                                    "signature": ack_signature,
                                });
                                if let Err(e) =
                                    send.write_all(format!("{}\n", ack).as_bytes()).await
                                {
                                    eprintln!("❌ QUIC ACK send failed: {}", e);
                                    continue;
                                }

                                let (mut target_read, mut target_write) = target_stream.split();

                                let up =
                                    async { tokio::io::copy(&mut recv, &mut target_write).await };

                                let down =
                                    async { tokio::io::copy(&mut target_read, &mut send).await };

                                let _ = tokio::join!(up, down);
                            }
                            Ok(Err(e)) => {
                                eprintln!(
                                    "[ERROR][QUIC][TARGET_CONNECT_FAILED] ❌ target connect failed {}: {}",
                                    target, e
                                );
                            }
                            Err(_) => {
                                eprintln!(
                                    "[ERROR][QUIC][TARGET_CONNECT_TIMEOUT] ❌ target connect timed out {} after 5s",
                                    target
                                );
                            }
                        }
                    }
                }
                Err(e) => eprintln!("QUIC connection failed: {}", e),
            }
        });
    }

    Ok(())
}
