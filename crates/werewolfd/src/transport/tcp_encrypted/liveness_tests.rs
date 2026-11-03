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

#[tokio::test]
async fn session_socket_disables_nagle_and_keeps_liveness() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    assert!(!stream.nodelay().unwrap());

    configure_session_socket(&stream).unwrap();

    assert!(stream.nodelay().unwrap());
    let socket = socket2::SockRef::from(&stream);
    assert!(socket.keepalive().unwrap());
    assert_eq!(socket.tcp_user_timeout().unwrap(), Some(TCP_USER_TIMEOUT));
}

#[tokio::test]
async fn initiator_socket_disables_nagle() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();

    let stream = open_tcp(&listener.local_addr().unwrap().to_string())
        .await
        .unwrap();

    assert!(stream.nodelay().unwrap());
}

#[tokio::test]
async fn target_liveness_keeps_nagle_and_reset_on_close() {
    // A receiver's target connection (Stage39): liveness only, with Nagle
    // left as it was, and the reset-on-close arming applied later by
    // forwarding still taking effect on top of it.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let nagle_before = stream.nodelay().unwrap();

    configure_liveness(&stream).unwrap();

    assert_eq!(stream.nodelay().unwrap(), nagle_before);
    let socket = socket2::SockRef::from(&stream);
    assert!(socket.keepalive().unwrap());
    assert_eq!(
        socket.tcp_keepalive_time().unwrap(),
        Duration::from_secs(30)
    );
    assert_eq!(
        socket.tcp_keepalive_interval().unwrap(),
        Duration::from_secs(10)
    );
    assert_eq!(socket.tcp_keepalive_retries().unwrap(), 3);
    assert_eq!(
        socket.tcp_user_timeout().unwrap(),
        Some(Duration::from_secs(60))
    );
    assert_eq!(socket.linger().unwrap(), None);

    crate::transport::arm_reset_on_close(&stream).unwrap();
    assert_eq!(socket.linger().unwrap(), Some(Duration::ZERO));
    assert_eq!(
        socket.tcp_user_timeout().unwrap(),
        Some(Duration::from_secs(60))
    );
    crate::transport::disarm_reset_on_close(&stream).unwrap();
    assert_eq!(socket.linger().unwrap(), None);
    assert!(socket.keepalive().unwrap());
}
