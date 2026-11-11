use super::fang_health::FailureClass;
use std::time::Duration;
use tokio::{io, net::TcpListener};

pub(super) async fn run_plain_tcp_forwarder(
    fang_id: &str,
    local: &str,
    remote: &str,
    cancellation: crate::fang_registry::FangCancellation,
    ready: tokio::sync::oneshot::Sender<io::Result<()>>,
) -> io::Result<()> {
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
    println!("🦷 {} plain TCP listening locally on {}", fang_id, local);

    let mut acceptor = super::fang_accept::FangAcceptor::new(listener, fang_id);
    loop {
        let (mut inbound, client_addr, slot) = acceptor.accept().await;
        let remote = remote.to_string();
        let health = cancellation.health();

        cancellation.spawn(async move {
            // Failures are counted and logged at a bounded rate (Stage45
            // S45-D2): a client retrying against a dead target cannot grow
            // the log.
            let session = async {
                // Every address the target names, raced (Stage46 T1).
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    super::dial::connect_tcp(&remote),
                )
                .await
                {
                    Ok(Ok(mut outbound)) => {
                        // Local activity, for Stage43 idle eviction.
                        let mut local = super::fang_accept::Tracked::new(&mut inbound);
                        if let Err(e) =
                            tokio::io::copy_bidirectional(&mut local, &mut outbound).await
                        {
                            health.pipe_error(&format_args!("client {client_addr}: {e}"));
                        }
                    }
                    Ok(Err(e)) => {
                        health.record(FailureClass::Target, &format_args!("{remote}: {e}"));
                    }
                    Err(_) => {
                        health.record(
                            FailureClass::Target,
                            &format_args!("{remote}: no answer within 5 s"),
                        );
                    }
                }
            };
            if slot.serve(session).await.is_none() {
                // Evicted (Stage43): close with a reset, not a FIN that
                // would look like a completed session.
                let _ = super::arm_reset_on_close(&inbound);
            }
        });
    }
}
