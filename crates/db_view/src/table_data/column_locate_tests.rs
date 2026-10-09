//! 「定位字段」面板的契约测试
//!
//! 表数据页的字段一多（几十上百列），横向滚到目标列全靠肉眼。工具栏在
//! 「字段过滤」右边加了「定位字段」入口：列出全部字段，点一个就把表格带到
//! 那一列。这里锁住几件不能漂移的事：
//!
//! 1. 原始列索引 ↔ 显示位序 ↔ 网格列号的换算口径与纵向视图共用同一份；
//! 2. 面板尺寸与「字段过滤」一致，且入口在工具栏上紧贴字段过滤右侧；
//! 3. 搜索框在 `new()` 里创建，面板重建不丢光标；
//! 4. 隐藏字段照列，点了先取消隐藏再定位；
//! 5. 定位后面板收起；高亮只走表格自己的「定位列」标记，网格只染表头一格、纵向只染
//!    字段名称一格——不给整列/整行逐格染色，因为那份额外绘制量每帧都要付。

use super::data_grid::{
    COLUMN_LOCATE_PANEL_MAX_HEIGHT, COLUMN_LOCATE_PANEL_WIDTH, COLUMN_LOCATE_ROW_HEIGHT,
    COLUMN_VISIBILITY_PANEL_MAX_HEIGHT, COLUMN_VISIBILITY_PANEL_WIDTH,
    COLUMN_VISIBILITY_ROW_HEIGHT, VerticalLine, display_position_of_original_column,
    grid_column_of_display_col, vertical_line_at, vertical_line_of_first_field,
};
use rust_i18n::t;

fn data_grid_source() -> &'static str {
    include_str!("data_grid.rs")
}

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

fn panel_source() -> &'static str {
    slice_between(
        data_grid_source(),
        "fn build_column_locate_panel(",
        "/// 构建「显示方式」菜单",
    )
}

fn list_source() -> &'static str {
    slice_between(
        data_grid_source(),
        "fn render_column_locate_list(",
        "/// 构建「字段定位」面板",
    )
}

fn locate_source() -> &'static str {
    slice_between(
        data_grid_source(),
        "fn locate_column(",
        "\n    fn execution_context(",
    )
}

fn table_state_source() -> &'static str {
    include_str!("../../../one_ui/src/edit_table/state.rs")
}

/// 单元格绘制（表头行 `row_ix == None` 与数据行共用）。
fn render_cell_source() -> &'static str {
    slice_between(
        table_state_source(),
        "fn render_cell(",
        "fn render_interactive_cell(",
    )
}

/// 整列染色的那条路径：只有真正的「选中列」才准走这里。
fn render_col_wrap_source() -> &'static str {
    slice_between(
        table_state_source(),
        "fn render_col_wrap(",
        "fn render_resize_handle(",
    )
}

fn set_located_col_source() -> &'static str {
    slice_between(
        table_state_source(),
        "pub fn set_located_col(",
        "    pub fn set_selected_col(",
    )
}

fn vertical_line_source() -> &'static str {
    slice_between(
        data_grid_source(),
        "fn render_vertical_line(",
        "impl DataGrid {",
    )
}

#[test]
fn a_hidden_field_has_no_display_position() {
    // 表格只渲染可见列，所以「显示位序」要在可见列里数，而不是原始下标。
    assert_eq!(Some(0), display_position_of_original_column(&[0, 1, 2], 0));
    assert_eq!(Some(2), display_position_of_original_column(&[0, 1, 2], 2));

    // 被「字段过滤」隐藏的列压根不在列组里：返回 None，由调用方先取消隐藏。
    assert_eq!(None, display_position_of_original_column(&[0, 2], 1));
    assert_eq!(
        Some(1),
        display_position_of_original_column(&[0, 2], 2),
        "隐藏列会让后面的列整体左移一位，这里数的是可见列中的位次"
    );
}

