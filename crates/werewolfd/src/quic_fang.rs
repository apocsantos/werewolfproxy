#![allow(dead_code)]

use crate::handshake as hs;
use crate::policy::ExpectedPeerIdentity;
use crate::transport::fang_accept::Tracked;
use quinn::{ClientConfig, Endpoint, RecvStream, SendStream};
use std::sync::Arc;
use std::time::Duration;
use std::{
    error::Error,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
};
use tokio::{
    io::{copy, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use werewolf_core::pelt::PeltIdentity;

/// The initiator sends an ack-eliciting PING after this much silence. An
/// idle Fang session therefore outlives the 30 s idle timeout while the path
/// is up; a dead path still ends it at that timeout.
const QUIC_KEEP_ALIVE: Duration = Duration::from_secs(10);

/// How often the initiator checks for the receiver's handshake confirmation.
const CONFIRM_POLL: Duration = Duration::from_millis(5);

// Resources the remote receiver controls on the initiator (Stage41). The
// initiator opens its own streams and never accepts one, and Werewolf has no
// QUIC DATAGRAM protocol, so the receiver gets no stream credit and no
// datagram buffer. The connection receive window equals Quinn's default
// stream receive window, so the one stream's data path is unchanged while
// the total the receiver can make the initiator hold is bounded. The stream
// receive and send windows stay at Quinn's defaults (the send window is the
// Stage34 surface). The receiver's own bounds are in `transport/quic.rs`.
const PEER_BIDI_STREAMS: u32 = 0;
const PEER_UNI_STREAMS: u32 = 0;
const CONNECTION_RECEIVE_WINDOW: u32 = 1_250_000;

pub async fn open_quic_fang(
    local_addr: String,
    quic_server: Vec<SocketAddr>,
    remote_addr: String,
    identity: PeltIdentity,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<std::io::Result<()>>,
    expected_peer: crate::policy::ExpectedPeerIdentity,
) -> Result<JoinHandle<()>, Box<dyn Error + Send + Sync>> {
    let handle = tokio::spawn(async move {
        let peer = match QuicPeer::open(&expected_peer, quic_server) {
            Ok(peer) => peer,
            Err(error) => {
                let _ = ready.send(Err(std::io::Error::new(error.kind(), error.to_string())));
                return;
            }
        };
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
            local_addr,
            peer.describe(),
            remote_addr
        );

        let mut acceptor = crate::transport::fang_accept::FangAcceptor::new(listener, &local_addr);
        loop {
            let (tcp, _, slot) = acceptor.accept().await;
            if crate::transport::arm_reset_on_close(&tcp).is_err() {
                continue;
            }

            let health = cancellation.health();
            let remote_addr = remote_addr.clone();
            let identity = identity.clone();
            let expected_peer = expected_peer.clone();
            let peer = peer.clone();

            cancellation.spawn(async move {
                // Evicted (Stage43): the session is dropped and the armed
                // local socket closes with a reset.
                slot.serve(async move {
                    let result =
                        connect_v3_on(&peer, &remote_addr, &identity, &expected_peer).await;
                    let (connection, send, recv) = match result {
                        Ok(session) => session,
                        Err(error) => {
                            health.setup_failed(&error);
                            return;
                        }
                    };
                    let _connection_close = CloseClientConnection(connection);
                    let mut tcp = tcp;
                    let _ = proxy_streams(&mut tcp, send, recv).await;
                })
                .await;
            });
        }
    });

    Ok(handle)
}

pub(crate) struct CloseClientConnection(pub(crate) quinn::Connection);

impl Drop for CloseClientConnection {
    fn drop(&mut self) {
        // The Fang reuses its UDP endpoint across local routes, so each
        // completed connection is closed explicitly instead of remaining
        // idle until Quinn's transport timeout.
        self.0.close(0u32.into(), b"");
    }
}

pub(super) async fn connect_v3(
    address: SocketAddr,
    remote: &str,
    identity: &PeltIdentity,
    expected_peer: &ExpectedPeerIdentity,
) -> std::io::Result<(quinn::Connection, SendStream, RecvStream)> {
    let peer = QuicPeer::open(expected_peer, vec![address])?;
    connect_v3_on(&peer, remote, identity, expected_peer).await
}

/// A QUIC peer's addresses and this node's client endpoints towards them
/// (Stage46). Every dial races the addresses (`transport::dial::race`)
/// inside `hs::READ_WINDOW`, the window one dial had before. One endpoint
/// per address family (S46-D3, E1): the first address's family is bound
/// when the peer is opened, so a failure to bind fails the open as before;
/// the other family only when one of its addresses is first tried, so a
/// host without IPv6 never binds it for an IPv4 peer.
#[derive(Clone)]
pub(crate) struct QuicPeer(Arc<QuicPeerInner>);

struct QuicPeerInner {
    expected: ExpectedPeerIdentity,
    addresses: Vec<SocketAddr>,
    v4: std::sync::Mutex<Option<Endpoint>>,
    v6: std::sync::Mutex<Option<Endpoint>>,
}

impl QuicPeer {
    pub(crate) fn open(
        expected: &ExpectedPeerIdentity,
        addresses: Vec<SocketAddr>,
    ) -> std::io::Result<Self> {
        let addresses = crate::transport::dial::interleave(addresses);
        let first = *addresses
            .first()
            .ok_or_else(|| hs::local_dial_failed("no address to dial"))?;
        let peer = Self(Arc::new(QuicPeerInner {
            expected: expected.clone(),
            addresses,
            v4: Default::default(),
            v6: Default::default(),
        }));
        peer.endpoint_for(first)?;
        Ok(peer)
    }

    pub(crate) fn addresses(&self) -> &[SocketAddr] {
        &self.0.addresses
    }

    /// The addresses, for a log line.
    pub(crate) fn describe(&self) -> String {
        let text: Vec<String> = self.0.addresses.iter().map(|a| a.to_string()).collect();
        text.join(", ")
    }

    fn endpoint_for(&self, address: SocketAddr) -> std::io::Result<Endpoint> {
        let slot = if address.is_ipv6() {
            &self.0.v6
        } else {
            &self.0.v4
        };
        let mut slot = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(endpoint) = slot.as_ref() {
            return Ok(endpoint.clone());
        }
        let endpoint = client_endpoint(&self.0.expected, Some(address))?;
        *slot = Some(endpoint.clone());
        Ok(endpoint)
    }

    /// The QUIC connection to the first address that answers.
    pub(crate) async fn dial(&self) -> std::io::Result<quinn::Connection> {
        let peer = self.clone();
        let raced = crate::transport::dial::race(&self.0.addresses, move |address| {
            let endpoint = peer.endpoint_for(address);
            async move {
                let endpoint = endpoint?;
                // Stage49 R1: a refusal ends the attempt at once; otherwise
                // the QUIC dial runs exactly as before.
                tokio::select! {
                    result = dial(&endpoint, address) => result,
                    refused = refusal(address) => Err(refused),
                }
            }
        });
        tokio::time::timeout(hs::READ_WINDOW, raced)
            .await
            .map_err(|_| hs::no_answer())?
    }
}

/// Stage49 R1: completes only if `address` refuses UDP, with that refusal.
///
/// quinn's endpoint socket serves every peer and is not connected, so the
/// peer's ICMP port-unreachable (nothing listens there) never reaches it and
/// the dial waits its whole window. A separate UDP socket connected to the
/// same address sends one 1-byte datagram, too short to be a QUIC packet (a
/// live receiver's endpoint drops it, Stage49 M1); the kernel reports the
/// ICMP error on that socket as `ECONNREFUSED`. Anything else (an answer, no
/// answer, a socket that cannot be opened, as under descriptor exhaustion)
/// leaves the QUIC dial alone: this future then never completes.
async fn refusal(address: SocketAddr) -> std::io::Error {
    match refusal_probe(address).await {
        Ok(refused) => refused,
        Err(_) => std::future::pending().await,
    }
}

async fn refusal_probe(address: SocketAddr) -> std::io::Result<std::io::Error> {
    let probe = tokio::net::UdpSocket::bind(client_bind(Some(address))).await?;
    let probe = distinct_probe_socket(probe, address).await?;
    probe.connect(address).await?;
    let mut buffer = [0u8; 16];
    let mut outcome = probe.send(&[0]).await.map(|_| ());
    loop {
        match outcome {
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                return Ok(hs::refused_by_peer());
            }
            Err(error) => return Err(error),
            // A datagram back (not expected from a QUIC peer): keep waiting.
            Ok(()) => outcome = probe.recv(&mut buffer).await.map(|_| ()),
        }
    }
}

