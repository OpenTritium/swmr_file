use std::sync::Arc;

use sync_file::SyncFile;
use tracing::error;
use crate::file::{FlushedRange, WritedRange};

// 写完执行
pub trait SyncStrategy: Sync + Send + 'static {
    async fn perform(&self, writed: WritedRange, flushed: FlushedRange, fd: Arc<SyncFile>);
}

pub struct ThresholdSyncStrategy(u128);
 
impl Default for ThresholdSyncStrategy {
    fn default() -> Self {
        const THRESHOLD:u128 = 128;
        Self(THRESHOLD)
    }
}

impl SyncStrategy for ThresholdSyncStrategy {
    async fn perform(&self, writed: WritedRange, flushed: FlushedRange, fd: Arc<SyncFile>) {
        let w = writed.read().await.clone();
        let p = flushed.lock().await.clone();
        if (w - p).len() >= self.0 {
            let _ = fd.sync_all().map_err(|err| error!("{err}"));
        }
    }
}

#[derive(Default)]
pub struct ImmediateSyncStrategy;

impl SyncStrategy for ImmediateSyncStrategy {
    async fn perform(&self, _: WritedRange, _: FlushedRange, fd: Arc<SyncFile>) {
        let _ = fd.sync_all().inspect_err(|err| error!("{err}"));
    }
}

impl<F> SyncStrategy for F
where
    F: (AsyncFn(WritedRange, FlushedRange, Arc<SyncFile>) -> ()) + Send + Sync + 'static,
{
    async fn perform(&self, writed: WritedRange, persisted: FlushedRange, fd: Arc<SyncFile>) {
        self(writed, persisted, fd).await
    }
}