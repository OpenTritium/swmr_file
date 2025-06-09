use crate::utils::{Obsolete, TaskState, spawn_mandatory_blocking};
use crate::{
    file::{SyncedRange, WritedRange},
    strategy::SyncStrategy,
    utils::{Operation, PollState, asyncify},
};
use std::io::{SeekFrom, Write};
use std::task::{Context, Poll, ready};
use std::{io::Seek, ops::Not, pin::Pin};
use sync_file::SyncFile;
use tokio::io::AsyncWrite;
use tokio::{
    io::{AsyncSeek, Error as IoError, ErrorKind as IoErrorKind, Result as IoResult},
    sync::Mutex,
    task::spawn_blocking,
};
use tokio_util::bytes::BytesMut;

pub struct StateWriter<S: SyncStrategy> {
    file: SyncFile,
    writed: WritedRange,
    synced: SyncedRange,
    strategy: S,
    inner: Mutex<PollState>,
}

impl<S: SyncStrategy> StateWriter<S> {
    pub(super) fn new(
        file: SyncFile,
        strategy: S,
        writed: WritedRange,
        synced: SyncedRange,
    ) -> Self {
        Self {
            file,
            strategy,
            writed,
            synced,
            inner: Default::default(),
        }
    }

    //todo sync range
    pub async fn sync_all(&self) -> IoResult<()> {
        self.inner.lock().await.complete_inflight().await;
        let file = self.file.clone();
        asyncify(move || file.sync_all()).await
    }

    //todo sync range
    pub async fn sync_data(&self) -> IoResult<()> {
        self.inner.lock().await.complete_inflight().await;
        let file = self.file.clone();
        asyncify(move || file.sync_data()).await
    }

    /// 执行此操作后 pos 会变为 0
    pub async fn set_len(&self, size: u64) -> IoResult<()> {
        use Operation::*;
        use TaskState::*;
        let mut poll_state = self.inner.lock().await;
        poll_state.complete_inflight().await;
        let task_state = &mut poll_state.inner;
        let Idle(buf) = task_state else {
            unreachable!();
        };
        let mut buf = buf.take().unwrap();
        let seek = buf.seek_compensate();
        let mut file = self.file.clone();
        *task_state = Busy(spawn_blocking(move || {
            let result = if let Some(seek) = seek {
                file.seek(seek).and_then(|_| file.set_len(size))
            } else {
                file.set_len(size)
            }
            .map(|()| 0); // the value is discarded later            
            (Seek(result), buf)
        }));
        let Busy(h) = task_state else {
            unreachable!();
        };
        let (op, buf) = h.await?;
        *task_state = Idle(Some(buf));
        let Seek(result) = op else {
            unreachable!();
        };
        result.map(|pos| poll_state.pos = pos)
    }

    pub async fn metadata(&self) -> IoResult<std::fs::Metadata> {
        let file = self.file.clone();
        asyncify(move || file.metadata()).await
    }

    pub async fn set_permissions(&self, perm: std::fs::Permissions) -> IoResult<()> {
        let file = self.file.clone();
        asyncify(move || file.set_permissions(perm)).await
    }
}

impl<S: SyncStrategy> AsyncSeek for StateWriter<S> {
    fn start_seek(self: Pin<&mut Self>, mut pos: SeekFrom) -> IoResult<()> {
        use TaskState::*;
        let this = unsafe { self.get_unchecked_mut() };
        let poll_state = this.inner.get_mut();
        let task_state = &mut poll_state.inner;
        match task_state {
            Busy(_) => Err(IoError::new(
                IoErrorKind::Other,
                "other file operation is pending, call poll_complete before start_seek",
            )),
            Idle(buf) => {
                let mut buf = buf.take().unwrap();
                if !buf.is_empty() {
                    if let SeekFrom::Current(ref mut offset) = pos {
                        *offset += buf.obsolete();
                    }
                }
                let mut file = this.file.clone();
                *task_state = Busy(spawn_blocking(move || {
                    (Operation::Seek(file.seek(pos)), buf)
                }));
                Ok(())
            }
        }
    }