/// A local closed destination can be the ephemeral port just allocated to
/// the probe itself. In that case UDP would deliver the probe back to
/// itself instead of reporting a refusal. Retry the bind once, keeping at
/// most one probe socket; another collision skips the probe like any other
/// local error rather than spinning or sending to itself.
async fn distinct_probe_socket(
    probe: tokio::net::UdpSocket,
    address: SocketAddr,
) -> std::io::Result<tokio::net::UdpSocket> {
    if probe.local_addr()?.port() != address.port() {
        return Ok(probe);
    }
    drop(probe);
    let replacement = tokio::net::UdpSocket::bind(client_bind(Some(address))).await?;
    if replacement.local_addr()?.port() == address.port() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "probe source port matches its destination",
        ));
    }
    Ok(replacement)
}

/// The local address of a QUIC client endpoint that will dial `destination`
/// (Stage44 V1). Quinn refuses an IPv6 destination on an IPv4 endpoint, so an
/// IPv6 destination gets an IPv6 endpoint (dual-stack where the host allows).
/// Every other case, including a destination that is not a literal socket
/// address, keeps the IPv4 endpoint exactly as before: binding IPv6 on a host
/// with IPv6 disabled would fail.
pub(crate) fn client_bind(destination: Option<SocketAddr>) -> SocketAddr {
    match destination {
        Some(address) if address.is_ipv6() => (Ipv6Addr::UNSPECIFIED, 0).into(),
        _ => (Ipv4Addr::UNSPECIFIED, 0).into(),
    }
}

