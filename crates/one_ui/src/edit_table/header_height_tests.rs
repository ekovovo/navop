//! 列头高度与数据行高的真实布局回归。
//!
//! `state.rs` 里的 `include_str!` 契约只能保证源码写的是哪个取高函数；「列头加高一行、
//! 数据行不受牵连」只有量过真实布局才说得准，所以这里用 `debug_bounds` 直接取列头行与
//! 数据行的矩形。

use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement, Pixels, Render,
    SharedString, Styled, TestAppContext, VisualTestContext, Window, WindowOptions, div, px,
};
use gpui_component::Root;

use crate::edit_table::{
    Column, EditTable, EditTableDelegate, EditTableState, TableKeybindings,
    state::HEAD_ROW_SELECTOR, state::ROW_SELECTOR_PREFIX,
};
use crate::{
    TableDisplaySettings, init_table_display_settings, set_table_column_comment_in_header,
};

/// 固定行高，避免断言跟着组件默认值漂移。
const ROW_HEIGHT: u32 = 40;

/// 最小 delegate：2 列 × 3 行，够把列头和数据行都渲染出来。
struct HeightDelegate;

impl EditTableDelegate for HeightDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        2
    }

    fn rows_count(&self, _cx: &App) -> usize {
        3
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        let name = SharedString::from(format!("col-{col_ix}"));
        Column::new(name.clone(), name)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<EditTableState<Self>>,
    ) -> impl IntoElement {
        div().child(format!("{row_ix}-{col_ix}"))
    }
}

struct HeightHost {
    table: Entity<EditTableState<HeightDelegate>>,
}

impl Render for HeightHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(EditTable::new(&self.table))
    }
}

/// `VisualTestContext::debug_bounds` 只接受 `&'static str`，而行 selector 带行号。
fn row_selector(row_ix: usize) -> &'static str {
    format!("{ROW_SELECTOR_PREFIX}{row_ix}").leak()
}

/// 开一个渲染了真实表格的窗口。
fn open_table(cx: &mut TestAppContext) -> VisualTestContext {
    cx.update(gpui_component::init);
    cx.update(|cx| {
        init_table_display_settings(cx, TableDisplaySettings::new(ROW_HEIGHT));
        crate::edit_table::init(cx, &TableKeybindings::default());
    });

    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let table = cx.new(|cx| EditTableState::new(HeightDelegate, window, cx));
            let host = cx.new(|_| HeightHost { table });
            cx.new(|cx| Root::new(host, window, cx))
        })
        .expect("open table header height window")
    });

    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx
}

/// 量一次列头行与第一行数据行的高度。
fn measure(cx: &mut VisualTestContext) -> (Pixels, Pixels) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let head = cx
        .debug_bounds(HEAD_ROW_SELECTOR)
        .expect("table header row lays out")
        .size
        .height;
    let row = cx
        .debug_bounds(row_selector(0))
        .expect("first table row lays out")
        .size
        .height;

    assert!(head > px(0.), "列头高度不能为 0，否则断言会空跑");
    assert!(row > px(0.), "数据行高度不能为 0，否则断言会空跑");

    (head, row)
}

#[gpui::test]
fn the_header_grows_by_exactly_one_comment_row(cx: &mut TestAppContext) {
    let mut cx = open_table(cx);

    // 开关关闭：列头与数据行等高，界面和改动前一致。
    let (head_off, row_off) = measure(&mut cx);
    assert_eq!(
        head_off, row_off,
        "默认关闭时列头必须和数据行等高，不该预留注释行"
    );

    // 拨动开关：已经打开的表格要立刻重排列头，而不是等下次打开才生效。
    cx.update(|_, app| set_table_column_comment_in_header(true, app));
    let (head_on, row_on) = measure(&mut cx);
    assert_eq!(
        head_on - row_on,
        px(TableDisplaySettings::COMMENT_LINE_HEIGHT as f32),
        "列头只应比数据行多出恰好一行注释的高度"
    );
    assert_eq!(row_on, row_off, "加高列头不能把数据行一起撑高");

    // 再关掉应回到等高，保证两个方向都能刷新。
    cx.update(|_, app| set_table_column_comment_in_header(false, app));
    let (head_off_again, _) = measure(&mut cx);
    assert_eq!(
        head_off_again, head_off,
        "关闭开关后列头高度应回到原来的单行高度"
    );
}

/// 设置页拨动开关后，已经打开的表格必须自己重绘。
///
/// 这里刻意不调用 `window.refresh()`：行高与开关都是应用级全局，改它们不会通知任何
/// View，只有 `EditTableState::new` 订阅了 `TableDisplaySettings` 才可能重新布局。
/// 去掉那个订阅，本用例就会停在旧高度上失败。
#[gpui::test]
fn toggling_the_setting_repaints_an_already_open_table(cx: &mut TestAppContext) {
    let mut cx = open_table(cx);
    let (head_off, _) = measure(&mut cx);

    cx.update(|_, app| set_table_column_comment_in_header(true, app));
    cx.run_until_parked();

    let head_on = cx
        .debug_bounds(HEAD_ROW_SELECTOR)
        .expect("table header row lays out")
        .size
        .height;
    assert_eq!(
        head_on - head_off,
        px(TableDisplaySettings::COMMENT_LINE_HEIGHT as f32),
        "拨动开关后已打开的表格应立即加高列头，而不是等下次打开"
    );
}
