use crate::{
    file_opt::FileOpt,
    ring_buf::{BUFFER_MAX_SIZE, RingBufferExt},
    rw::{
        poll_write_state::PollWriteState,
        rw_file::{SyncedRange, WritedRange},
        strategy::SyncStrategy,
    },
    sync_readable::SyncReadable,
    sync_writable::SyncWritable,
    task_state::{Operation::*, TaskState::*},
    utils::{asyncify, new_io_other_err, spawn_mandatory_blocking},
};
use std::{
    io::{Seek, SeekFrom, Write},
    ops::Sub,
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
        AsyncRead, AsyncReadExt, AsyncSeek, AsyncWrite, AsyncWriteExt, Error as IoError,
        ErrorKind as IoErrorKind, ReadBuf, Result as IoResult,
    },
    sync::Mutex,
    task::spawn_blocking,
};

pub struct RwAccessor<S: SyncStrategy> {
    file: SyncFile,
    writed: WritedRange,
    synced: SyncedRange,
    strategy: S,
    inner: Mutex<PollWriteState>,
}

impl<S: SyncStrategy> RwAccessor<S> {
    pub(crate) fn new(
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

    /// 同步时会将 `writed_range`` 覆盖 `synced_range`
    async fn sync_with<F>(&self, op: F) -> IoResult<()>
    where
        F: Fn(SyncFile) -> IoResult<()> + Send + 'static,
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
}

/// 由于克隆后的文件游标有自己的游标，需要在后面的返回中更新用户游标
impl<S: SyncStrategy> AsyncSeek for RwAccessor<S> {
    fn start_seek(self: Pin<&mut Self>, mut seek: SeekFrom) -> IoResult<()> {
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
            && let SeekFrom::Current(ref mut rel) = seek
        {
            *rel += buf.obsolete() as i64;
        }
        let mut file = this.file.clone();
        let pos = poll_state.pos;
        *task_state = Busy(spawn_blocking(move || {
            // 只用于换算游标位置，不需要真的移动游标
            let result = file
                .seek(SeekFrom::Start(pos))
                .and_then(|_| file.seek(seek));
            (Seek(result), buf)
        }));
        Ok(())
    }

    /// 空闲时直接返回游标，否则去轮询任务以获取最新内核游标，再同步到用户游标
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
                Seek(result) => {
                    return Ready(result.inspect(|&pos| poll_state.pos = pos));
                }
                Write(Ok(pos)) | Read(Ok(pos)) => {
                    poll_state.pos = pos;
                    continue;
                }
                Write(Err(err)) => {
                    debug_assert!(poll_state.last_write_err.is_none()); // 异步写调用会消耗这个异常
                    poll_state.last_write_err = Some(err.kind());
                }
                Read(_) => {
                    continue;
                }
            }
        }
    }
}

