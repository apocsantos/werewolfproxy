use crate::{
    admission::{HandshakePermit, QuicReplayV3},
    authority::{AuthorityWriter, ConnectionId, SessionLease, Transport},
    handshake as hs,
    state::DaemonState,
};

// Closing the owning supervisor must also close the underlying connection;
// dropping a SendStream alone may finish queued bytes rather than reset it.
struct CloseConnection(quinn::Connection);
impl Drop for CloseConnection {
    fn drop(&mut self) {
        self.0.close(hs::V3_REJECT_CODE.into(), b"");
    }
}
use std::{sync::Arc, time::Duration};
use tokio::{
    io,
    sync::Mutex,
    time::{timeout, timeout_at, Instant},
};

#[cfg(test)]
mod authority_tests;

pub(super) async fn run_quic_fang_listener(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let addr = listen_addr.parse().map_err(|_| hs::rejected())?;
    let (admission, authority, runtime_tls_identity) = {
        let state = state.lock().await;
        (
            state.admission.clone(),
            state.inbound_authority.clone(),
            state.runtime_tls_identity.clone(),
        )
    };
    let runtime_tls_identity = runtime_tls_identity.ok_or_else(hs::rejected)?;
    let crypto =
        crate::tls_identity::server_config(&runtime_tls_identity).map_err(|_| hs::rejected())?;
    let quic_crypto =
        quinn::crypto::rustls::QuicServerConfig::try_from(crypto).map_err(|_| hs::rejected())?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    let endpoint = quinn::Endpoint::server(server_config, addr).map_err(|_| hs::rejected())?;
    let mut connections = tokio::task::JoinSet::new();
    loop {
        let incoming = tokio::select! {
            biased;
            Some(_) = connections.join_next(), if !connections.is_empty() => continue,
            incoming = endpoint.accept() => { let Some(incoming) = incoming else { break }; incoming },
        };
        if authority.is_locked() {
            incoming.refuse();
            continue;
        }
        let Ok(context_permit) = admission.quic_context() else {
            incoming.refuse();
            continue;
        };
        let Ok(tls_permit) = admission.handshake() else {
            incoming.refuse();
            continue;
        };
        let state = state.clone();
        let admission = admission.clone();
        let authority = authority.clone();
        let mut silver = authority.silver_watch();
        connections.spawn(async move {
            let _context_permit = context_permit;
            let result = tokio::select! {
                biased;
                _ = silver.changed() => return,
                result = timeout(hs::READ_WINDOW, incoming) => result,
            };
            let Ok(Ok(connection)) = result else {
                return;
            };
            let _close = CloseConnection(connection.clone());
            if authority.is_locked() { return; }
            let Ok(connection_id) = authority.connection() else { return; };
            drop(tls_permit);
            // Completed handshake only: no early/0-RTT OPEN authorization.
            let Ok(binding) = hs::quic_binding(&connection) else {
                connection.close(hs::V3_REJECT_CODE.into(), b"");
                return;
            };
            let replay = Arc::new(QuicReplayV3::default());
            let first_deadline = Instant::now() + hs::READ_WINDOW;
            let mut first = true;
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _=silver.changed()=>{connection.close(hs::V3_REJECT_CODE.into(),b"");break},
                    _=connection.closed()=>break,
                    Some(_)=tasks.join_next(), if !tasks.is_empty()=>{},
                    incoming=async {
                        if first { timeout_at(first_deadline,connection.accept_bi()).await.map_err(|_|hs::rejected())?.map_err(|_|hs::rejected()) }
                        else {connection.accept_bi().await.map_err(|_|hs::rejected())}
                    }=>{
                        let Ok((mut send,mut recv))=incoming else {connection.close(hs::V3_REJECT_CODE.into(),b"");break};
                        first=false;
                        let started=Instant::now();
                        let Ok(mut permit)=admission.handshake() else {connection.close(hs::V3_REJECT_CODE.into(),b"");break};
                        let Ok(open_permit)=replay.opens.clone().try_acquire_owned() else {connection.close(hs::V3_REJECT_CODE.into(),b"");break};
                        let state=state.clone();let replay=replay.clone();
                        tasks.spawn(async move {
                            let result=timeout_at(started+hs::SERVER_WINDOW,server_handshake(&mut send,&mut recv,state,&mut permit,&replay,&binding,started,connection_id)).await;
                            drop(permit);drop(open_permit);
                            match result {
                                Ok(Ok((mut target,lease)))=>{
                                    let result=tokio::select! {
                                        biased;
                                        _=lease.cancelled()=>Err(hs::rejected()),
                                        result=async {
                                            let (mut read,write)=target.split();
                                            let mut write=AuthorityWriter::new(write,lease.clone());
                                            let mut send=AuthorityWriter::new(&mut send,lease.clone());
                                            let up=tokio::io::copy(&mut recv,&mut write);
                                            let down=tokio::io::copy(&mut read,&mut send);
                                            tokio::try_join!(up,down).map(|_|())
                                        }=>result,
                                    };
                                    if result.is_err() {
                                        if lease.is_current() {
                                            // The authenticated ACK has already been
                                            // submitted. A forwarding/target error is
                                            // an established-session close, not a
                                            // handshake rejection; reset() could drop
                                            // the queued ACK. Finish the send direction
                                            // and stop only receive-side input.
                                            let _ = send.finish();
                                            let _ = recv.stop(hs::V3_REJECT_CODE.into());
                                        } else {
                                            // Revocation/Silver is intentionally
                                            // abortive and may discard in-flight data.
                                            let _=send.reset(hs::V3_REJECT_CODE.into());
                                            let _=recv.stop(hs::V3_REJECT_CODE.into());
                                        }
                                    }
                                }
                                _=>{
                                    let _=send.reset(hs::V3_REJECT_CODE.into());
                                    let _=recv.stop(hs::V3_REJECT_CODE.into());
                                    eprintln!("QUIC v3 handshake rejected");
                                }
                            }
                        });
                    }
                }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            // Replay reservations and the context permit are released only after
            // this connection has closed and its child streams have been dropped.
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn server_handshake(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    state: Arc<Mutex<DaemonState>>,
    permit: &mut HandshakePermit,
    replay: &QuicReplayV3,
    binding: &[u8; 32],
    started: Instant,
    connection_id: ConnectionId,
) -> io::Result<(tokio::net::TcpStream, SessionLease)> {
    let open: hs::QuicOpen = hs::read(recv, hs::MESSAGE_LIMIT, started + hs::READ_WINDOW).await?;
    let retained = open.transcript(binding)?;
    let (sender_key, receiver, policy, authority) = {
        let st = state.lock().await;
        let key = st
            .peers
            .iter()
            .find(|p| p.fingerprint == open.sender_fingerprint)
            .and_then(|p| p.public_key_b64.clone())
            .ok_or_else(hs::rejected)?;
        (
            key,
            st.pelt.clone().ok_or_else(hs::rejected)?,
            st.target_policy.clone(),
            st.inbound_authority.clone(),
        )
    };
    hs::require(open.receiver_fingerprint == receiver.fingerprint)?;
    hs::verify(&sender_key, &retained, &open.signature)?;
    hs::require(Instant::now() < started + hs::READ_WINDOW)?;
    permit.authenticated(&open.sender_fingerprint)?;
    let ticket = authority.ticket(&open.sender_fingerprint)?;
    replay.reserve(&open.sender_fingerprint, &open.nonce)?;
    let lease = authority.reserve(ticket, Transport::Quic(connection_id))?;
    tokio::select! {
        biased;
        _ = lease.cancelled() => Err(hs::rejected()),
        result = async {
    let target_deadline = Instant::now() + Duration::from_secs(5);
    let authorized_targets = timeout_at(
        target_deadline,
        crate::target_policy::authorize(&policy, &open.sender_fingerprint, &open.remote),
    )
    .await
    .map_err(|_| hs::rejected())?
    .map_err(|_| hs::rejected())?;
    let target = timeout_at(
        target_deadline,
        lease.connect_poll(tokio::net::TcpStream::connect(authorized_targets.as_slice())),
    )
    .await
    .map_err(|_| hs::rejected())??;
    lease.publish()?;
    let ack = hs::QuicAck::new(&open, &retained, &receiver)?;
    let mut writer = AuthorityWriter::new(send, lease.clone());
    hs::write(&mut writer, &ack, hs::MESSAGE_LIMIT, started + hs::SERVER_WINDOW).await?;
    Ok((target, lease.clone()))
        } => result,
    }
}
