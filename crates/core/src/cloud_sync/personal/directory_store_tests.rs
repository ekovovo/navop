use crate::cloud_sync::models::data_type;
use crate::cloud_sync::personal::test_support::test_record;
use crate::cloud_sync::personal::{
    DirectorySyncStore, PersonalSyncStore, SyncDeviceId, SyncStoreError, SyncStoreHealth,
};

#[tokio::test]
async fn probe_initializes_missing_sync_package() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());

    let status = store.probe().await.expect("probe succeeds");

    assert_eq!(SyncStoreHealth::Ready, status.health);
    assert!(temp.path().join(".onetcli-sync/manifest.json").exists());
}

#[tokio::test]
async fn upsert_record_writes_and_lists_by_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let connection = test_record(
        "shared-cloud-id",
        data_type::CONNECTION,
        1,
        "connection-checksum",
    );
    let credential = test_record(
        "shared-cloud-id",
        data_type::CREDENTIAL,
        1,
        "credential-checksum",
    );

    let stored_connection = store
        .upsert_record(&connection, None)
        .await
        .expect("connection upsert succeeds");
    let stored_credential = store
        .upsert_record(&credential, None)
        .await
        .expect("credential upsert succeeds");
    let connections = store
        .list_records(Some(data_type::CONNECTION), None)
        .await
        .expect("connection list succeeds");
    let credentials = store
        .list_records(Some(data_type::CREDENTIAL), None)
        .await
        .expect("credential list succeeds");

    assert_eq!(connection.id, stored_connection.id);
    assert_eq!(credential.id, stored_credential.id);
    assert_eq!(1, connections.len());
    assert_eq!(data_type::CONNECTION, connections[0].data_type);
    assert_eq!("shared-cloud-id", connections[0].id);
    assert_eq!(1, credentials.len());
    assert_eq!(data_type::CREDENTIAL, credentials[0].data_type);
    assert_eq!("shared-cloud-id", credentials[0].id);
}

#[tokio::test]
async fn upsert_rejects_stale_expected_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let record = test_record("connection-1", data_type::CONNECTION, 3, "checksum-1");
    store
        .upsert_record(&record, None)
        .await
        .expect("seed succeeds");

    let err = store
        .upsert_record(&record, Some(2))
        .await
        .expect_err("stale write conflicts");

    assert!(matches!(err, SyncStoreError::Conflict(_)));
}

#[tokio::test]
async fn upsert_advances_version_after_expected_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");
    let first = store
        .upsert_record(&record, None)
        .await
        .expect("seed succeeds");

    let mut changed = first.clone();
    changed.checksum = "checksum-2".to_string();
    let second = store
        .upsert_record(&changed, Some(first.version))
        .await
        .expect("expected write succeeds");

    assert_eq!(first.version + 1, second.version);
    assert!(second.updated_at >= first.updated_at);
}

#[tokio::test]
async fn tombstone_marks_record_deleted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let connection = test_record(
        "shared-cloud-id",
        data_type::CONNECTION,
        1,
        "connection-checksum",
    );
    let credential = test_record(
        "shared-cloud-id",
        data_type::CREDENTIAL,
        1,
        "credential-checksum",
    );
    store
        .upsert_record(&connection, None)
        .await
        .expect("connection upsert succeeds");
    store
        .upsert_record(&credential, None)
        .await
        .expect("credential upsert succeeds");

    store
        .tombstone_record(data_type::CONNECTION, "shared-cloud-id", Some(1))
        .await
        .expect("tombstone succeeds");
    let connections = store
        .list_records(Some(data_type::CONNECTION), None)
        .await
        .expect("connection list succeeds");
    let credentials = store
        .list_records(Some(data_type::CREDENTIAL), None)
        .await
        .expect("credential list succeeds");

    assert_eq!(1, connections.len());
    assert!(connections[0].deleted_at.is_some());
    assert_eq!(1, credentials.len());
    assert!(credentials[0].deleted_at.is_none());
    assert!(
        temp.path()
            .join(".onetcli-sync/tombstones/connection/shared-cloud-id.json")
            .exists()
    );
    assert!(
        !temp
            .path()
            .join(".onetcli-sync/tombstones/credential/shared-cloud-id.json")
            .exists()
    );
}

