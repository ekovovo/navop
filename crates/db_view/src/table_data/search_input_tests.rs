//! 面板搜索框高度的回归测试
//!
//! 「字段过滤」「定位字段」两个面板里的搜索框曾经用默认档（Medium），实测盒高
//! 32px，而 gpui-component 的单行 `Input` 把行高写死成 `Rems(1.25)`=20px，Medium
//! 档还要在盒内扣掉上下各 8px 内边距和 1px 边框，只剩 14px —— placeholder 上下
//! 各被裁掉一截，用户输入更看不清。这里用真实布局量出各档高度，把「为什么必须
//! 选 Large」钉住。

use gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement,
    Pixels, Render, Styled, TestAppContext, VisualTestContext, Window, WindowOptions, div, px,
};
use gpui_component::{
    Root, Sizable as _, Size,
    input::{Input, InputState},
    v_flex,
};

/// 单行 `Input` 的行盒高度：`Rems(1.25)` × rem 基准 16px。
const INPUT_LINE_HEIGHT: Pixels = px(20.);
/// 各档在盒内扣掉的上下内边距（`Size::input_py`），顺序与 [`SIZED_INPUTS`] 对齐。
const INPUT_PY: [f32; 4] = [0., 2., 8., 10.];
/// `Input` 自带的边框宽度。
const INPUT_BORDER: f32 = 1.;

/// 文字不被裁掉所需的最小盒高。
fn min_input_height(py: f32) -> f32 {
    INPUT_LINE_HEIGHT.as_f32() + py * 2. + INPUT_BORDER * 2.
}

const SIZED_INPUTS: [(&str, Size); 4] = [
    ("XSmall", Size::XSmall),
    ("Small", Size::Small),
    ("Medium", Size::Medium),
    ("Large", Size::Large),
];

struct Probe {
    states: Vec<Entity<InputState>>,
}

impl Probe {
    fn new(window: &mut Window, cx: &mut App) -> Self {
        let states = SIZED_INPUTS
            .iter()
            .map(|_| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("搜索字段…")
                        .clean_on_escape()
                })
            })
            .collect();
        Self { states }
    }
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .children(self.states.iter().enumerate().map(|(ix, state)| {
                div()
                    .id(("input-probe", ix))
                    .debug_selector(move || format!("input-probe-{ix}"))
                    .child(
                        Input::new(state)
                            .with_size(SIZED_INPUTS[ix].1)
                            // 与面板同款：带前缀图标、可清空、占满宽度。
                            .prefix(div())
                            .cleanable(true)
                            .w_full(),
                    )
            }))
    }
}

fn data_grid_source() -> &'static str {
    include_str!("data_grid.rs")
}

fn panel_source(start: &str, end: &str) -> &'static str {
    let source = data_grid_source();
    let start = source
        .find(start)
        .unwrap_or_else(|| panic!("找不到起点 `{start}`"));
    let body = &source[start..];
    let end = body
        .find(end)
        .unwrap_or_else(|| panic!("找不到终点 `{end}`"));
    &body[..end]
}

/// 面板用的档位必须真的容得下行盒，更小的档位必须真的容不下。
///
/// 两边都断言：只断言 Large 够用，将来 gpui-kit 把 Medium 的内边距改小、
/// 这个测试也不会提醒我们回到更紧凑的档位。
#[gpui::test]
fn the_panel_search_box_uses_the_smallest_size_that_fits_the_text_line(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx
        .update(|cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| Probe::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
        })
        .expect("open search input probe window");

    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let last = SIZED_INPUTS.len() - 1;
    for (ix, (name, _)) in SIZED_INPUTS.iter().enumerate() {
        let selector: &'static str = Box::leak(format!("input-probe-{ix}").into_boxed_str());
        let height = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("input-probe-{ix} 没有参与布局"))
            .size
            .height
            .as_f32();
        let minimum = min_input_height(INPUT_PY[ix]);
        if ix == last {
            assert!(
                height >= minimum,
                "面板用 {name} 档：盒高 {height}px，至少要 {minimum}px 才装得下行盒"
            );
        } else {
            assert!(
                height < minimum,
                "{name} 档（{height}px）本该装不下行盒；若它已经够用，就该改用更紧凑的档位"
            );
        }
    }

    // 测量对象要和界面一致：两个面板都得真的用上 Large。
    assert!(
        visibility_panel_source().contains(".with_size(Size::Large)"),
        "「字段过滤」面板的搜索框没有用 Large 档"
    );
    assert!(
        locate_panel_source().contains(".with_size(Size::Large)"),
        "「定位字段」面板的搜索框没有用 Large 档"
    );
}

fn visibility_panel_source() -> &'static str {
    panel_source(
        "fn build_column_visibility_panel(",
        "/// 「字段定位」面板的尺寸与行高。",
    )
}

fn locate_panel_source() -> &'static str {
    panel_source("fn build_column_locate_panel(", "/// 构建「显示方式」菜单")
}
