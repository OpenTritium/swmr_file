use std::task::{Context, Poll};
use tokio::io::Result as IoResult;

pub(crate) trait PollState {
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<IoResult<()>>;
    fn poll_complete_inflight(&mut self, cx: &mut Context<'_>) -> Poll<()>;
    async fn complete_inflight(&mut self);
}
