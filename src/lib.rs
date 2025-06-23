#![feature(slice_pattern)]
#![feature(read_buf)]
#![feature(seek_stream_len)]
// 数值安全默认按照64位平台处理
pub mod file;
pub mod reader;
pub mod strategy;
pub mod utils;
pub mod writer;
pub mod poll_state;
pub mod ring_buf;
use tokio::fs::File;
