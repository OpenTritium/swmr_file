use crate::write_read_file::{SyncedRange, WritedRange};

pub trait SyncStrategy: Sync + Send + 'static + Unpin {
    #[must_use]
    fn should_sync(
        &self,
        writed: WritedRange,
        synced: SyncedRange,
    ) -> impl Future<Output = bool> + Send;
}

pub struct ThresholdSyncStrategy(u128);

impl Default for ThresholdSyncStrategy {
    fn default() -> Self {
        const SYNC_THRESHOLD: u128 = 8 * 0x400;
        Self(SYNC_THRESHOLD)
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

impl<F, Fut> SyncStrategy for F
where
    Fut: Future<Output = bool> + Send,
    F: (Fn(WritedRange, SyncedRange) -> Fut) + Send + Sync + 'static + Unpin,
{
    async fn should_sync(&self, writed: WritedRange, synced: SyncedRange) -> bool {
        self(writed, synced).await
    }
}
