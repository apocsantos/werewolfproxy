use crate::{admission::HandshakePermit, handshake as hs, state::DaemonState};
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use rand_core::{OsRng, RngCore};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use werewolf_core::pelt::PeltIdentity;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

pub(super) async fn run_fang_listener(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let listener = TcpListener::bind(listen_addr).await?;
    let admission = state.lock().await.admission.clone();

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let started = tokio::time::Instant::now();
        let Ok(permit) = admission.handshake() else {
            continue;
        };
        let state_for_client = state.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_fang_pipe(stream, state_for_client, permit, started).await {
                eprintln!("fang pipe from {} error: {}", peer_addr, e);
            }
        });
    }
}

async fn handle_fang_pipe(
    mut stream: TcpStream,
    state: Arc<Mutex<DaemonState>>,
    permit: HandshakePermit,
    started: tokio::time::Instant,
) -> io::Result<()> {
    let (remote, key) = tokio::time::timeout_at(
        started + hs::SERVER_WINDOW,
        server_handshake(&mut stream, state, permit, started),
    )
    .await
    .map_err(|_| hs::rejected())??;
    secure_copy_server_side(stream, remote, key).await
}

async fn server_handshake(
    stream: &mut TcpStream,
    state: Arc<Mutex<DaemonState>>,
    mut permit: HandshakePermit,
    started: tokio::time::Instant,
) -> io::Result<(TcpStream, [u8; 32])> {
    let receiver = state.lock().await.pelt.clone().ok_or_else(hs::rejected)?;
    // The identity and key snapshot travel with this challenge through ACK signing.
    let mut context = hs::TcpChallengeContext::new(receiver)?;
    context.deadline = context.deadline.min(started + hs::READ_WINDOW);
    hs::write(
        stream,
        &context.message,
        hs::CHALLENGE_LIMIT,
        context.deadline,
    )
    .await?;
    let open: hs::TcpOpen = hs::read(stream, hs::MESSAGE_LIMIT, context.deadline).await?;
    let retained = open.transcript()?;
    {
        let st = state.lock().await;
        hs::require(
            st.peers
                .iter()
                .any(|p| p.fingerprint == open.sender_fingerprint),
        )?;
    }
    context.check(&open)?;
    hs::verify(&open.sender_pubkey, &retained, &open.signature)?;
    permit.authenticated(&open.sender_fingerprint)?;
    context.consume(&open)?;

    let server_secret = StaticSecret::from(hs::random::<32>()?);
    let server_public = X25519PublicKey::from(&server_secret);
    let key = derive_shared_key(&server_secret, &open.client_x25519)?;
    let policy = state.lock().await.target_policy.clone();
    let target_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let authorized_targets = tokio::time::timeout_at(
        target_deadline,
        crate::target_policy::authorize(&policy, &open.sender_fingerprint, &open.remote),
    )
    .await
    .map_err(|_| hs::rejected())?
    .map_err(|_| hs::rejected())?;
    let remote = tokio::time::timeout_at(
        target_deadline,
        TcpStream::connect(authorized_targets.as_slice()),
    )
    .await
    .map_err(|_| hs::rejected())??;
    let ack = hs::TcpAck::new(
        &open,
        &retained,
        &context.receiver,
        server_public.as_bytes(),
    )?;
    hs::write(stream, &ack, hs::MESSAGE_LIMIT, started + hs::SERVER_WINDOW).await?;
    Ok((remote, key))
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
    cancellation.await_activation().await?;
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
        .map_err(|_| hs::rejected())??;
    let deadline = tokio::time::Instant::now() + hs::CLIENT_WINDOW;
    let key = tokio::time::timeout_at(deadline, async {
        let challenge: hs::TcpChallenge = hs::read(
            &mut outbound,
            hs::CHALLENGE_LIMIT,
            (tokio::time::Instant::now() + hs::READ_WINDOW).min(deadline),
        )
        .await?;
        challenge.validate()?;
        hs::require(challenge.receiver_fingerprint == expected_peer.fingerprint)?;
        let client_secret = StaticSecret::from(hs::random::<32>()?);
        let client_public = X25519PublicKey::from(&client_secret);
        let open = hs::TcpOpen::new(&identity, &challenge, remote, client_public.as_bytes())?;
        let retained = open.transcript()?;
        hs::write(&mut outbound, &open, hs::MESSAGE_LIMIT, deadline).await?;
        let ack: hs::TcpAck = hs::read(&mut outbound, hs::MESSAGE_LIMIT, deadline).await?;
        hs::require(ack.receiver_fingerprint == expected_peer.fingerprint)?;
        hs::verify(
            &ack.receiver_pubkey,
            &ack.transcript(&open, &retained)?,
            &ack.signature,
        )?;
        derive_shared_key(&client_secret, &ack.server_x25519)
    })
    .await
    .map_err(|_| hs::rejected())??;
    secure_copy_client_side(inbound, outbound, key).await
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
