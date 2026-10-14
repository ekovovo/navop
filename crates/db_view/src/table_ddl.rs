//! 建表 DDL：先看驱动/服务端提供的权威建表文本，取不到再走表设计器的元数据重建链路。
//!
//! 悬停浮层、对象详情页签、右键「复制 DDL」三处共用这里的结果。达梦这类方言的
//! `DBMS_METADATA.GET_DDL` 能给出 IDENTITY、列尺寸、主键和 `STORAGE(...)` 私有子句，
//! 而字典视图里的 `DATA_TYPE` 只是裸类型名，按元数据重建必然丢这些；因此优先采用服务端
//! 文本。驱动没有该来源、查询失败或只回占位注释时，回退到与表设计器一致的驱动建表 SQL：
//! 引号风格、主键、索引、自增、表选项都由驱动决定。服务端文本没带注释语句时，用元数据的
//! 表/列注释补齐，避免换来源反而丢掉注释。

use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use db::GlobalDbState;
use db::plugin::DatabasePlugin;
use db::types::{ColumnInfo, IndexInfo, TableInfo};
use gpui::{AsyncApp, Task};
use one_core::gpui_tokio::Tokio;
use one_core::storage::DatabaseType;
use rust_i18n::t;
use tracing::log::warn;

use crate::table_designer_tab::{build_table_design_from_metadata, find_loaded_table_info};

/// 快照里一张表/视图的坐标：表名 + 它所在的 database/schema。
///
/// 表可能来自当前 scope，也可能来自限定名（`otherdb.users`）里的外库/外 schema，
/// 所以坐标跟着解析结果走，而不是一律当成当前 scope。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlTableRef {
    pub name: String,
    pub database: String,
    pub schema: Option<String>,
}

/// 生成 DDL 需要的连接上下文。编辑器提供它；内嵌在单元格里的编辑器没有，
/// 那种场景就不生成 DDL，而不是退回到「看起来像 DDL」的近似文本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDdlContext {
    pub connection_id: String,
    pub database_type: DatabaseType,
}

/// 一次 DDL 生成的完整目标：表坐标 + 连接上下文。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TableDdlTarget {
    pub connection_id: String,
    pub database_type: DatabaseType,
    pub name: String,
    pub database: String,
    pub schema: Option<String>,
}

impl SqlTableRef {
    /// 补上连接上下文，得到驱动查询需要的完整目标。
    pub fn ddl_target(&self, context: &TableDdlContext) -> TableDdlTarget {
        TableDdlTarget {
            connection_id: context.connection_id.clone(),
            database_type: context.database_type.clone(),
            name: self.name.clone(),
            database: self.database.clone(),
            schema: self.schema.clone(),
        }
    }
}

/// 建表 DDL 的加载器。
///
/// 生产用 [`TableDdlLoader::driven`]；测试注入桩，这样悬停/详情页签/复制 DDL
/// 的行为不连库也能验证。
#[derive(Clone)]
pub struct TableDdlLoader(Rc<dyn Fn(&mut AsyncApp, TableDdlTarget) -> Task<Result<String>>>);

impl TableDdlLoader {
    pub fn new(
        load: impl Fn(&mut AsyncApp, TableDdlTarget) -> Task<Result<String>> + 'static,
    ) -> Self {
        Self(Rc::new(load))
    }

    /// 走驱动生成（与表设计器的 DDL 预览同一条链路）。
    ///
    /// 全局状态在这里就捉住带进异步块：`AsyncApp` 没有 `global::<T>()`。
    pub fn driven(global_state: GlobalDbState) -> Self {
        Self::new(move |cx, target| {
            let global_state = global_state.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                fetch_driven_ddl(cx, &global_state, &target).await
            })
        })
    }

    pub fn load(&self, cx: &mut AsyncApp, target: TableDdlTarget) -> Task<Result<String>> {
        (self.0)(cx, target)
    }
}

/// 建表 DDL 的来源：加载器 + 编辑器所在的连接。
#[derive(Clone)]
pub struct TableDdlSources {
    loader: TableDdlLoader,
    context: TableDdlContext,
}

impl TableDdlSources {
    pub fn new(loader: TableDdlLoader, context: TableDdlContext) -> Self {
        Self { loader, context }
    }

    /// 生产来源：驱动加载器 + 当前连接的上下文。
    pub fn driven(
        global_state: GlobalDbState,
        connection_id: impl Into<String>,
        database_type: DatabaseType,
    ) -> Self {
        Self::new(
            TableDdlLoader::driven(global_state),
            TableDdlContext {
                connection_id: connection_id.into(),
                database_type,
            },
        )
    }
}

/// DDL 区段的渲染状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableDdlSection {
    /// 正在生成：详情页签先出列信息，DDL 稍后就位。
    Loading,
    Ready(String),
    Failed(String),
}

