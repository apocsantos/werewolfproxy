use crate::authority::{AuthorityTcpStream, AuthorityWriter, SessionLease, Transport};
use crate::{admission::HandshakePermit, handshake as hs, state::DaemonState};
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use rand_core::{OsRng, RngCore};
use rustls::pki_types::ServerName;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use tokio_rustls::{
    client::TlsStream as ClientTlsStream, server::TlsStream as ServerTlsStream, TlsAcceptor,
    TlsConnector,
};
use werewolf_core::pelt::PeltIdentity;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::Zeroizing;

#[cfg(test)]
mod authority_tests;
#[cfg(test)]
mod crypto_hygiene_tests;
#[cfg(test)]
mod frame_write_tests;
#[cfg(test)]
mod receiver_auth_tests;

#[cfg(test)]
pub(super) async fn run_fang_listener(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    run_fang_listener_with_ready(listen_addr, state, None).await
}

pub(crate) async fn run_fang_listener_with_ready(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
    mut ready: Option<tokio::sync::oneshot::Sender<io::Result<()>>>,
) -> io::Result<()> {
    let runtime_tls_identity = match crate::state::wait_for_runtime_tls_identity(&state).await {
        Ok(identity) => identity,
        Err(error) => {
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err(io::Error::new(error.kind(), error.to_string())));
            }
            return Err(error);
        }
    };
    let (admission, authority) = {
        let state = state.lock().await;
        (state.admission.clone(), state.inbound_authority.clone())
    };
    let tls_config =
        crate::tls_identity::server_config(&runtime_tls_identity).map_err(|_| hs::rejected())?;
    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let listener = match TcpListener::bind(listen_addr).await {
        Ok(listener) => listener,
        Err(error) => {
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err(io::Error::new(error.kind(), error.to_string())));
            }
            return Err(error);
        }
    };
    if let Some(ready) = ready.take() {
        let _ = ready.send(Ok(()));
    }
    let mut tasks = tokio::task::JoinSet::new();

    loop {
        // Reap ready children before admitting more. Dropping the listener
        // supervisor aborts all owned children; no inbound task is detached.
        let (stream, _) = tokio::select! {
            biased;
            Some(_) = tasks.join_next(), if !tasks.is_empty() => continue,
            accepted = listener.accept() => accepted?,
        };
        if authority.is_locked() {
            continue;
        }
        let started = tokio::time::Instant::now();
        let Ok(permit) = admission.handshake() else {
            continue;
        };
        let state_for_client = state.clone();
        let tls_acceptor = tls_acceptor.clone();
        let mut silver = authority.silver_watch();
        // Subscribe before this second check. A Silver transition before the
        // subscription is caught here; one after it is caught by changed().
        if authority.is_locked() {
            continue;
        }

        tasks.spawn(async move {
            let tls_stream = tokio::select! {
                biased;
                _ = silver.changed() => Err(hs::rejected()),
                result = tokio::time::timeout_at(
                    started + hs::READ_WINDOW,
                    tls_acceptor.accept(AuthorityTcpStream::new(stream)),
                ) => result
                    .map_err(|_| hs::rejected())
                    .and_then(|result| result.map_err(|_| hs::rejected())),
            };
            let result = match tls_stream {
                Ok(tls_stream) => tokio::select! {
                    biased;
                    _ = silver.changed() => Err(hs::rejected()),
                    result = handle_fang_pipe(tls_stream, state_for_client, permit, started) => result,
                },
                Err(error) => Err(error),
            };
            let _ = result;
            // Routine unauthenticated failures are intentionally silent. A
            // network peer must not be able to turn handshake failures into
            // unbounded local log volume.
        });
    }
}

async fn handle_fang_pipe(
    mut stream: ServerTlsStream<AuthorityTcpStream>,
    state: Arc<Mutex<DaemonState>>,
    permit: HandshakePermit,
    started: tokio::time::Instant,
) -> io::Result<()> {
    let (remote, key, lease) = tokio::time::timeout_at(
        started + hs::SERVER_WINDOW,
        server_handshake(&mut stream, state, permit, started),
    )
    .await
    .map_err(|_| hs::rejected())??;
    secure_copy_server_side(stream, remote, key, lease).await
}

