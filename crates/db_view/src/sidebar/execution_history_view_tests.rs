//! 「执行记录」面板渲染的契约测试
//!
//! 面板原先把最多 `MAX_EXECUTION_RECORDS`(1000) 条记录全量铺进 element 树：
//! 每条一个 `Popover` + 一个 `Button`，约两万个元素。于是打开「执行记录」后
//! SQL 编辑器每敲一个字都要重建它们，表现为「每打一个字卡一下」，清空记录后
//! 立刻消失（issue #368）。这里锁住三件不能漂移的事：
//!
//! 1. 列表走 `uniform_list` 虚拟化，行高是固定常量（它要求所有行等高，
//!    且不支持 item 间距）；
//! 2. 渲染回调只按原始下标回查记录，不再为整个列表克隆记录；
//! 3. 「过滤 + 倒序」的下标映射由纯函数承载，可以单独测。

use super::super::execution_history::ExecutionContext;
use super::*;

fn view_source() -> &'static str {
    include_str!("execution_history_view.rs")
}

/// 取 `start` 到 `end` 之间的源码片段。
fn slice_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source
        .find(start)
        .unwrap_or_else(|| panic!("找不到起点 `{start}`"));
    let body = &source[start..];
    let end = body
        .find(end)
        .unwrap_or_else(|| panic!("找不到终点 `{end}`"));
    &body[..end]
}

/// `Render` 实现体：从实现头到文件末尾的测试模块挂载点。
fn render_body() -> &'static str {
    slice_between(
        view_source(),
        "impl Render for ExecutionHistoryPanel",
        "#[cfg(test)]",
    )
}

fn record(status: ExecutionStatus) -> ExecutionRecord {
    ExecutionRecord {
        context: ExecutionContext {
            connection_id: "1".to_string(),
            database: None,
            schema: None,
        },
        status,
        sql: "select 1".to_string(),
        summary: "ok".to_string(),
        details: Vec::new(),
        affected_rows: 0,
        returned_rows: None,
        elapsed_ms: 1,
        executed_at: 0,
    }
}

#[test]
fn visible_indices_are_filtered_but_keep_their_original_position() {
    let records = vec![
        record(ExecutionStatus::Success), // 下标 0，最旧
        record(ExecutionStatus::Error),   // 下标 1
        record(ExecutionStatus::Success), // 下标 2，最新
    ];

    // 全量：新 → 旧。
    assert_eq!(
        vec![2, 1, 0],
        visible_record_indices(&records, ExecutionHistoryFilter::All)
    );
    // 过滤后仍然是「原始下标」，不是重新编号后的位置 —— 渲染层靠它回查记录，
    // 一旦压成 0/1 就会把别的记录画到这一行上。
    assert_eq!(
        vec![2, 0],
        visible_record_indices(&records, ExecutionHistoryFilter::Success)
    );
    assert_eq!(
        vec![1],
        visible_record_indices(&records, ExecutionHistoryFilter::Error)
    );
}

#[test]
fn the_record_list_is_virtualized_instead_of_fully_expanded() {
    let body = render_body();

    assert!(body.contains("uniform_list("));
    assert!(body.contains("\"database-execution-history-list\""));
    assert!(body.contains("visible_record_indices(self.history.records(), self.filter)"));
    // 旧写法（整份记录先建成 element 再塞进 v_flex）不能再回来。
    assert!(!body.contains(".children(records)"));
    assert!(!body.contains("collect::<Vec<AnyElement>>()"));
}

#[test]
fn the_render_callback_looks_records_up_by_index_instead_of_cloning_the_list() {
    let body = render_body();

    assert!(body.contains("visible_indices.get(row_ix)"));
    assert!(body.contains("panel.history.records().get(record_ix)"));
}

#[test]
fn the_row_height_is_a_fixed_constant_covering_the_gap() {
    let source = view_source();

    // `uniform_list` 要求所有行等高，行高就必须是常量，而不是交给父容器用
    // 「按钮 + gap」排出来。
    assert!(source.contains("const EXECUTION_RECORD_ROW_HEIGHT: f32 ="));
    assert!(source.contains("EXECUTION_RECORD_BUTTON_HEIGHT + EXECUTION_RECORD_ROW_GAP"));

    let render_record = slice_between(source, "fn render_record(", "fn render_metadata(");
    assert!(render_record.contains(".h(px(EXECUTION_RECORD_ROW_HEIGHT))"));
    assert!(render_record.contains(".h(px(EXECUTION_RECORD_BUTTON_HEIGHT))"));
    assert!(!render_record.contains(".h(px(88.))"));
}
