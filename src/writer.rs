use crate::{
    file::{SyncedRange, WritedRange},
    poll_state::{Operation::*, PollState, TaskState::*},
    ring_buf::{BUFFER_MAX_SIZE, BUFFER_MIN_SIZE, RingBufferExt},
    strategy::SyncStrategy,
    utils::{SyncRedable, SyncWritable, asyncify, new_io_other_err, spawn_mandatory_blocking},
};
use std::{
    io::{Seek, SeekFrom, Write},
    pin::Pin,
    task::{
        Context,
        Poll::{self, *},
        ready,
    },
};
use sync_file::SyncFile;
use tokio::{
    io::{
        AsyncRead, AsyncReadExt, AsyncSeek, AsyncWrite, AsyncWriteExt, Error as IoError, ReadBuf,
        Result as IoResult,
    },
    sync::Mutex,
    task::spawn_blocking,
};

pub struct StateWriter<S: SyncStrategy> {
    file: SyncFile, //每个file都有自己的游标
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

    /// 同步时会将写入range覆盖同步range
    async fn sync_with<F>(&self, op: F) -> IoResult<()>
    where
        F: Fn(SyncFile) -> Result<(), IoError> + Send + 'static,
    {
        self.inner.lock().await.complete_inflight().await;
        let file = self.file.clone();
        let writed = self.writed.clone();
        let synced = self.synced.clone();
        asyncify(move || {
            let result = op(file);
            let writed = writed.blocking_read().clone();
            *synced.blocking_lock() = writed;
            result
        })
        .await
    }
    /// 与 `sync_all` 不同的是，此操作不同步文件元数据（例如修改日期）
    pub async fn sync_data(&self) -> IoResult<()> {
        self.sync_with(|file| file.sync_data()).await
    }

    pub async fn sync_all(&self) -> IoResult<()> {
        self.sync_with(|file| file.sync_all()).await
    }

