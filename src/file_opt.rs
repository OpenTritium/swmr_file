use std::fs::{Metadata, Permissions};
use tokio::io::Result as IoResult;

pub trait FileOpt {
    fn sync_data(&self) -> impl Future<Output = IoResult<()>> + Send;
    fn sync_all(&self) -> impl Future<Output = IoResult<()>> + Send;
    fn set_len(&self, size: u64) -> impl Future<Output = IoResult<()>> + Send;
    fn metadata(&self) -> impl Future<Output = IoResult<Metadata>> + Send;
    fn set_permissions(&self, perm: Permissions) -> impl Future<Output = IoResult<()>> + Send;
}