    fn poll_complete(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<u64>> {
        let poll_state = unsafe { self.get_unchecked_mut().inner.get_mut() };
        let task_state = &mut poll_state.inner;
        loop {
            use Operation::*;
            use Poll::*;
            use TaskState::*;
            match task_state {
                Idle(_) => return Ready(Ok(poll_state.pos)),
                Busy(h) => {
                    let (op, buf) = ready!(Pin::new(h).poll(cx))?;
                    *task_state = Idle(Some(buf));
                    match op {
                        Write(Err(err)) => {
                            debug_assert!(poll_state.last_err.is_none(), "poll_state has an error");
                            poll_state.last_err = Some(err.kind());
                        }
                        Seek(result) => {
                            if let Ok(pos) = result {
                                poll_state.pos = pos;
                            }
                            return Ready(result);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

impl<S: SyncStrategy> AsyncWrite for StateWriter<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        src: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = unsafe { self.get_unchecked_mut() };
        let poll_state = this.inner.get_mut();
        let task_state = &mut poll_state.inner;
        if let Some(err) = poll_state.last_err.take() {
            return Poll::Ready(Err(err.into()));
        }
        loop {
            use Operation::*;
            use Poll::*;
            use TaskState::*;
            match task_state {
                Idle(buf) => {
                    let mut buf = buf.take().unwrap();
                    let seek = buf.seek_compensate();
                    buf.extend_from_slice(src);
                    let n = buf.len();
                    let mut file = this.file.clone();
                    let cur_pos = poll_state.pos as usize;
                    let writed = this.writed.clone();
                    let h = spawn_mandatory_blocking(move || {
                        let result = if let Some(seek) = seek {
                            file.seek(seek).and_then(|_| {
                                // `write_all` already ignores interrupts
                                let result = file.write_all(&buf);
                                buf.clear();
                                result
                            })
                        } else {
                            file.write_all(&buf)
                        }
                        .inspect(|_| {
                            let rng = cur_pos..=(cur_pos + buf.len() - 1);
                            writed.blocking_write().ranges_insert(rng);
                        });
                        (Write(result), buf)
                    })
                    .ok_or_else(|| IoError::new(IoErrorKind::Other, "background task failed"))?;
                    *task_state = Busy(h);
                    return Ready(Ok(n));
                }
                Busy(h) => {
                    let (op, buf) = ready!(Pin::new(h).poll(cx))?;
                    *task_state = Idle(Some(buf));
                    if let Write(result) = op {
                        result?;
                    }
                    continue;
                }
            }
        }
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        unsafe { self.get_unchecked_mut().inner.get_mut().poll_flush(cx) }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        self.poll_flush(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, IoError>> {
        use Operation::*;
        use Poll::*;
        use TaskState::*;
        let this = unsafe { self.get_unchecked_mut() };
        let poll_state = this.inner.get_mut();
        let task_state = &mut poll_state.inner;
        if let Some(e) = poll_state.last_err.take() {
            return Poll::Ready(Err(e.into()));
        }
        loop {
            match task_state {
                Idle(buf) => {
                    let mut buf = buf.take().unwrap();
                    let seek = buf
                        .is_empty()
                        .not()
                        .then(|| SeekFrom::Current(buf.obsolete()));
                    let n = bufs.iter().map(|b| b.len()).sum::<usize>();
                    buf.reserve(n);
                    let mut file = this.file.clone();
                    for b in bufs {
                        buf.extend_from_slice(b);
                    }
                    let cur_pos = poll_state.pos as usize;
                    let writed = this.writed.clone();
                    let h = spawn_mandatory_blocking(move || {
                        let res = if let Some(seek) = seek {
                            file.seek(seek).and_then(|_| {
                                // `write_all` already ignores interrupts
                                let result = file.write_all(&buf);
                                buf.clear();
                                result
                            })
                        } else {
                            file.write_all(&buf)
                        }
                        .inspect(|_| {
                            let rng = cur_pos..=(cur_pos + buf.len() - 1);
                            writed.blocking_write().ranges_insert(rng);
                        });
                        (Write(res), buf)
                    })
                    .ok_or_else(|| IoError::new(IoErrorKind::Other, "background task failed"))?;
                    *task_state = Busy(h);
                    return Ready(Ok(n));
                }
                Busy(h) => {
                    let (op, buf) = ready!(Pin::new(h).poll(cx))?;
                    *task_state = Idle(Some(buf));
                    if let Write(result) = op {
                        result?;
                    }
                    continue;
                }
            }
        }
    }
}