#[test]
fn the_first_record_field_line_is_one_past_the_record_header() {
    // 每条记录先占一行记录头，再跟 column_count 行字段。
    // 正向映射（`vertical_line_at`）已经锁死，这里钉住反向映射与它一致。
    for column_count in [1usize, 5, 40] {
        for display_col in 0..column_count {
            let line = vertical_line_of_first_field(display_col);
            assert_eq!(
                Some(VerticalLine::Field {
                    row: 0,
                    col: display_col
                }),
                vertical_line_at(line, column_count),
                "第 0 条记录的第 {display_col} 个字段必须落在第 {line} 行"
            );
        }
    }
}

#[test]
fn the_locate_button_sits_right_of_the_field_filter() {
    let toolbar = slice_between(
        data_grid_source(),
        "pub fn render_toolbar(",
        "\n    pub fn render_table_area(",
    );

    // 相邻右边：两个入口管的是同一批字段，隔开了就找不到。
    let filter_at = toolbar
        .find("self.render_column_visibility_button(cx)")
        .expect("工具栏要有字段过滤入口");
    let locate_at = toolbar
        .find("self.render_column_locate_button(cx)")
        .expect("工具栏要有字段定位入口");
    assert!(
        locate_at > filter_at,
        "定位字段必须排在字段过滤右边，不能跑到它左边"
    );
    assert!(toolbar.contains(".child(self.render_column_visibility_button(cx))\n            .child(self.render_column_locate_button(cx))"));
}

#[test]
fn the_locate_panel_is_a_bounded_popover_the_same_size_as_the_filter_one() {
    let grid = data_grid_source();
    let button = slice_between(
        grid,
        "fn render_column_locate_button(",
        "\n    /// 工具栏里的查找框",
    );

    assert!(button.contains("Popover::new(\"column-locate\")"));
    assert!(button.contains("build_column_locate_panel("));
    assert!(!button.contains("dropdown_menu("));
    assert!(button.contains(".w(COLUMN_LOCATE_PANEL_WIDTH)"));

    // 两个面板紧挨着弹出，尺寸不一致会一眼看出是两套东西。
    assert_eq!(COLUMN_VISIBILITY_PANEL_WIDTH, COLUMN_LOCATE_PANEL_WIDTH);
    assert_eq!(
        COLUMN_VISIBILITY_PANEL_MAX_HEIGHT,
        COLUMN_LOCATE_PANEL_MAX_HEIGHT
    );
    assert_eq!(COLUMN_VISIBILITY_ROW_HEIGHT, COLUMN_LOCATE_ROW_HEIGHT);
}

#[test]
fn the_locate_panel_keeps_a_deterministic_scrollable_list() {
    let list = list_source();

    // 与字段过滤同款：确定高度（`h`）而不是上限（`max_h`），否则滚动条永不出现。
    assert!(list.contains(".h(list_height)"));
    assert!(list.contains(".min(COLUMN_LOCATE_PANEL_MAX_HEIGHT)"));
    assert!(list.contains(".overflow_y_scroll()"));
    assert!(list.contains(".track_scroll("));
    assert!(list.contains(".h(COLUMN_LOCATE_ROW_HEIGHT)"));

    let panel = panel_source();
    // 滚动条是绝对定位的，放进滚动容器会跟着内容跑。
    assert!(panel.contains(".relative()"));
    assert!(panel.contains("div().absolute().inset_0()"));
    assert!(panel.contains("Scrollbar::vertical("));
    assert!(panel.contains("ScrollbarMode::Always"));

    // 长字段名截断，不能把面板撑宽。
    assert!(list.contains(".text_ellipsis()"));
}

