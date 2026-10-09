use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Context, IntoElement, ParentElement, Render, Styled, Window, div, px,
    uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable, Size,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    h_flex,
    popover::Popover,
    scroll::ScrollableElement,
    v_flex,
};
use one_assets::IconName;
use one_ui::IconButton;
use rust_i18n::t;
use std::ops::Range;

use super::execution_history::{ExecutionRecord, ExecutionStatus};
use super::execution_history_panel::{ExecutionHistoryFilter, ExecutionHistoryPanel};

/// 记录触发行（一条记录占的可点区域）的高度。
const EXECUTION_RECORD_BUTTON_HEIGHT: f32 = 88.;
/// 记录之间的间距。
const EXECUTION_RECORD_ROW_GAP: f32 = 8.;
/// `uniform_list` 的固定行高。
///
/// 它要求所有行等高，又不支持 item 间距，所以记录之间的间隔只能并进行高里；
/// 行高与实际按钮高度对不上会把列表压成一条（issue #368）。
const EXECUTION_RECORD_ROW_HEIGHT: f32 = EXECUTION_RECORD_BUTTON_HEIGHT + EXECUTION_RECORD_ROW_GAP;

/// 可见记录在 `history.records()` 里的原始下标，按「新 → 旧」排列。
///
/// 返回下标而不是记录本身：列表只渲染可见的十来行，回查远比克隆便宜 ——
/// `ExecutionRecord` 的 `sql` / `details` 都是字符串，旧实现每条都
/// `record.clone()`、还全量建元素，两者一起构成了 issue #368 里「每敲一个字
/// 卡一下」的开销。
pub(super) fn visible_record_indices(
    records: &[ExecutionRecord],
    filter: ExecutionHistoryFilter,
) -> Vec<usize> {
    records
        .iter()
        .enumerate()
        .filter(|(_, record)| filter.matches(record))
        .map(|(index, _)| index)
        .rev()
        .collect()
}

impl ExecutionHistoryPanel {
    /// 渲染一条记录。
    ///
    /// 返回 `AnyElement` 而不是 `impl IntoElement`：`uniform_list` 的渲染回调
    /// 只能返回一个固定类型，带生命周期的 opaque 类型会让每个可见行各自
    /// 实例化一个 `R`，编译不过。
    fn render_record(
        index: usize,
        record: &ExecutionRecord,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (status_color, status_icon) = match record.status {
            ExecutionStatus::Success => (cx.theme().success, IconName::CircleCheck),
            ExecutionStatus::Error => (cx.theme().danger, IconName::TriangleAlert),
        };
        let sql_preview = record
            .sql
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        let popover_record = record.clone();

        let popover = Popover::new(("database-execution-history-record", index))
            .trigger(
                Button::new(("database-execution-history-trigger", index))
                    .ghost()
                    .w_full()
                    .h(px(EXECUTION_RECORD_BUTTON_HEIGHT))
                    .p_2()
                    .child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        Icon::new(status_icon)
                                            .with_size(Size::XSmall)
                                            .text_color(status_color),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .text_color(status_color)
                                            .text_ellipsis()
                                            .child(record.summary.clone()),
                                    ),
                            )
                            .when(!sql_preview.is_empty(), |this| {
                                this.child(
                                    div()
                                        .pl_5()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .text_ellipsis()
                                        .child(sql_preview),
                                )
                            })
                            .child(Self::render_metadata(record, cx)),
                    ),
            )
            .content(move |_state, _window, cx| Self::render_details(index, &popover_record, cx))
            .max_w(px(680.));

        // 固定行高是 `uniform_list` 虚拟化的前提：行间距并进来，再加按钮就撑满了。
        div()
            .h(px(EXECUTION_RECORD_ROW_HEIGHT))
            .child(popover)
            .into_any_element()
    }

    fn render_metadata(record: &ExecutionRecord, cx: &App) -> AnyElement {
        let scope = record
            .context
            .database
            .iter()
            .chain(record.context.schema.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(".");

        h_flex()
            .pl_5()
            .gap_3()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .when(!scope.is_empty(), |this| this.child(scope))
            .when_some(record.returned_rows, |this, count| {
                this.child(t!("DatabaseSidebar.returned_rows", count = count).to_string())
            })
            .when(
                record.returned_rows.is_none()
                    && (record.status == ExecutionStatus::Success || record.affected_rows > 0),
                |this| {
                    this.child(
                        t!(
                            "DatabaseSidebar.affected_rows",
                            count = record.affected_rows
                        )
                        .to_string(),
                    )
                },
            )
            .child(
                t!(
                    "DatabaseSidebar.execution_time",
                    duration = record.elapsed_ms
                )
                .to_string(),
            )
            .into_any_element()
    }

    fn render_details(index: usize, record: &ExecutionRecord, cx: &App) -> AnyElement {
        let details = record.details.join("\n");
        let sql = record.sql.clone();
        let (status_color, status_icon) = match record.status {
            ExecutionStatus::Success => (cx.theme().success, IconName::CircleCheck),
            ExecutionStatus::Error => (cx.theme().danger, IconName::TriangleAlert),
        };

        v_flex()
            .w(px(620.))
            .h(px(460.))
            .gap_2()
            .p_3()
            .overflow_y_scrollbar()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(status_icon)
                            .with_size(Size::Small)
                            .text_color(status_color),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(status_color)
                            .child(record.summary.clone()),
                    ),
            )
            .child(Self::render_metadata(record, cx))
            .when(!details.is_empty(), |this| {
                this.child(Self::render_code_block(
                    t!("DatabaseSidebar.server_result").to_string(),
                    details.clone(),
                    Clipboard::new(("database-execution-history-copy-result", index))
                        .value(details),
                    cx,
                ))
            })
            .when(!sql.is_empty(), |this| {
                this.child(Self::render_code_block(
                    t!("DatabaseSidebar.executed_sql").to_string(),
                    sql.clone(),
                    Clipboard::new(("database-execution-history-copy-sql", index)).value(sql),
                    cx,
                ))
            })
            .into_any_element()
    }

    fn render_code_block(
        label: String,
        content: String,
        action: Clipboard,
        cx: &App,
    ) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(action),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .max_h(px(220.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted.opacity(0.14))
                    .p_2()
                    .overflow_scrollbar()
                    .child(
                        div()
                            .min_w_full()
                            .flex_shrink_0()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_xs()
                            .child(content),
                    ),
            )
    }

    fn render_filter_button(
        &self,
        filter: ExecutionHistoryFilter,
        label: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(("database-execution-history-filter", filter as usize))
            .ghost()
            .with_size(Size::XSmall)
            .when(self.filter == filter, |this| this.primary())
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter, cx)))
    }
}

