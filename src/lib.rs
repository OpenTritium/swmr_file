#![feature(slice_pattern)]
#![feature(read_buf)]
#![feature(seek_stream_len)]

// 数值安全默认按照64位平台处理
pub mod accessor_assign;
pub mod file_opt;
mod poll_state;
pub mod ring_buf;
mod ro;
mod rw;
pub mod sync_readable;
pub mod sync_writable;
pub mod task_state;
pub mod utils;
