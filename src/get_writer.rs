use crate::{file_opt::FileOpt, strategy::SyncStrategy, sync_writable::SyncWritable};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub trait GetWriter<S: SyncStrategy> {
    fn get_writer(
        &self,
        strategy: S,
    ) -> impl SyncWritable + AsyncWriteExt + AsyncSeekExt + AsyncReadExt + AsyncSeekExt + FileOpt + Send;
}