impl Render for ExecutionHistoryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.history.records().len();
        let success_count = self
            .history
            .records()
            .iter()
            .filter(|record| record.status == ExecutionStatus::Success)
            .count();
        let error_count = count - success_count;
        // 只算「可见记录在 records() 里的原始下标」，交给 `uniform_list` 按需渲染。
        // 旧实现把最多 1000 条记录（每条一个 `Popover`）全量铺进 element 树，
        // 打开面板后 SQL 编辑器每敲一个字都要重建上万个元素（issue #368）。
        let visible_indices = visible_record_indices(self.history.records(), self.filter);
        let visible_count = visible_indices.len();
        let list = if visible_count == 0 {
            div()
                .p_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("DatabaseSidebar.no_execution_history").to_string())
                .into_any_element()
        } else {
            uniform_list(
                "database-execution-history-list",
                visible_count,
                cx.processor(
                    move |panel: &mut Self, range: Range<usize>, _window, cx| {
                        range
                            .filter_map(|row_ix| {
                                let record_ix = visible_indices.get(row_ix).copied()?;
                                let record = panel.history.records().get(record_ix)?;
                                Some(Self::render_record(record_ix, record, cx))
                            })
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .size_full()
            .into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(40.))
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("DatabaseSidebar.execution_history").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                t!("DatabaseSidebar.execution_history_count", count = count)
                                    .to_string(),
                            ),
                    )
                    .child(div().flex_1())
                    .when(count > 0, |this| {
                        this.child(
                            IconButton::new("clear-database-execution-history", IconName::Delete)
                                .tooltip(t!("DatabaseSidebar.clear_execution_history").to_string())
                                .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                        )
                    }),
            )
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.render_filter_button(
                        ExecutionHistoryFilter::All,
                        t!("DatabaseSidebar.filter_all", count = count).to_string(),
                        cx,
                    ))
                    .child(self.render_filter_button(
                        ExecutionHistoryFilter::Success,
                        t!("DatabaseSidebar.filter_success", count = success_count).to_string(),
                        cx,
                    ))
                    .child(self.render_filter_button(
                        ExecutionHistoryFilter::Error,
                        t!("DatabaseSidebar.filter_failed", count = error_count).to_string(),
                        cx,
                    )),
            )
            .child(
                div().flex_1().h_full().min_h_0().overflow_hidden().child(
                    // 保留原来的 8px 内边距；滚动交给 `uniform_list` 自己。
                    div().size_full().min_h_0().min_w_0().p_2().child(list),
                ),
            )
    }
}

#[cfg(test)]
#[path = "execution_history_view_tests.rs"]
mod tests;
