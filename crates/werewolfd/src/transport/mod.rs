pub(super) mod quic;
pub(super) mod relay;
pub(super) mod tcp_encrypted;
pub(super) mod tcp_plain;

/// Closing a socket with zero linger sends RST instead of FIN. A Fang arms
/// this on the local application's socket and on the receiver's target socket
/// before forwarding, and disarms it only after a clean end. An error,
/// cancellation or abort then reaches the application or target as a
/// connection reset instead of an EOF that looks like completion.
pub(crate) fn arm_reset_on_close(stream: &tokio::net::TcpStream) -> std::io::Result<()> {
    socket2::SockRef::from(stream).set_linger(Some(std::time::Duration::ZERO))
}

pub(crate) fn disarm_reset_on_close(stream: &tokio::net::TcpStream) -> std::io::Result<()> {
    socket2::SockRef::from(stream).set_linger(None)
}

/// Test helper: the bytes a socket receives until it closes, and whether it
/// closed with a reset (abnormal end) rather than a clean EOF.
#[cfg(test)]
pub(crate) async fn drain_until_closed(stream: &mut tokio::net::TcpStream) -> (Vec<u8>, bool) {
    use tokio::io::AsyncReadExt;
    let mut received = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => return (received, false),
            Ok(n) => received.extend_from_slice(&buf[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {
                return (received, true)
            }
            Err(error) => panic!("unexpected socket error: {error}"),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum FangTransport {
    Tcp(String),
    Quic(String),
}

#[cfg(test)]
pub(super) async fn run_quic_fang_listener(
    listen_addr: &str,
    state: std::sync::Arc<tokio::sync::Mutex<crate::state::DaemonState>>,
) -> tokio::io::Result<()> {
    quic::run_quic_fang_listener(listen_addr, state).await
}

pub(crate) async fn run_quic_fang_listener_with_ready(
    listen_addr: &str,
    state: std::sync::Arc<tokio::sync::Mutex<crate::state::DaemonState>>,
    ready: tokio::sync::oneshot::Sender<tokio::io::Result<()>>,
) -> tokio::io::Result<()> {
    quic::run_quic_fang_listener_with_ready(listen_addr, state, Some(ready)).await
}

#[cfg(test)]
pub(super) async fn run_fang_listener(
    listen_addr: &str,
    state: std::sync::Arc<tokio::sync::Mutex<crate::state::DaemonState>>,
) -> tokio::io::Result<()> {
    tcp_encrypted::run_fang_listener(listen_addr, state).await
}

pub(crate) async fn run_fang_listener_with_ready(
    listen_addr: &str,
    state: std::sync::Arc<tokio::sync::Mutex<crate::state::DaemonState>>,
    ready: tokio::sync::oneshot::Sender<tokio::io::Result<()>>,
) -> tokio::io::Result<()> {
    tcp_encrypted::run_fang_listener_with_ready(listen_addr, state, Some(ready)).await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_local_fang_forwarder(
    fang_id: &str,
    local: &str,
    peer_addr: &str,
    remote: &str,
    identity: werewolf_core::pelt::PeltIdentity,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<tokio::io::Result<()>>,
    expected_peer: crate::policy::ExpectedPeerIdentity,
) -> tokio::io::Result<()> {
    tcp_encrypted::run_local_fang_forwarder(
        fang_id,
        local,
        peer_addr,
        remote,
        identity,
        cancellation,
        ready,
        expected_peer,
    )
    .await
}

pub(super) async fn run_plain_tcp_forwarder(
    fang_id: &str,
    local: &str,
    remote: &str,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<tokio::io::Result<()>>,
    expected_peer: crate::policy::ExpectedPeerIdentity,
) -> tokio::io::Result<()> {
    let _ = expected_peer;
    tcp_plain::run_plain_tcp_forwarder(fang_id, local, remote, cancellation, ready).await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_selected_forwarder(
    fang_id: &str,
    local: &str,
    peer_addr: &str,
    remote: &str,
    identity: werewolf_core::pelt::PeltIdentity,
    plain_tcp: bool,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<tokio::io::Result<()>>,
    expected_peer: crate::policy::ExpectedPeerIdentity,
) -> tokio::io::Result<()> {
    if plain_tcp {
        run_plain_tcp_forwarder(fang_id, local, remote, cancellation, ready, expected_peer).await
    } else {
        run_local_fang_forwarder(
            fang_id,
            local,
            peer_addr,
            remote,
            identity,
            cancellation,
            ready,
            expected_peer,
        )
        .await
    }
}
