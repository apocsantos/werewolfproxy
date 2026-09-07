use crate::state::DaemonState;
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use rand_core::{OsRng, RngCore};
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use werewolf_core::pelt::{
    fingerprint_from_public_key_b64, sign_message, verify_message, PeltIdentity,
};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

pub(super) async fn run_fang_listener(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let listener = TcpListener::bind(listen_addr).await?;

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let state_for_client = state.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_fang_pipe(stream, state_for_client).await {
                eprintln!("fang pipe from {} error: {}", peer_addr, e);
            }
        });
    }
}

async fn handle_fang_pipe(stream: TcpStream, state: Arc<Mutex<DaemonState>>) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    reader.read_line(&mut line).await?;
    let mut stream = reader.into_inner();

    let value: serde_json::Value = serde_json::from_str(&line)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let remote = value["remote"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing remote"))?;

    let sender_pubkey = value["sender_pubkey"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing sender_pubkey"))?;

    let sender_fingerprint = value["sender_fingerprint"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing sender_fingerprint"))?;

    let client_x25519 = value["client_x25519"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing client_x25519"))?;

    let nonce = value["nonce"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let signature = value["signature"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing signature"))?;

    let derived_fp = fingerprint_from_public_key_b64(sender_pubkey)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    if derived_fp != sender_fingerprint {
        stream
            .write_all(b"{\"ok\":false,\"error\":\"fingerprint mismatch\"}\n")
            .await?;
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fingerprint mismatch",
        ));
    }

    let signed_text = format!(
        "fang.pipe|{}|{}|{}|{}",
        sender_fingerprint, remote, nonce, client_x25519
    );

    if let Err(e) = verify_message(sender_pubkey, signed_text.as_bytes(), signature) {
        stream
            .write_all(b"{\"ok\":false,\"error\":\"bad signature\"}\n")
            .await?;
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, e));
    }

    {
        let mut st = state.lock().await;

        if !st.peers.iter().any(|p| p.fingerprint == sender_fingerprint) {
            stream
                .write_all(b"{\"ok\":false,\"error\":\"sender not in pack\"}\n")
                .await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "sender not in pack",
            ));
        }

        const REPLAY_WINDOW: Duration = Duration::from_secs(300);

        let now = Instant::now();

        st.seen_nonces
            .retain(|_, seen_at| now.duration_since(*seen_at) < REPLAY_WINDOW);

        let replay_key = format!("{}|{}", sender_fingerprint, nonce);

        if st.seen_nonces.contains_key(&replay_key) {
            stream
                .write_all(b"{\"ok\":false,\"error\":\"replay detected\"}\n")
                .await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "replay detected",
            ));
        }

        st.seen_nonces.insert(replay_key, now);
    }

    let receiver_identity = {
        let st = state.lock().await;
        st.pelt.clone().ok_or_else(|| {
            io::Error::new(io::ErrorKind::PermissionDenied, "receiver has no Pelt")
        })?
    };

    let target_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let authorized_targets = {
        let policy = state.lock().await.target_policy.clone();
        tokio::time::timeout_at(
            target_deadline,
            crate::target_policy::authorize(&policy, sender_fingerprint, remote),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "target operation timed out"))?
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "target authorization denied",
            )
        })?
    };

    let server_secret = StaticSecret::random_from_rng(OsRng);
    let server_public = X25519PublicKey::from(&server_secret);
    let server_public_b64 = STANDARD.encode(server_public.as_bytes());

    let ack_text = format!(
        "fang.ack|{}|{}|{}|{}",
        receiver_identity.fingerprint, sender_fingerprint, nonce, server_public_b64
    );

    let ack_signature = sign_message(&receiver_identity, ack_text.as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let remote_stream = tokio::time::timeout_at(
        target_deadline,
        TcpStream::connect(authorized_targets.as_slice()),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "remote connect timed out"))??;

    let ack = json!({
        "ok": true,
        "session": "fang-v1-secure",
        "receiver_pubkey": receiver_identity.public_key_b64,
        "receiver_fingerprint": receiver_identity.fingerprint,
        "server_x25519": server_public_b64,
        "nonce": nonce,
        "signature": ack_signature
    });

    stream
        .write_all(serde_json::to_string(&ack).unwrap().as_bytes())
        .await?;
    stream.write_all(b"\n").await?;

    let key = derive_shared_key(&server_secret, client_x25519)?;

    secure_copy_server_side(stream, remote_stream, key).await?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_local_fang_forwarder(
    fang_id: &str,
    local: &str,
    peer_addr: &str,
    remote: &str,
    identity: PeltIdentity,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<io::Result<()>>,
    _expected_peer: crate::policy::ExpectedPeerIdentity,
) -> io::Result<()> {
    let listener = match TcpListener::bind(local).await {
        Ok(listener) => {
            let _ = ready.send(Ok(()));
            listener
        }
        Err(error) => {
            let _ = ready.send(Err(io::Error::new(error.kind(), error.to_string())));
            return Err(error);
        }
    };
    println!("🦷 {} listening locally on {}", fang_id, local);

    loop {
        let (mut inbound, client_addr) = listener.accept().await?;
        let peer_addr = peer_addr.to_string();
        let remote = remote.to_string();
        let fang_id = fang_id.to_string();
        let identity = identity.clone();
        let expected_peer = _expected_peer.clone();

        let handle = tokio::spawn(async move {
            if let Err(e) =
                pipe_one_fang_connection(&mut inbound, &peer_addr, &remote, identity, expected_peer)
                    .await
            {
                eprintln!("🦷 {} client {} pipe error: {}", fang_id, client_addr, e);
            }
        });
        cancellation.track(&handle);
    }
}

