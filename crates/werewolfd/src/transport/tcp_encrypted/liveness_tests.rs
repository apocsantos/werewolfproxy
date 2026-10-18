use super::*;

#[tokio::test]
async fn liveness_enables_kernel_keepalive_and_user_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();

    configure_liveness(&stream).unwrap();

    let socket = socket2::SockRef::from(&stream);
    assert!(socket.keepalive().unwrap());
    assert_eq!(socket.tcp_keepalive_time().unwrap(), KEEPALIVE_IDLE);
    assert_eq!(socket.tcp_keepalive_interval().unwrap(), KEEPALIVE_INTERVAL);
    assert_eq!(socket.tcp_keepalive_retries().unwrap(), KEEPALIVE_RETRIES);
    assert_eq!(socket.tcp_user_timeout().unwrap(), Some(TCP_USER_TIMEOUT));
}
