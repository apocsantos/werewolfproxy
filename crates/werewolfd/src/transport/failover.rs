//! Stage27 provider Fangs (`docs/STAGE27_PROVIDER_SELECTION_DESIGN.md`).
//!
//! A service with fallbacks is served by one local listener whose every
//! connection tries the providers in operator order (decision 3a). Each
//! attempt is the ordinary session to that provider (direct QUIC, direct
//! encrypted TCP, or Stage26 routed), so each provider authorizes the
//! requester itself. A provider is abandoned only before its session is
//! established, that is before its ACK verifies and before any application
//! byte is read from the local socket (decision 2a); a failure after that
//! point resets the local connection, never retried. The whole selection
//! for one connection has `hs::CLIENT_WINDOW` (decision 4a); when every
//! provider fails, or the window ends, the local connection is reset.

use super::fang_health::FailureClass;
use super::relay::{self, Routed};
use super::tcp_encrypted::{connect_tcp_session, secure_copy_client_side};
use crate::{
    handshake as hs,
    policy::{select_peer_transport, ExpectedPeerIdentity},
    quic_fang::{connect_v3_on, proxy_streams, CloseClientConnection},
};
use std::net::SocketAddr;
use tokio::{
    io,
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::{sleep_until, Instant},
};
use werewolf_core::{
    pack::PeerRecord, pelt::PeltIdentity, route::Route, service::ServiceDefinition,
};

/// How one provider is reached, resolved against the Pack when the Fang
/// opens.
#[derive(Clone)]
pub(crate) enum Path {
    /// Every address the provider's QUIC address names, resolved at open.
    Quic(Vec<SocketAddr>),
    Tcp(String),
    Routed(Box<Routed>),
}

#[derive(Clone)]
pub(crate) struct Provider {
    pub(crate) peer: String,
    pub(crate) remote: String,
    pub(crate) expected: ExpectedPeerIdentity,
    pub(crate) path: Path,
}

impl Provider {
    /// The Pack members this provider depends on: itself and its relay.
    pub(crate) fn uses(&self) -> Vec<String> {
        match &self.path {
            Path::Routed(routed) => vec![self.peer.clone(), routed.via.clone()],
            _ => vec![self.peer.clone()],
        }
    }
}

/// Resolves every provider of `service` for `requested` (`quic` or `tcp`;
/// a routed provider's hop comes from its route). Every provider must
/// resolve, or the open fails: the list never silently shrinks. Plain TCP
/// is not used with fallbacks.
pub(crate) fn resolve(
    service: &ServiceDefinition,
    peers: &[PeerRecord],
    routes: &[Route],
    requested: &str,
    resolved: &super::dial::Resolved,
) -> Result<Vec<Provider>, (&'static str, String)> {
    if requested == "tcp-plain" {
        return Err((
            "FANG_INVALID",
            "plain TCP is not used with service fallbacks".into(),
        ));
    }
    service
        .providers()
        .map(|(peer, remote)| {
            let record = peers
                .iter()
                .find(|p| p.name == peer)
                .ok_or_else(|| ("FANG_UNKNOWN_PEER", format!("Unknown peer: {peer}")))?;
            let expected = ExpectedPeerIdentity {
                fingerprint: record.fingerprint.clone(),
                public_key_b64: record.public_key_b64.clone(),
            };
            let path = match routes.iter().find(|route| route.peer == peer) {
                Some(route) => Path::Routed(Box::new(relay::resolve(
                    route, record, peers, requested, resolved,
                )?)),
                None => match select_peer_transport(requested, &record.address) {
                    Ok(super::FangTransport::Quic(address)) => {
                        // Stage46 A1: a name is resolved before the open.
                        Path::Quic(resolved.addresses(&address).map_err(|reason| {
                            (
                                "FANG_BAD_QUIC_ADDRESS",
                                format!("{peer}: QUIC address {address} {reason}"),
                            )
                        })?)
                    }
                    Ok(super::FangTransport::Tcp(address)) => Path::Tcp(address),
                    Err(_) => {
                        return Err((
                            "FANG_BAD_ADDRESS",
                            format!("{peer} has no usable {requested} address"),
                        ))
                    }
                },
            };
            Ok(Provider {
                peer: peer.to_owned(),
                remote: remote.to_owned(),
                expected,
                path,
            })
        })
        .collect()
}

/// A provider with the QUIC peer (its endpoints) its attempts reuse, as a
/// direct QUIC Fang reuses one per activation.
struct Ready {
    provider: Provider,
    endpoint: Option<crate::quic_fang::QuicPeer>,
}

