use async_trait::async_trait;

use crate::cloud_sync::models::CloudSyncData;

use super::{SyncDeviceId, SyncStoreError, SyncStoreLock, SyncStoreStatus};

#[async_trait]
pub trait PersonalSyncStore: Send + Sync {
    fn backend_id(&self) -> &'static str;

    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError>;

    async fn list_records(
        &self,
        data_type: Option<&str>,
        since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, SyncStoreError>;

    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError>;

    async fn tombstone_record(
        &self,
        data_type: &str,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError>;

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError>;

    /// 为 [`Self::acquire_lock`] 拿到的锁做**异步**释放。
    ///
    /// 本地后端在 `Drop` 里就删掉锁文件了，默认实现是空操作；只有远端锁
    /// （WebDAV）必须显式调用 —— `Drop` 不能 `await`。实现方不应让释放失败影响
    /// 本轮同步结果：远端锁还有 TTL 兜底，worker 只会记一条 warning。
    async fn release_lock(&self, _lock: &SyncStoreLock) -> Result<(), SyncStoreError> {
        Ok(())
    }
}
