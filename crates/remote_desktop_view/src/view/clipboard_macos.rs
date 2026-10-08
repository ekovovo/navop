use std::path::PathBuf;

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSFilenamesPboardType, NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::{NSArray, NSData, NSString, NSURL};

/// Writes validated staging paths from a GPUI foreground callback.
///
/// Keep this on the UI thread: it calls AppKit's process-global pasteboard.
pub(super) fn write_files_to_system_clipboard(paths: &[PathBuf]) -> anyhow::Result<()> {
    let pasteboard = NSPasteboard::generalPasteboard();
    write_files_to_pasteboard(&pasteboard, paths)
}

#[allow(deprecated)]
fn write_files_to_pasteboard(pasteboard: &NSPasteboard, paths: &[PathBuf]) -> anyhow::Result<()> {
    anyhow::ensure!(!paths.is_empty(), "clipboard file list is empty");

    let path_strings = paths
        .iter()
        .map(|path| {
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("clipboard file path is not valid UTF-8"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    autoreleasepool(|_| {
        let filenames_type = unsafe { NSFilenamesPboardType };
        let string_type = unsafe { NSPasteboardTypeString };

        // Finder 复制文件的经典写入序列:declareTypes + setPropertyList。
        // 不能用 writeObjects(NSURL):在本机(受限进程/部分 macOS 版本下)
        // 它只产出文本表示,既没有 public.file-url,也让 GPUI 的 pasteboard
        // 读回退化成 String —— 结果是 Finder 粘贴出「已粘贴 <日期>」的文本
        // 文件、宿主每 500ms 把自己刚装的文件剪贴板当成本地新文本回推远端。
        //
        // NSFilenamesPboardType 是 Finder 与 GPUI(gpui_macos/pasteboard.rs
        // 的 read 只认它来还原 ExternalPaths)共同的规范表示;AppKit 会由它
        // 自动派生 public.file-url / Apple URL pasteboard type 等类型。
        let types = NSArray::from_slice(&[filenames_type, string_type]);
        unsafe { pasteboard.declareTypes_owner(&types, None) };

        let ns_paths: Vec<Retained<NSString>> = path_strings
            .iter()
            .map(|path| NSString::from_str(path))
            .collect();
        let paths_array = NSArray::from_retained_slice(&ns_paths);
        let paths_plist = unsafe { paths_array.cast_unchecked::<AnyObject>() };
        if !unsafe { pasteboard.setPropertyList_forType(&paths_plist, filenames_type) } {
            tracing::warn!("macOS rejected the clipboard filenames property list");
        }

        // 纯文本回退:以换行分隔的路径列表,供只认文本的接收方使用。
        let joined_paths = path_strings.join("\n");
        let text = NSData::with_bytes(joined_paths.as_bytes());
        if !pasteboard.setData_forType(Some(&text), string_type) {
            tracing::warn!("macOS rejected the clipboard path text fallback");
        }

        Ok(())
    })
}

#[allow(deprecated)]
#[cfg(test)]
mod tests {
    use objc2::msg_send;
    use objc2::ClassType as _;
    use objc2::ffi::NSUInteger;

    use super::*;

    #[test]
    fn native_file_clipboard_rejects_empty_file_lists() {
        let pasteboard = NSPasteboard::pasteboardWithUniqueName();

        assert!(write_files_to_pasteboard(&pasteboard, &[]).is_err());
    }

    #[test]
    fn native_file_clipboard_writes_finder_compatible_file_urls() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let first = temp.path().join("报告 one.txt");
        let second = temp.path().join("data.csv");
        std::fs::write(&first, b"report").expect("write first clipboard file");
        std::fs::write(&second, b"data").expect("write second clipboard file");

        let pasteboard = NSPasteboard::pasteboardWithUniqueName();

        write_files_to_pasteboard(&pasteboard, &[first.clone(), second.clone()])
            .expect("write native file clipboard");

        // declareTypes 走 Filenames 表示,AppKit 由它派生 public.file-url;
        // 规范读法是 readObjectsForClasses: 直接取回 NSURL 对象。
        let classes = NSArray::from_slice(&[NSURL::class()]);
        let urls = unsafe { pasteboard.readObjectsForClasses_options(&classes, None) }
            .expect("file URL objects readable from pasteboard");
        let urls = unsafe { urls.cast_unchecked::<NSURL>() };
        assert_eq!(urls.len(), 2);

        let first_url = unsafe { urls.objectAtIndex_unchecked(0) };
        let first_path = first_url.path().expect("file URL path");
        assert_eq!(first.to_string_lossy(), first_path.to_string());

        let string_type = unsafe { NSPasteboardTypeString };
        let fallback = pasteboard
            .stringForType(string_type)
            .expect("string fallback on pasteboard");
        assert_eq!(
            format!("{}\n{}", first.to_string_lossy(), second.to_string_lossy()),
            fallback.to_string()
        );

        // GPUI 的读回与 Finder 粘贴都依赖旧式 Filenames 类型:必须能按
        // 原顺序还原全部路径。
        let filenames_type = unsafe { NSFilenamesPboardType };
        let filenames = pasteboard
            .propertyListForType(filenames_type)
            .expect("filenames property list on pasteboard");
        let count: usize = unsafe { msg_send![&filenames, count] };
        assert_eq!(count, 2);
        for (index, expected) in [&first, &second].into_iter().enumerate() {
            let item: *mut NSString =
                unsafe { msg_send![&filenames, objectAtIndex: index as NSUInteger] };
            assert!(!item.is_null());
            let item = unsafe { &*item };
            assert_eq!(item.to_string(), expected.to_string_lossy());
        }
    }
}