impl TableDdlSection {
    /// 分隔线 + `**DDL**` 标题 + 代码块（或加载中/失败提示）。
    pub fn markdown(&self) -> String {
        let mut md = String::from("---\n\n**DDL**\n\n");
        match self {
            Self::Loading => {
                md.push_str(t!("Query.object_details_ddl_loading").as_ref());
                md.push('\n');
            }
            Self::Ready(ddl) => {
                md.push_str("```sql\n");
                md.push_str(ddl.trim_end());
                md.push_str("\n```\n");
            }
            Self::Failed(error) => {
                md.push_str(t!("Query.object_details_ddl_failed", error = error.as_str()).as_ref());
                md.push('\n');
            }
        }
        md
    }
}

/// 主体 markdown + 可选 DDL 区段。
pub fn with_ddl_section(body: &str, section: Option<&TableDdlSection>) -> String {
    match section {
        Some(section) => format!("{}\n\n{}", body.trim_end(), section.markdown()),
        None => body.to_string(),
    }
}

/// 加载 DDL 区段。表坐标与来源齐全才有区段；列/函数/视图，或没有连接上下文的
/// 内嵌编辑器都返回 `None`。
pub async fn load_ddl_section(
    table: Option<&SqlTableRef>,
    sources: Option<&TableDdlSources>,
    cx: &mut AsyncApp,
) -> Option<TableDdlSection> {
    let table = table?;
    let sources = sources?;
    let target = table.ddl_target(&sources.context);
    Some(match sources.loader.load(cx, target).await {
        Ok(ddl) => TableDdlSection::Ready(ddl),
        Err(error) => TableDdlSection::Failed(error.to_string()),
    })
}

/// 与表设计器预览完全相同的链路：
/// 列/索引/表信息 → `TableDesign` → 驱动建表 SQL。
async fn fetch_driven_ddl(
    cx: &mut AsyncApp,
    global_state: &GlobalDbState,
    target: &TableDdlTarget,
) -> Result<String> {
    let metadata = Tokio::spawn_result(
        cx,
        load_table_metadata(global_state.clone(), target.clone()),
    )
    .await?;
    if let Some(ddl) = server_structure_ddl(target, &metadata) {
        return Ok(ddl);
    }
    let design = build_table_design_from_metadata(
        target.database_type.clone(),
        target.database.clone(),
        target.name.clone(),
        &metadata.columns,
        &metadata.indexes,
        metadata.table_info.as_ref(),
        metadata.plugin.as_deref(),
    );
    global_state
        .build_table_design_sql(
            cx,
            target.connection_id.clone(),
            target.database.clone(),
            target.schema.clone(),
            None,
            design,
            Vec::new(),
        )
        .await
}

/// 表结构元数据（列 + 索引 + 表信息 + 方言插件 + 可选的服务端建表文本）。
struct TableMetadata {
    columns: Vec<ColumnInfo>,
    indexes: Vec<IndexInfo>,
    table_info: Option<TableInfo>,
    plugin: Option<Arc<dyn DatabasePlugin>>,
    /// 驱动/服务端提供的权威建表文本；`None` 表示需要按元数据重建。
    server_ddl: Option<String>,
}

/// 有服务端文本就用它（必要时补齐注释），否则返回 `None` 走元数据重建。
fn server_structure_ddl(target: &TableDdlTarget, metadata: &TableMetadata) -> Option<String> {
    let dump = metadata.server_ddl.as_deref()?;
    let plugin = metadata.plugin.as_deref()?;
    let ddl = plugin.structure_ddl_with_comments(
        dump,
        &target.database,
        target.schema.as_deref(),
        &target.name,
        metadata
            .table_info
            .as_ref()
            .and_then(|info| info.comment.as_deref()),
        &metadata.columns,
    );
    (!ddl.is_empty()).then_some(ddl)
}

