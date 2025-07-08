use crate::{
    poll_read_state::PollReadState,
    ring_buf::{BUFFER_MAX_SIZE, RingBufferExt},
    sync_readable::SyncReadable,
    task_state::{Operation::*, TaskState::*},
    utils::new_io_other_err,
    write_read_file::WritedRange,
};
use futures_util::lock::Mutex;
use std::{
    io::{ErrorKind as IoErrorKind, Seek, SeekFrom},
    pin::Pin,
    task::{
        Context,
        Poll::{self, *},
        ready,
    },
};
use sync_file::SyncFile;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, ReadBuf};
use tokio::{io::Result as IoResult, task::spawn_blocking};

pub struct StateReader {
    writed: WritedRange,
    file: SyncFile,
    inner: Mutex<PollReadState>,
}

impl StateReader {
    pub fn new(writed: WritedRange, file: SyncFile) -> Self {
        Self {
            writed,
            file,
            inner: Default::default(),
        }
    }
}

impl AsyncRead for StateReader {
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
                        Write(_) => {
                            unreachable!("unexpected write in reader")
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

impl SyncReadable for StateReader {
    async fn sync_read(&mut self, mut dst: impl AsMut<[u8]>) -> IoResult<usize> {
        self.inner.get_mut().complete_inflight().await;
        let n = self.read(dst.as_mut()).await?;
        let poll_state = self.inner.get_mut();
        let start = poll_state.pos + 1;
        let end = start + n as u64 - 1;
        let sub = range_set_blaze::RangeSetBlaze::from_iter([start..=end]);
        let sup = self.writed.read().await.clone();
        let itv = (&sub & &sup).ranges().next(); // 获取第一个连续区间
        match itv {
            Some(itv) if *itv.start() == start => {
                poll_state.pos = itv.end() + 1;
                Ok(itv.count())
            }
            _ => {
                poll_state.pos = start - 1;
                Err(IoErrorKind::UnexpectedEof.into())
            }
        }
    }

    // todo 还有很多跟 sync 相关的部分没修
    /// 读到最长没有空洞的地方，注意这并不代表文件结束了，你还可以移动游标继续读取
    async fn sync_read_to_end(&mut self, mut dst: impl AsMut<Vec<u8>>) -> IoResult<usize> {
        self.inner.get_mut().complete_inflight().await;
        let n = self.read_to_end(dst.as_mut()).await?;
        dbg!(n);
        let poll_state = self.inner.get_mut();
        let end = poll_state.pos - 1;
        let start = end - n as u64 + 1;
        let sub = range_set_blaze::RangeSetBlaze::from_iter([start..=end]);
        dbg!(&sub);
        let sup = self.writed.read().await.clone();
        let itv = (&sub & &sup).ranges().next(); // 获取第一个连续区间
        dbg!(&itv);
        match itv {
            Some(itv) if *itv.start() == start => {
                poll_state.pos = itv.end() + 1;
                Ok(itv.count())
            }
            _ => {
                poll_state.pos = start - 1;
                Err(IoErrorKind::UnexpectedEof.into())
            }
        }
    }
}

impl AsyncSeek for StateReader {
    fn start_seek(self: Pin<&mut Self>, mut seek: SeekFrom) -> std::io::Result<()> {
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

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
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
                Read(Ok(pos)) => {
                    poll_state.pos = pos;
                    continue;
                }
                Write(_) => {
                    unreachable!("unexpected write op")
                }
                Read(_) => {
                    continue;
                }
            }
        }
    }
}