pub(crate) async fn run_failover_forwarder(
    fang_id: &str,
    local: &str,
    providers: Vec<Provider>,
    identity: PeltIdentity,
    cancellation: crate::fang_registry::FangCancellation,
    ready: oneshot::Sender<io::Result<()>>,
) -> io::Result<()> {
    let mut prepared = Vec::with_capacity(providers.len());
    for provider in providers {
        let peer = match &provider.path {
            Path::Quic(addresses) => Some(crate::quic_fang::QuicPeer::open(
                &provider.expected,
                addresses.clone(),
            )),
            Path::Routed(routed) if routed.hop == werewolf_core::route::RouteHop::Quic => {
                Some(relay::relay_peer(routed).await)
            }
            _ => None,
        };
        let endpoint = match peer {
            None => None,
            Some(Ok(peer)) => Some(peer),
            Some(Err(error)) => {
                let _ = ready.send(Err(io::Error::new(error.kind(), error.to_string())));
                return Err(error);
            }
        };
        prepared.push(Ready { provider, endpoint });
    }
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
    let order: Vec<&str> = prepared.iter().map(|p| p.provider.peer.as_str()).collect();
    println!(
        "🦷 {} listening locally on {} → {}",
        fang_id,
        local,
        order.join(", ")
    );
    let prepared = std::sync::Arc::new(prepared);

    let mut acceptor = super::fang_accept::FangAcceptor::new(listener, fang_id);
    loop {
        let (mut inbound, _, slot) = acceptor.accept().await;
        if super::arm_reset_on_close(&inbound).is_err() {
            continue;
        }
        let prepared = prepared.clone();
        let identity = identity.clone();
        let health = cancellation.health();
        cancellation.spawn(async move {
            let served = slot
                .serve(select_and_serve(&mut inbound, &prepared, &identity))
                .await;
            match served {
                Some(Served::Clean) => {
                    let _ = super::disarm_reset_on_close(&inbound);
                }
                // No provider established a session (Stage45).
                Some(Served::NoProvider(class, detail)) => health.record(class, &detail),
                Some(Served::Failed) | None => {}
            }
            // Otherwise (failed, or evicted in Stage43) the socket stays
            // armed: the application sees a reset, never a session that
            // looks complete.
        });
    }
}

/// How one local connection's provider selection ended.
#[derive(Debug, PartialEq)]
enum Served {
    /// A provider's session ended cleanly.
    Clean,
    /// A provider's session failed after it was established.
    Failed,
    /// No provider established a session (every one failed, or the window
    /// ended): `handshake` if any provider was reached, otherwise `dial`;
    /// and each provider's error, in order.
    NoProvider(FailureClass, String),
}

/// Tries the providers in order for one local connection.
async fn select_and_serve(
    inbound: &mut TcpStream,
    providers: &[Ready],
    identity: &PeltIdentity,
) -> Served {
    let deadline = Instant::now() + hs::CLIENT_WINDOW;
    let mut class = FailureClass::Dial;
    let mut errors = Vec::new();
    for ready in providers {
        let (established_tx, mut established) = oneshot::channel();
        let attempt = attempt(inbound, ready, identity, established_tx);
        tokio::pin!(attempt);
        tokio::select! {
            result = &mut attempt => {
                let error = match result {
                    Ok(()) => return Served::Clean,
                    Err(error) => error,
                };
                if established.try_recv().is_ok() {
                    // Failed after the session was established: bytes may
                    // have moved, so never try another provider.
                    return Served::Failed;
                }
                // Failed before establishment: nothing was read from the
                // local socket; try the next provider.
                if FailureClass::of_setup(&error) == FailureClass::Handshake {
                    class = FailureClass::Handshake;
                }
                errors.push(format!("{}: {error}", ready.provider.peer));
            }
            Ok(()) = &mut established => {
                // Committed to this provider for the rest of the session.
                return match attempt.await {
                    Ok(()) => Served::Clean,
                    Err(_) => Served::Failed,
                };
            }
            _ = sleep_until(deadline) => {
                errors.push(format!("{}: selection window ended", ready.provider.peer));
                break;
            }
        }
    }
    Served::NoProvider(class, errors.join("; "))
}

