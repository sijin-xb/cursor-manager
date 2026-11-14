//! 把选中的光标主题/尺寸应用到本机各配置源，并读取当前状态。
//!
//! 目标（与本机 matugen / generate_cursor_theme.py 的覆盖面一致）：
//!   1. ~/.icons/default/index.theme   经典 X11 fallback 继承
//!   2. gsettings                      GTK3/GTK4（niri 会话里的 GTK 走 xdg-desktop-portal）
//!   3. environment.d/cursor.conf      systemd/dbus 启动的程序
//!   4. niri                           扫描 ~/.config/niri 下的 *.kdl，
//!                                     改写 xcursor-theme/xcursor-size 与
//!                                     environment 块里的 XCURSOR_*，然后热重载
//!   5. Hyprland                       custom/env.lua 的 hl.env + ~/.cache/cursor_theme
//!   6. Xresources                     仅当已存在 Xcursor.theme 条目时改写

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TargetResult {
    pub label: &'static str,
    /// 该目标存在且本次操作成功（读取时表示"成功读到了值"）
    pub ok: bool,
    pub detail: String,
}

pub struct Snapshot {
    pub theme: Option<String>,
    pub size: Option<u32>,
    pub targets: Vec<TargetResult>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// 逐行改写：closure 对某行返回 Some(新行) 表示替换。
/// 返回 None 表示文件没有变化。
fn map_lines(text: &str, f: &mut impl FnMut(&str) -> Option<String>) -> Option<String> {
    let mut changed = false;
    let mut out = Vec::with_capacity(text.lines().count());
    for line in text.lines() {
        match f(line) {
            Some(new) => {
                changed = true;
                out.push(new);
            }
            None => out.push(line.to_string()),
        }
    }
    let mut s = out.join("\n");
    if text.ends_with('\n') {
        s.push('\n');
    }
    changed.then_some(s)
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("cursor-manager.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

// ─────────────────────────── 读取 ───────────────────────────

fn read_default_index() -> Option<String> {
    let text = fs::read_to_string(home().join(".icons/default/index.theme")).ok()?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Inherits="))
        .map(|v| v.trim().to_string())
}

fn read_envd() -> (Option<String>, Option<u32>) {
    let Ok(text) = fs::read_to_string(home().join(".config/environment.d/cursor.conf")) else {
        return (None, None);
    };
    let mut theme = None;
    let mut size = None;
    for l in text.lines() {
        let t = l.trim();
        if let Some(v) = t.strip_prefix("XCURSOR_THEME=") {
            theme = Some(v.trim().to_string());
        } else if let Some(v) = t.strip_prefix("XCURSOR_SIZE=") {
            size = v.trim().parse().ok();
        }
    }
    (theme, size)
}

/// niri：找含 xcursor-theme 的 kdl 文件，取主题（及尺寸）。
fn niri_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
        if depth > 3 {
            return;
        }
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().map(|n| n == "backup").unwrap_or(false) {
                    continue;
                }
                walk(&p, out, depth + 1);
            } else if p.extension().map(|x| x == "kdl").unwrap_or(false)
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.contains(".bak") || n.contains(".pre-inir"))
                    .unwrap_or(false)
                && fs::read_to_string(&p)
                    .map(|t| t.contains("xcursor-theme") || t.contains("XCURSOR_THEME"))
                    .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&home().join(".config/niri"), &mut out, 0);
    out
}

fn kdl_string_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let t = l.trim();
        let v = t.strip_prefix(key)?.trim();
        let v = v.strip_prefix('"')?;
        let v = v.split('"').next()?;
        Some(v.to_string())
    })
}

fn read_niri() -> Option<(String, Option<u32>)> {
    let p = niri_files().into_iter().next()?;
    let text = fs::read_to_string(p).ok()?;
    let theme = kdl_string_value(&text, "xcursor-theme")?;
    let size = kdl_string_value(&text, "xcursor-size").and_then(|v| v.parse().ok());
    Some((theme, size))
}

fn read_xresources() -> (Option<String>, Option<u32>) {
    let Ok(text) = fs::read_to_string(home().join(".Xresources")) else {
        return (None, None);
    };
    let mut theme = None;
    let mut size = None;
    for l in text.lines() {
        let t = l.trim();
        let lower = t.to_lowercase();
        if lower.starts_with("xcursor.theme:") {
            // 值要从原行截取，避免 to_lowercase 改掉主题名的大小写
            if let Some(idx) = t.find(':') {
                theme = Some(t[idx + 1..].trim().to_string());
            }
        } else if lower.starts_with("xcursor.size:") {
            if let Some(idx) = t.find(':') {
                size = t[idx + 1..].trim().parse().ok();
            }
        }
    }
    (theme, size)
}

