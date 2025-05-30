use std::{
    io::Write, pin::Pin, sync::{atomic::{AtomicU64, AtomicUsize}, Arc}, task::{Context, Poll}
};

use crate::{
    file::{FlushedRange, WritedRange},
    strategy::SyncStrategy,
};
use futures_util::FutureExt;
use sync_file::SyncFile;
use tokio::{
    io::AsyncWrite,
    task::{JoinHandle, spawn_blocking},
};

#[derive(Debug)]
pub(crate) struct Buf {
    buf: Vec<u8>,
    pos: usize,
}

struct Inner {
    state: State,
    last_write_err: Option<std::io::ErrorKind>,
    pos: usize,
}

#[derive(Debug)]
enum State {
    Idle(Option<Buf>),
    Busy(JoinHandle<(Operation, Buf)>),
}

#[derive(Debug)]
enum Operation {
    Read(std::io::Result<usize>),
    Write(std::io::Result<()>),
    Seek(std::io::Result<u64>),
}

pub struct StateWriter<S: SyncStrategy> {
    fd: Arc<SyncFile>,
    writed: WritedRange,
    flushed: FlushedRange,
    task: Option<JoinHandle<Result<usize, std::io::Error>>>,
    strategy: S,
    offset:AtomicUsize,
}

impl<S: SyncStrategy> StateWriter<S> {
    pub fn new(fd: SyncFile, strategy: S, writed: WritedRange, flushed: FlushedRange) -> Self {
        let fd = Arc::new(fd);
        let offset = AtomicUsize::new(fd.offset() as usize);
        Self {
            fd,
            task: None,
            strategy,
            writed,
            flushed,
            offset,
        }
    }
}

impl<S: SyncStrategy> Unpin for StateWriter<S> {}

impl<S: SyncStrategy> AsyncWrite for StateWriter<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        if let Some(fut) = this.task.as_mut() {
            match fut.poll_unpin(cx) {
                Poll::Ready(Ok(Ok(n))) => {
                    let offset = this.offset.load(std::sync::atomic::Ordering::Acquire);
                    this.writed.blocking_write().ranges_insert(offset..=(offset+n-1));
                    
                    this.task = None;
                    Poll::Ready(res)
                }
                Poll::Ready(Err(e)) => {
                    this.task = None;
                    Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
                }
                Poll::Pending => Poll::Pending,
            }
        } else {
            let mut fd = this.fd.clone();
            let owned_buf = buf.to_vec();
            let join_handle = spawn_blocking(move || fd.write(&owned_buf));
            this.task = Some(join_handle);
            Pin::new(this).poll_write(cx, buf)
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
        let this = self.get_mut();
        this.fd.flush()?;
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
        let this = self.get_mut();
        this.fd.flush()?;
        std::task::Poll::Ready(Ok(()))
    }
}
