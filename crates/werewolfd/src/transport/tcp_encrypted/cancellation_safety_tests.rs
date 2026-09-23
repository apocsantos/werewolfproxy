use super::*;
use std::{
    pin::Pin,
    sync::{Arc, Mutex as StdMutex},
    task::{Context, Poll, Waker},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::watch,
    time::timeout,
};

struct WriteGate {
    allowed: bool,
    waker: Option<Waker>,
}

impl WriteGate {
    fn closed() -> Arc<StdMutex<Self>> {
        Arc::new(StdMutex::new(Self {
            allowed: false,
            waker: None,
        }))
    }

    fn release(gate: &Arc<StdMutex<Self>>) {
        let mut gate = gate.lock().unwrap();
        gate.allowed = true;
        if let Some(waker) = gate.waker.take() {
            waker.wake();
        }
    }
}

struct ObservedIo {
    inner: tokio::io::DuplexStream,
    read_bytes: usize,
    write_bytes: usize,
    reads: watch::Sender<usize>,
    writes: watch::Sender<usize>,
    write_gate: Option<Arc<StdMutex<WriteGate>>>,
}

impl ObservedIo {
    fn new(
        inner: tokio::io::DuplexStream,
        write_gate: Option<Arc<StdMutex<WriteGate>>>,
    ) -> (Self, watch::Receiver<usize>, watch::Receiver<usize>) {
        let (reads, read_progress) = watch::channel(0);
        let (writes, write_progress) = watch::channel(0);
        (
            Self {
                inner,
                read_bytes: 0,
                write_bytes: 0,
                reads,
                writes,
                write_gate,
            },
            read_progress,
            write_progress,
        )
    }
}

impl AsyncRead for ObservedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &result {
            self.read_bytes += buf.filled().len() - before;
            self.reads.send_replace(self.read_bytes);
        }
        result
    }
}

impl AsyncWrite for ObservedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(gate) = &self.write_gate {
            let mut gate = gate.lock().unwrap();
            if self.write_bytes >= 3 && !gate.allowed {
                gate.waker = Some(cx.waker().clone());
                return Poll::Pending;
            }
        }
        let result = Pin::new(&mut self.inner).poll_write(cx, &buf[..buf.len().min(3)]);
        if let Poll::Ready(Ok(n)) = result {
            self.write_bytes += n;
            self.writes.send_replace(self.write_bytes);
            Poll::Ready(Ok(n))
        } else {
            result
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

async fn client_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (forwarder, _) = listener.accept().await.unwrap();
    (app, forwarder)
}

async fn encoded_frame(payload: &[u8], direction: u8) -> Vec<u8> {
    let (mut writer, mut reader) = tokio::io::duplex(4096);
    let mut counter = 0;
    write_encrypted_frame(&mut writer, &[7; 32], direction, &mut counter, payload)
        .await
        .unwrap();
    writer.shutdown().await.unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    bytes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partial_reverse_frame_survives_opposite_direction_ready() {
    timeout(Duration::from_secs(5), async {
        let (mut app, mut forwarder) = client_pair().await;
        let (encrypted, mut peer) = tokio::io::duplex(4096);
        let (observed, mut read_progress, _) = ObservedIo::new(encrypted, None);
        let task = tokio::spawn(async move {
            secure_copy_client_side(&mut forwarder, observed, Zeroizing::new([7; 32])).await
        });
        let response = b"reverse-direction-payload";
        let frame = encoded_frame(response, 1).await;
        peer.write_all(&frame[..5]).await.unwrap();
        read_progress.wait_for(|count| *count >= 5).await.unwrap();

        // The old select! abandons the five consumed encrypted bytes when
        // this request branch becomes ready.
        app.write_all(b"request").await.unwrap();
        let mut request_counter = 0;
        assert_eq!(
            read_encrypted_frame(&mut peer, &[7; 32], 0, &mut request_counter)
                .await
                .unwrap(),
            b"request"
        );
        peer.write_all(&frame[5..]).await.unwrap();
        let mut actual = vec![0; response.len()];
        app.read_exact(&mut actual).await.unwrap();
        assert_eq!(actual, response);
        peer.shutdown().await.unwrap();
        assert!(task.await.unwrap().is_ok());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partial_request_frame_survives_reverse_traffic() {
    timeout(Duration::from_secs(5), async {
        let (mut app, mut forwarder) = client_pair().await;
        let (encrypted, mut peer) = tokio::io::duplex(4096);
        let gate = WriteGate::closed();
        let (observed, _, mut write_progress) = ObservedIo::new(encrypted, Some(gate.clone()));
        let task = tokio::spawn(async move {
            secure_copy_client_side(&mut forwarder, observed, Zeroizing::new([7; 32])).await
        });
        let request = b"request-frame-must-finish";
        app.write_all(request).await.unwrap();
        write_progress.wait_for(|count| *count >= 3).await.unwrap();

        // Deliver a complete reverse frame while the request frame is only
        // partially submitted. The request writer must resume the same frame.
        let response = b"reverse-while-write-pending";
        peer.write_all(&encoded_frame(response, 1).await)
            .await
            .unwrap();
        let mut actual = vec![0; response.len()];
        app.read_exact(&mut actual).await.unwrap();
        assert_eq!(actual, response);
        app.shutdown().await.unwrap();
        WriteGate::release(&gate);

        let mut request_counter = 0;
        assert_eq!(
            read_encrypted_frame(&mut peer, &[7; 32], 0, &mut request_counter)
                .await
                .unwrap(),
            request
        );
        peer.shutdown().await.unwrap();
        assert!(task.await.unwrap().is_ok());
    })
    .await
    .unwrap();
}