async fn pipe_one_fang_connection(
    inbound: &mut TcpStream,
    peer_addr: &str,
    remote: &str,
    identity: PeltIdentity,
    expected_peer: crate::policy::ExpectedPeerIdentity,
) -> io::Result<()> {
    let mut outbound = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(peer_addr))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "peer connect timed out"))??;

    let nonce = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    let client_secret = StaticSecret::random_from_rng(OsRng);
    let client_public = X25519PublicKey::from(&client_secret);
    let client_public_b64 = STANDARD.encode(client_public.as_bytes());

    let signed_text = format!(
        "fang.pipe|{}|{}|{}|{}",
        identity.fingerprint, remote, nonce, client_public_b64
    );

    let signature = sign_message(&identity, signed_text.as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let hello = json!({
        "cmd": "fang.pipe",
        "remote": remote,
        "sender_pubkey": identity.public_key_b64,
        "sender_fingerprint": identity.fingerprint,
        "client_x25519": client_public_b64,
        "nonce": nonce,
        "signature": signature
    });

    outbound
        .write_all(serde_json::to_string(&hello).unwrap().as_bytes())
        .await?;
    outbound.write_all(b"\n").await?;

    let mut reader = BufReader::new(outbound);
    let mut ack = String::new();
    reader.read_line(&mut ack).await?;
    let outbound = reader.into_inner();

    let ack_value: serde_json::Value = serde_json::from_str(&ack)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    if ack_value["ok"].as_bool() != Some(true) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("fang rejected: {}", ack),
        ));
    }

    let receiver_pubkey = ack_value["receiver_pubkey"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing receiver_pubkey"))?;

    let receiver_fingerprint = ack_value["receiver_fingerprint"].as_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing receiver_fingerprint")
    })?;

    let server_x25519 = ack_value["server_x25519"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing server_x25519"))?;

    let ack_nonce = ack_value["nonce"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing ack nonce"))?;

    let ack_signature = ack_value["signature"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing ack signature"))?;

    if ack_nonce != nonce {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "ack nonce mismatch",
        ));
    }

    let derived_receiver_fp = fingerprint_from_public_key_b64(receiver_pubkey)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    if derived_receiver_fp != receiver_fingerprint {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "receiver fingerprint mismatch",
        ));
    }

    if receiver_fingerprint != expected_peer.fingerprint {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "receiver identity not expected",
        ));
    }

    let expected_ack_text = format!(
        "fang.ack|{}|{}|{}|{}",
        receiver_fingerprint, identity.fingerprint, nonce, server_x25519
    );

    verify_message(receiver_pubkey, expected_ack_text.as_bytes(), ack_signature)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    let key = derive_shared_key(&client_secret, server_x25519)?;

    secure_copy_client_side(inbound, outbound, key).await?;

    Ok(())
}

async fn secure_copy_client_side(
    inbound: &mut TcpStream,
    outbound: TcpStream,
    key: [u8; 32],
) -> io::Result<()> {
    let (mut in_r, mut in_w) = inbound.split();
    let (mut out_r, mut out_w) = outbound.into_split();

    let client_to_server = async {
        let mut buf = vec![0u8; 1400];
        let mut counter = 0u64;

        loop {
            let n = in_r.read(&mut buf).await?;
            if n == 0 {
                let _ = out_w.shutdown().await;
                return Ok::<(), io::Error>(());
            }

            eprintln!("TCPV2 client->server plaintext={} counter={}", n, counter);
            write_encrypted_frame(&mut out_w, &key, 0, &mut counter, &buf[..n]).await?;
            eprintln!("TCPV2 client->server sent counter={}", counter);
        }
    };

    let server_to_client = async {
        let mut counter = 0u64;

        loop {
            eprintln!("TCPV2 client waiting server->client counter={}", counter);
            match tokio::time::timeout(
                Duration::from_secs(60),
                read_encrypted_frame(&mut out_r, &key, 1, &mut counter),
            )
            .await
            {
                Ok(Ok(plaintext)) => {
                    eprintln!(
                        "TCPV2 client recv server->client plaintext={} counter={}",
                        plaintext.len(),
                        counter
                    );
                    in_w.write_all(&plaintext).await?;
                    in_w.flush().await?;
                }
                Ok(Err(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    let _ = in_w.shutdown().await;
                    return Ok::<(), io::Error>(());
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "tcp-v2 client read idle timeout",
                    ));
                }
            }
        }
    };

    let _ = tokio::join!(client_to_server, server_to_client);
    Ok(())
}

