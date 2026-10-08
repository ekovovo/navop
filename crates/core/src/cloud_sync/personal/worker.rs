use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cloud_sync::models::CloudSyncData;

use super::{
    PersonalConflictType, PersonalSyncCloudKey, PersonalSyncItemSnapshot, PersonalSyncPlan,
    PersonalSyncPlanner, PersonalSyncRecordConflict, PersonalSyncStore, SyncDeviceId,
    SyncStoreError,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PersonalSyncEvent {
    FullScan,
    LocalChanged { data_type: String, local_id: String },
    LocalDeleted { data_type: String, cloud_id: String },
    RemoteChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    pub backend_profile_id: String,
    pub device_id: SyncDeviceId,
}

impl WorkerConfig {
    #[cfg(test)]
    pub fn test() -> Self {
        Self {
            backend_profile_id: "personal-test".to_string(),
            device_id: SyncDeviceId("test-device".to_string()),
        }
    }
}

#[async_trait]
pub trait PersonalSyncLocalSource: Send + Sync {
    async fn list_items(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError>;

    async fn export_item(
        &self,
        item: &PersonalSyncItemSnapshot,
    ) -> Result<CloudSyncData, SyncStoreError>;

    async fn apply_remote(
        &self,
        record: &CloudSyncData,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError>;

    /// 一轮同步的收尾（在本地写入全部完成后调用）
    ///
    /// 用于解析跨记录的云端引用，例如按本次拉取到的云端记录对齐分组层级。
    async fn finalize_pass(&self, _records: &[CloudSyncData]) -> Result<(), SyncStoreError> {
        Ok(())
    }

    async fn mark_synced(
        &self,
        local_id: &str,
        cloud_id: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError>;

    async fn delete_item(&self, item: &PersonalSyncItemSnapshot) -> Result<(), SyncStoreError>;
}

#[async_trait]
pub trait PersonalSyncConflictSink: Send + Sync {
    async fn paused_record_keys(&self) -> Result<HashSet<PersonalSyncCloudKey>, SyncStoreError> {
        Ok(HashSet::new())
    }

    async fn pause_record(
        &self,
        conflict: &PersonalSyncRecordConflict,
        local: Option<&PersonalSyncItemSnapshot>,
        remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError>;

    /// 本地条目被删除后调用：这条记录上遗留的冲突已经失去意义，直接丢掉。
    ///
    /// 冲突解决要按云端记录反查本地条目（`local.list_items()`），本地条目消失后
    /// 冲突就再也解不开了，所以不能让它留在表里。
    async fn forget_record(&self, _data_type: &str, _cloud_id: &str) -> Result<(), SyncStoreError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct NoopConflictSink;

#[async_trait]
impl PersonalSyncConflictSink for NoopConflictSink {
    async fn pause_record(
        &self,
        _conflict: &PersonalSyncRecordConflict,
        _local: Option<&PersonalSyncItemSnapshot>,
        _remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct WorkerState {
    pending: HashSet<PersonalSyncEvent>,
    active: bool,
    dirty: bool,
}

#[derive(Clone)]
pub struct PersonalSyncWorker<S, L, C = NoopConflictSink> {
    store: S,
    local: L,
    conflicts: C,
    config: WorkerConfig,
    planner: PersonalSyncPlanner,
    state: Arc<Mutex<WorkerState>>,
}

impl<S, L> PersonalSyncWorker<S, L, NoopConflictSink>
where
    S: PersonalSyncStore + Clone + Send + Sync,
    L: PersonalSyncLocalSource + Clone + Send + Sync,
{
    pub fn new(store: S, local: L, config: WorkerConfig) -> Self {
        Self::with_conflict_sink(store, local, NoopConflictSink, config)
    }
}

impl<S, L, C> PersonalSyncWorker<S, L, C>
where
    S: PersonalSyncStore + Clone + Send + Sync,
    L: PersonalSyncLocalSource + Clone + Send + Sync,
    C: PersonalSyncConflictSink + Clone + Send + Sync,
{
    pub fn with_conflict_sink(store: S, local: L, conflicts: C, config: WorkerConfig) -> Self {
        Self {
            store,
            local,
            conflicts,
            config,
            planner: PersonalSyncPlanner::new(),
            state: Arc::new(Mutex::new(WorkerState::default())),
        }
    }

    pub fn enqueue(&self, event: PersonalSyncEvent) {
        let mut state = self.state.lock().expect("personal sync worker state");
        if state.active {
            state.dirty = true;
        }
        state.pending.insert(event);
    }

    pub async fn drain_once(&self) -> Result<(), SyncStoreError> {
        let Some(events) = self.begin_drain() else {
            return Ok(());
        };

        let result = self.run_pass(events).await;
        self.finish_drain();
        result
    }

    fn begin_drain(&self) -> Option<HashSet<PersonalSyncEvent>> {
        let mut state = self.state.lock().expect("personal sync worker state");
        if state.active || state.pending.is_empty() {
            return None;
        }

        let events = std::mem::take(&mut state.pending);
        state.active = true;
        Some(events)
    }

    fn finish_drain(&self) {
        let mut state = self.state.lock().expect("personal sync worker state");
        state.active = false;
        if state.dirty {
            state.pending.insert(PersonalSyncEvent::FullScan);
            state.dirty = false;
        }
    }

    async fn run_pass(&self, events: HashSet<PersonalSyncEvent>) -> Result<(), SyncStoreError> {
        self.store.probe().await?;
        let _lock = self.store.acquire_lock(&self.config.device_id).await?;
        let local_items = self.local.list_items().await?;
        let remote_records = self.store.list_records(None, None).await?;
        self.apply_local_delete_events(&events, &remote_records)
            .await?;
        // 挂起状态必须在处理远端墓碑之前取出来：墓碑处理要跳过已挂起的记录，
        // 否则会把冲突指向的那条本地行删掉，留下再也解不开的悬挂冲突。
        let mut paused = self.conflicts.paused_record_keys().await?;
        let tombstone_paused = self
            .apply_remote_tombstones(&local_items, &remote_records, &paused)
            .await?;
        // 本轮刚挂起的记录也要排除出 planner：否则同一次 pass 会把它重新上传，
        // 与刚记下的冲突自相矛盾（远端删了、本地又推回去）。
        paused.extend(tombstone_paused.iter().cloned());
        let tombstone_conflicts = tombstone_paused.len();
        let deleted = local_deleted_cloud_ids(&events);
        let active_remote_records = remote_records
            .into_iter()
            .filter(|record| {
                record.deleted_at.is_none()
                    && !deleted.contains(&PersonalSyncCloudKey {
                        data_type: record.data_type.clone(),
                        cloud_id: record.id.clone(),
                    })
            })
            .collect::<Vec<_>>();
        let plan = self
            .planner
            .plan(&local_items, &active_remote_records, &paused);

        self.apply_plan(&plan, &local_items, &active_remote_records)
            .await?;
        self.local.finalize_pass(&active_remote_records).await?;
        if tombstone_conflicts > 0 {
            return Err(SyncStoreError::Conflict(format!(
                "{tombstone_conflicts} remote deletion conflict(s) paused"
            )));
        }
        Ok(())
    }

    async fn apply_local_delete_events(
        &self,
        events: &HashSet<PersonalSyncEvent>,
        records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        for key in local_deleted_cloud_ids(events) {
            // 用户显式删掉了本地条目 ⇒ 这条记录上遗留的冲突已经没有任何意义，
            // 顺手清掉，免得对话框里留下一个点哪个按钮都报错的冲突。
            self.conflicts
                .forget_record(&key.data_type, &key.cloud_id)
                .await?;
            let Some(record) = find_remote_by_cloud_key(records, &key) else {
                continue;
            };
            if record.deleted_at.is_none() {
                self.store
                    .tombstone_record(&key.data_type, &key.cloud_id, Some(record.version))
                    .await?;
            }
        }
        Ok(())
    }

    /// 处理远端墓碑；返回本轮因此新挂起冲突的记录键。
    async fn apply_remote_tombstones(
        &self,
        items: &[PersonalSyncItemSnapshot],
        records: &[CloudSyncData],
        paused: &HashSet<PersonalSyncCloudKey>,
    ) -> Result<HashSet<PersonalSyncCloudKey>, SyncStoreError> {
        let mut conflicts = HashSet::new();
        for record in records.iter().filter(|record| record.deleted_at.is_some()) {
            let key = PersonalSyncCloudKey {
                data_type: record.data_type.clone(),
                cloud_id: record.id.clone(),
            };
            // 已经挂起冲突的记录保持原样：再删本地行会让冲突里存档的
            // `local_snapshot` 指向一个不存在的本地条目，冲突就永远解不开了。
            if paused.contains(&key) {
                continue;
            }
            let Some(item) = find_local_by_cloud_key(items, &key) else {
                continue;
            };
            if local_changed_since_sync(item) {
                // 本地自上次同步之后改过 ⇒ 不能跟着远端墓碑静默丢掉用户的改动，
                // 记一条 `LocalModifiedRemoteDeleted` 交给用户决定。
                self.pause_tombstone_conflict(
                    item,
                    record,
                    PersonalConflictType::LocalModifiedRemoteDeleted,
                )
                .await?;
                conflicts.insert(key);
                continue;
            }
            match self.local.delete_item(item).await {
                Ok(()) => {}
                Err(SyncStoreError::Conflict(_)) => {
                    // 本地拒绝删除（例如凭据仍被连接引用）⇒ 同样交给用户决定。
                    self.pause_tombstone_conflict(
                        item,
                        record,
                        PersonalConflictType::LocalModifiedRemoteDeleted,
                    )
                    .await?;
                    conflicts.insert(key);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(conflicts)
    }

    async fn pause_tombstone_conflict(
        &self,
        item: &PersonalSyncItemSnapshot,
        record: &CloudSyncData,
        conflict_type: PersonalConflictType,
    ) -> Result<(), SyncStoreError> {
        let conflict = PersonalSyncRecordConflict {
            local_id: item.local_id.clone(),
            cloud_id: record.id.clone(),
            data_type: record.data_type.clone(),
            conflict_type,
        };
        self.conflicts
            .pause_record(&conflict, Some(item), Some(record))
            .await
    }

    async fn apply_plan(
        &self,
        plan: &PersonalSyncPlan,
        local_items: &[PersonalSyncItemSnapshot],
        remote_records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        self.apply_uploads(plan).await?;
        self.apply_cloud_updates(plan).await?;
        self.apply_local_updates(plan).await?;
        self.apply_downloads(plan).await?;
        self.apply_synced_marks(plan, local_items, remote_records)
            .await?;
        self.apply_conflicts(plan, local_items, remote_records)
            .await
    }

    async fn apply_uploads(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for item in &plan.to_upload {
            let record = self.local.export_item(item).await?;
            let stored = self.store.upsert_record(&record, None).await?;
            self.mark_synced(item, &stored).await?;
        }
        Ok(())
    }

    async fn apply_cloud_updates(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for (item, remote) in &plan.to_update_cloud {
            let record = self.local.export_item(item).await?;
            let stored = self
                .store
                .upsert_record(&record, Some(remote.version))
                .await?;
            self.mark_synced(item, &stored).await?;
        }
        Ok(())
    }

    async fn apply_local_updates(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for (record, item) in &plan.to_update_local {
            self.local.apply_remote(record, Some(item)).await?;
            self.mark_synced(item, record).await?;
        }
        Ok(())
    }

    async fn apply_downloads(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for record in &plan.to_download {
            self.local.apply_remote(record, None).await?;
        }
        Ok(())
    }

    async fn apply_synced_marks(
        &self,
        plan: &PersonalSyncPlan,
        items: &[PersonalSyncItemSnapshot],
        records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        for key in &plan.to_mark_synced {
            let Some(item) = find_local_by_cloud_key(items, key) else {
                continue;
            };
            let Some(record) = find_remote_by_cloud_key(records, key) else {
                continue;
            };
            self.mark_synced(item, record).await?;
        }
        Ok(())
    }

    async fn apply_conflicts(
        &self,
        plan: &PersonalSyncPlan,
        items: &[PersonalSyncItemSnapshot],
        records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        for conflict in &plan.conflicts {
            let local = find_local_by_id(items, &conflict.local_id);
            let remote = find_remote_by_cloud_key(
                records,
                &PersonalSyncCloudKey {
                    data_type: conflict.data_type.clone(),
                    cloud_id: conflict.cloud_id.clone(),
                },
            );
            self.conflicts.pause_record(conflict, local, remote).await?;
        }
        if !plan.conflicts.is_empty() {
            return Err(SyncStoreError::Conflict(format!(
                "{} personal sync conflict(s) paused",
                plan.conflicts.len()
            )));
        }
        Ok(())
    }

    async fn mark_synced(
        &self,
        item: &PersonalSyncItemSnapshot,
        record: &CloudSyncData,
    ) -> Result<(), SyncStoreError> {
        self.local
            .mark_synced(&item.local_id, &record.id, record.updated_at / 1000)
            .await
    }
}

fn find_local_by_cloud_key<'a>(
    items: &'a [PersonalSyncItemSnapshot],
    key: &PersonalSyncCloudKey,
) -> Option<&'a PersonalSyncItemSnapshot> {
    items.iter().find(|item| {
        item.data_type == key.data_type && item.cloud_id.as_deref() == Some(key.cloud_id.as_str())
    })
}

fn find_local_by_id<'a>(
    items: &'a [PersonalSyncItemSnapshot],
    local_id: &str,
) -> Option<&'a PersonalSyncItemSnapshot> {
    items.iter().find(|item| item.local_id == local_id)
}

/// 本地条目自上次同步之后是否被改过。
///
/// 与 planner 判定 `local_changed` 的口径一致：`updated_at` 比 `last_synced_at` 新。
fn local_changed_since_sync(item: &PersonalSyncItemSnapshot) -> bool {
    item.updated_at > item.last_synced_at.unwrap_or(0)
}

fn find_remote_by_cloud_key<'a>(
    records: &'a [CloudSyncData],
    key: &PersonalSyncCloudKey,
) -> Option<&'a CloudSyncData> {
    records
        .iter()
        .find(|record| record.data_type == key.data_type && record.id == key.cloud_id)
}

fn local_deleted_cloud_ids(events: &HashSet<PersonalSyncEvent>) -> HashSet<PersonalSyncCloudKey> {
    events
        .iter()
        .filter_map(|event| match event {
            PersonalSyncEvent::LocalDeleted {
                data_type,
                cloud_id,
            } => Some(PersonalSyncCloudKey {
                data_type: data_type.clone(),
                cloud_id: cloud_id.clone(),
            }),
            _ => None,
        })
        .collect()
}