pub(crate) fn client_endpoint(
    expected_peer: &ExpectedPeerIdentity,
    destination: Option<SocketAddr>,
) -> std::io::Result<Endpoint> {
    // A selected peer's full Pack key is required before creating an endpoint
    // or emitting a QUIC datagram. Fingerprints remain transcript identifiers,
    // not TLS trust material.
    let crypto = crate::tls_identity::client_config_for_selected_peer_key(
        expected_peer.public_key_b64.as_deref(),
    )
    .map_err(|_| hs::rejected())?;
    let quic_crypto =
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto).map_err(|_| hs::rejected())?;
    // Binding the UDP socket is local: a failure (no descriptor, no IPv6) is
    // a dial that made no connection (Stage46 L1), not a refusal.
    let mut endpoint = Endpoint::client(client_bind(destination)).map_err(hs::local_dial_failed)?;
    let mut transport = quinn::TransportConfig::default();
    transport
        .keep_alive_interval(Some(QUIC_KEEP_ALIVE))
        .max_concurrent_bidi_streams(quinn::VarInt::from_u32(PEER_BIDI_STREAMS))
        .max_concurrent_uni_streams(quinn::VarInt::from_u32(PEER_UNI_STREAMS))
        .receive_window(quinn::VarInt::from_u32(CONNECTION_RECEIVE_WINDOW))
        .datagram_receive_buffer_size(None);
    let mut config = ClientConfig::new(Arc::new(quic_crypto));
    config.transport_config(Arc::new(transport));
    endpoint.set_default_client_config(config);
    Ok(endpoint)
}

/// Waits until the receiver has confirmed the QUIC handshake (Stage32 F5).
/// Quinn completes the initiator's handshake when it sends its Finished, but
/// discards 1-RTT packets that reach the receiver before that Finished. On a
/// reordering path the first application stream was lost that way, unacked,
/// and the receiver's stream window closed before a retransmission. The
/// receiver sends HANDSHAKE_DONE only after its own handshake completed;
/// Quinn has no event for it, so this polls its frame counter.
pub(crate) async fn handshake_confirmed(connection: &quinn::Connection) -> std::io::Result<()> {
    while connection.stats().frame_rx.handshake_done == 0 {
        if connection.close_reason().is_some() {
            return Err(hs::rejected());
        }
        tokio::time::sleep(CONFIRM_POLL).await;
    }
    Ok(())
}