async fn server_handshake(
    stream: &mut ServerTlsStream<AuthorityTcpStream>,
    state: Arc<Mutex<DaemonState>>,
    mut permit: HandshakePermit,
    started: tokio::time::Instant,
) -> io::Result<(TcpStream, Zeroizing<[u8; 32]>, SessionLease)> {
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
    let authority = {
        let st = state.lock().await;
        hs::require(
            st.peers
                .iter()
                .any(|p| p.fingerprint == open.sender_fingerprint),
        )?;
        st.inbound_authority.clone()
    };
    context.check(&open)?;
    hs::verify(&open.sender_pubkey, &retained, &open.signature)?;
    permit.authenticated(&open.sender_fingerprint)?;
    context.consume(&open)?;
    let _target_work = permit.target_work(&open.sender_fingerprint)?;
    let ticket = authority.ticket(&open.sender_fingerprint)?;
    let lease = authority.reserve(ticket, Transport::Tcp)?;
    // Place the sender authority gate beneath rustls before target admission or
    // ACK. Buffered TLS ciphertext can therefore never bypass a later revoke.
    stream.get_mut().0.authorize(lease.clone())?;

    tokio::select! {
        biased;
        _ = lease.cancelled() => Err(hs::rejected()),
        result = async {
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
        lease.connect_poll(TcpStream::connect(authorized_targets.as_slice())),
    )
    .await
    .map_err(|_| hs::rejected())??;
    // Publication and every subsequent submission share the authority gate.
    // A target connected before a concurrent revoke is closed without an ACK.
    lease.publish()?;
    let ack = hs::TcpAck::new(
        &open,
        &retained,
        &context.receiver,
        server_public.as_bytes(),
    )?;
    hs::write(stream, &ack, hs::MESSAGE_LIMIT, started + hs::SERVER_WINDOW).await?;
    Ok((remote, key, lease.clone()))
        } => result,
    }
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
    expected_peer: crate::policy::ExpectedPeerIdentity,
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
        let (mut inbound, _) = listener.accept().await?;
        let peer_addr = peer_addr.to_string();
        let remote = remote.to_string();
        let identity = identity.clone();
        let expected_peer = expected_peer.clone();

        let handle = tokio::spawn(async move {
            let _ = pipe_one_fang_connection(
                &mut inbound,
                &peer_addr,
                &remote,
                identity,
                expected_peer,
            )
            .await;
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
    let mut outbound = connect_authenticated_tcp(peer_addr, &expected_peer).await?;
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

pub(crate) async fn connect_authenticated_tcp(
    peer_addr: &str,
    expected_peer: &crate::policy::ExpectedPeerIdentity,
) -> io::Result<ClientTlsStream<TcpStream>> {
    // Validate and construct exact selected-peer trust before opening a TCP
    // socket. Historical fingerprint-only Pack entries therefore fail closed
    // without sending transport or application bytes.
    let config = crate::tls_identity::client_config_for_selected_peer_key(
        expected_peer.public_key_b64.as_deref(),
    )
    .map_err(|_| hs::rejected())?;
    let connector = TlsConnector::from(Arc::new(config));
    let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(peer_addr))
        .await
        .map_err(|_| hs::rejected())??;
    let server_name = ServerName::from(stream.peer_addr()?.ip());
    tokio::time::timeout(hs::READ_WINDOW, connector.connect(server_name, stream))
        .await
        .map_err(|_| hs::rejected())?
        .map_err(|_| hs::rejected())
}

async fn secure_copy_client_side<S>(
    inbound: &mut TcpStream,
    outbound: S,
    key: Zeroizing<[u8; 32]>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut in_r, mut in_w) = inbound.split();
    let (mut out_r, out_w) = tokio::io::split(outbound);
    let mut out_w = Some(out_w);
    let mut client_to_server_done = false;
    let mut server_to_client_done = false;
    let mut client_to_server_buf = Zeroizing::new(vec![0u8; 1400]);
    let mut client_to_server_counter = 0u64;
    let mut server_to_client_counter = 0u64;

    while !(client_to_server_done && server_to_client_done) {
        tokio::select! {
            result = async {
                let n = in_r.read(&mut client_to_server_buf).await?;
                if n == 0 {
                    return Ok::<bool, io::Error>(true);
                }
                let writer = out_w.as_mut().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "encrypted TCP writer closed")
                })?;
                write_encrypted_frame(
                    writer,
                    &key,
                    0,
                    &mut client_to_server_counter,
                    &client_to_server_buf[..n],
                )
                .await?;
                Ok(false)
            }, if !client_to_server_done => {
                match result {
                    Ok(true) => {
                        if let Some(mut writer) = out_w.take() {
                            let _ = writer.shutdown().await;
                        }
                        client_to_server_done = true;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        if let Some(mut writer) = out_w.take() {
                            let _ = writer.shutdown().await;
                        }
                        return Err(error);
                    }
                }
            }
            result = async {
                match tokio::time::timeout(
                    Duration::from_secs(60),
                    read_encrypted_frame(&mut out_r, &key, 1, &mut server_to_client_counter),
                )
                .await
                {
                    Ok(Ok(plaintext)) => Ok::<Option<Vec<u8>>, io::Error>(Some(plaintext)),
                    Ok(Err(error)) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "tcp-v2 client read idle timeout",
                    )),
                }
            }, if !server_to_client_done => {
                match result {
                    Ok(Some(plaintext)) => {
                        let plaintext = Zeroizing::new(plaintext);
                        if let Err(error) = in_w.write_all(&plaintext).await {
                            if let Some(mut writer) = out_w.take() {
                                let _ = writer.shutdown().await;
                            }
                            return Err(error);
                        }
                        if let Err(error) = in_w.flush().await {
                            if let Some(mut writer) = out_w.take() {
                                let _ = writer.shutdown().await;
                            }
                            return Err(error);
                        }
                    }
                    Ok(None) => {
                        let _ = in_w.shutdown().await;
                        if let Some(mut writer) = out_w.take() {
                            let _ = writer.shutdown().await;
                        }
                        server_to_client_done = true;
                        // The remote write direction is closed, so no further
                        // response can arrive. Terminate the local request
                        // direction instead of waiting for local SHUT_WR.
                        client_to_server_done = true;
                    }
                    Err(error) => {
                        let _ = in_w.shutdown().await;
                        if let Some(mut writer) = out_w.take() {
                            let _ = writer.shutdown().await;
                        }
                        return Err(error);
                    }
                }
            }
        }
    }

    Ok(())
}

