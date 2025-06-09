use crate::file::{SyncedRange, WritedRange};
use sync_file::SyncFile;
use tracing::error;

// 写完执行
pub trait SyncStrategy: Sync + Send + 'static {
    async fn perform(&self, writed: WritedRange, synced: SyncedRange, file: SyncFile);
}

pub struct ThresholdSyncStrategy(u128);

impl Default for ThresholdSyncStrategy {
    fn default() -> Self {
        const THRESHOLD: u128 = 128;
        Self(THRESHOLD)
    }
}

impl SyncStrategy for ThresholdSyncStrategy {
    async fn perform(&self, writed: WritedRange, synced: SyncedRange, file: SyncFile) {
        let w = writed.read().await.clone();
        let p = synced.lock().await.clone();
        if (w - p).len() >= self.0 {
            let _ = file.sync_all().map_err(|err| error!("{err}"));
        }
    }
}

#[derive(Default)]
pub struct ImmediateSyncStrategy;

impl SyncStrategy for ImmediateSyncStrategy {
    async fn perform(&self, _: WritedRange, _: SyncedRange, file: SyncFile) {
        let _ = file.sync_all().inspect_err(|err| error!("{err}"));
    }
}

impl<F> SyncStrategy for F
where
    F: (AsyncFn(WritedRange, SyncedRange, SyncFile) -> ()) + Send + Sync + 'static,
{
    async fn perform(&self, writed: WritedRange, synced: SyncedRange, file: SyncFile) {
        self(writed, synced, file).await
    }
}
