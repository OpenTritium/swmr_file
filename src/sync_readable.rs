use std::io::Result as IoResult;
use tokio::io::AsyncReadExt;

pub trait SyncReadable: AsyncReadExt + Unpin + Send {
    fn sync_read(
        &mut self,
        dst: impl AsMut<[u8]> + Send,
    ) -> impl Future<Output = IoResult<usize>> + Send;

    fn sync_read_to_end(
        &mut self,
        dst: impl AsMut<Vec<u8>> + Send,
    ) -> impl Future<Output = IoResult<usize>> + Send;
}
