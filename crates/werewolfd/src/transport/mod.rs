pub(super) mod quic;
pub(super) mod tcp_encrypted;
pub(super) mod tcp_plain;

#[derive(Debug, Clone)]
pub(super) enum FangTransport {
    Tcp(String),
    Quic(String),
}

pub(super) fn parse_fang_transport(address: &str) -> Result<FangTransport, String> {
    if let Some(rest) = address.strip_prefix("tcp://") {
        return Ok(FangTransport::Tcp(rest.to_string()));
    }

    if let Some(rest) = address.strip_prefix("quic://") {
        return Ok(FangTransport::Quic(rest.to_string()));
    }

    Err("peer address must start with tcp:// or quic://".to_string())
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
) -> tokio::io::Result<()> {
    tcp_encrypted::run_local_fang_forwarder(fang_id, local, peer_addr, remote, identity).await
}

pub(super) async fn run_plain_tcp_forwarder(
    fang_id: &str,
    local: &str,
    remote: &str,
) -> tokio::io::Result<()> {
    tcp_plain::run_plain_tcp_forwarder(fang_id, local, remote).await
}
