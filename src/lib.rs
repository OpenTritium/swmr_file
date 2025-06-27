#![feature(slice_pattern)]
#![feature(read_buf)]
#![feature(seek_stream_len)]
// 数值安全默认按照64位平台处理
pub mod file;
pub mod poll_state;
pub mod reader;
pub mod ring_buf;
pub mod strategy;
pub mod utils;
pub mod writer;
