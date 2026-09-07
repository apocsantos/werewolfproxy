use std::time::Duration;
use tokio::{
    io,
    net::{TcpListener, TcpStream},
};

pub(super) async fn run_plain_tcp_forwarder(
    fang_id: &str,
    local: &str,
    remote: &str,
) -> io::Result<()> {
    let listener = TcpListener::bind(local).await?;
    println!("🦷 {} plain TCP listening locally on {}", fang_id, local);

    loop {
        let (mut inbound, client_addr) = listener.accept().await?;
        let remote = remote.to_string();
        let fang_id = fang_id.to_string();

        tokio::spawn(async move {
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
    }
}