/// 与表设计器加载表结构用的是同一组查询，`list_*_direct` 要求跑在 Tokio runtime 里。
async fn load_table_metadata(
    global_state: GlobalDbState,
    target: TableDdlTarget,
) -> Result<TableMetadata> {
    let columns = global_state
        .list_columns_direct(
            &target.connection_id,
            &target.database,
            target.schema.clone(),
            &target.name,
        )
        .await?;
    let indexes = global_state
        .list_indexes_direct(
            &target.connection_id,
            &target.database,
            target.schema.clone(),
            &target.name,
        )
        .await?;
    let tables = global_state
        .list_tables_direct(
            &target.connection_id,
            &target.database,
            target.schema.clone(),
        )
        .await?;
    let plugin = global_state
        .db_manager
        .get_plugin(&target.database_type)
        .ok();
    // 服务端权威文本优先。取不到（方言不支持、权限不足、只有占位注释）不算失败，
    // 与结构导出一样回退到按元数据重建。
    let server_ddl = match global_state
        .dump_table_create_sql_direct(
            &target.connection_id,
            &target.database,
            target.schema.clone(),
            &target.name,
        )
        .await
    {
        Ok(dump) => dump,
        Err(error) => {
            warn!("表结构 DDL 服务端文本不可用，回退到按元数据重建: {error}");
            None
        }
    };
    Ok(TableMetadata {
        columns,
        indexes,
        // 表注释会进建表 DDL（MySQL 的 COMMENT=），所以按表名取回 TableInfo。
        table_info: find_loaded_table_info(tables, &target.name, target.schema.as_deref()),
        plugin,
        server_ddl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::DatabaseType;

    fn table_ref() -> SqlTableRef {
        SqlTableRef {
            name: "users".into(),
            database: "app".into(),
            schema: Some("public".into()),
        }
    }

    fn context() -> TableDdlContext {
        TableDdlContext {
            connection_id: "conn-1".into(),
            database_type: DatabaseType::MySQL,
        }
    }

    /// 桩加载器：不连库，直接把给定 DDL 当成驱动输出。
    fn stub_sources(ddl: &'static str) -> TableDdlSources {
        TableDdlSources::new(
            TableDdlLoader::new(move |_cx, _target| gpui::Task::ready(Ok(ddl.to_string()))),
            context(),
        )
    }

    /// 桩加载器：不连库，直接给出失败。
    fn failing_sources(error: &'static str) -> TableDdlSources {
        TableDdlSources::new(
            TableDdlLoader::new(move |_cx, _target| gpui::Task::ready(Err(anyhow::anyhow!(error)))),
            context(),
        )
    }

    fn column_with_comment(name: &str, comment: Option<&str>) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: "VARCHAR2".to_string(),
            is_nullable: true,
            is_primary_key: false,
            default_value: None,
            comment: comment.map(str::to_string),
            charset: None,
            collation: None,
            is_auto_increment: false,
        }
    }

    /// 引号固定成达梦口径，避免测试依赖本机驱动注册表。
    fn dump_plugin() -> Arc<dyn DatabasePlugin> {
        let driver = serde_json::from_str(
            r#"{"id":"dm","name":"dm","entry":{"command":"driver"},"transport":{"name":"dm.sock"}}"#,
        )
        .expect("minimal dm driver manifest");
        Arc::new(db::ipc::ExternalDatabasePlugin::for_driver(driver))
    }

    fn metadata(server_ddl: Option<&str>) -> TableMetadata {
        TableMetadata {
            columns: vec![
                column_with_comment("ID", None),
                column_with_comment("STAT_PERIOD", Some("统计周期")),
            ],
            indexes: Vec::new(),
            table_info: None,
            plugin: Some(dump_plugin()),
            server_ddl: server_ddl.map(str::to_string),
        }
    }

    /// 达梦字典里的 `DATA_TYPE` 只是裸类型名，IDENTITY 与主键只能靠服务端文本还原。
    #[test]
    fn server_dump_drives_the_ddl_and_gets_missing_comments_appended() {
        let target = table_ref().ddl_target(&context());
        let metadata = metadata(Some(
            "CREATE TABLE \"public\".\"users\" (\"ID\" BIGINT IDENTITY(1, 1) NOT NULL)",
        ));

        let ddl =
            server_structure_ddl(&target, &metadata).expect("server text should drive the DDL");

        assert_eq!(
            "CREATE TABLE \"public\".\"users\" (\"ID\" BIGINT IDENTITY(1, 1) NOT NULL);\n\
             COMMENT ON COLUMN \"public\".\"users\".\"STAT_PERIOD\" IS '统计周期';",
            ddl
        );
    }

    /// 服务端文本自带注释时不重复追加，否则同一列会出现两条 `COMMENT ON`。
    #[test]
    fn server_dump_that_already_carries_comments_is_left_as_is() {
        let target = table_ref().ddl_target(&context());
        let dump = "CREATE TABLE t (\n    id INT\n)\nCOMMENT ON TABLE t IS '县区局评价数据'";
        let metadata = metadata(Some(dump));

        let ddl =
            server_structure_ddl(&target, &metadata).expect("server text should drive the DDL");

        assert_eq!(format!("{dump};"), ddl);
    }

    /// 没有服务端来源时必须继续走元数据重建。
    #[test]
    fn metadata_rebuild_runs_without_server_dump_or_plugin() {
        let target = table_ref().ddl_target(&context());

        assert_eq!(None, server_structure_ddl(&target, &metadata(None)));

        let mut without_plugin = metadata(Some("CREATE TABLE t (id INT)"));
        without_plugin.plugin = None;
        assert_eq!(None, server_structure_ddl(&target, &without_plugin));
    }

    /// 接线契约：`fetch_driven_ddl` 必须在按元数据建 design 之前先问驱动要服务端文本，
    /// 并且元数据仍然要加载（注释补齐与回退都依赖它）。
    #[test]
    fn the_ddl_view_asks_the_driver_before_rebuilding_from_metadata() {
        let source = include_str!("table_ddl.rs");
        let fetch_body = source
            .split("async fn fetch_driven_ddl(")
            .nth(1)
            .expect("fetch_driven_ddl")
            .split("/// 表结构元数据")
            .next()
            .expect("fetch_driven_ddl body");
        let dump_at = fetch_body
            .find("server_structure_ddl(")
            .expect("DDL 查看必须优先采用驱动的服务端文本");
        let design_at = fetch_body
            .find("build_table_design_from_metadata(")
            .expect("元数据重建的回退链路要保留");
        assert!(dump_at < design_at, "服务端文本命中时不该再按元数据重建");

        let loader_body = source
            .split("async fn load_table_metadata(")
            .nth(1)
            .expect("load_table_metadata")
            .split("#[cfg(test)]")
            .next()
            .expect("load_table_metadata body");
        assert!(
            loader_body.contains("dump_table_create_sql_direct("),
            "服务端文本要在 Tokio 元数据加载里取"
        );
        assert!(
            loader_body.contains("list_columns_direct("),
            "列元数据要继续加载，用于注释补齐与回退"
        );
    }

    #[test]
    fn ddl_target_carries_the_connection_and_the_table_coordinates() {
        let target = table_ref().ddl_target(&context());

        assert_eq!(
            TableDdlTarget {
                connection_id: "conn-1".into(),
                database_type: DatabaseType::MySQL,
                name: "users".into(),
                database: "app".into(),
                schema: Some("public".into()),
            },
            target
        );
    }

    #[test]
    fn a_ready_section_renders_a_sql_fence() {
        let md =
            TableDdlSection::Ready("CREATE TABLE `users` (\n  `id` INT\n);\n".into()).markdown();

        assert!(md.starts_with("---\n\n**DDL**\n\n"));
        assert!(md.contains("```sql\nCREATE TABLE `users` (\n  `id` INT\n);\n```"));
    }

    #[test]
    fn a_loading_section_says_it_is_generating() {
        let md = TableDdlSection::Loading.markdown();

        assert!(md.contains(&t!("Query.object_details_ddl_loading").to_string()));
        assert!(!md.contains("```sql"), "加载中不该先摆一个空代码块");
    }

    #[test]
    fn a_failed_section_carries_the_error() {
        let md = TableDdlSection::Failed("connection closed".into()).markdown();

        assert!(
            md.contains(
                &t!(
                    "Query.object_details_ddl_failed",
                    error = "connection closed"
                )
                .to_string()
            )
        );
    }

    #[test]
    fn a_missing_section_leaves_the_body_untouched() {
        assert_eq!(
            "**TABLE** `users`",
            with_ddl_section("**TABLE** `users`", None)
        );
        assert!(
            with_ddl_section("**TABLE** `users`", Some(&TableDdlSection::Loading))
                .starts_with("**TABLE** `users`\n\n---")
        );
    }

    #[gpui::test]
    async fn there_is_no_ddl_section_without_a_table_or_a_source(cx: &mut gpui::TestAppContext) {
        let sources = stub_sources("CREATE TABLE `users` (`id` INT);");
        let table = table_ref();
        let mut cx = cx.to_async();

        assert_eq!(None, load_ddl_section(None, Some(&sources), &mut cx).await);
        assert_eq!(None, load_ddl_section(Some(&table), None, &mut cx).await);
    }

    #[gpui::test]
    async fn the_loader_output_becomes_the_ready_section(cx: &mut gpui::TestAppContext) {
        const DDL: &str = "CREATE TABLE `users` (`id` INT);";
        let sources = stub_sources(DDL);
        let table = table_ref();
        let mut cx = cx.to_async();

        assert_eq!(
            Some(TableDdlSection::Ready(DDL.into())),
            load_ddl_section(Some(&table), Some(&sources), &mut cx).await
        );
    }

    #[gpui::test]
    async fn a_failing_loader_becomes_the_failed_section(cx: &mut gpui::TestAppContext) {
        let sources = failing_sources("connection closed");
        let table = table_ref();
        let mut cx = cx.to_async();

        assert_eq!(
            Some(TableDdlSection::Failed("connection closed".into())),
            load_ddl_section(Some(&table), Some(&sources), &mut cx).await
        );
    }
}
