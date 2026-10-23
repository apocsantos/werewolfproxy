//! Stage26 routed Fangs (`docs/STAGE26_MULTIHOP_ROUTING_DESIGN.md`,
//! decision 1A).
//!
//! For each local connection, the initiator opens an ordinary Fang session
//! to the relay whose OPEN target is the destination's encrypted-TCP
//! listener; the relay authorizes that like any target. Inside that stream
//! it then runs the unchanged encrypted-TCP client to the destination: TLS
//! pinned to the destination's Pack key, the destination's challenge, a
//! signed OPEN for the real target and a verified ACK. The relay sees the
//! outer session only; the destination authorizes the initiator end to end.

use super::tcp_encrypted::{
    client_handshake, connect_authenticated_over, connect_authenticated_tcp,
    secure_copy_client_side,
};
use crate::{
    handshake as hs,
    policy::{select_peer_transport, ExpectedPeerIdentity},
    quic_fang::{AbortiveSend, CloseClientConnection, UPLOAD_ACK_WINDOW},
};
use std::net::SocketAddr;
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
};
use werewolf_core::{pack::PeerRecord, pelt::PeltIdentity, route::RouteHop};

/// A route resolved against the current Pack at Fang open.
#[derive(Clone)]
pub(crate) struct Routed {
    pub(crate) via: String,
    pub(crate) hop: RouteHop,
    /// The relay's address for `hop`, `IP:port`.
    pub(crate) relay_address: String,
    pub(crate) relay: ExpectedPeerIdentity,
    /// The destination's encrypted-TCP listener, a literal address: the
    /// relay's OPEN target and the inner TLS server name.
    pub(crate) destination: SocketAddr,
}

impl Routed {
    /// What `fang active` shows, e.g. `tcp via wolf-b (quic)`.
    pub(crate) fn describe(&self) -> String {
        format!("tcp via {} ({})", self.via, self.hop.as_str())
    }
}

