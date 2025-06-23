use crate::ring_buf::RingBuffer;
use std::{
    io::{ErrorKind as IoErrorKind, Result as IoResult},
    pin::Pin,
    task::{Context, Poll, ready},
};
use tokio::task::JoinHandle;

#[derive(Debug)]
pub(crate) enum TaskState {
    Idle(Option<Box<RingBuffer>>),
    Busy(JoinHandle<(Operation, Box<RingBuffer>)>),
}

impl Default for TaskState {
    fn default() -> Self {
        TaskState::Idle(Some(Default::default()))
    }
}

#[derive(Debug)]
pub(crate) enum Operation {
    Read(IoResult<usize>),
    Write(IoResult<()>),
    Seek(IoResult<u64>),
}

#[derive(Default)]
pub(crate) struct PollState {
    pub(crate) inner: TaskState,
    pub(crate) last_write_err: Option<IoErrorKind>,
    // 仅仅是缓存值，你需要 `SeekFrom::Curretn(0)` 刷新
    pub(crate) pos: u64,
}

impl PollState {
    /// accquire poll result
    pub(crate) fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        use Operation::*;
        use Poll::*;
        use TaskState::*;
        let task_state = &mut self.inner;
        if let Some(err) = self.last_write_err.take() {
            return Ready(Err(err.into()));
        }
        let Busy(h) = task_state else {
            return Ready(Ok(()));
        };
        let (op, buf) = ready!(Pin::new(h).poll(cx))?;
        *task_state = Idle(Some(buf));
        if let Write(result) = op {
            return Ready(result);
        }
        Ready(Ok(()))
    }

    /// accquire poll result, but don't care about err
    pub(crate) fn poll_complete_inflight(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        use Poll::*;
        match self.poll_flush(cx) {
            Ready(Err(err)) => {
                // just return `()`, put err back
                self.last_write_err = Some(err.kind());
                Ready(())
            }
            Ready(Ok(())) => Ready(()),
            Pending => Pending,
        }
    }

    /// convert `poll` into `future`
    pub(crate) async fn complete_inflight(&mut self) {
        std::future::poll_fn(|cx| self.poll_complete_inflight(cx)).await;
    }
}
