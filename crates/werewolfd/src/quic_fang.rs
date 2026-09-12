#![allow(dead_code)]

use crate::handshake as hs;
use crate::policy::ExpectedPeerIdentity;
use quinn::{ClientConfig, Endpoint, RecvStream, SendStream};
use std::sync::Arc;
use std::{error::Error, net::SocketAddr};
use tokio::{
    io::copy,
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use werewolf_core::pelt::PeltIdentity;

pub async fn open_quic_fang(
    local_addr: String,
    quic_server: String,
    remote_addr: String,
    identity: PeltIdentity,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<std::io::Result<()>>,
    expected_peer: crate::policy::ExpectedPeerIdentity,
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
        if cancellation.await_activation().await.is_err() {
            return;
        }

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
            let expected_peer = expected_peer.clone();

            let handle = tokio::spawn(async move {
                let server_addr: SocketAddr = match quic_server.parse() {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("bad server addr: {}", e);
                        return;
                    }
                };

                let result = connect_v3(server_addr, &remote_addr, &identity, &expected_peer).await;
                let Ok((_connection, send, recv)) = result else {
                    eprintln!("QUIC v3 handshake rejected");
                    return;
                };
                let _ = proxy_streams(tcp, send, recv).await;
            });
            cancellation.track(&handle);
        }
    });

    Ok(handle)
}

pub(super) async fn connect_v3(
    address: SocketAddr,
    remote: &str,
    identity: &PeltIdentity,
    expected_peer: &ExpectedPeerIdentity,
) -> std::io::Result<(quinn::Connection, SendStream, RecvStream)> {
    // A selected peer's full Pack key is required before creating an endpoint
    // or emitting a QUIC datagram. Fingerprints remain transcript identifiers,
    // not TLS trust material.
    let crypto = crate::tls_identity::client_config_for_selected_peer_key(
        expected_peer.public_key_b64.as_deref(),
    )
    .map_err(|_| hs::rejected())?;
    let quic_crypto =
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto).map_err(|_| hs::rejected())?;
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse().map_err(|_| hs::rejected())?)
        .map_err(|_| hs::rejected())?;
    endpoint.set_default_client_config(ClientConfig::new(Arc::new(quic_crypto)));

    // Quinn parses this textual IP as ServerName::IpAddress, which suppresses
    // SNI. No synthetic DNS identity is introduced.
    let server_name = address.ip().to_string();
    let connecting = endpoint
        .connect(address, &server_name)
        .map_err(|_| hs::rejected())?;
    let connection = tokio::time::timeout(hs::READ_WINDOW, connecting)
        .await
        .map_err(|_| hs::rejected())?
        .map_err(|_| hs::rejected())?;
    let deadline = tokio::time::Instant::now() + hs::CLIENT_WINDOW;
    let result = tokio::time::timeout_at(deadline, async {
        let binding = hs::quic_binding(&connection)?;
        let (mut send, mut recv) = connection.open_bi().await.map_err(|_| hs::rejected())?;
        let open = hs::QuicOpen::new(identity, &expected_peer.fingerprint, remote, &binding)?;
        let retained = open.transcript(&binding)?;
        hs::write(&mut send, &open, hs::MESSAGE_LIMIT, deadline).await?;
        let ack: hs::QuicAck = hs::read(&mut recv, hs::MESSAGE_LIMIT, deadline).await?;
        hs::require(ack.receiver_fingerprint == expected_peer.fingerprint)?;
        hs::verify(
            &ack.receiver_pubkey,
            &ack.transcript(&open, &retained)?,
            &ack.signature,
        )?;
        Ok::<_, std::io::Error>((send, recv))
    })
    .await
    .map_err(|_| hs::rejected())
    .and_then(|v| v);
    match result {
        Ok((send, recv)) => Ok((connection, send, recv)),
        Err(error) => {
            connection.close(hs::V3_REJECT_CODE.into(), b"");
            Err(error)
        }
    }
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
