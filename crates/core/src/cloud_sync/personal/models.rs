use serde::{Deserialize, Serialize};
use std::fmt;

pub const APP_ID: &str = "onetcli";
pub const PERSONAL_PROFILE_ID: &str = "personal";
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalSyncManifest {
    pub schema_version: u32,
    pub app: String,
    pub profile_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl PersonalSyncManifest {
    pub fn validate(&self) -> Result<(), SyncStoreError> {
        if self.schema_version > SUPPORTED_SCHEMA_VERSION {
            return Err(SyncStoreError::SchemaUnsupported {
                found: self.schema_version,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTombstone {
    pub id: String,
    pub data_type: String,
    pub deleted_at: i64,
    pub version: u32,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStoreError {
    NotConfigured,
    DirectoryUnavailable(String),
    SchemaUnsupported { found: u32 },
    Conflict(String),
    LockTimeout,
    GitAuthRequired,
    GitMergeConflict,
    Io(String),
    Parse(String),
    /// WebDAV 凭据被服务端拒绝（401 / 403）。
    WebdavAuthFailed,
    /// WebDAV 服务端不可达：DNS 失败、连接超时、TLS 失败等。
    WebdavUnreachable(String),
    /// 服务端返回了非预期状态码，且不属于「未认证」类。
    WebdavStatus { status: u16, message: String },
}

impl fmt::Display for SyncStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "personal sync is not configured"),
            Self::DirectoryUnavailable(message) => write!(f, "directory unavailable: {message}"),
            Self::SchemaUnsupported { found } => {
                write!(f, "unsupported personal sync schema: {found}")
            }
            Self::Conflict(message) => write!(f, "personal sync conflict: {message}"),
            Self::LockTimeout => write!(
                f,
                "another instance is syncing this profile right now; it will retry automatically"
            ),
            Self::GitAuthRequired => write!(f, "git authentication required"),
            Self::GitMergeConflict => write!(f, "git merge conflict"),
            Self::Io(message) => write!(f, "personal sync io error: {message}"),
            Self::Parse(message) => write!(f, "personal sync parse error: {message}"),
            Self::WebdavAuthFailed => write!(
                f,
                "WebDAV authentication failed: check the server URL, username and password"
            ),
            Self::WebdavUnreachable(message) => write!(f, "WebDAV server unreachable: {message}"),
            Self::WebdavStatus { status, message } => {
                write!(f, "WebDAV request failed ({status}): {message}")
            }
        }
    }
}

impl std::error::Error for SyncStoreError {}

impl From<std::io::Error> for SyncStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for SyncStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Parse(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStoreHealth {
    Ready,
    NotConfigured,
    DirectoryUnavailable,
    SchemaUnsupported,
    GitAuthRequired,
    GitMergeConflict,
    PausedAfterRepeatedFailures,
    /// WebDAV 用户名或密码被服务端拒绝。
    WebdavAuthFailed,
    /// WebDAV 服务端无法访问。
    WebdavUnreachable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStoreStatus {
    pub health: SyncStoreHealth,
    pub message: Option<String>,
}

impl SyncStoreStatus {
    pub fn ready() -> Self {
        Self {
            health: SyncStoreHealth::Ready,
            message: None,
        }
    }

    pub fn ready_with_message(message: impl Into<String>) -> Self {
        Self {
            health: SyncStoreHealth::Ready,
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncDeviceId(pub String);

/// 一次同步 pass 持有的互斥锁。
///
/// 后端分两类，释放方式也就分两种，所以这里同时保留两条路径：
///
/// - **本地目录 / Git 后端**：释放只是删掉本机的锁文件，用 [`SyncStoreLock::with_release`]
///   挂一个句柄，`Drop`（也就是 pass 结束、包括提前 `?` 返回）时自动释放。
/// - **WebDAV 后端**：远端释放必须走异步 HTTP，`Drop` 里 await 不了，所以把释放凭据
///   放在 `token` 里，由 worker 在 pass 结束后调用
///   [`PersonalSyncStore::release_lock`](super::PersonalSyncStore::release_lock)。
///
/// 进程中途崩溃时两条路径都不会执行，远端/本地锁靠 TTL 过期自愈。
pub struct SyncStoreLock {
    pub owner: SyncDeviceId,
    token: Option<String>,
    /// `Sync` 是必需的：`release_lock(&self, lock: &SyncStoreLock)` 会把 `&SyncStoreLock`
    /// 跨 `.await` 持有，`#[async_trait]` 要求这个 future 是 `Send`。
    release: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl SyncStoreLock {
    /// 没有实际互斥语义的锁：内存实现、测试替身，以及「基础设施不支持加锁」时
    /// 用来降级继续同步（宁可退化成老行为，也不要把同步整个卡死）。
    pub fn owned_by(owner: SyncDeviceId) -> Self {
        Self {
            owner,
            token: None,
            release: None,
        }
    }

    /// 同步释放（删本机锁文件）。
    pub fn with_release(
        owner: SyncDeviceId,
        release: impl FnOnce() + Send + Sync + 'static,
    ) -> Self {
        Self {
            owner,
            token: None,
            release: Some(Box::new(release)),
        }
    }

    /// 异步释放：`token` 是远端释放凭据（只有持有者能用它释放，避免把别人抢到的锁删掉）。
    pub fn with_token(owner: SyncDeviceId, token: impl Into<String>) -> Self {
        Self {
            owner,
            token: Some(token.into()),
            release: None,
        }
    }

    /// 远端释放凭据；`None` 表示这个锁不需要异步释放。
    pub(crate) fn release_token(&self) -> Option<&str> {
        self.token.as_deref()
    }
}

impl fmt::Debug for SyncStoreLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 手写 Debug：`release` 是闭包、`token` 是实现细节，都不该被打印。
        f.debug_struct("SyncStoreLock")
            .field("owner", &self.owner)
            .field("has_release", &self.release.is_some())
            .field("has_release_token", &self.token.is_some())
            .finish()
    }
}

impl Drop for SyncStoreLock {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

/// 生成一把锁的随机标识（[`SyncStoreLock::with_token`] 的释放凭据）。
///
/// 目录后端与 WebDAV 后端都要用它：释放时必须能证明「这把锁是我的」，否则 TTL
/// 过期、锁被别人接管之后，前一个持有者会把对方的互斥删掉。
pub(crate) fn new_lock_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

impl SyncStoreHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotConfigured => "not_configured",
            Self::DirectoryUnavailable => "directory_unavailable",
            Self::SchemaUnsupported => "schema_unsupported",
            Self::GitAuthRequired => "git_auth_required",
            Self::GitMergeConflict => "git_merge_conflict",
            Self::PausedAfterRepeatedFailures => "paused_after_repeated_failures",
            Self::WebdavAuthFailed => "webdav_auth_failed",
            Self::WebdavUnreachable => "webdav_unreachable",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "ready" => Self::Ready,
            "directory_unavailable" => Self::DirectoryUnavailable,
            "schema_unsupported" => Self::SchemaUnsupported,
            "git_auth_required" => Self::GitAuthRequired,
            "git_merge_conflict" => Self::GitMergeConflict,
            "paused_after_repeated_failures" => Self::PausedAfterRepeatedFailures,
            "webdav_auth_failed" => Self::WebdavAuthFailed,
            "webdav_unreachable" => Self::WebdavUnreachable,
            _ => Self::NotConfigured,
        }
    }
}
