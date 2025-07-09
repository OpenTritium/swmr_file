use crate::{
    poll_state::PollState,
    task_state::TaskState::{self, *},
};
use std::{
    pin::Pin,
    task::{
        Context,
        Poll::{self, *},
        ready,
    },
};
use tokio::io::Result as IoResult;

#[derive(Default)]
pub struct PollReadState {
    pub(crate) inner: TaskState,
    pub(crate) pos: u64,
}

impl PollState for PollReadState {
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        let task_state = &mut self.inner;
        let Busy(h) = task_state else {
            return Ready(Ok(()));
        };
        let (_, buf) = ready!(Pin::new(h).poll(cx))?;
        *task_state = Idle(Some(buf));
        Ready(Ok(()))
    }

    fn poll_complete_inflight(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.poll_flush(cx).map(|_| ())
    }

    async fn complete_inflight(&mut self) {
        std::future::poll_fn(|cx| self.poll_complete_inflight(cx)).await;
    }
}