/// The QUIC dial shared by Fang sessions, discovery and sync: the TLS
/// handshake to `address`, within `hs::READ_WINDOW`. Only the error kind
/// changes with Stage45 (S1a): a dial refused locally before any packet is
/// `AddrNotAvailable` and one that hears nothing back is `TimedOut`, both
/// "no connection was made"; a peer that answered and refused stays
/// `hs::rejected()`. Nothing on the wire changes.
pub(crate) async fn dial(
    endpoint: &Endpoint,
    address: SocketAddr,
) -> std::io::Result<quinn::Connection> {
    // Quinn parses this textual IP as ServerName::IpAddress, which suppresses
    // SNI. No synthetic DNS identity is introduced.
    let connecting = endpoint
        .connect(address, &address.ip().to_string())
        .map_err(hs::local_dial_failed)?;
    tokio::time::timeout(hs::READ_WINDOW, connecting)
        .await
        .map_err(|_| hs::no_answer())?
        .map_err(|error| match error {
            quinn::ConnectionError::TimedOut => hs::no_answer(),
            _ => hs::rejected(),
        })
}

pub(crate) async fn connect_v3_on(
    peer: &QuicPeer,
    remote: &str,
    identity: &PeltIdentity,
    expected_peer: &ExpectedPeerIdentity,
) -> std::io::Result<(quinn::Connection, SendStream, RecvStream)> {
    let connection = peer.dial().await?;
    let deadline = tokio::time::Instant::now() + hs::CLIENT_WINDOW;
    let result = tokio::time::timeout_at(deadline, async {
        handshake_confirmed(&connection).await?;
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

/// How long a finished upload may wait for the receiver to acknowledge all of
/// it before the session counts as failed. The connection's idle timeout still
/// ends it sooner on a dead path.
pub(crate) const UPLOAD_ACK_WINDOW: Duration = Duration::from_secs(60);

/// Owns the QUIC send stream and resets it when dropped unless the session
/// ended cleanly. Quinn's own `Drop` would finish the stream, a clean FIN, so
/// an error or a cancelled task could otherwise look like a complete upload
/// (docs/SESSION_END_SIGNALLING_PROPOSAL.md, gap 2).
pub(crate) struct AbortiveSend {
    pub(crate) stream: SendStream,
    pub(crate) clean: bool,
}

impl Drop for AbortiveSend {
    fn drop(&mut self) {
        if !self.clean {
            let _ = self.stream.reset(hs::V3_REJECT_CODE.into());
        }
    }
}

pub(crate) async fn proxy_streams(
    tcp: &mut TcpStream,
    send: SendStream,
    mut recv: RecvStream,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut send = AbortiveSend {
        stream: send,
        clean: false,
    };
    // Borrowed halves do not shut down on drop, so a failed session closes
    // the armed socket with RST instead of an implicit FIN.
    let (rd, wr) = tcp.split();
    // Local activity, for Stage43 idle eviction.
    let (mut rd, mut wr) = (Tracked::new(rd), Tracked::new(wr));

    let up = async {
        copy(&mut rd, &mut send.stream).await?;
        // A local TCP EOF is a completed QUIC send direction.  Without this
        // explicit FIN, the receiver's stream copy waits for its idle timeout
        // and retains the Stage11C authority lease for every short-lived Fang
        // connection.
        send.stream.finish()?;
        Ok::<(), std::io::Error>(())
    };
    let down = async {
        copy(&mut recv, &mut wr).await?;
        wr.shutdown().await?;
        Ok::<(), std::io::Error>(())
    };

    tokio::try_join!(up, down)?;
    // The caller closes the connection next, and Quinn's close abandons data
    // the peer has not acknowledged. Wait until the receiver has acknowledged
    // the whole upload; a stop or a lost connection is a failed session.
    match tokio::time::timeout(UPLOAD_ACK_WINDOW, send.stream.stopped()).await {
        Ok(Ok(None)) => {}
        _ => return Err("upload not acknowledged by the receiver".into()),
    }
    send.clean = true;
    crate::transport::disarm_reset_on_close(tcp)?;

    Ok(())
}

#[cfg(test)]
mod tests;
