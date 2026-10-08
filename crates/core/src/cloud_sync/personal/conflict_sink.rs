use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::cloud_sync::CloudSyncData;
use crate::storage::now;

use super::{
    PersonalSyncCloudKey, PersonalSyncConflict, PersonalSyncConflictRepository,
    PersonalSyncConflictSink, PersonalSyncItemSnapshot, PersonalSyncRecordConflict, SyncStoreError,
};

#[derive(Clone)]
pub struct SqlitePersonalSyncConflictSink {
    backend_profile_id: String,
    conflicts: Arc<PersonalSyncConflictRepository>,
}

impl SqlitePersonalSyncConflictSink {
    pub fn new(backend_profile_id: String, conflicts: Arc<PersonalSyncConflictRepository>) -> Self {
        Self {
            backend_profile_id,
            conflicts,
        }
    }
}

#[async_trait]
impl PersonalSyncConflictSink for SqlitePersonalSyncConflictSink {
    async fn paused_record_keys(&self) -> Result<HashSet<PersonalSyncCloudKey>, SyncStoreError> {
        let conflicts = self
            .conflicts
            .list(&self.backend_profile_id)
            .map_err(|error| SyncStoreError::Io(error.to_string()))?;
        Ok(conflicts
            .into_iter()
            .map(|conflict| PersonalSyncCloudKey {
                data_type: conflict.data_type,
                cloud_id: conflict.record_id,
            })
            .collect())
    }

    async fn pause_record(
        &self,
        conflict: &PersonalSyncRecordConflict,
        local: Option<&PersonalSyncItemSnapshot>,
        remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError> {
        let stored = PersonalSyncConflict {
            backend_profile_id: self.backend_profile_id.clone(),
            record_id: conflict.cloud_id.clone(),
            data_type: conflict.data_type.clone(),
            conflict_type: conflict.conflict_type,
            local_snapshot: serialize_snapshot(local)?,
            remote_snapshot: serialize_snapshot(remote)?,
            detected_at: now(),
        };
        self.conflicts
            .upsert(&stored)
            .map_err(|error| SyncStoreError::Io(error.to_string()))
    }

    async fn forget_record(&self, data_type: &str, cloud_id: &str) -> Result<(), SyncStoreError> {
        self.conflicts
            .delete(&self.backend_profile_id, data_type, cloud_id)
            .map_err(|error| SyncStoreError::Io(error.to_string()))
    }
}

fn serialize_snapshot<T: serde::Serialize>(
    snapshot: Option<&T>,
) -> Result<Option<String>, SyncStoreError> {
    snapshot
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| SyncStoreError::Parse(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::cloud_sync::models::{CloudSyncData, data_type};
    use crate::cloud_sync::personal::test_support::test_record;
    use crate::cloud_sync::personal::{
        PersonalSyncCloudKey, PersonalSyncConflictRepository, PersonalSyncConflictSink,
        PersonalSyncItemSnapshot, PersonalSyncRecordConflict,
    };
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;

    use super::SqlitePersonalSyncConflictSink;

    #[tokio::test]
    async fn sink_pauses_and_forgets_only_the_targeted_record() {
        let repo = sqlite_conflict_repo();
        let sink =
            SqlitePersonalSyncConflictSink::new("personal".to_string(), Arc::new(repo.clone()));
        let connection_item = item("connection:22", "cloud-1", data_type::CONNECTION);
        let credential_item = item("credential:7", "cloud-1", data_type::CREDENTIAL);

        for (item, data_type) in [
            (&connection_item, data_type::CONNECTION),
            (&credential_item, data_type::CREDENTIAL),
        ] {
            sink.pause_record(
                &conflict("cloud-1", data_type),
                Some(item),
                Some(&record("cloud-1", data_type)),
            )
            .await
            .expect("conflict paused");
        }
        assert_eq!(
            2,
            sink.paused_record_keys().await.expect("paused keys").len()
        );

        // 删掉那条连接 ⇒ 只应该清掉 (connection, cloud-1) 这一条。
        sink.forget_record(data_type::CONNECTION, "cloud-1")
            .await
            .expect("conflict forgotten");

        let keys = sink.paused_record_keys().await.expect("paused keys");
        assert_eq!(1, keys.len());
        assert!(keys.contains(&PersonalSyncCloudKey {
            data_type: data_type::CREDENTIAL.to_string(),
            cloud_id: "cloud-1".to_string(),
        }));
        assert_eq!(1, repo.list("personal").expect("conflicts list").len());
    }

    /// 清掉一条记录后，另一个云端 id 的冲突不受影响。
    #[tokio::test]
    async fn sink_forget_is_scoped_to_the_cloud_id() {
        let repo = sqlite_conflict_repo();
        let sink =
            SqlitePersonalSyncConflictSink::new("personal".to_string(), Arc::new(repo.clone()));
        let item = item("connection:22", "cloud-1", data_type::CONNECTION);
        for cloud_id in ["cloud-1", "cloud-2"] {
            sink.pause_record(
                &conflict(cloud_id, data_type::CONNECTION),
                Some(&item),
                Some(&record(cloud_id, data_type::CONNECTION)),
            )
            .await
            .expect("conflict paused");
        }

        sink.forget_record(data_type::CONNECTION, "cloud-1")
            .await
            .expect("conflict forgotten");

        assert_eq!(
            vec![PersonalSyncCloudKey {
                data_type: data_type::CONNECTION.to_string(),
                cloud_id: "cloud-2".to_string(),
            }],
            sink.paused_record_keys()
                .await
                .expect("paused keys")
                .into_iter()
                .collect::<Vec<_>>()
        );
    }

    fn sqlite_conflict_repo() -> PersonalSyncConflictRepository {
        let temp = tempfile::tempdir().expect("tempdir");
        let conn = SqliteConnection::open(temp.path().join("test.db")).expect("sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("migrations run");
        PersonalSyncConflictRepository::new(conn)
    }

    fn conflict(cloud_id: &str, data_type: &str) -> PersonalSyncRecordConflict {
        PersonalSyncRecordConflict {
            local_id: format!("local-{cloud_id}"),
            cloud_id: cloud_id.to_string(),
            data_type: data_type.to_string(),
            conflict_type: crate::cloud_sync::personal::PersonalConflictType::BothModified,
        }
    }

    fn item(local_id: &str, cloud_id: &str, data_type: &str) -> PersonalSyncItemSnapshot {
        PersonalSyncItemSnapshot {
            local_id: local_id.to_string(),
            cloud_id: Some(cloud_id.to_string()),
            data_type: data_type.to_string(),
            updated_at: 300,
            last_synced_at: Some(100),
            checksum: "checksum".to_string(),
            team_id: None,
        }
    }

    fn record(cloud_id: &str, data_type: &str) -> CloudSyncData {
        test_record(cloud_id, data_type, 4, "checksum")
    }
}