impl<S: SyncStrategy> SyncWritable for RwAccessor<S> {
    /// 尽可能地填满内置buf，每次调用此方法都会出发同步策略
    async fn sync_write(&mut self, src: impl AsRef<[u8]>) -> IoResult<usize> {
        let poll_state = self.inner.get_mut();
        poll_state.complete_inflight().await;
        let writed = self.writed.clone();
        let start = poll_state.pos;
        let n = self.write(src.as_ref()).await?;
        let end = start + n.sub(1) as u64;
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

    /// 持续写入直到流结束，需要注意的是流写入完才会同步状态
    async fn sync_write_all(&mut self, src: impl AsRef<[u8]> + Send) -> IoResult<()> {
        let poll_state = self.inner.get_mut();
        poll_state.complete_inflight().await;
        let start = poll_state.pos;
        let end = start + src.as_ref().len().sub(1) as u64;
        let rng = start..=end;
        self.write_all(src.as_ref()).await?;
        println!("write: {:?}", rng);
        dbg!(&self.writed);
        self.writed.write().await.ranges_insert(rng.clone());
        dbg!(&self.writed);
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
}

impl<S: SyncStrategy> AsyncWrite for RwAccessor<S> {
    /// 文件写入后会返回当前游标的位置
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
                    // 当缓冲区有数据且处于空闲状态时，说明有缓冲等待外部读取，此时游标超前移动，这里做回退处理
                    let mv = buf.obsolete() as i64;
                    // 读取一部分，待会儿去另一个线程里往文件里写
                    buf.read_from(src);
                    let mut file = this.file.clone();
                    let payload_len = buf.len();
                    let pos = poll_state.pos;
                    let writed = this.writed.clone();
                    // 实际上被传入的 File 对象的游标停滞在相当早的状态，进入线程后就需要进行补偿
                    let h = spawn_mandatory_blocking(move || {
                        let result = file
                            .seek(SeekFrom::Start(pos))
                            .and_then(|_| file.seek(SeekFrom::Current(mv)))
                            .and_then(|offset| {
                                let result = file.write_all(&buf).map(|_| offset);
                                buf.clear();
                                result
                                // 你必须在这里消费完
                            })
                            .inspect_err(|_| buf.clear())
                            .map(|start| {
                                let end = start + payload_len.sub(1) as u64;
                                let rng = start..=end;
                                writed.blocking_write().ranges_insert(rng);
                                end + 1
                            });
                        (Write(result), buf)
                    })
                    .ok_or_else(|| new_io_other_err("background task failed"))?;
                    *task_state = Busy(h);
                    return Ready(Ok(payload_len));
                }
                Busy(h) => {
                    let (op, buf) = ready!(Pin::new(h).poll(cx))?;
                    *task_state = Idle(Some(buf));
                    if let Write(result) = op {
                        poll_state.pos = result?;
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
                    let mv = buf.obsolete() as i64;
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
                    let payload_len = buf.len();
                    let pos = poll_state.pos;
                    let writed = this.writed.clone();
                    let h = spawn_mandatory_blocking(move || {
                        let result = file
                            .seek(SeekFrom::Start(pos))
                            .and_then(|_| file.seek(SeekFrom::Current(mv)))
                            .and_then(|offset| {
                                let result = file.write_all(&buf).map(|_| offset);
                                buf.clear();
                                result
                            })
                            .inspect_err(|_| buf.clear())
                            .map(|start| {
                                let end = start + payload_len.sub(1) as u64;
                                let rng = start..=end;
                                writed.blocking_write().ranges_insert(rng);
                                end + 1
                            });
                        (Write(result), buf)
                    })
                    .ok_or_else(|| new_io_other_err("background task failed"))?;
                    *task_state = Busy(h);
                    return Ready(Ok(payload_len));
                }
                Busy(h) => {
                    let (op, buf) = ready!(Pin::new(h).poll(cx))?;
                    *task_state = Idle(Some(buf));
                    if let Write(result) = op {
                        poll_state.pos = result?;
                    }
                    continue;
                }
            }
        }
    }
}