fn gsettings_get(key: &str) -> Option<String> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(s.trim_matches('\'').to_string())
}

pub fn snapshot() -> Snapshot {
    let (envd_theme, envd_size) = read_envd();
    let (xr_theme, xr_size) = read_xresources();
    let niri = read_niri();

    let gset_theme = gsettings_get("cursor-theme");
    let gset_size: Option<u32> = gsettings_get("cursor-size").and_then(|v| v.parse().ok());

    let hypr_theme = fs::read_to_string(home().join(".cache/cursor_theme"))
        .ok()
        .and_then(|t| t.lines().next().map(|l| l.trim().to_string()))
        .filter(|s| !s.is_empty());

    let fmt = |t: &Option<String>, s: Option<u32>| match (t, s) {
        (Some(t), Some(s)) => format!("{t} / {s}"),
        (Some(t), None) => t.clone(),
        (None, _) => "未配置".into(),
    };

    let targets = vec![
        TargetResult {
            label: "default 继承",
            ok: read_default_index().is_some(),
            detail: fmt(&read_default_index(), None),
        },
        TargetResult {
            label: "GTK (gsettings)",
            ok: gset_theme.is_some(),
            detail: fmt(&gset_theme, gset_size),
        },
        TargetResult {
            label: "environment.d",
            ok: envd_theme.is_some(),
            detail: fmt(&envd_theme, envd_size),
        },
        TargetResult {
            label: "niri",
            ok: niri.is_some(),
            detail: match &niri {
                Some((t, s)) => fmt(&Some(t.clone()), *s),
                None => "未找到含 xcursor-theme 的 kdl".into(),
            },
        },
        TargetResult {
            label: "Hyprland 缓存",
            ok: hypr_theme.is_some(),
            detail: fmt(&hypr_theme, None),
        },
        TargetResult {
            label: "Xresources",
            ok: xr_theme.is_some(),
            detail: if xr_theme.is_some() {
                fmt(&xr_theme, xr_size)
            } else {
                "无 Xcursor 条目，跳过".to_string()
            },
        },
    ];

    // "当前主题"取 gsettings 值（niri 会话里它和合成器一致；-alt 是重载用的孪生名）
    let mut theme = gset_theme.or_else(|| read_default_index());
    if let Some(t) = &theme {
        if let Some(stripped) = t.strip_suffix("-alt") {
            theme = Some(stripped.to_string());
        }
    }
    Snapshot {
        theme,
        size: gset_size.or(envd_size),
        targets,
    }
}

// ─────────────────────────── 应用 ───────────────────────────

pub fn apply(theme: &str, size: u32) -> Vec<TargetResult> {
    vec![
        apply_default_index(theme),
        apply_gsettings(theme, size),
        apply_envd(theme, size),
        apply_niri(theme, size),
        apply_hypr(theme, size),
        apply_xresources(theme, size),
    ]
}

fn apply_default_index(theme: &str) -> TargetResult {
    let label = "default 继承";
    let p = home().join(".icons/default/index.theme");
    let body = format!(
        "[Icon Theme]\nName=Default\nComment=Default Cursor Theme\nInherits={theme}\n"
    );
    match fs::read_to_string(&p) {
        Ok(text) => match map_lines(&text, &mut |l| {
            let t = l.trim();
            t.starts_with("Inherits=")
                .then(|| format!("Inherits={theme}"))
        }) {
            Some(new) => match write_atomic(&p, &new) {
                Ok(()) => TargetResult {
                    label,
                    ok: true,
                    detail: format!("已指向 {theme}"),
                },
                Err(e) => TargetResult {
                    label,
                    ok: false,
                    detail: format!("写入失败: {e}"),
                },
            },
            None => TargetResult {
                label,
                ok: true,
                detail: format!("未变（{theme}）"),
            },
        },
        Err(_) => match fs::create_dir_all(p.parent().unwrap()).and_then(|()| {
            fs::write(&p, &body)
        }) {
            Ok(()) => TargetResult {
                label,
                ok: true,
                detail: format!("新建并指向 {theme}"),
            },
            Err(e) => TargetResult {
                label,
                ok: false,
                detail: format!("新建失败: {e}"),
            },
        },
    }
}

