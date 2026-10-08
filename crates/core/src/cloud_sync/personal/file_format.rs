use std::path::{Path, PathBuf};

pub const SYNC_PACKAGE_DIR: &str = ".onetcli-sync";
/// 同步互斥锁文件名。
///
/// **刻意放在包目录之外**：Git 后端用 `git add .onetcli-sync` 提交整个包，锁文件落
/// 在包里就会被提交、被另一台机器拉到，既产生噪声又会让对方误以为锁被持有。
/// 放在 `<root>/.onetcli-sync.lock` 既能被「目录后端」的共享目录带着走（跨机器仍然
/// 是同一个文件），又不会被 Git 后端纳入版本管理。
pub const SYNC_LOCK_FILE: &str = ".onetcli-sync.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPackageLayout {
    root: PathBuf,
}

impl SyncPackageLayout {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.package_dir().join("manifest.json")
    }

    pub fn record_path(&self, data_type: &str, id: &str) -> PathBuf {
        self.records_dir()
            .join(data_type)
            .join(format!("{id}.json"))
    }

    pub fn tombstone_path(&self, data_type: &str, id: &str) -> PathBuf {
        self.tombstones_dir()
            .join(data_type)
            .join(format!("{id}.json"))
    }

    pub fn records_dir(&self) -> PathBuf {
        self.package_dir().join("records")
    }

    pub fn tombstones_dir(&self) -> PathBuf {
        self.package_dir().join("tombstones")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.package_dir().join("state")
    }

    /// 同步根目录（`package_dir()` 的父目录）。锁文件就放在这里。
    pub fn root_dir(&self) -> PathBuf {
        self.root.clone()
    }

    /// 同步互斥锁文件路径（见 [`SYNC_LOCK_FILE`] 的说明：在包目录之外）。
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(SYNC_LOCK_FILE)
    }

    pub fn package_dir(&self) -> PathBuf {
        self.root.join(Path::new(SYNC_PACKAGE_DIR))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::cloud_sync::models::data_type;

    use crate::cloud_sync::personal::{
        APP_ID, PERSONAL_PROFILE_ID, PersonalSyncManifest, SUPPORTED_SCHEMA_VERSION,
        SyncPackageLayout, SyncStoreError, SyncTombstone,
    };

    #[test]
    fn layout_builds_expected_paths() {
        let layout = SyncPackageLayout::new(PathBuf::from("/sync-root"));

        assert_eq!(
            Path::new("/sync-root/.onetcli-sync/manifest.json"),
            layout.manifest_path()
        );
        assert_eq!(
            Path::new("/sync-root/.onetcli-sync/records/connection/record-1.json"),
            layout.record_path("connection", "record-1")
        );
        assert_eq!(
            Path::new("/sync-root/.onetcli-sync/tombstones/connection/record-1.json"),
            layout.tombstone_path("connection", "record-1")
        );
        // 锁文件必须落在包目录之外，否则 Git 后端会把它 `git add .onetcli-sync` 提交上去。
        assert_eq!(
            Path::new("/sync-root/.onetcli-sync.lock"),
            layout.lock_path()
        );
        assert!(
            !layout.lock_path().starts_with(layout.package_dir()),
            "锁文件不能位于同步包内"
        );
    }

    #[test]
    fn manifest_rejects_newer_schema() {
        let manifest = PersonalSyncManifest {
            schema_version: SUPPORTED_SCHEMA_VERSION + 1,
            app: APP_ID.to_string(),
            profile_id: PERSONAL_PROFILE_ID.to_string(),
            created_at: 10,
            updated_at: 20,
        };

        assert_eq!(
            Err(SyncStoreError::SchemaUnsupported {
                found: SUPPORTED_SCHEMA_VERSION + 1
            }),
            manifest.validate()
        );
    }

    #[test]
    fn tombstone_round_trip_preserves_delete_metadata() {
        let tombstone = SyncTombstone {
            id: "record-1".to_string(),
            data_type: data_type::CONNECTION.to_string(),
            deleted_at: 1000,
            version: 4,
            checksum: "abc".to_string(),
        };

        let json = serde_json::to_string(&tombstone).expect("tombstone serializes");
        let parsed: SyncTombstone = serde_json::from_str(&json).expect("tombstone parses");

        assert_eq!(tombstone, parsed);
    }
}
