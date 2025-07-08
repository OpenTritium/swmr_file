use std::{
    io::Result as IoResult,
    pin::Pin,
    task::{Context, Poll, ready},
};

use crate::task_state::TaskState::{self, *};

#[derive(Default)]
pub struct PollReadState {
    pub(crate) inner: TaskState,
    pub(crate) pos: u64, // 总是指向下个待处理的位置
}

impl PollReadState {
    /// 获取完成状态
    pub(crate) fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        use crate::task_state::Operation::*;
        use Poll::*;
        let task_state = &mut self.inner;
        let Busy(h) = task_state else {
            return Ready(Ok(()));
        };
        let (op, buf) = ready!(Pin::new(h).poll(cx))?;
        *task_state = Idle(Some(buf));
        if let Write(result) = op {
            return Ready(result.map(|_| ()));
        }
        Ready(Ok(()))
    }

    /// 获取完成这状态，即使出错了也算完成
    pub(crate) fn poll_complete_inflight(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.poll_flush(cx).map(|_| ())
    }

    /// convert `poll` into `future`
    pub(crate) async fn complete_inflight(&mut self) {
        std::future::poll_fn(|cx| self.poll_complete_inflight(cx)).await;
    }
}
