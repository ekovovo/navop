//! 会挤占相邻控件的短标签（连接名、端点名等）的字符级省略。
//!
//! 终端文件侧边栏的目标选择器与 SFTP 视图的端点切换按钮共用同一套规则，
//! 避免两处各自调宽度上限后再次漂移。

/// 短标签最多保留的字符数。
pub const SHORT_LABEL_MAX_CHARS: usize = 2;

/// 短标签的兜底宽度上限（两个汉字加省略号的量级）。
///
/// 字符截断已经保证标签很短，这里只是防止异常字体或超宽字形把相邻控件挤走。
pub const SHORT_LABEL_MAX_WIDTH: f32 = 80.;

/// 只保留前 [`SHORT_LABEL_MAX_CHARS`] 个字符，其余用 `...` 省略。
///
/// 按 `char` 计数，中文与 emoji 都不会被切坏；不超过上限时原样返回。
pub fn short_label(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(SHORT_LABEL_MAX_CHARS).collect();

    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::{SHORT_LABEL_MAX_CHARS, short_label};

    #[test]
    fn keeps_only_the_first_two_characters() {
        assert_eq!("开发...", short_label("开发测试环境"));
        assert_eq!("co...", short_label("comi"));
    }

    #[test]
    fn short_names_stay_untouched() {
        assert_eq!("开发", short_label("开发"));
        assert_eq!("a", short_label("a"));
        assert_eq!("", short_label(""));
    }

    #[test]
    fn counts_characters_instead_of_bytes() {
        assert_eq!("🐦🐦...", short_label("🐦🐦🐦"));
        assert_eq!(2, SHORT_LABEL_MAX_CHARS);
    }
}
