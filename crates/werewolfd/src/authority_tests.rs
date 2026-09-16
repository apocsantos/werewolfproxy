//! Frozen Stage 11B gap and submission-boundary characterization.
//! Gate below is a test prototype, not production authorization.
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

struct Prototype<W> {
    inner: W,
    allowed: Arc<Mutex<bool>>,
}
impl<W: AsyncWrite + Unpin> AsyncWrite for Prototype<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let gate = self.allowed.clone();
        let allowed = gate.lock().unwrap();
        if !*allowed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "rejected",
            )));
        }
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[test]
fn certified_baseline_has_no_inbound_authority_registry() {
    let baseline = |file: &str| {
        let output = std::process::Command::new("git")
            .args([
                "show",
                &format!("725f1908e60d2b1c1b8a13780c78d5ccedd1048d:{file}"),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let tcp = baseline("crates/werewolfd/src/transport/tcp_encrypted.rs");
    let quic = baseline("crates/werewolfd/src/transport/quic.rs");
    let mutation = baseline("crates/werewolfd/src/control/mutation.rs");
    assert!(tcp.contains("tokio::spawn(async move"));
    assert!(quic.contains("tokio::task::JoinSet::new()"));
    assert!(!tcp.contains("WolfMode::Silver"));
    assert!(!quic.contains("WolfMode::Silver"));
    assert!(mutation.contains("if req.cmd == \"pack.revoke\""));
    assert!(mutation.contains("live.fang_registry.terminate_peer(&name)"));
}

#[derive(Default)]
struct PartialThenPending {
    bytes: Vec<u8>,
    polls: usize,
}
impl AsyncWrite for PartialThenPending {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.polls += 1;
        if self.polls == 1 {
            self.bytes.extend_from_slice(&bytes[..2]);
            Poll::Ready(Ok(2))
        } else {
            Poll::Pending
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn write_all_partial_progress_must_recheck_every_submission_poll() {
    let allowed = Arc::new(Mutex::new(true));
    let mut writer = Prototype {
        inner: PartialThenPending::default(),
        allowed: allowed.clone(),
    };
    let mut future = Box::pin(writer.write_all(b"abcd"));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(future.as_mut().poll(&mut cx).is_pending());
    // The pending operation does not retain the gate; it must recheck on wake.
    *allowed.try_lock().unwrap() = false;
    assert!(matches!(future.as_mut().poll(&mut cx), Poll::Ready(Err(_))));
    drop(future);
    assert_eq!(writer.inner.bytes, b"ab");
    assert_eq!(writer.inner.polls, 2);
}

#[tokio::test]
async fn tcp_submission_precedes_delivery_and_revoked_poll_cannot_write() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut receiver, _) = listener.accept().await.unwrap();
        let allowed = Arc::new(Mutex::new(true));
        let mut writer = Prototype {
            inner: stream,
            allowed: allowed.clone(),
        };
        writer.write_all(b"before").await.unwrap();
        *allowed.lock().unwrap() = false;
        assert!(writer.write_all(b"after").await.is_err());
        drop(writer);
        let mut received = Vec::new();
        receiver.read_to_end(&mut received).await.unwrap();
        // Delivery after the fence is allowed for bytes submitted before it.
        assert_eq!(received, b"before");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quinn_submission_precedes_delivery_and_revoked_poll_cannot_write() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let server = crate::quic_lab::make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
        let client = crate::quic_lab::make_client_endpoint().unwrap();
        let connecting = client
            .connect(server.local_addr().unwrap(), "localhost")
            .unwrap();
        let (outbound, inbound) =
            tokio::join!(connecting, async { server.accept().await.unwrap().await });
        let outbound = outbound.unwrap();
        let inbound = inbound.unwrap();
        let (send, _recv) = outbound.open_bi().await.unwrap();
        let allowed = Arc::new(Mutex::new(true));
        let mut writer = Prototype {
            inner: send,
            allowed: allowed.clone(),
        };
        writer.write_all(b"before").await.unwrap();
        *allowed.lock().unwrap() = false;
        assert!(writer.write_all(b"after").await.is_err());
        writer.inner.finish().unwrap();
        let (_send, mut receive) = inbound.accept_bi().await.unwrap();
        assert_eq!(receive.read_to_end(64).await.unwrap(), b"before");
        outbound.close(0u32.into(), b"");
        inbound.close(0u32.into(), b"");
    })
    .await
    .unwrap();
}
