#![feature(slice_pattern)]
#![feature(read_buf)]
#![feature(seek_stream_len)]

// 数值安全默认按照64位平台处理
pub mod file_opt;
pub mod get_reader;
pub mod get_writer;
pub mod just_reader;
pub mod poll_read_state;
pub mod poll_write_state;
pub mod readonly_file;
pub mod ring_buf;
pub mod state_reader;
pub mod state_writer;
pub mod strategy;
pub mod sync_readable;
pub mod sync_writable;
pub mod task_state;
pub mod utils;
pub mod write_read_file;
