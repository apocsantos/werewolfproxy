#[path = "../quic_lab.rs"]
mod quic_lab;

use quic_lab::make_client_endpoint;
use std::{error::Error, net::SocketAddr};


#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let server = "127.0.0.1:9560";
    let server_addr: SocketAddr = server.parse()?;

    let endpoint = make_client_endpoint()?;

    println!("🐺 connecting to native QUIC Fang at {}", server);

    let connection = endpoint
        .connect(server_addr, "localhost")?
        .await?;

    println!("⚡ connected");

    let (mut send, mut recv) =
        connection.open_bi().await?;

    // Send target first
    send
        .write_all(b"127.0.0.1:8080\n")
        .await?;

    tokio::time::sleep(
        std::time::Duration::from_millis(100),
    )
    .await;

    // Send HTTP request
    send
        .write_all(
            b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .await?;

    send.finish()?;

    println!("🦷 HTTP request sent");

    let buf = recv.read_to_end(1024 * 1024).await?;

    println!(
        "📩 response:\n{}",
        String::from_utf8_lossy(&buf)
    );

    endpoint.wait_idle().await;

    Ok(())
}