/// Resolves `route` for a Fang to `destination`. Plain TCP is never carried
/// through a relay, both ends must still be Pack members, and the
/// destination's TCP address must be a literal IP: it is both the relay's
/// target and the inner TLS server name, so it can never resolve to
/// something else.
pub(crate) fn resolve(
    route: &werewolf_core::route::Route,
    destination: &PeerRecord,
    peers: &[PeerRecord],
    requested: &str,
) -> Result<Routed, (&'static str, String)> {
    if requested == "tcp-plain" {
        return Err((
            "FANG_ROUTE_INVALID",
            "plain TCP is never carried through a relay".into(),
        ));
    }
    let relay = peers
        .iter()
        .find(|peer| peer.name == route.via)
        .ok_or_else(|| {
            (
                "FANG_ROUTE_INVALID",
                format!("relay {} is not a Pack member", route.via),
            )
        })?;
    let relay_address = match select_peer_transport(route.hop.as_str(), &relay.address) {
        Ok(super::FangTransport::Quic(address) | super::FangTransport::Tcp(address)) => address,
        Err(_) => {
            return Err((
                "FANG_ROUTE_INVALID",
                format!(
                    "relay {} has no usable {} address",
                    route.via,
                    route.hop.as_str()
                ),
            ))
        }
    };
    let destination_address = match select_peer_transport("tcp", &destination.address) {
        Ok(super::FangTransport::Tcp(address)) => address.parse::<SocketAddr>().ok(),
        _ => None,
    }
    .ok_or_else(|| {
        (
            "FANG_BAD_TCP_ADDRESS",
            "a routed peer needs a literal tcp:// IP:port address".into(),
        )
    })?;
    Ok(Routed {
        via: route.via.clone(),
        hop: route.hop,
        relay_address,
        relay: ExpectedPeerIdentity {
            fingerprint: relay.fingerprint.clone(),
            public_key_b64: relay.public_key_b64.clone(),
        },
        destination: destination_address,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_routed_forwarder(
    fang_id: &str,
    local: &str,
    routed: Routed,
    remote: &str,
    identity: PeltIdentity,
    destination: ExpectedPeerIdentity,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<io::Result<()>>,
) -> io::Result<()> {
    // One QUIC endpoint per activation, as a direct QUIC Fang does.
    let endpoint = match routed.hop {
        RouteHop::Quic => match crate::quic_fang::client_endpoint(&routed.relay) {
            Ok(endpoint) => Some(endpoint),
            Err(error) => {
                let _ = ready.send(Err(io::Error::new(error.kind(), error.to_string())));
                return Err(error);
            }
        },
        RouteHop::Tcp => None,
    };
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
    println!(
        "🦷 {} listening locally on {} → {} via {}",
        fang_id, local, routed.destination, routed.via
    );

    loop {
        let (mut inbound, _) = listener.accept().await?;
        if super::arm_reset_on_close(&inbound).is_err() {
            continue;
        }
        let routed = routed.clone();
        let remote = remote.to_string();
        let identity = identity.clone();
        let destination = destination.clone();
        let endpoint = endpoint.clone();
        cancellation.spawn(async move {
            let result = routed_session(
                &mut inbound,
                &routed,
                endpoint.as_ref(),
                &remote,
                &identity,
                &destination,
            )
            .await;
            // Any failure leaves the local socket armed, so the application
            // sees a reset, never a session that looks complete.
            if result.is_ok() {
                let _ = super::disarm_reset_on_close(&inbound);
            }
        });
    }
}

async fn routed_session(
    inbound: &mut TcpStream,
    routed: &Routed,
    endpoint: Option<&quinn::Endpoint>,
    remote: &str,
    identity: &PeltIdentity,
    destination: &ExpectedPeerIdentity,
) -> io::Result<()> {
    let target = routed.destination.to_string();
    match (routed.hop, endpoint) {
        (RouteHop::Quic, Some(endpoint)) => {
            let relay: SocketAddr = routed.relay_address.parse().map_err(|_| hs::rejected())?;
            let (connection, send, mut recv) =
                crate::quic_fang::connect_v3_on(endpoint, relay, &target, identity, &routed.relay)
                    .await?;
            let _close = CloseClientConnection(connection);
            let mut send = AbortiveSend {
                stream: send,
                clean: false,
            };
            inner_session(
                inbound,
                tokio::io::join(&mut recv, &mut send.stream),
                routed,
                remote,
                identity,
                destination,
            )
            .await?;
            // When the destination finished first, the inner session ends
            // without finishing the outer stream; finish it now (an error
            // means it already was), or the wait below would last the whole
            // window. The destination still sees the upload end without
            // close_notify, so its target gets a reset, never a clean end.
            let _ = send.stream.finish();
            // As a direct QUIC Fang: the session is complete only once the
            // relay has acknowledged the whole upload.
            match tokio::time::timeout(UPLOAD_ACK_WINDOW, send.stream.stopped()).await {
                Ok(Ok(None)) => {}
                _ => return Err(hs::rejected()),
            }
            send.clean = true;
            Ok(())
        }
        (RouteHop::Tcp, _) => {
            let mut outer = connect_authenticated_tcp(&routed.relay_address, &routed.relay).await?;
            let key = client_handshake(&mut outer, &target, identity, &routed.relay).await?;
            // The outer session's framed copy loop feeds one end of a pipe;
            // the inner session runs over the other. `far` outlives the pump:
            // when the relay ends the outer session first (the destination
            // finished), the inner session must still drain what already
            // arrived, so its writes into the pipe wait instead of failing.
            let (near, mut far) = tokio::io::duplex(64 * 1024);
            let pump = secure_copy_client_side(&mut far, outer, key);
            let inner = inner_session(inbound, near, routed, remote, identity, destination);
            tokio::pin!(pump, inner);
            tokio::select! {
                result = &mut inner => {
                    // A failed inner session drops the pump, so the relay
                    // sees the outer session end without close_notify.
                    result?;
                    pump.await
                }
                result = &mut pump => {
                    result?;
                    inner.await
                }
            }
        }
        (RouteHop::Quic, None) => Err(hs::rejected()),
    }
}

/// The byte stream to the destination's listener through the relay, as
/// handed to `over_relay`'s closure.
pub(crate) type RelayStream = Box<dyn RelayIo>;
pub(crate) trait RelayIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> RelayIo for T {}

/// One short exchange with the destination through the relay (Stage26
/// decision 5b: discovery RPCs). Opens the outer session exactly as a
/// routed Fang does and runs `exchange` over the stream inside it. Unlike a
/// Fang session there is no upload to account for: when `exchange` ends,
/// both sessions are dropped.
pub(crate) async fn over_relay<T, F, Fut>(
    routed: &Routed,
    identity: &PeltIdentity,
    exchange: F,
) -> io::Result<T>
where
    F: FnOnce(RelayStream) -> Fut,
    Fut: std::future::Future<Output = io::Result<T>>,
{
    let target = routed.destination.to_string();
    match routed.hop {
        RouteHop::Quic => {
            let relay: SocketAddr = routed.relay_address.parse().map_err(|_| hs::rejected())?;
            let endpoint = crate::quic_fang::client_endpoint(&routed.relay)?;
            let (connection, send, recv) =
                crate::quic_fang::connect_v3_on(&endpoint, relay, &target, identity, &routed.relay)
                    .await?;
            let _close = CloseClientConnection(connection);
            exchange(Box::new(tokio::io::join(recv, send))).await
        }
        RouteHop::Tcp => {
            let mut outer = connect_authenticated_tcp(&routed.relay_address, &routed.relay).await?;
            let key = client_handshake(&mut outer, &target, identity, &routed.relay).await?;
            let (near, mut far) = tokio::io::duplex(64 * 1024);
            let pump = secure_copy_client_side(&mut far, outer, key);
            let exchange = exchange(Box::new(near));
            tokio::pin!(pump, exchange);
            tokio::select! {
                result = &mut exchange => result,
                result = &mut pump => {
                    result?;
                    exchange.await
                }
            }
        }
    }
}

/// The unchanged encrypted-TCP client to the destination, over `carrier`.
async fn inner_session<C>(
    inbound: &mut TcpStream,
    carrier: C,
    routed: &Routed,
    remote: &str,
    identity: &PeltIdentity,
    destination: &ExpectedPeerIdentity,
) -> io::Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin,
{
    let mut tls = connect_authenticated_over(carrier, routed.destination.ip(), destination).await?;
    let key = client_handshake(&mut tls, remote, identity, destination).await?;
    secure_copy_client_side(inbound, tls, key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use werewolf_core::{pack::TrustLevel, route::Route};

    fn member(name: &str, address: &str) -> PeerRecord {
        PeerRecord {
            name: name.into(),
            fingerprint: format!("wwp1:{name}"),
            address: address.into(),
            trust: TrustLevel::Packmate,
            public_key_b64: Some(format!("key-{name}")),
        }
    }

    fn route(hop: RouteHop) -> Route {
        Route {
            peer: "wolf-d".into(),
            via: "wolf-b".into(),
            hop,
        }
    }

    #[test]
    fn resolves_relay_hop_and_literal_destination() {
        let peers = [
            member("wolf-b", "quic://198.51.100.2:9560"),
            member("wolf-d", "tcp://10.0.0.4:8443"),
        ];
        let routed = resolve(&route(RouteHop::Quic), &peers[1], &peers, "quic").unwrap();
        assert_eq!(routed.relay_address, "198.51.100.2:9560");
        assert_eq!(routed.relay.fingerprint, "wwp1:wolf-b");
        assert_eq!(routed.relay.public_key_b64.as_deref(), Some("key-wolf-b"));
        assert_eq!(routed.destination, "10.0.0.4:8443".parse().unwrap());
        assert_eq!(routed.describe(), "tcp via wolf-b (quic)");
        // A tcp hop to a quic:// relay dials TCP on the same port, as for
        // direct Fangs; a quic:// destination is reached by TCP the same way.
        let peers = [
            member("wolf-b", "quic://198.51.100.2:9560"),
            member("wolf-d", "quic://10.0.0.4:9560"),
        ];
        let routed = resolve(&route(RouteHop::Tcp), &peers[1], &peers, "tcp").unwrap();
        assert_eq!(routed.relay_address, "198.51.100.2:9560");
        assert_eq!(routed.destination, "10.0.0.4:9560".parse().unwrap());
    }

    #[test]
    fn refuses_plain_tcp_missing_relays_and_unusable_addresses() {
        let peers = [
            member("wolf-b", "tcp://198.51.100.2:8443"),
            member("wolf-d", "tcp://10.0.0.4:8443"),
        ];
        let code = |result: Result<Routed, (&'static str, String)>| result.err().unwrap().0;
        assert_eq!(
            code(resolve(
                &route(RouteHop::Tcp),
                &peers[1],
                &peers,
                "tcp-plain"
            )),
            "FANG_ROUTE_INVALID"
        );
        assert_eq!(
            code(resolve(
                &route(RouteHop::Tcp),
                &peers[1],
                &peers[1..],
                "quic"
            )),
            "FANG_ROUTE_INVALID"
        );
        // A tcp:// relay is never dialled as QUIC.
        assert_eq!(
            code(resolve(&route(RouteHop::Quic), &peers[1], &peers, "quic")),
            "FANG_ROUTE_INVALID"
        );
        // The destination must be a literal IP: it is the relay's target and
        // the inner TLS server name.
        let named = member("wolf-d", "tcp://wolf-d.internal:8443");
        assert_eq!(
            code(resolve(&route(RouteHop::Tcp), &named, &peers, "quic")),
            "FANG_BAD_TCP_ADDRESS"
        );
    }
}