#[tokio::test]
async fn tombstone_advances_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");
    let stored = store
        .upsert_record(&record, None)
        .await
        .expect("seed succeeds");

    store
        .tombstone_record(data_type::CONNECTION, "connection-1", Some(stored.version))
        .await
        .expect("tombstone succeeds");
    let records = store
        .list_records(Some(data_type::CONNECTION), None)
        .await
        .expect("list succeeds");

    assert_eq!(stored.version + 1, records[0].version);
    assert!(records[0].deleted_at.is_some());
}

// ---------------------------------------------------------------------------
// 同步互斥
// ---------------------------------------------------------------------------

/// 锁文件必须落在同步包之外：Git 后端用 `git add .onetcli-sync` 提交整个包，
/// 放到包里会被提交、被另一台机器拉到。
#[tokio::test]
async fn lock_file_lives_outside_the_sync_package_and_is_removed_on_release() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    let lock_path = temp.path().join(".onetcli-sync.lock");

    let lock = store
        .acquire_lock(&SyncDeviceId("first".to_string()))
        .await
        .expect("acquire succeeds");

    assert!(lock_path.exists(), "持锁期间应存在锁文件");
    assert!(
        !temp.path().join(".onetcli-sync/lock").exists(),
        "锁文件不能写进同步包"
    );

    drop(lock);
    assert!(!lock_path.exists(), "释放后锁文件应被删掉");
}

/// 同一时刻只允许一个实例同步：第二个实例直接让路，互斥结束后立刻能拿到。
#[tokio::test]
async fn second_instance_is_refused_while_the_lock_is_held() {
    let temp = tempfile::tempdir().expect("tempdir");
    let holder = DirectorySyncStore::new(temp.path().to_path_buf());
    let other = DirectorySyncStore::new(temp.path().to_path_buf());

    let lock = holder
        .acquire_lock(&SyncDeviceId("first".to_string()))
        .await
        .expect("first acquire succeeds");

    let refused = other
        .acquire_lock(&SyncDeviceId("second".to_string()))
        .await;
    assert!(matches!(refused, Err(SyncStoreError::LockTimeout)));

    drop(lock);
    other
        .acquire_lock(&SyncDeviceId("second".to_string()))
        .await
        .expect("互斥结束后应立刻能拿到锁");
}

/// 持有者卡死（超过 TTL）时锁可以被接管；接管之后原持有者再释放，
/// **不能**把后来者的锁删掉。
#[tokio::test]
async fn expired_lock_can_be_taken_over_and_the_old_holder_cannot_delete_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let holder = DirectorySyncStore::new(temp.path().to_path_buf());
    let other = DirectorySyncStore::new(temp.path().to_path_buf());
    let lock_path = temp.path().join(".onetcli-sync.lock");

    let held = holder
        .acquire_lock(&SyncDeviceId("first".to_string()))
        .await
        .expect("acquire succeeds");

    // 模拟「持有者超过 TTL 没释放」：只改获取时间，nonce 保持原样。
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lock_path).expect("lock file readable"))
            .expect("lock file is json");
    value["acquired_at"] = serde_json::json!(0);
    std::fs::write(&lock_path, serde_json::to_vec(&value).expect("json"))
        .expect("rewrite lock file");

    let stolen = other
        .acquire_lock(&SyncDeviceId("second".to_string()))
        .await
        .expect("过期锁应能被接管");

    // 原持有者这时才 drop：锁已经不是它的了，不能删。
    drop(held);
    assert!(lock_path.exists(), "被接管的锁不该被原持有者删掉");

    drop(stolen);
    assert!(!lock_path.exists(), "真正的持有者释放后才删锁文件");
}

/// 锁文件损坏时按「已过期」处理，否则一个坏文件会把同步永久卡死。
#[tokio::test]
async fn corrupt_lock_file_does_not_block_sync_forever() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    std::fs::write(temp.path().join(".onetcli-sync.lock"), b"{ not json")
        .expect("write corrupt lock file");

    let lock = store
        .acquire_lock(&SyncDeviceId("first".to_string()))
        .await
        .expect("坏锁文件必须能被接管");

    drop(lock);
    assert!(!temp.path().join(".onetcli-sync.lock").exists());
}
