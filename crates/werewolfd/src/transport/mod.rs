pub(super) mod tcp_plain;

pub(super) async fn run_plain_tcp_forwarder(
    fang_id: &str,
    local: &str,
    remote: &str,
) -> tokio::io::Result<()> {
    tcp_plain::run_plain_tcp_forwarder(fang_id, local, remote).await
}
