pub(super) mod quic;
pub(super) mod tcp_encrypted;
pub(super) mod tcp_plain;

#[derive(Debug, Clone)]
pub(super) enum FangTransport {
    Tcp(String),
    Quic(String),
}

pub(super) async fn run_quic_fang_listener(
    listen_addr: &str,
    state: std::sync::Arc<tokio::sync::Mutex<crate::state::DaemonState>>,
) -> tokio::io::Result<()> {
    quic::run_quic_fang_listener(listen_addr, state).await
}

pub(super) async fn run_fang_listener(
    listen_addr: &str,
    state: std::sync::Arc<tokio::sync::Mutex<crate::state::DaemonState>>,
) -> tokio::io::Result<()> {
    tcp_encrypted::run_fang_listener(listen_addr, state).await
}

pub(super) async fn run_local_fang_forwarder(
    fang_id: &str,
    local: &str,
    peer_addr: &str,
    remote: &str,
    identity: werewolf_core::pelt::PeltIdentity,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<tokio::io::Result<()>>,
) -> tokio::io::Result<()> {
    tcp_encrypted::run_local_fang_forwarder(
        fang_id,
        local,
        peer_addr,
        remote,
        identity,
        cancellation,
        ready,
    )
    .await
}

pub(super) async fn run_plain_tcp_forwarder(
    fang_id: &str,
    local: &str,
    remote: &str,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<tokio::io::Result<()>>,
) -> tokio::io::Result<()> {
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
) -> tokio::io::Result<()> {
    if plain_tcp {
        run_plain_tcp_forwarder(fang_id, local, remote, cancellation, ready).await
    } else {
        run_local_fang_forwarder(
            fang_id,
            local,
            peer_addr,
            remote,
            identity,
            cancellation,
            ready,
        )
        .await
    }
}
