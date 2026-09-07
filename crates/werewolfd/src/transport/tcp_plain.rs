use std::time::Duration;
use tokio::{
    io,
    net::{TcpListener, TcpStream},
};

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
    println!("🦷 {} plain TCP listening locally on {}", fang_id, local);

    loop {
        let (mut inbound, client_addr) = listener.accept().await?;
        let remote = remote.to_string();
        let fang_id = fang_id.to_string();

        let handle = tokio::spawn(async move {
            match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&remote)).await {
                Ok(Ok(mut outbound)) => {
                    if let Err(e) = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await
                    {
                        eprintln!(
                            "🦷 {} plain client {} pipe error: {}",
                            fang_id, client_addr, e
                        );
                    }
                }
                Ok(Err(e)) => {
                    eprintln!(
                        "🦷 {} plain target connect error {}: {}",
                        fang_id, remote, e
                    );
                }
                Err(_) => {
                    eprintln!(
                        "🦷 {} plain target connect timed out after 5s: {}",
                        fang_id, remote
                    );
                }
            }
        });
        cancellation.track(&handle);
    }
}