#[test]
fn the_locate_search_input_is_created_once_outside_the_panel() {
    let grid = data_grid_source();

    // 面板内容闭包每次渲染都会重跑：在里面建实体会每次换一个新的，光标也丢。
    assert!(!panel_source().contains("cx.new("));
    assert!(!list_source().contains("cx.new("));
    assert!(grid.contains("let column_locate_search = cx.new(|cx| {"));

    // 与字段过滤共用占位文案与搜索图标，两个面板看起来是一回事。
    assert!(list_source().contains("TableDataGrid.search_no_match"));
    assert!(panel_source().contains("Icon::new(IconName::Search)"));
    assert!(panel_source().contains(".cleanable(true)"));
    // 搜索词只影响面板列表，宿主重画即可。
    assert!(grid.contains("fn bind_column_locate_search_event("));
    assert!(grid.contains("result.bind_column_locate_search_event(window, cx);"));
}

#[test]
fn a_hidden_field_shows_itself_before_the_table_scrolls() {
    let list = list_source();
    let locate = locate_source();

    // 隐藏字段照样列出来，并标注状态——用户正是靠定位找回被筛掉的列。
    assert!(list.contains(".when(!visible, |this| {"));
    assert!(list.contains("TableDataGrid.locate_column_hidden"));

    // 整行可点即定位。
    assert!(list.contains("grid.locate_column(original_ix, cx)"));

    // 隐藏的列不在列组里，先取消隐藏（内部会重建列组），否则无处可滚。
    assert!(locate.contains("self.set_column_visibility(original_ix, true, cx);"));
    assert!(
        locate
            .find("self.set_column_visibility(original_ix, true, cx);")
            .unwrap()
            < locate.find("display_position_of_original_column").unwrap(),
        "必须先取消隐藏，再算显示位序"
    );
}

#[test]
fn grid_and_vertical_locate_share_one_marker_and_scroll_their_own_way() {
    let locate = locate_source();

    // 面板必须收起：展开着正好挡住「定位到了哪一列」这个结果。
    assert!(locate.contains("self.column_locate_open = false;"));
    assert!(
        locate.find("self.column_locate_open = false;").unwrap()
            < locate.find("self.set_column_visibility").unwrap(),
        "收起面板要放在最前面，别被后面的提前 return 跳过"
    );

    // 两种形态都只算一次网格列号，标记动作才共用同一个坐标。
    assert!(locate.contains("grid_column_of_display_col(display_col, row_number_enabled)"));

    // 高亮只走「定位列」标记，一次调用覆盖网格与纵向。
    assert!(locate.contains("state.set_located_col(grid_col, cx)"));

    // 不许改道去走选中态：选中列会给整列逐格染色、选中行会给整行染色，
    // 绘制量随可见行数线性增长，滚动时每帧都要付这笔钱。
    assert!(
        !locate.contains("set_selected_col"),
        "定位不能借用选中列，那会把整列染一遍"
    );
    assert!(
        !locate.contains("select_cell"),
        "定位不能借用选中单元格，那会把整行染一遍"
    );

    // 纵向再自己纵滚到那一行：uniform_list 的滚动句柄不归表格状态管。
    assert!(locate.contains("self.vertical_scroll_handle.scroll_to_item("));
    assert!(locate.contains("vertical_line_of_first_field(display_col)"));
    assert!(locate.contains("ScrollStrategy::Top"));
    assert!(locate.contains("let vertical = self.is_vertical_view(cx);"));

    // 受控 Popover 的开关状态要回写，否则点trigger后再也打不开。
    let button = slice_between(
        data_grid_source(),
        "fn render_column_locate_button(",
        "\n    /// 工具栏里的查找框",
    );
    assert!(button.contains(".open(self.column_locate_open)"));
    assert!(button.contains(".on_open_change("));
    assert!(button.contains("grid.column_locate_open = new_open;"));
}