fn apply_gsettings(theme: &str, size: u32) -> TargetResult {
    let label = "GTK (gsettings)";
    if Command::new("gsettings")
        .arg("--version")
        .output()
        .is_err()
    {
        return TargetResult {
            label,
            ok: false,
            detail: "gsettings 不可用，跳过".into(),
        };
    }
    let r1 = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "cursor-theme", theme])
        .status();
    let r2 = Command::new("gsettings")
        .args([
            "set",
            "org.gnome.desktop.interface",
            "cursor-size",
            &size.to_string(),
        ])
        .status();
    match (r1, r2) {
        (Ok(a), Ok(b)) if a.success() && b.success() => TargetResult {
            label,
            ok: true,
            detail: format!("已设置 {theme} / {size}"),
        },
        _ => TargetResult {
            label,
            ok: false,
            detail: "gsettings set 失败".into(),
        },
    }
}

fn apply_envd(theme: &str, size: u32) -> TargetResult {
    let label = "environment.d";
    let p = home().join(".config/environment.d/cursor.conf");
    let text = match fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => format!(
            "# 由 cursor-manager 维护：经 systemd/dbus 启动的程序读这里\nXCURSOR_THEME={theme}\nXCURSOR_SIZE={size}\n"
        )
        .to_string(),
    };
    let Some(new) = map_lines(&text, &mut |l| {
        let t = l.trim_start();
        if t.starts_with("XCURSOR_THEME=") {
            Some(format!("XCURSOR_THEME={theme}"))
        } else if t.starts_with("XCURSOR_SIZE=") {
            Some(format!("XCURSOR_SIZE={size}"))
        } else {
            None
        }
    }) else {
        return TargetResult {
            label,
            ok: true,
            detail: "未变".into(),
        };
    };
    let _ = fs::create_dir_all(p.parent().unwrap());
    match write_atomic(&p, &new) {
        Ok(()) => TargetResult {
            label,
            ok: true,
            detail: format!("已写入 {theme} / {size}（新会话生效）"),
        },
        Err(e) => TargetResult {
            label,
            ok: false,
            detail: format!("写入失败: {e}"),
        },
    }
}

/// niri/hypr 通用的行改写：kdl 键、环境变量、hl.env 字面量。
/// hl.env 的第二个参数若不是带引号的字面量（如 execs.lua 里的动态变量
/// cursorTheme）则保持不动。
fn shell_line_map(line: &str, theme: &str, size: u32) -> Option<String> {
    let t = line.trim_start();
    let indent = &line[..line.len() - t.len()];
    let quoted = |v: &str| format!("\"{v}\"");
    if t.starts_with("xcursor-theme") {
        Some(format!("{indent}xcursor-theme {}", quoted(theme)))
    } else if t.starts_with("xcursor-size") {
        Some(format!("{indent}xcursor-size {size}"))
    } else if t.starts_with("XCURSOR_THEME=") {
        Some(format!("{indent}XCURSOR_THEME={theme}"))
    } else if t.starts_with("XCURSOR_SIZE=") {
        Some(format!("{indent}XCURSOR_SIZE={size}"))
    } else if t.starts_with("XCURSOR_THEME") {
        Some(format!("{indent}XCURSOR_THEME {}", quoted(theme)))
    } else if t.starts_with("XCURSOR_SIZE") {
        Some(format!("{indent}XCURSOR_SIZE {}", quoted(&size.to_string())))
    } else if t.starts_with("hl.env(\"XCURSOR_THEME\"") {
        hl_env_literal(t, "XCURSOR_THEME", theme).map(|v| format!("{indent}{v}"))
    } else if t.starts_with("hl.env(\"XCURSOR_SIZE\"") {
        hl_env_literal(t, "XCURSOR_SIZE", &size.to_string()).map(|v| format!("{indent}{v}"))
    } else {
        None
    }
}

fn hl_env_literal(trimmed: &str, key: &str, value: &str) -> Option<String> {
    let rest = trimmed.strip_prefix(&format!("hl.env(\"{key}\""))?;
    let rest = rest.trim_start().strip_prefix(',')?.trim_start();
    rest.starts_with('"')
        .then(|| format!("hl.env(\"{key}\", \"{value}\")"))
}

