use std::{
    io::SeekFrom,
    ops::Not,
    pin::Pin,
    task::{Context, Poll, ready},
};
use tokio::{
    io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult},
    task::JoinHandle,
};
use tokio_util::bytes::{Buf, BytesMut};

pub(crate) async fn asyncify<F, T>(f: F) -> IoResult<T>
where
    F: FnOnce() -> IoResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|err| Err(IoError::new(IoErrorKind::Other, err)))
}

#[derive(Debug)]
pub(crate) enum TaskState {
    Idle(Option<BytesMut>),
    Busy(JoinHandle<(Operation, BytesMut)>),
}

impl Default for TaskState {
    fn default() -> Self {
        TaskState::Idle(Some(BytesMut::new()))
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
    pub(crate) last_err: Option<IoErrorKind>,
    pub(crate) pos: u64,
}

impl PollState {
    /// accquire poll result
    pub(crate) fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        use Operation::*;
        use Poll::*;
        use TaskState::*;
        if let Some(e) = self.last_err.take() {
            return Ready(Err(e.into()));
        }
        let (op, buf) = match self.inner {
            Idle(_) => return Ready(Ok(())),
            Busy(ref mut h) => ready!(Pin::new(h).poll(cx))?,
        };
        // store the buf for later use
        self.inner = Idle(Some(buf));
        match op {
            Read(_) | Seek(_) => Ready(Ok(())),
            Write(result) => Ready(result),
        }
    }

    /// accquire poll result, but don't care about err
    pub(crate) fn poll_complete_inflight(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        use Poll::*;
        match self.poll_flush(cx) {
            Ready(Err(e)) => {
                // just return `()`, put err back
                self.last_err = Some(e.kind());
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

pub(crate) trait Obsolete {
    ///clear the buf, and return discarded bytes count (negtive)
    fn obsolete(&mut self) -> i64;
    fn seek_compensate(&mut self) -> Option<SeekFrom>;
}

impl Obsolete for BytesMut {
    #[inline(always)]
    fn obsolete(&mut self) -> i64 {
        use std::ops::Neg;
        let n = self.len();
        self.clear();
        i64::try_from(n)
            .expect("drained buf offset overflowed")
            .neg()
    }

    #[inline(always)]
    /// if buf is not empty, return `SeekFrom::Current(-buf.len())`
    fn seek_compensate(&mut self) -> Option<SeekFrom> {
        self.is_empty()
            .not()
            .then(|| SeekFrom::Current(self.obsolete()))
    }
}

// async fn spawn_mandatory_blocking<F, R>(f: F) -> Option<JoinHandle<R>>
// where
//     F: FnOnce() -> R + Send + 'static,
//     R: Send + 'static,
// {
//     let h = tokio::task::spawn_blocking(|| tokio::task::coop::unconstrained(f));
//     if tokio::runtime::Handle::try_current().is_err() {
//         return None; // 运行时已关闭
//     }
//     let h = h.await.unwrap();
//     Some()
// }

#[inline(always)]
pub fn spawn_mandatory_blocking<F, R>(f: F) -> Option<tokio::task::JoinHandle<R>>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let handle = tokio::runtime::Handle::try_current().ok()?;
    Some(handle.spawn_blocking(f))
}
