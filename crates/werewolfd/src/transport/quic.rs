use crate::{
    admission::{HandshakePermit, QuicReplayV3},
    handshake as hs,
    quic_lab::make_server_endpoint,
    state::DaemonState,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io,
    sync::Mutex,
    time::{timeout, timeout_at, Instant},
};

pub(super) async fn run_quic_fang_listener(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let addr = listen_addr.parse().map_err(|_| hs::rejected())?;
    let endpoint = make_server_endpoint(addr).map_err(|_| hs::rejected())?;
    let admission = state.lock().await.admission.clone();
    while let Some(incoming) = endpoint.accept().await {
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
        tokio::spawn(async move {
            let _context_permit = context_permit;
            let Ok(Ok(connection)) = timeout(hs::READ_WINDOW, incoming).await else {
                return;
            };
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
                            let result=timeout_at(started+hs::SERVER_WINDOW,server_handshake(&mut send,&mut recv,state,&mut permit,&replay,&binding,started)).await;
                            drop(permit);drop(open_permit);
                            match result {
                                Ok(Ok(mut target))=>{
                                    let (mut read,mut write)=target.split();
                                    let up=tokio::io::copy(&mut recv,&mut write);
                                    let down=tokio::io::copy(&mut read,&mut send);
                                    let _=tokio::join!(up,down);
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
) -> io::Result<tokio::net::TcpStream> {
    let open: hs::QuicOpen = hs::read(recv, hs::MESSAGE_LIMIT, started + hs::READ_WINDOW).await?;
    let retained = open.transcript(binding)?;
    let (sender_key, receiver, policy) = {
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
        )
    };
    hs::require(open.receiver_fingerprint == receiver.fingerprint)?;
    hs::verify(&sender_key, &retained, &open.signature)?;
    hs::require(Instant::now() < started + hs::READ_WINDOW)?;
    permit.authenticated(&open.sender_fingerprint)?;
    replay.reserve(&open.sender_fingerprint, &open.nonce)?;
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
        tokio::net::TcpStream::connect(authorized_targets.as_slice()),
    )
    .await
    .map_err(|_| hs::rejected())??;
    let ack = hs::QuicAck::new(&open, &retained, &receiver)?;
    hs::write(send, &ack, hs::MESSAGE_LIMIT, started + hs::SERVER_WINDOW).await?;
    Ok(target)
}
