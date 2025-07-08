use tokio::io::AsyncWrite;
use tokio::io::Result as IoResult;

use crate::file_opt::FileOpt;

pub trait SyncWritable: AsyncWrite + FileOpt + Send {
    fn sync_write_all(
        &mut self,
        src: impl AsRef<[u8]> + Send,
    ) -> impl Future<Output = IoResult<()>> + Send;

    fn sync_write(
        &mut self,
        src: impl AsRef<[u8]> + Send,
    ) -> impl Future<Output = IoResult<usize>> + Send;
}
