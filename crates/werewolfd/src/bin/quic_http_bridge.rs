#[path = "../quic_lab.rs"]
mod quic_lab;

use clap::{Parser, Subcommand};
use quic_lab::{make_client_endpoint, make_server_endpoint};
use quinn::{RecvStream, SendStream};
use std::{error::Error, net::SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[derive(Parser)]
#[command(name = "quic_http_bridge")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Server {
        #[arg(long, default_value = "127.0.0.1:9555")]
        listen: String,

        #[arg(long, default_value = "127.0.0.1:8080")]
        remote: String,
    },

    Client {
        #[arg(long, default_value = "127.0.0.1:9012")]
        listen: String,

        #[arg(long, default_value = "127.0.0.1:9555")]
        server: String,
    },
}

async fn proxy_tcp_to_quic(
    mut tcp: TcpStream,
    mut send: SendStream,
    mut recv: RecvStream,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (mut tcp_read, mut tcp_write) = tcp.split();

    let up = async {
        let mut buf = [0u8; 8192];

        loop {
            let n = tcp_read.read(&mut buf).await?;
            if n == 0 {
                break;
            }

            send.write_all(&buf[..n]).await?;
        }

        send.finish()?;
        Ok::<_, std::io::Error>(())
    };

    let down = async {
        while let Some(chunk) = recv.read_chunk(8192, true).await? {
            tcp_write.write_all(&chunk.bytes).await?;
        }

        Ok::<_, std::io::Error>(())
    };

    tokio::try_join!(up, down)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Server { listen, remote } => {
            let listen_addr: SocketAddr = listen.parse()?;
            let endpoint = make_server_endpoint(listen_addr)?;

            println!("🐺 QUIC bridge server on {}", listen);
            println!("🦷 forwarding QUIC streams to {}", remote);

            while let Some(incoming) = endpoint.accept().await {
                let remote = remote.clone();

                tokio::spawn(async move {
                    if let Ok(connection) = incoming.await {
                        println!("🦷 QUIC connection");

                        while let Ok((send, recv)) = connection.accept_bi().await {
                            let remote = remote.clone();

                            tokio::spawn(async move {
                                match TcpStream::connect(&remote).await {
                                    Ok(tcp) => {
                                        let _ = proxy_tcp_to_quic(tcp, send, recv).await;
                                    }
                                    Err(e) => eprintln!("tcp connect failed: {}", e),
                                }
                            });
                        }
                    }
                });
            }
        }

        Commands::Client { listen, server } => {
            let listener = TcpListener::bind(&listen).await?;
            println!("🐺 Local bridge on {}", listen);
            println!("⚡ QUIC server: {}", server);

            loop {
                let (tcp, _) = listener.accept().await?;
                let server = server.clone();

                tokio::spawn(async move {
                    match make_client_endpoint() {
                        Ok(endpoint) => {
                            let server_addr: SocketAddr = match server.parse() {
                                Ok(addr) => addr,
                                Err(e) => {
                                    eprintln!("bad server address: {}", e);
                                    return;
                                }
                            };

                            match endpoint.connect(server_addr, "localhost").unwrap().await {
                                Ok(connection) => match connection.open_bi().await {
                                    Ok((send, recv)) => {
                                        let _ = proxy_tcp_to_quic(tcp, send, recv).await;
                                    }
                                    Err(e) => eprintln!("open_bi failed: {}", e),
                                },
                                Err(e) => eprintln!("connect failed: {}", e),
                            }
                        }
                        Err(e) => eprintln!("client endpoint failed: {}", e),
                    }
                });
            }
        }
    }

    Ok(())
}