    /// 用于截断或拓展文件，不论如何游标请保持操作前的样子
    pub async fn set_len(&self, size: u64) -> IoResult<()> {
        let mut poll_state = self.inner.lock().await;
        poll_state.complete_inflight().await;
        let task_state = &mut poll_state.inner;
        let Idle(buf) = task_state else {
            unreachable!();
        };
        let mut buf = buf.take().unwrap();
        let mut file = self.file.clone();
        let writed = self.writed.clone();
        let synced = self.synced.clone();
        *task_state = Busy(spawn_blocking(move || {
            let result = file
                .seek_relative(buf.obsolete() as i64)
                .and_then(|_| file.set_len(size))
                .map(|_| {
                    // retain `1..=size` of file means `0..size` of range
                    let mut writed = writed.blocking_write();
                    writed.retain(|&n| n < size);
                    synced.blocking_lock().clone_from(&writed);
                    0 // 原版的 set_len 貌似并不会重置游标
                });
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
        result.map(|_| ())
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
    /// 此操作会启动一个任务，旨在消耗缓冲并进行偏移量补偿
    fn start_seek(self: Pin<&mut Self>, mut pos: SeekFrom) -> IoResult<()> {
        let this = self.get_mut();
        let poll_state = this.inner.get_mut();
        let task_state = &mut poll_state.inner;
        let Idle(buf) = task_state else {
            return Err(new_io_other_err(
                "other file operation is pending, call poll_complete before start_seek",
            ));
        };
        let mut buf = buf.take().unwrap();
        if !buf.is_empty()
            && let SeekFrom::Current(ref mut offset) = pos
        {
            *offset += buf.obsolete() as i64;
        }
        let mut file = this.file.clone();
        *task_state = Busy(spawn_blocking(move || (Seek(file.seek(pos)), buf)));
        Ok(())
    }

    /// 空闲时直接返回 pos，否则 poll 任务获取更新后的 pos，再更新内部 pos 状态
    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<u64>> {
        let poll_state = self.inner.get_mut();
        let task_state = &mut poll_state.inner;
        loop {
            let Busy(h) = task_state else {
                return Ready(Ok(poll_state.pos));
            };
            let (op, buf) = ready!(Pin::new(h).poll(cx))?;
            *task_state = Idle(Some(buf));
            match op {
                Write(Err(err)) => {
                    debug_assert!(poll_state.last_write_err.is_none()); // 理论上异步写调用会消耗这个异常
                    poll_state.last_write_err = Some(err.kind());
                }
                Seek(result) => {
                    return Ready(result.inspect(|&pos| poll_state.pos = pos));
                }
                _ => {}
            }
        }
    }
}

impl<S: SyncStrategy> SyncWritable for StateWriter<S> {
    /// 持续写入直到流结束，需要注意的是流写入完才会同步状态
    async fn sync_write_all(&mut self, src: impl AsRef<[u8]>) -> IoResult<()> {
        self.inner.lock().await.complete_inflight().await;
        let start = self.file.offset() + 1;
        let end = start + src.as_ref().len() as u64 - 1;
        let rng = start..=end;
        self.write_all(src.as_ref()).await?;
        self.writed.write().await.ranges_insert(rng.clone());
        let need_sync = self
            .strategy
            .should_sync(self.writed.clone(), self.synced.clone())
            .await;
        if need_sync {
            self.sync_all().await?;
            self.synced.lock().await.ranges_insert(rng);
        }
        Ok(())
    }

    /// 尽可能地填满内置buf，每次调用此方法都会出发同步策略
    async fn sync_write(&mut self, src: impl AsRef<[u8]>) -> IoResult<usize> {
        self.inner.lock().await.complete_inflight().await;
        let writed = self.writed.clone();
        let start = self.file.offset() + 1;
        let n = self.write(src.as_ref()).await?;
        let end = start + n as u64 - 1;
        let rng = start..=end;
        writed.write().await.ranges_insert(rng.clone());
        let need_sync = self
            .strategy
            .should_sync(self.writed.clone(), self.synced.clone())
            .await;
        if need_sync {
            self.sync_all().await?;
            self.synced.lock().await.ranges_insert(rng);
        }
        Ok(n)
    }
}

impl<S: SyncStrategy> AsyncWrite for StateWriter<S> {
    ///
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, src: &[u8]) -> Poll<IoResult<usize>> {
        let this = self.get_mut();
        let poll_state = this.inner.get_mut();
        let task_state = &mut poll_state.inner;
        if let Some(err) = poll_state.last_write_err.take() {
            return Ready(Err(err.into()));
        }
        loop {
            match task_state {
                Idle(buf) => {
                    let mut buf = buf.take().unwrap();
                    let mv = buf.obsolete() as i64;
                    // 读取一部分，待会儿去另一个线程里往文件里写
                    buf.read_from(src);
                    let mut file = this.file.clone();
                    let n = buf.len();
                    let writed = this.writed.clone();
                    let h = spawn_mandatory_blocking(move || {
                        let result = file
                            .seek(SeekFrom::Current(mv))
                            .and_then(|offset| {
                                let result = file.write_all(&buf).map(|_| offset);
                                buf.clear_shrink(BUFFER_MAX_SIZE);
                                result
                                // 你必须在这里消费完
                            })
                            .inspect_err(|_| buf.clear_shrink(BUFFER_MIN_SIZE))
                            .map(|offset| {
                                let rng = offset..=(offset + n as u64 - 1);
                                writed.blocking_write().ranges_insert(rng);
                            });
                        (Write(result), buf)
                    })
                    .ok_or_else(|| new_io_other_err("background task failed"))?;
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

    /// 支持向量化写入
    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        self.inner.get_mut().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        self.poll_flush(cx)
    }

    /// 此操作会同步 `writed_range`，但并不会触发同步策略
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, IoError>> {
        let this = self.get_mut();
        let poll_state = this.inner.get_mut();
        let task_state = &mut poll_state.inner;
        if let Some(e) = poll_state.last_write_err.take() {
            return Poll::Ready(Err(e.into()));
        }
        loop {
            match task_state {
                Idle(buf) => {
                    let mut buf = buf.take().unwrap();
                    let mut n = BUFFER_MAX_SIZE.min(bufs.iter().map(|b| b.len()).sum());
                    let mv = buf.obsolete();
                    let mut file = this.file.clone();
                    // 至少会读入完整的一片
                    for b in bufs {
                        if n == 0 {
                            break;
                        }
                        let len = buf.len().min(n);
                        buf.extend_from_slice(b);
                        n -= len
                    }
                    let n = buf.len();
                    let writed = this.writed.clone();
                    let h = spawn_mandatory_blocking(move || {
                        let result = file
                            .seek(SeekFrom::Current(mv as i64))
                            .and_then(|offset| {
                                file.write_all(&buf).map(|_| offset)
                                // 你必须在这里消费完
                            })
                            .inspect_err(|_| buf.clear_shrink(BUFFER_MIN_SIZE))
                            .map(|offset| {
                                let rng = offset..=(offset + n as u64 - 1);
                                writed.blocking_write().ranges_insert(rng);
                                buf.clear_shrink(BUFFER_MAX_SIZE); // 因为是向量化写入就保留大一点的缓冲
                            });
                        (Write(result), buf)
                    })
                    .ok_or_else(|| new_io_other_err("background task failed"))?;
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

impl<S: SyncStrategy> AsyncRead for StateWriter<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        dst: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        let this = self.get_mut();
        let poll_state = this.inner.get_mut();
        let task_state = &mut poll_state.inner;
        loop {
            match task_state {
                Idle(buf_cell) => {
                    let mut buf = buf_cell.take().unwrap();
                    // 快速从缓冲区读结果
                    // 满了也快速返回
                    if !buf.is_empty() || dst.remaining() == 0 {
                        buf.write_to(dst);
                        *buf_cell = Some(buf);
                        return Ready(Ok(()));
                    }
                    let mut file = this.file.clone();
                    // 获取文件内部的游标，并计算读取多少字节才有效
                    *task_state = Busy(spawn_blocking(move || {
                        debug_assert!(buf.is_empty());
                        buf.reserve(BUFFER_MAX_SIZE);
                        let result = dbg!(buf.read_file(&mut file));
                        (Read(result), buf)
                    }));
                }
                Busy(h) => {
                    let (op, mut buf) = ready!(Pin::new(h).poll(cx))?;
                    match op {
                        // 如果bool 为true 返回pending
                        Read(Ok(_)) => {
                            // 写不完怎么办，可以看看 Idle 状态的代码
                            buf.write_to(dst);
                            *task_state = Idle(Some(buf));
                            return Ready(Ok(()));
                        }
                        Read(Err(err)) => {
                            debug_assert!(buf.is_empty());
                            *task_state = Idle(Some(buf));
                            return Ready(Err(err));
                        }
                        Write(Ok(())) => {
                            debug_assert!(buf.is_empty());
                            *task_state = Idle(Some(buf));
                            continue;
                        }
                        Write(Err(e)) => {
                            debug_assert!(poll_state.last_write_err.is_none());
                            poll_state.last_write_err = Some(e.kind());
                            *task_state = Idle(Some(buf));
                        }
                        Seek(result) => {
                            debug_assert!(buf.is_empty());
                            *task_state = Idle(Some(buf));
                            if let Ok(pos) = result {
                                poll_state.pos = pos;
                            }
                            continue;
                        }
                    }
                }
            }
        }
    }
}

impl<S: SyncStrategy> SyncRedable for StateWriter<S> {
    async fn get_poll_state(&'_ self) -> tokio::sync::MutexGuard<'_, PollState> {
        self.inner.lock().await
    }

    fn get_writed(&self) -> &WritedRange {
        &self.writed
    }

    async fn read_inherit(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        self.read(buf).await
    }

    async fn read_to_end_inherit(&mut self, buf: &mut Vec<u8>) -> IoResult<usize> {
        self.read_to_end(buf).await
    }

    fn offset(&self) -> u64 {
        self.file.offset()
    }
}