impl<S: SyncStrategy> AsyncRead for RwAccessor<S> {
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
                    if !buf.is_empty() || dst.remaining() == 0 {
                        buf.write_into(dst);
                        *buf_cell = Some(buf);
                        return Ready(Ok(()));
                    }
                    let mut file = this.file.clone();
                    let pos = poll_state.pos;
                    // 获取文件内部的游标，并计算读取多少字节才有效
                    *task_state = Busy(spawn_blocking(move || {
                        debug_assert!(buf.is_empty());
                        buf.reserve(BUFFER_MAX_SIZE);
                        let result = file.seek(SeekFrom::Start(pos)).and_then(|pos| {
                            buf.read_file(&mut file)
                                .map(|payload_len| pos + payload_len as u64)
                        });
                        (Read(result), buf)
                    }));
                }
                Busy(h) => {
                    let (op, mut buf) = ready!(Pin::new(h).poll(cx))?;
                    match op {
                        Read(Ok(pos)) => {
                            // 这里写不完可以下次进入 Idle 处理
                            buf.write_into(dst);
                            poll_state.pos = pos;
                            *task_state = Idle(Some(buf));
                            return Ready(Ok(()));
                        }
                        Read(Err(err)) => {
                            debug_assert!(buf.is_empty());
                            *task_state = Idle(Some(buf));
                            return Ready(Err(err));
                        }
                        Write(Ok(pos)) => {
                            debug_assert!(buf.is_empty());
                            poll_state.pos = pos;
                            *task_state = Idle(Some(buf));
                            continue;
                        }
                        Write(Err(err)) => {
                            debug_assert!(poll_state.last_write_err.is_none());
                            poll_state.last_write_err = Some(err.kind());
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

impl<S: SyncStrategy> FileOpt for RwAccessor<S> {
    async fn sync_all(&self) -> IoResult<()> {
        self.sync_with(|file| file.sync_all()).await
    }

    async fn set_len(&self, size: u64) -> IoResult<()> {
        let mut poll_state = self.inner.lock().await;
        poll_state.complete_inflight().await;
        let pos = poll_state.pos;
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
                .seek(SeekFrom::Start(pos))
                .and_then(|_| file.seek(SeekFrom::Current(buf.obsolete() as i64)))
                .and_then(|pos| file.set_len(size).map(|_| pos))
                .inspect(|_| {
                    // retain `1..=size` of file means `0..size` of range
                    let mut writed = writed.blocking_write();
                    writed.retain(|&n| n < size);
                    synced.blocking_lock().clone_from(&writed);
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
        result.map(|pos| poll_state.pos = pos)
    }

    async fn metadata(&self) -> IoResult<std::fs::Metadata> {
        let file = self.file.clone();
        asyncify(move || file.metadata()).await
    }

    async fn set_permissions(&self, perm: std::fs::Permissions) -> IoResult<()> {
        let file = self.file.clone();
        asyncify(move || file.set_permissions(perm)).await
    }

    fn sync_data(&self) -> impl Future<Output = IoResult<()>> + Send {
        self.sync_with(|file| file.sync_data())
    }
}

impl<S: SyncStrategy> SyncReadable for RwAccessor<S> {
    async fn sync_read(&mut self, mut dst: impl AsMut<[u8]>) -> IoResult<usize> {
        let poll_state = self.inner.get_mut();
        poll_state.complete_inflight().await;
        let start = poll_state.pos + 1;
        let n = self.read(dst.as_mut()).await?;
        let end = start + n as u64 - 1;
        let sub = range_set_blaze::RangeSetBlaze::from_iter([start..=end]);
        let sup = self.writed.read().await.clone();
        let itv = (&sub & &sup).ranges().next(); // 获取第一个连续区间
        match itv {
            Some(itv) if *itv.start() == start => Ok(itv.count()),
            _ => Err(IoErrorKind::UnexpectedEof.into()),
        }
    }

    /// 读到最长没有空洞的地方，注意这并不代表文件结束了，你还可以移动游标继续读取
    async fn sync_read_to_end(&mut self, mut dst: impl AsMut<Vec<u8>>) -> IoResult<usize> {
        let poll_state = self.inner.get_mut();
        poll_state.complete_inflight().await;
        let start = poll_state.pos + 1;
        let n = self.read_to_end(dst.as_mut()).await?;
        let end = start + n as u64 - 1;
        let sub = range_set_blaze::RangeSetBlaze::from_iter([start..=end]);
        let sup = self.writed.read().await.clone();
        let itv = (&sub & &sup).ranges().next(); // 获取第一个连续区间
        match itv {
            Some(itv) if *itv.start() == start => Ok(itv.count()),
            _ => Err(IoErrorKind::UnexpectedEof.into()),
        }
    }
}