#[test]
fn the_locate_marker_paints_one_cell_and_drags_no_events() {
    // 标记的定义侧也锁住：只染表头一格。数据行一旦跟着染色，就等于回到了
    // 「逐行绘制」，而定位是个每帧都要重画的高频路径。
    let cell = render_cell_source();
    assert!(
        cell.contains("row_ix.is_none() && self.located_col == Some(col_ix)"),
        "定位标记必须只作用于表头（`row_ix == None`）"
    );
    assert!(cell.contains(".when(is_located_header, |this| this.bg(cx.theme().table_active))"));

    // 整列染色的那条路径与定位无关：它只能被真正的「选中列」触发。
    assert!(
        !render_col_wrap_source().contains("located_col"),
        "整列染色不许接手定位标记，否则又变成逐格绘制"
    );

    // 只改标记 + 滚动，不发事件：宿主订阅 `SelectColumn` 等事件会连带重建整张表。
    let set_marker = set_located_col_source();
    assert!(set_marker.contains("self.ensure_col_visible(col_ix, cx);"));
    assert!(
        !set_marker.contains("cx.emit("),
        "定位标记不能发选中事件，那会牵动宿主视图整棵重绘"
    );
}

#[test]
fn the_vertical_locate_marker_tints_only_the_field_name_cell() {
    let line = vertical_line_source();

    // 纵向没有表头，改成染字段名称那一格；值区保持干净，也避免整行逐格上色。
    let label_at = line
        .find("VERTICAL_LABEL_WIDTH")
        .expect("纵向字段名称格要有固定宽度");
    let located_at = line.find(".when(line.located").expect("定位标记要染名称格");
    assert!(
        label_at < located_at,
        "定位染色必须落在固定宽度的名称格上，不是整行"
    );
    assert!(line.contains("this.bg(cx.theme().table_active)"));

    // 名称格之后紧跟 `.child(line.name)`，即染色确实挂在名称那一层。
    let name_at = line[located_at..]
        .find(".child(line.name)")
        .expect("名称格的内容就是字段名");
    assert!(
        line[located_at..][..name_at].contains("table_active"),
        "染色与字段名之间不能夹进别的可染元素"
    );

    // 行级染色仍然只属于「选中该行」，定位不参与。
    let row_wash = slice_between(line, "let mut row_element", ".when(line.located");
    assert!(row_wash.contains("line.selected && !line.editing"));
    assert!(!row_wash.contains("line.located"));
}

#[test]
fn a_stale_locate_marker_is_cleared_when_the_table_moves_on() {
    // 标记记的是列号。列组整套重建（隐藏列、移动列、换页）或选中态一变化，
    // 旧列号就不再指向同一字段了，留着会染错表头。
    let state = table_state_source();
    for anchor in [
        "fn prepare_col_groups(",
        "pub fn set_selected_col(",
        "pub fn clear_selection(",
        "fn sync_legacy_selection(",
    ] {
        let body = slice_between(state, anchor, "\n    }");
        assert!(
            body.contains("self.located_col = None;"),
            "`{anchor}` 之后必须清掉定位标记"
        );
    }
}

#[test]
fn locate_reuses_the_shared_display_index_conversion() {
    // 「定位字段」与纵向视图的字段行都必须按同一口径补行号列偏移，
    // 各自算一次就会有一位之差，选中会落到隔壁列。所以这里只准调共享函数，
    // 不准在定位路径上手写偏移。
    assert_eq!(1, grid_column_of_display_col(0, true));
    assert_eq!(0, grid_column_of_display_col(0, false));
    assert!(!locate_source().contains("row_number_offset"));
    assert!(locate_source().contains("grid_column_of_display_col"));
}

#[test]
fn the_locate_entries_have_all_their_words() {
    let panel = panel_source();
    assert!(panel.contains("TableDataGrid.locate_column"));
    assert!(panel.contains("TableDataGrid.locate_column_hint"));

    // 词条缺失时 `t!` 会把 key 原样返回，界面上就会直接出现 `TableDataGrid.xxx`。
    for key in [
        "TableDataGrid.locate_column",
        "TableDataGrid.locate_column_hint",
        "TableDataGrid.locate_column_hidden",
        "TableDataGrid.column_visibility_search_placeholder",
    ] {
        assert_ne!(key, t!(key).as_ref(), "词条 `{key}` 缺失");
    }
}
