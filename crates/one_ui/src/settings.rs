use gpui::{App, Global, Pixels, px};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableDisplaySettings {
    pub row_height: u32,
    /// 在列头下方常驻一行字段注释，并把列头加高一行。
    pub column_comment_in_header: bool,
}

impl TableDisplaySettings {
    pub const DEFAULT_ROW_HEIGHT: u32 = 44;
    pub const MIN_ROW_HEIGHT: u32 = 24;
    pub const MAX_ROW_HEIGHT: u32 = 100;
    /// 注释行的高度，用固定像素而不是字体度量，避免各操作系统行高不一致。
    pub const COMMENT_LINE_HEIGHT: u32 = 18;

    pub fn new(row_height: u32) -> Self {
        Self {
            row_height: clamp_row_height(row_height),
            column_comment_in_header: false,
        }
    }

    pub fn with_column_comment(mut self, enabled: bool) -> Self {
        self.column_comment_in_header = enabled;
        self
    }

    /// 列头高度：数据行高加上注释行（仅当开关打开）。
    pub fn header_height(&self, row_height: Pixels) -> Pixels {
        if self.column_comment_in_header {
            row_height + px(Self::COMMENT_LINE_HEIGHT as f32)
        } else {
            row_height
        }
    }
}

impl Default for TableDisplaySettings {
    fn default() -> Self {
        Self {
            row_height: Self::DEFAULT_ROW_HEIGHT,
            column_comment_in_header: false,
        }
    }
}

impl Global for TableDisplaySettings {}

pub fn init_table_display_settings(cx: &mut App, settings: TableDisplaySettings) {
    cx.set_global(settings);
}

pub fn set_table_row_height(height: u32, cx: &mut App) {
    update_display_settings(cx, |settings| {
        settings.row_height = clamp_row_height(height);
    });
}

pub fn set_table_column_comment_in_header(enabled: bool, cx: &mut App) {
    update_display_settings(cx, |settings| {
        settings.column_comment_in_header = enabled;
    });
}

fn update_display_settings(cx: &mut App, update: impl FnOnce(&mut TableDisplaySettings)) {
    let mut settings = cx
        .try_global::<TableDisplaySettings>()
        .copied()
        .unwrap_or_default();
    update(&mut settings);
    cx.set_global(settings);
}

/// 列头是否需要为注释多留一行。
pub fn table_column_comment_in_header(cx: &App) -> bool {
    cx.try_global::<TableDisplaySettings>()
        .is_some_and(|settings| settings.column_comment_in_header)
}

pub fn table_row_height(cx: &App) -> Pixels {
    table_row_height_or(cx, px(TableDisplaySettings::DEFAULT_ROW_HEIGHT as f32))
}

pub fn table_row_height_or(cx: &App, fallback: Pixels) -> Pixels {
    cx.try_global::<TableDisplaySettings>()
        .map(|settings| px(settings.row_height as f32))
        .unwrap_or(fallback)
}

/// 列头高度，与 [`table_row_height_or`] 同源，开关打开时多一行注释。
pub fn table_header_height_or(cx: &App, fallback: Pixels) -> Pixels {
    cx.try_global::<TableDisplaySettings>()
        .map(|settings| settings.header_height(px(settings.row_height as f32)))
        .unwrap_or(fallback)
}

fn clamp_row_height(height: u32) -> u32 {
    height.clamp(
        TableDisplaySettings::MIN_ROW_HEIGHT,
        TableDisplaySettings::MAX_ROW_HEIGHT,
    )
}

#[cfg(test)]
mod tests {
    use super::{TableDisplaySettings, clamp_row_height};
    use gpui::px;

    #[test]
    fn table_row_height_clamps_to_supported_range() {
        assert_eq!(
            TableDisplaySettings::MIN_ROW_HEIGHT,
            clamp_row_height(TableDisplaySettings::MIN_ROW_HEIGHT - 1)
        );
        assert_eq!(44, clamp_row_height(44));
        assert_eq!(
            TableDisplaySettings::MAX_ROW_HEIGHT,
            clamp_row_height(TableDisplaySettings::MAX_ROW_HEIGHT + 1)
        );
    }

    #[test]
    fn column_comment_in_header_is_off_by_default_and_keeps_header_single_line() {
        let settings = TableDisplaySettings::new(TableDisplaySettings::DEFAULT_ROW_HEIGHT);

        assert!(!settings.column_comment_in_header, "列头注释行默认应关闭");
        assert_eq!(settings.header_height(px(44.)), px(44.));
    }

    #[test]
    fn column_comment_in_header_adds_one_extra_line_to_header_height() {
        let settings = TableDisplaySettings {
            row_height: 24,
            column_comment_in_header: true,
        };

        assert_eq!(
            settings.header_height(px(24.)),
            px(24. + TableDisplaySettings::COMMENT_LINE_HEIGHT as f32)
        );
    }

    #[gpui::test]
    fn header_height_follows_row_height_plus_the_comment_line_when_enabled(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            // 全局缺失时退回调用方给定的单行高度。
            assert_eq!(super::table_header_height_or(cx, px(30.)), px(30.));

            super::init_table_display_settings(cx, TableDisplaySettings::new(40));
            assert_eq!(super::table_header_height_or(cx, px(30.)), px(40.));

            super::set_table_column_comment_in_header(true, cx);
            assert_eq!(
                super::table_header_height_or(cx, px(30.)),
                px(40. + TableDisplaySettings::COMMENT_LINE_HEIGHT as f32)
            );

            super::set_table_column_comment_in_header(false, cx);
            assert_eq!(super::table_header_height_or(cx, px(30.)), px(40.));
        });
    }
}
