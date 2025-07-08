use crate::sync_readable::SyncReadable;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub trait GetReader {
    fn get_reader(&self) -> impl SyncReadable + AsyncReadExt + AsyncSeekExt + Send;
}
