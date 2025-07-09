use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::{file_opt::FileOpt, sync_readable::SyncReadable, sync_writable::SyncWritable};

pub trait AccessorAssign {
    fn assign_mut(
        &mut self,
    ) -> impl SyncWritable
    + SyncReadable
    + AsyncReadExt
    + AsyncWriteExt
    + AsyncSeekExt
    + FileOpt
    + Send
    + 'static;

    fn assign(
        &mut self,
    ) -> impl SyncWritable + SyncReadable + AsyncReadExt + AsyncSeekExt + FileOpt + Send + 'static;
}
