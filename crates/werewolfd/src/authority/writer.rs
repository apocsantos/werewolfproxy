//! Only raw nonbuffering transport writers may enter the submission gate.
//! A BufWriter above this wrapper is safe; one below it would not be.
use super::SessionLease;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::AsyncWrite;

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