fn apply_niri(theme: &str, size: u32) -> TargetResult {
    let label = "niri";
    let files = niri_files();
    if files.is_empty() {
        return TargetResult {
            label,
            ok: false,
            detail: "未找到含 xcursor-theme 的 kdl，跳过".into(),
        };
    }
    let mut changed = Vec::new();
    for p in &files {
        let Ok(text) = fs::read_to_string(p) else {
            continue;
        };
        if let Some(new) = map_lines(&text, &mut |l| shell_line_map(l, theme, size)) {
            if write_atomic(p, &new).is_ok() {
                changed.push(p.to_string_lossy().to_string());
            }
        }
    }
    if changed.is_empty() {
        return TargetResult {
            label,
            ok: true,
            detail: "配置未变，无需重载".into(),
        };
    }
    let short: Vec<String> = changed
        .iter()
        .map(|p| {
            Path::new(p)
                .strip_prefix(home())
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_else(|_| p.clone())
        })
        .collect();
    let reloaded = reload_niri();
    TargetResult {
        label,
        ok: true,
        detail: format!(
            "{}已改 {}；热重载{}",
            if reloaded { "" } else { "⚠ " },
            short.join(", "),
            if reloaded { "成功" } else { "失败（新窗口生效）" }
        ),
    }
}

fn reload_niri() -> bool {
    let run = |env_socket: Option<&Path>| {
        let mut c = Command::new("niri");
        c.args(["msg", "action", "load-config-file"]);
        if let Some(s) = env_socket {
            c.env("NIRI_SOCKET", s);
        }
        c.status().map(|s| s.success()).unwrap_or(false)
    };
    if run(None) {
        return true;
    }
    // niri 重启后继承的 NIRI_SOCKET 会失效，回退到 runtime 目录里最新的 socket
    let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") else {
        return false;
    };
    let mut socks: Vec<PathBuf> = fs::read_dir(&rt)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("niri.") && n.ends_with(".sock"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    socks.sort_by_key(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    socks.pop().map(|s| run(Some(&s))).unwrap_or(false)
}

fn apply_hypr(theme: &str, size: u32) -> TargetResult {
    let label = "Hyprland";
    let hypr_dir = home().join(".config/hypr");
    if !hypr_dir.is_dir() {
        return TargetResult {
            label,
            ok: false,
            detail: "无 ~/.config/hypr，跳过".into(),
        };
    }
    let mut notes = Vec::new();
    for rel in ["custom/env.lua", "hyprland/execs.lua"] {
        let p = hypr_dir.join(rel);
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        if let Some(new) = map_lines(&text, &mut |l| shell_line_map(l, theme, size)) {
            if write_atomic(&p, &new).is_ok() {
                notes.push(rel.to_string());
            }
        }
    }
    // execs.lua 从这里读主题名；只写第一行
    let cache = home().join(".cache/cursor_theme");
    if fs::write(&cache, format!("{theme}\n")).is_ok() {
        notes.push("~/.cache/cursor_theme".to_string());
    }
    TargetResult {
        label,
        ok: true,
        detail: if notes.is_empty() {
            "未变".into()
        } else {
            format!("已写入 {}", notes.join(", "))
        },
    }
}

fn apply_xresources(theme: &str, size: u32) -> TargetResult {
    let label = "Xresources";
    let p = home().join(".Xresources");
    let Ok(text) = fs::read_to_string(&p) else {
        return TargetResult {
            label,
            ok: false,
            detail: "无 ~/.Xresources，跳过".into(),
        };
    };
    let has_key = text.to_lowercase().contains("xcursor.theme");
    if !has_key {
        return TargetResult {
            label,
            ok: false,
            detail: "无 Xcursor 条目，跳过".into(),
        };
    }
    let Some(new) = map_lines(&text, &mut |l| {
        let t = l.trim_start();
        let lower = t.to_lowercase();
        if lower.starts_with("xcursor.theme") {
            let indent = &l[..l.len() - t.len()];
            Some(format!("{indent}Xcursor.theme: {theme}"))
        } else if lower.starts_with("xcursor.size") {
            let indent = &l[..l.len() - t.len()];
            Some(format!("{indent}Xcursor.size: {size}"))
        } else {
            None
        }
    }) else {
        return TargetResult {
            label,
            ok: true,
            detail: "未变".into(),
        };
    };
    let written = write_atomic(&p, &new).is_ok();
    let merged = Command::new("xrdb")
        .args(["-merge", &p.to_string_lossy()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    TargetResult {
        label,
        ok: written,
        detail: format!(
            "已改写{}",
            if merged { "并 xrdb -merge" } else { "（xrdb 不可用）" }
        ),
    }
}
