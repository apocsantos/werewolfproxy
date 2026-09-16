//! Only raw nonbuffering transport writers may enter the submission gate.
//! A BufWriter above this wrapper is safe; one below it would not be.
use super::SessionLease;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

/// A raw TCP stream whose writes become authority-gated after sender
/// authentication. TLS is layered above this type so rustls cannot flush
/// buffered ciphertext after the associated Pack authority is revoked.
pub(crate) struct AuthorityTcpStream {
    inner: TcpStream,
    lease: Option<SessionLease>,
}

impl AuthorityTcpStream {
    pub(crate) fn new(inner: TcpStream) -> Self {
        Self { inner, lease: None }
    }

    /// Install the authenticated sender's lease exactly once. Handshake and
    /// challenge writes precede this transition; ACK and forwarding writes do
    /// not.
    pub(crate) fn authorize(&mut self, lease: SessionLease) -> io::Result<()> {
        if self.lease.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "TCP authority is already installed",
            ));
        }
        self.lease = Some(lease);
        Ok(())
    }
}

impl AsyncRead for AuthorityTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for AuthorityTcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let mut poll = || Pin::new(&mut this.inner).poll_write(cx, bytes);
        let result = match &this.lease {
            Some(lease) => lease.submit(poll),
            None => Ok(poll()),
        };
        match result {
            Ok(result) => result,
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let mut poll = || Pin::new(&mut this.inner).poll_flush(cx);
        let result = match &this.lease {
            Some(lease) => lease.submit(poll),
            None => Ok(poll()),
        };
        match result {
            Ok(result) => result,
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let mut poll = || Pin::new(&mut this.inner).poll_shutdown(cx);
        let result = match &this.lease {
            Some(lease) => lease.submit(poll),
            None => Ok(poll()),
        };
        match result {
            Ok(result) => result,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

mod sealed {
    pub trait Raw {}
    impl Raw for tokio::net::TcpStream {}
    impl Raw for tokio::net::tcp::OwnedWriteHalf {}
    impl Raw for tokio::net::tcp::WriteHalf<'_> {}
    impl Raw for quinn::SendStream {}
    impl<T: Raw + ?Sized> Raw for &mut T {}
}
pub(crate) struct AuthorityWriter<W> {
    inner: W,
    lease: SessionLease,
}
impl<W: AsyncWrite + Unpin + sealed::Raw> AuthorityWriter<W> {
    pub(crate) fn new(inner: W, lease: SessionLease) -> Self {
        Self { inner, lease }
    }
}
impl<W: AsyncWrite + Unpin + sealed::Raw> AsyncWrite for AuthorityWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match this
            .lease
            .submit(|| Pin::new(&mut this.inner).poll_write(cx, bytes))
        {
            Ok(result) => result,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this
            .lease
            .submit(|| Pin::new(&mut this.inner).poll_flush(cx))
        {
            Ok(result) => result,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this
            .lease
            .submit(|| Pin::new(&mut this.inner).poll_shutdown(cx))
        {
            Ok(result) => result,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}