/// One ordinary session to one provider. `established` fires after the
/// provider's ACK verifies and before the first read from `inbound`.
async fn attempt(
    inbound: &mut TcpStream,
    ready: &Ready,
    identity: &PeltIdentity,
    established: oneshot::Sender<()>,
) -> io::Result<()> {
    let provider = &ready.provider;
    match (&provider.path, ready.endpoint.as_ref()) {
        (Path::Tcp(address), _) => {
            let (outbound, key) =
                connect_tcp_session(address, &provider.remote, identity, &provider.expected)
                    .await?;
            let _ = established.send(());
            secure_copy_client_side(inbound, outbound, key).await
        }
        (Path::Quic(_), Some(peer)) => {
            let (connection, send, recv) =
                connect_v3_on(peer, &provider.remote, identity, &provider.expected).await?;
            let _close = CloseClientConnection(connection);
            let _ = established.send(());
            proxy_streams(inbound, send, recv)
                .await
                .map_err(|_| hs::rejected())
        }
        (Path::Routed(routed), endpoint) => {
            relay::routed_session(
                inbound,
                routed,
                endpoint,
                &provider.remote,
                identity,
                &provider.expected,
                Some(established),
            )
            .await
        }
        (Path::Quic(_), None) => Err(hs::rejected()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use werewolf_core::{
        pack::TrustLevel,
        route::RouteHop,
        service::{ServiceKind, ServiceProvider},
    };

    fn member(name: &str, address: &str) -> PeerRecord {
        PeerRecord {
            name: name.into(),
            fingerprint: format!("wwp1:{name}"),
            address: address.into(),
            trust: TrustLevel::Packmate,
            public_key_b64: Some(format!("key-{name}")),
        }
    }

    fn web() -> ServiceDefinition {
        ServiceDefinition {
            name: "web".into(),
            peer: "wolf-b".into(),
            target: "127.0.0.1:8001".into(),
            kind: ServiceKind::Tcp,
            fallbacks: vec![
                ServiceProvider {
                    peer: "wolf-c".into(),
                    target: "127.0.0.1:8002".into(),
                },
                ServiceProvider {
                    peer: "wolf-d".into(),
                    target: "127.0.0.1:8003".into(),
                },
            ],
        }
    }

    fn peers() -> Vec<PeerRecord> {
        vec![
            member("wolf-b", "quic://198.51.100.2:9560"),
            member("wolf-c", "tcp://198.51.100.3:8443"),
            member("wolf-d", "tcp://10.0.0.4:8443"),
            member("wolf-r", "quic://198.51.100.9:9560"),
        ]
    }

    #[test]
    fn resolves_every_provider_in_order_with_its_path() {
        let routes = [Route {
            peer: "wolf-d".into(),
            via: "wolf-r".into(),
            hop: RouteHop::Quic,
        }];
        let mut quic = peers();
        quic[1].address = "quic://198.51.100.3:9560".into();
        let providers = resolve(&web(), &quic, &routes, "quic", &Default::default()).unwrap();
        let order: Vec<_> = providers.iter().map(|p| p.peer.as_str()).collect();
        assert_eq!(order, ["wolf-b", "wolf-c", "wolf-d"]);
        assert!(
            matches!(&providers[0].path, Path::Quic(a) if a == &["198.51.100.2:9560".parse::<SocketAddr>().unwrap()])
        );
        // A tcp:// provider is never dialled as QUIC: requesting quic for it
        // fails the whole open rather than skipping it.
        assert!(resolve(&web(), &peers(), &[], "quic", &Default::default()).is_err());
        assert!(matches!(&providers[2].path, Path::Routed(r) if r.via == "wolf-r"));
        assert_eq!(providers[1].remote, "127.0.0.1:8002");
        assert_eq!(providers[2].uses(), ["wolf-d", "wolf-r"]);
        assert_eq!(providers[1].expected.fingerprint, "wwp1:wolf-c");
    }

    #[test]
    fn every_provider_must_resolve_and_plain_tcp_is_refused() {
        let all_tcp: Vec<_> = peers()
            .into_iter()
            .map(|mut p| {
                p.address = p.address.replace("quic://", "tcp://");
                p
            })
            .collect();
        assert_eq!(
            resolve(&web(), &all_tcp, &[], "tcp", &Default::default())
                .unwrap()
                .len(),
            3
        );
        let code = |r: Result<Vec<Provider>, (&'static str, String)>| r.err().unwrap().0;
        assert_eq!(
            code(resolve(
                &web(),
                &all_tcp,
                &[],
                "tcp-plain",
                &Default::default()
            )),
            "FANG_INVALID"
        );
        let missing: Vec<_> = all_tcp
            .iter()
            .filter(|p| p.name != "wolf-c")
            .cloned()
            .collect();
        assert_eq!(
            code(resolve(&web(), &missing, &[], "tcp", &Default::default())),
            "FANG_UNKNOWN_PEER"
        );
    }

    /// Stage46 D3: a QUIC provider given by name takes every address
    /// resolved at open; a name that did not resolve, or was not resolved,
    /// fails the open with its reason.
    #[test]
    fn a_named_quic_provider_is_resolved_at_open() {
        let mut named = peers();
        named[0].address = "quic://provider.stage46:9560".into();
        named[1].address = "quic://198.51.100.3:9560".into();
        named[2].address = "quic://10.0.0.4:9560".into();
        let both: Vec<SocketAddr> = vec![
            "[fd46::2]:9560".parse().unwrap(),
            "198.51.100.2:9560".parse().unwrap(),
        ];
        let resolved = super::super::dial::Resolved::from_entries([(
            "provider.stage46:9560",
            Ok(both.clone()),
        )]);
        let providers = resolve(&web(), &named, &[], "quic", &resolved).unwrap();
        assert!(matches!(&providers[0].path, Path::Quic(a) if a == &both));

        let failed = super::super::dial::Resolved::from_entries([(
            "provider.stage46:9560",
            Err("no such host".into()),
        )]);
        let error = resolve(&web(), &named, &[], "quic", &failed).err().unwrap();
        assert_eq!(error.0, "FANG_BAD_QUIC_ADDRESS");
        assert!(error.1.contains("does not resolve"), "{}", error.1);
        let error = resolve(&web(), &named, &[], "quic", &Default::default())
            .err()
            .unwrap();
        assert!(error.1.contains("retry"), "{}", error.1);
    }
}