async fn secure_copy_server_side(
    stream: TcpStream,
    remote_stream: TcpStream,
    key: [u8; 32],
) -> io::Result<()> {
    let (mut fang_r, mut fang_w) = stream.into_split();
    let (mut remote_r, mut remote_w) = remote_stream.into_split();

    let client_to_remote = async {
        let mut counter = 0u64;

        loop {
            eprintln!("TCPV2 server waiting client->remote counter={}", counter);
            match tokio::time::timeout(
                Duration::from_secs(60),
                read_encrypted_frame(&mut fang_r, &key, 0, &mut counter),
            )
            .await
            {
                Ok(Ok(plaintext)) => {
                    eprintln!(
                        "TCPV2 server recv client->remote plaintext={} counter={}",
                        plaintext.len(),
                        counter
                    );
                    remote_w.write_all(&plaintext).await?;
                    remote_w.flush().await?;
                }
                Ok(Err(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    let _ = remote_w.shutdown().await;
                    return Ok::<(), io::Error>(());
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "tcp-v2 server read idle timeout",
                    ));
                }
            }
        }
    };

    let remote_to_client = async {
        let mut buf = vec![0u8; 1400];
        let mut counter = 0u64;

        loop {
            let n = remote_r.read(&mut buf).await?;
            if n == 0 {
                let _ = fang_w.shutdown().await;
                return Ok::<(), io::Error>(());
            }

            eprintln!("TCPV2 server->client plaintext={} counter={}", n, counter);
            write_encrypted_frame(&mut fang_w, &key, 1, &mut counter, &buf[..n]).await?;
            eprintln!("TCPV2 server->client sent counter={}", counter);
        }
    };

    let _ = tokio::join!(client_to_remote, remote_to_client);
    Ok(())
}

async fn write_encrypted_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    key: &[u8; 32],
    direction: u8,
    counter: &mut u64,
    plaintext: &[u8],
) -> io::Result<()> {
    const MIN_FRAME_SIZE: usize = 768;
    const MAX_FRAME_SIZE: usize = 2048;
    const STEP: usize = 128;

    if plaintext.len() > MAX_FRAME_SIZE - 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plaintext frame exceeds hide limit",
        ));
    }

    let minimum_needed = plaintext.len() + 2;
    let min_bucket = minimum_needed.max(MIN_FRAME_SIZE);
    let buckets = ((MAX_FRAME_SIZE - min_bucket) / STEP) + 1;

    let random_bucket = if buckets > 1 {
        (OsRng.next_u32() as usize) % buckets
    } else {
        0
    };

    let frame_size = min_bucket + (random_bucket * STEP);

    let mut padded_plaintext = Vec::with_capacity(frame_size);
    padded_plaintext.extend_from_slice(&(plaintext.len() as u16).to_be_bytes());
    padded_plaintext.extend_from_slice(plaintext);
    padded_plaintext.resize(frame_size, 0);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce_bytes = make_nonce(direction, *counter);
    *counter += 1;

    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), padded_plaintext.as_ref())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "encrypt failed"))?;

    let len = ciphertext.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&ciphertext).await?;
    writer.flush().await?;

    Ok(())
}

async fn read_encrypted_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    key: &[u8; 32],
    direction: u8,
    counter: &mut u64,
) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];

    if reader.read_exact(&mut len_buf).await.is_err() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed"));
    }

    let len = u32::from_be_bytes(len_buf) as usize;

    if len > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }

    let mut ciphertext = vec![0u8; len];
    reader.read_exact(&mut ciphertext).await?;

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce_bytes = make_nonce(direction, *counter);
    *counter += 1;

    let padded_plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "decrypt failed"))?;

    if padded_plaintext.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad frame"));
    }

    let real_len = u16::from_be_bytes([padded_plaintext[0], padded_plaintext[1]]) as usize;

    if real_len > padded_plaintext.len() - 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad plaintext length",
        ));
    }

    Ok(padded_plaintext[2..2 + real_len].to_vec())
}

fn derive_shared_key(secret: &StaticSecret, peer_public_b64: &str) -> io::Result<[u8; 32]> {
    let peer_bytes = STANDARD
        .decode(peer_public_b64)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let peer_arr: [u8; 32] = peer_bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad x25519 public key length"))?;

    let peer_public = X25519PublicKey::from(peer_arr);
    let shared = secret.diffie_hellman(&peer_public);

    let hash = blake3::hash(shared.as_bytes());
    Ok(*hash.as_bytes())
}

fn make_nonce(direction: u8, counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[0] = direction;
    nonce[4..12].copy_from_slice(&counter.to_be_bytes());
    nonce
}
