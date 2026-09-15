use super::*;
use tokio::time::timeout;
use werewolf_core::pelt::generate_identity;

#[tokio::test]
async fn transport_configuration_bounds_remote_stream_and_datagram_resources() {
    let pelt = generate_identity();
    let identity = crate::tls_identity::RuntimeTlsIdentity::from_pelt(&pelt).unwrap();
    let tls = crate::tls_identity::server_config(&identity).unwrap();
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap();
    let server = quinn::Endpoint::server(
        resource_bounded_server_config(crypto).unwrap(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    let server_address = server.local_addr().unwrap();
    let server_task = tokio::spawn(async move {
        let connection = server.accept().await.unwrap().await.unwrap();
        connection.closed().await;
    });
    let client = crate::quic_lab::make_client_endpoint().unwrap();
    let connection = timeout(
        Duration::from_secs(2),
        client.connect(server_address, "localhost").unwrap(),
    )
    .await
    .unwrap()
    .unwrap();

    // The remote transport parameter permits no unidirectional streams.
    assert!(matches!(
        timeout(Duration::from_millis(200), connection.open_uni()).await,
        Err(_) | Ok(Err(_))
    ));
    let mut streams = Vec::with_capacity(MAX_BIDIRECTIONAL_STREAMS as usize);
    for _ in 0..MAX_BIDIRECTIONAL_STREAMS {
        streams.push(
            timeout(Duration::from_secs(1), connection.open_bi())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    // No additional remote stream credit is available while all 64 existing
    // streams remain open. A fixed test timeout keeps this assertion bounded.
    assert!(matches!(
        timeout(Duration::from_millis(200), connection.open_bi()).await,
        Err(_) | Ok(Err(_))
    ));
    drop(streams);
    connection.close(hs::V3_REJECT_CODE.into(), b"");
    timeout(Duration::from_secs(2), server_task)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn quic_resource_budget_constants_are_consistent() {
    assert!(MAX_INCOMING_CONNECTIONS <= 128);
    assert!(
        INCOMING_BUFFER_PER_CONNECTION * MAX_INCOMING_CONNECTIONS as u64 <= INCOMING_BUFFER_TOTAL
    );
    assert!(STREAM_RECEIVE_WINDOW <= CONNECTION_RECEIVE_WINDOW);
    assert_eq!(MAX_BIDIRECTIONAL_STREAMS, 64);
    assert_eq!(QUIC_IDLE_TIMEOUT, Duration::from_secs(30));
    assert_eq!(SEND_WINDOW, CONNECTION_RECEIVE_WINDOW as u64);
}
