//! 光标主题扫描与形状文件解析。

use std::fs;
use std::path::{Path, PathBuf};

/// 一个已安装的光标主题。
#[derive(Clone)]
pub struct CursorTheme {
    /// 目录名，即 XCURSOR_THEME 的值
    pub id: String,
    /// 显示名（index.theme 的 Name，缺省为 id）
    pub name: String,
    pub comment: String,
    pub dir: PathBuf,
}

/// 预览形状及候选光标文件名（手工维护的 X11 alias 链）。
pub const PREVIEW_SHAPES: &[(&str, &[&str])] = &[
    (
        "箭头",
        &["left_ptr", "default", "arrow", "top_left_arrow"],
    ),
    (
        "手型",
        &["pointing_hand", "hand2", "hand1", "hand"],
    ),
    ("文本", &["xterm", "text", "ibeam"]),
    ("忙碌", &["watch", "wait", "progress", "left_ptr_watch"]),
    ("移动", &["fleur", "move", "move_arrow", "size_all"]),
    ("禁止", &["crossed_circle", "not-allowed", "forbidden"]),
];

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// 图标主题根目录，按优先级排列（同名主题取靠前者）。
pub fn scan_roots() -> Vec<PathBuf> {
    let h = home();
    vec![
        h.join(".local/share/icons"),
        h.join(".icons"),
        PathBuf::from("/usr/share/icons"),
    ]
}

/// 扫描所有根目录下含非空 cursors/ 子目录的主题。
pub fn scan() -> Vec<CursorTheme> {
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for root in scan_roots() {
        let Ok(rd) = fs::read_dir(&root) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let id = id.to_string();
            if seen.contains(&id) {
                continue;
            }
            let cursors = dir.join("cursors");
            let n_entries = fs::read_dir(&cursors).map(|rd| rd.count()).unwrap_or(0);
            if n_entries == 0 {
                continue;
            }
            let meta = parse_index_theme(&dir.join("index.theme"));
            if meta.get("hidden").map(|v| v == "true").unwrap_or(false) {
                continue;
            }
            seen.push(id.clone());
            out.push(CursorTheme {
                name: meta.get("name").cloned().unwrap_or_else(|| id.clone()),
                comment: meta.get("comment").cloned().unwrap_or_default(),
                dir,
                id,
            });
        }
    }
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    out
}

/// 极简 INI 解析：只取 [Icon Theme] 段的 Name/Comment/Hidden。
fn parse_index_theme(path: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(text) = fs::read_to_string(path) else {
        return map;
    };
    let mut in_section = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t.eq_ignore_ascii_case("[Icon Theme]");
            continue;
        }
        if !in_section || t.starts_with('#') || t.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            map.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    map
}

/// 在主题里找形状文件：先查本主题 cursors/，再借 xcursor 的继承解析。
pub fn find_shape(theme: &CursorTheme, candidates: &[&str]) -> Option<PathBuf> {
    for name in candidates {
        let p = theme.dir.join("cursors").join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    let ct = xcursor::CursorTheme::load(&theme.id);
    candidates.iter().find_map(|n| ct.load_icon(n))
}
