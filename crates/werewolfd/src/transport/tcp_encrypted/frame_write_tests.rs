use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
};

struct RecordingWriter {
    bytes: Vec<u8>,
    write_polls: usize,
    flushes: usize,
    max_chunk: usize,
    fail_after: Option<usize>,
    allowed: Option<Arc<AtomicBool>>,
    pending_after_first: bool,
}

impl RecordingWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            write_polls: 0,
            flushes: 0,
            max_chunk: usize::MAX,
            fail_after: None,
            allowed: None,
            pending_after_first: false,
        }
    }
}

impl AsyncWrite for RecordingWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.write_polls += 1;
        if self.pending_after_first && self.write_polls == 2 {
            return Poll::Pending;
        }
        if self
            .allowed
            .as_ref()
            .is_some_and(|allowed| !allowed.load(Ordering::SeqCst))
        {
            return Poll::Ready(Err(io::ErrorKind::PermissionDenied.into()));
        }
        let remaining = self
            .fail_after
            .map_or(usize::MAX, |limit| limit.saturating_sub(self.bytes.len()));
        if remaining == 0 {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let accepted = bytes.len().min(self.max_chunk).min(remaining);
        self.bytes.extend_from_slice(&bytes[..accepted]);
        Poll::Ready(Ok(accepted))
    }

    fn poll_flush(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.flushes += 1;
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn assert_one_original_frame(frame: &[u8], payload: &[u8], direction: u8, counter: u64) {
    assert!(frame.len() >= 4 + 16);
    let ciphertext = &frame[4..];
    // This is the pre-change conceptual serialization, with the actual
    // ciphertext produced by production encryption in this very frame.
    let old_serialization = [
        (ciphertext.len() as u32).to_be_bytes().as_slice(),
        ciphertext,
    ]
    .concat();
    assert_eq!(frame, old_serialization);
    assert!(frame.len() <= 4 + 2048 + 16);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(&[7; 32]));
    let padded = cipher
        .decrypt(
            Nonce::from_slice(&make_nonce(direction, counter)),
            ciphertext,
        )
        .unwrap();
    let length = u16::from_be_bytes([padded[0], padded[1]]) as usize;
    assert_eq!(length, payload.len());
    assert_eq!(&padded[2..2 + length], payload);
    assert!(padded[2 + length..].iter().all(|byte| *byte == 0));
    let minimum = (payload.len() + 2).max(768);
    assert!((minimum..=2048).contains(&padded.len()));
    assert_eq!((padded.len() - minimum) % 128, 0);
}

#[tokio::test]
async fn coalesced_frame_preserves_format_padding_and_one_write_for_payload_boundaries() {
    for size in [0, 1, 16, 32, 256, 1024, 1400, 2046] {
        let payload = vec![0x5a; size];
        let mut writer = RecordingWriter::new();
        let mut counter = 0;
        write_encrypted_frame(&mut writer, &[7; 32], 0, &mut counter, &payload)
            .await
            .unwrap();
        assert_eq!(writer.write_polls, 1);
        assert_eq!(writer.flushes, 1);
        assert_eq!(counter, 1);
        assert_one_original_frame(&writer.bytes, &payload, 0, 0);
        let mut reader = writer.bytes.as_slice();
        let mut read_counter = 0;
        assert_eq!(
            read_encrypted_frame(&mut reader, &[7; 32], 0, &mut read_counter)
                .await
                .unwrap(),
            payload
        );
        assert!(reader.is_empty());
        assert_eq!(read_counter, 1);
    }
}

#[tokio::test]
async fn each_frame_has_one_independent_write_without_cross_frame_batching() {
    let payloads: [&[u8]; 4] = [b"a", b"interactive", b"", b"last"];
    let mut writer = RecordingWriter::new();
    let mut counter = 0;
    for payload in payloads {
        write_encrypted_frame(&mut writer, &[7; 32], 1, &mut counter, payload)
            .await
            .unwrap();
    }
    assert_eq!(writer.write_polls, payloads.len());
    assert_eq!(writer.flushes, payloads.len());
    assert_eq!(counter as usize, payloads.len());
    let mut reader = writer.bytes.as_slice();
    let mut read_counter = 0;
    for payload in payloads {
        let before = reader.len();
        assert_eq!(
            read_encrypted_frame(&mut reader, &[7; 32], 1, &mut read_counter)
                .await
                .unwrap(),
            payload
        );
        assert!(before > reader.len());
    }
    assert!(reader.is_empty());
}

#[tokio::test]
async fn partial_writes_complete_exactly_one_frame_without_duplicate_prefix() {
    let mut writer = RecordingWriter::new();
    writer.max_chunk = 3;
    let mut counter = 0;
    write_encrypted_frame(&mut writer, &[7; 32], 0, &mut counter, b"tiny")
        .await
        .unwrap();
    assert!(writer.write_polls > 2);
    assert_eq!(writer.flushes, 1);
    assert_eq!(counter, 1);
    assert_one_original_frame(&writer.bytes, b"tiny", 0, 0);
}

#[tokio::test]
async fn write_failures_do_not_retry_or_reuse_a_nonce() {
    for fail_after in [0, 2, 4, 16, 100] {
        let mut writer = RecordingWriter::new();
        writer.max_chunk = 3;
        writer.fail_after = Some(fail_after);
        let mut counter = 0;
        let error = write_encrypted_frame(&mut writer, &[7; 32], 0, &mut counter, b"payload")
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(writer.bytes.len(), fail_after);
        assert_eq!(writer.flushes, 0);
        // The production routine increments before write and returns on
        // failure. Its caller terminates this direction; no resend occurs.
        assert_eq!(counter, 1);
    }
}

#[test]
fn invalidated_authority_between_partial_submission_polls_rejects_remainder() {
    let allowed = Arc::new(AtomicBool::new(true));
    let mut writer = RecordingWriter::new();
    writer.max_chunk = 2;
    writer.pending_after_first = true;
    writer.allowed = Some(allowed.clone());
    let mut counter = 0;
    let mut future = Box::pin(write_encrypted_frame(
        &mut writer,
        &[7; 32],
        0,
        &mut counter,
        b"not delivered",
    ));
    let mut context = Context::from_waker(Waker::noop());
    assert!(future.as_mut().poll(&mut context).is_pending());
    allowed.store(false, Ordering::SeqCst);
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(Err(error)) if error.kind() == io::ErrorKind::PermissionDenied
    ));
    drop(future);
    assert_eq!(writer.bytes.len(), 2);
    assert_eq!(writer.write_polls, 3);
    assert_eq!(writer.flushes, 0);
    assert_eq!(counter, 1);
}
