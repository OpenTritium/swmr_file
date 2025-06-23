use crate::file::{SyncedRange, WritedRange};

// 写完执行
pub trait SyncStrategy: Sync + Send + 'static + Unpin {
    #[must_use]
    async fn should_sync(&self, writed: WritedRange, synced: SyncedRange) -> bool;
}

pub struct ThresholdSyncStrategy(u128);

impl Default for ThresholdSyncStrategy {
    fn default() -> Self {
        const THRESHOLD: u128 = 0x400;
        Self(THRESHOLD)
    }
}

impl SyncStrategy for ThresholdSyncStrategy {
    async fn should_sync(&self, writed: WritedRange, synced: SyncedRange) -> bool {
        let w = writed.read().await.clone();
        let p = synced.lock().await.clone();
        (w - p).len() >= self.0
    }
}

#[derive(Default)]
pub struct ImmediateSyncStrategy;

impl SyncStrategy for ImmediateSyncStrategy {
    async fn should_sync(&self, _: WritedRange, _: SyncedRange) -> bool {
        true
    }
}

impl<F> SyncStrategy for F
where
    F: (AsyncFn(WritedRange, SyncedRange) -> bool) + Send + Sync + 'static + Unpin,
{
    async fn should_sync(&self, writed: WritedRange, synced: SyncedRange) -> bool {
        self(writed, synced).await
    }
}
