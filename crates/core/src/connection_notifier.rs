use crate::storage::StoredConnection;
use gpui::{App, AppContext, Context, Entity, EventEmitter};

/// 连接数据变更事件
#[derive(Debug, Clone)]
pub enum ConnectionDataEvent {
    /// 连接被创建
    ConnectionCreated { connection: StoredConnection },
    /// 连接被更新（名称、配置等）
    ConnectionUpdated { connection: StoredConnection },
    /// 连接被删除
    ///
    /// `last_synced_at` 是**删除前**那一行记录的同步基线（远端 `updated_at` 的秒值）。
    /// 个人同步要靠它判断「远端是否在本地删除之后又被别的设备改过」——那时直接推
    /// 墓碑会抹掉对方的改动，改为挂起 `LocalDeletedRemoteModified` 交给用户决定。
    /// 拿不到基线时传 `None`（保持「删除胜出」的既有行为）。
    ConnectionDeleted {
        connection_id: i64,
        cloud_id: Option<String>,
        last_synced_at: Option<i64>,
    },
    /// 工作区被创建
    WorkspaceCreated { workspace_id: i64 },
    /// 工作区被更新
    WorkspaceUpdated { workspace_id: i64 },
    /// 工作区被删除（`last_synced_at` 语义同 [`Self::ConnectionDeleted`]）
    WorkspaceDeleted {
        workspace_id: i64,
        cloud_id: Option<String>,
        last_synced_at: Option<i64>,
    },
    /// 钥匙串条目被创建。事件不得携带任何秘密字段。
    CredentialCreated { credential_id: i64 },
    /// 钥匙串条目被更新。事件不得携带任何秘密字段。
    CredentialUpdated { credential_id: i64 },
    /// 钥匙串条目被删除（`last_synced_at` 语义同 [`Self::ConnectionDeleted`]）。
    CredentialDeleted {
        credential_id: i64,
        cloud_id: Option<String>,
        last_synced_at: Option<i64>,
    },
    /// Schema 结构变更（DDL 执行后触发）
    SchemaChanged {
        connection_id: String,
        database: String,
        schema: Option<String>,
    },
    /// 请求执行一次云同步（由表单等非首页入口触发）
    CloudSyncRequested,
    /// 团队缓存已刷新，打开的表单应重新读取团队选项
    TeamCacheUpdated,
}

/// 全局连接数据通知器
pub struct ConnectionDataNotifier;

impl EventEmitter<ConnectionDataEvent> for ConnectionDataNotifier {}

/// 全局包装器，存储 Entity<ConnectionDataNotifier>
#[derive(Clone)]
pub struct GlobalConnectionNotifier(pub Entity<ConnectionDataNotifier>);

impl gpui::Global for GlobalConnectionNotifier {}

/// 初始化全局通知器
pub fn init(cx: &mut App) {
    let notifier = cx.new(|_| ConnectionDataNotifier);
    cx.set_global(GlobalConnectionNotifier(notifier));
}

/// 获取全局通知器 Entity
pub fn get_notifier(cx: &App) -> Option<Entity<ConnectionDataNotifier>> {
    cx.try_global::<GlobalConnectionNotifier>()
        .map(|g| g.0.clone())
}

/// 辅助函数：发送连接事件
pub fn emit_connection_event<T>(event: ConnectionDataEvent, cx: &mut Context<T>) {
    if let Some(notifier) = cx.try_global::<GlobalConnectionNotifier>().cloned() {
        notifier.0.update(cx, |_, cx| {
            cx.emit(event);
        });
    }
}

/// 从 `App` 上下文发送全局数据事件。
pub fn emit_connection_event_from_app(event: ConnectionDataEvent, cx: &mut App) {
    if let Some(notifier) = cx.try_global::<GlobalConnectionNotifier>().cloned() {
        notifier.0.update(cx, |_, cx| {
            cx.emit(event);
        });
    }
}