async fn secure_copy_server_side(
    stream: ServerTlsStream<AuthorityTcpStream>,
    remote_stream: TcpStream,
    key: Zeroizing<[u8; 32]>,
    lease: SessionLease,
) -> io::Result<()> {
    let (mut fang_r, fang_w) = tokio::io::split(stream);
    let (mut remote_r, remote_w) = remote_stream.into_split();
    let mut fang_w = fang_w;
    let mut remote_w = AuthorityWriter::new(remote_w, lease.clone());

    let client_to_remote = async {
        let mut counter = 0u64;

        loop {
            match tokio::time::timeout(
                Duration::from_secs(60),
                read_encrypted_frame(&mut fang_r, &key, 0, &mut counter),
            )
            .await
            {
                Ok(Ok(plaintext)) => {
                    let plaintext = Zeroizing::new(plaintext);
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
        let mut buf = Zeroizing::new(vec![0u8; 1400]);
        let mut counter = 0u64;

        loop {
            let n = remote_r.read(&mut buf).await?;
            if n == 0 {
                let _ = fang_w.shutdown().await;
                return Ok::<(), io::Error>(());
            }

            write_encrypted_frame(&mut fang_w, &key, 1, &mut counter, &buf[..n]).await?;
        }
    };

    tokio::select! {
        biased;
        _ = lease.cancelled() => Err(hs::rejected()),
        result = async { tokio::try_join!(client_to_remote, remote_to_client).map(|_| ()) } => result,
    }
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

    let mut padded_plaintext = Zeroizing::new(Vec::with_capacity(frame_size));
    padded_plaintext.extend_from_slice(&(plaintext.len() as u16).to_be_bytes());
    padded_plaintext.extend_from_slice(plaintext);
    padded_plaintext.resize(frame_size, 0);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce_bytes = make_nonce(direction, *counter);
    *counter += 1;

    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), padded_plaintext.as_ref())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "encrypt failed"))?;

    // Keep the Stage10 byte stream unchanged while submitting this bounded
    // inner frame to outer TLS in one write. Do not queue across frames: the
    // authority gate below rustls must still check each submission poll.
    let mut frame = Vec::with_capacity(4 + ciphertext.len());
    frame.extend_from_slice(&(ciphertext.len() as u32).to_be_bytes());
    frame.extend_from_slice(&ciphertext);
    writer.write_all(&frame).await?;
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

    let len = usize::try_from(u32::from_be_bytes(len_buf)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length is not representable",
        )
    })?;

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

    let padded_plaintext = Zeroizing::new(
        cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "decrypt failed"))?,
    );

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

fn derive_shared_key(
    secret: &StaticSecret,
    peer_public_b64: &str,
) -> io::Result<Zeroizing<[u8; 32]>> {
    let peer_bytes = STANDARD
        .decode(peer_public_b64)
        .map_err(|_| hs::rejected())?;

    let peer_arr: [u8; 32] = peer_bytes.try_into().map_err(|_| hs::rejected())?;

    let peer_public = X25519PublicKey::from(peer_arr);
    let shared = secret.diffie_hellman(&peer_public);
    // Both TCP handshake roles use this function. A low-order peer point can
    // produce the public all-zero value; it must never enter the session KDF.
    hs::require(shared.as_bytes() != &[0u8; 32])?;

    // The first 32 XOF bytes are exactly BLAKE3's ordinary 32-byte digest.
    // Write them directly into a drop-wiped owner instead of leaving an
    // additional application-owned raw digest copy on the stack.
    let mut key = Zeroizing::new([0u8; 32]);
    let mut hasher = blake3::Hasher::new();
    hasher.update(shared.as_bytes());
    hasher.finalize_xof().fill(&mut key[..]);
    Ok(key)
}

fn make_nonce(direction: u8, counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[0] = direction;
    nonce[4..12].copy_from_slice(&counter.to_be_bytes());
    nonce
}
