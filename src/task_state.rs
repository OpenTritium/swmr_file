use crate::ring_buf::RingBuffer;
use std::io::Result as IoResult;
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
    Read(IoResult<u64>),
    Write(IoResult<u64>),
    Seek(IoResult<u64>),
}
