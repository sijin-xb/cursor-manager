//! 安装外部格式的光标包到 ~/.local/share/icons/。
//!
//! 支持：
//!   · 现成的 XCursor 主题（目录或 zip，含 cursors/）→ 直接复制
//!   · Windows 光标包：.cur / .ani / .inf（目录或 zip）→ 解码并转成 XCursor
//!   · macOS 光标：.icns → 解码最大尺寸，按文件名猜角色、按角色猜热点
//!
//! XCursor 文件由本模块直接写出（libXcursor 格式），写完用
//! xcursor::parser 做一次往返自校验。

use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct InstallOutcome {
    pub name: String,
    pub detail: String,
}

/// 一帧光标图像（直通 RGBA）。
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub xhot: u32,
    pub yhot: u32,
    pub delay_ms: u32,
    pub rgba: Vec<u8>,
}

// ─────────────────────────── 入口 ───────────────────────────

/// 按文件头嗅探压缩包类型（下载的文件经常没有扩展名）。
pub fn sniff_archive_ext(path: &Path) -> Option<&'static str> {
    let mut b = [0u8; 300];
    let n = fs::File::open(path).ok()?.read(&mut b).ok()?;
    if n < 8 {
        return None;
    }
    match b[..8] {
        [0x1f, 0x8b, ..] => return Some("gz"),       // gzip
        [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, ..] => return Some("xz"), // xz
        [0x42, 0x5a, 0x68, ..] => return Some("bz2"), // bzip2
        [0x50, 0x4b, 0x03, 0x04, ..] | [0x50, 0x4b, 0x05, 0x06, ..] => return Some("zip"),
        _ => {}
    }
    // tar：magic "ustar" 在偏移 257
    if n >= 262 && &b[257..262] == b"ustar" {
        return Some("tar");
    }
    None
}

const HANDLED_EXTS: &[&str] = &["zip", "xz", "gz", "txz", "tgz", "bz2", "tar", "cur", "ani", "icns"];

pub fn install(src: &Path) -> Result<InstallOutcome, String> {
    let raw_name = src
        .file_stem()
        .or_else(|| src.file_name())
        .and_then(|n| n.to_str())
        .ok_or("路径没有有效名称")?;
    let staging = std::env::temp_dir().join(format!("cursor-manager-install-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| format!("无法创建临时目录: {e}"))?;

    let result = install_inner(src, raw_name, &staging);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn install_inner(src: &Path, raw_name: &str, staging: &Path) -> Result<InstallOutcome, String> {
    let mut ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase());
    // 无扩展名 / 不认识的扩展名 → 按文件头嗅探（在线下载的临时文件常见）
    if src.is_file() && !ext.as_deref().map(|e| HANDLED_EXTS.contains(&e)).unwrap_or(false) {
        if let Some(sn) = sniff_archive_ext(src) {
            ext = Some(sn.to_string());
        }
    }

    let (pack_root, name_base) = if src.is_dir() {
        (src.to_path_buf(), raw_name.to_string())
    } else {
        match ext.as_deref() {
            Some("zip") => {
                let dest = staging.join("zip");
                extract_zip(src, &dest)?;
                (dest, raw_name.to_string())
            }
            Some("xz") | Some("gz") | Some("txz") | Some("tgz") | Some("bz2") | Some("tar") => {
                let dest = staging.join("tar");
                fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
                // 交给系统 tar：xz/gz/bz2 无需额外依赖
                let st = Command::new("tar")
                    .arg("-xf")
                    .arg(src)
                    .arg("-C")
                    .arg(&dest)
                    .status()
                    .map_err(|e| format!("调用 tar 失败: {e}"))?;
                if !st.success() {
                    return Err("tar 解压失败（文件损坏或格式不支持）".into());
                }
                (dest, raw_name.to_string())
            }
            Some("cur") | Some("ani") | Some("icns") => {
                let dest = staging.join("single");
                fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
                let fname = src.file_name().ok_or("无文件名")?;
                fs::copy(src, dest.join(fname)).map_err(|e| format!("读取失败: {e}"))?;
                (dest, raw_name.to_string())
            }
            _ => {
                return Err(format!(
                    "不支持的来源：{}（支持 zip / tar.xz / tar.gz / 目录 / .cur / .ani / .icns）",
                    src.display()
                ))
            }
        }
    };

    // zip/tar 常见 "PackName/PackName/..." 双层包裹，一路下探
    let pack_root = descend_single_dirs(pack_root, 3);

    // 待安装的主题目录：根自带 cursors/ → 单主题（用来源名命名）；
    // 否则收集带 cursors/ 的子目录（GitHub 多主题包，如 Bibata 全家桶）
    let mut theme_dirs: Vec<PathBuf> = Vec::new();
    if pack_root.join("cursors").is_dir() {
        theme_dirs.push(pack_root.clone());
    } else if let Ok(rd) = fs::read_dir(&pack_root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && p.join("cursors").is_dir() {
                theme_dirs.push(p);
            }
        }
        theme_dirs.sort();
    }

    if !theme_dirs.is_empty() {
        let mut installed: Vec<String> = Vec::new();
        for d in &theme_dirs {
            let base = d
                .file_name()
                .and_then(|n| n.to_str())
                .map(sanitize_name)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| sanitize_name(raw_name));
            let n = unique_theme_name(&base);
            let dest = icons_root().join(&n);
            copy_tree(d, &dest)?;
            write_index_theme(&dest, &n);
            installed.push(n);
        }
        let detail = if installed.len() == 1 {
            format!("已是 XCursor 主题，直接复制（{}）", installed[0])
        } else {
            format!("安装了 {} 个主题：{}", installed.len(), installed.join("、"))
        };
        let name = installed[0].clone();
        return Ok(InstallOutcome { name, detail });
    }

    // 单文件 / Windows / macOS 转换路径
    let name = unique_theme_name(&sanitize_name(&name_base));
    let theme_dir = icons_root().join(&name);
    let rollback = |e: String| {
        let _ = fs::remove_dir_all(&theme_dir);
        e
    };

    fs::create_dir_all(theme_dir.join("cursors")).map_err(|e| rollback(e.to_string()))?;

    let detail = {
        let cursors = collect_ext(&pack_root, 0, &["cur", "ani"]);
        let icns_files = collect_ext(&pack_root, 0, &["icns"]);
        let res = if !cursors.is_empty() {
            install_windows_pack(&pack_root, &cursors, &theme_dir)
        } else if !icns_files.is_empty() {
            install_macos_icons(&icns_files, &theme_dir)
        } else {
            Err("没有找到可安装的光标（.cur/.ani/.icns，或现成的 cursors/ 目录）".to_string())
        };
        res.map_err(rollback)?
    };

    write_index_theme(&theme_dir, &name);
    Ok(InstallOutcome { name, detail })
}

/// 把主题目录打包成 zip 到 ~/Downloads，返回输出路径。
pub fn export_zip(dir: &Path) -> Result<String, String> {
    use std::io::Write as _;
    let name = dir.file_name().and_then(|n| n.to_str()).ok_or("无名称")?;
    let out_dir = home().join("Downloads");
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let out = out_dir.join(format!("{name}.zip"));
    let f = std::fs::File::create(&out).map_err(|e| format!("创建 zip 失败: {e}"))?;
    let mut zw = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default();
    fn walk(
        zw: &mut zip::ZipWriter<std::fs::File>,
        dir: &Path,
        rel: &str,
        opts: zip::write::SimpleFileOptions,
    ) -> Result<(), String> {
        for e in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            let fname = e.file_name().to_string_lossy().to_string();
            let rel2 = if rel.is_empty() {
                fname.clone()
            } else {
                format!("{rel}/{fname}")
            };
            if p.is_dir() {
                zw.add_directory(rel2.clone(), opts)
                    .map_err(|e| e.to_string())?;
                walk(zw, &p, &rel2, opts)?;
            } else {
                zw.start_file(rel2.clone(), opts).map_err(|e| e.to_string())?;
                let data = fs::read(&p).map_err(|e| e.to_string())?;
                zw.write_all(&data).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    walk(&mut zw, dir, "", opts)?;
    zw.finish().map_err(|e| format!("收尾 zip 失败: {e}"))?;
    Ok(out.to_string_lossy().to_string())
}

// ─────────────────────────── 通用工具 ───────────────────────────

/// 是否为用户目录（可卸载/可导出）下的主题。
pub fn is_user_theme(dir: &Path) -> bool {
    let roots = [home().join(".local/share/icons"), home().join(".icons")];
    roots.iter().any(|r| dir.starts_with(r))
}

/// 卸载用户目录下的光标主题。系统目录（/usr/share/icons）拒绝，
/// 符号链接（如 Matugen-Cursors-alt）只删链接本身。
pub fn uninstall(dir: &Path) -> Result<String, String> {
    if !is_user_theme(dir) {
        return Err(
            "只能卸载用户目录（~/.local/share/icons、~/.icons）下的主题；系统主题请用包管理器移除"
                .into(),
        );
    }
    if !dir.exists() && !dir.is_symlink() {
        return Err("目录已不存在".into());
    }
    if dir.is_symlink() {
        fs::remove_file(dir).map_err(|e| format!("删除符号链接失败: {e}"))?;
        return Ok(format!("已删除符号链接 {}", dir.display()));
    }
    fs::remove_dir_all(dir).map_err(|e| format!("删除失败: {e}"))?;
    Ok(format!("已删除 {}", dir.display()))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

fn icons_root() -> PathBuf {
    home().join(".local/share/icons")
}

fn sanitize_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    let t = cleaned.trim_matches('-');
    if t.is_empty() {
        "Installed-Cursors".into()
    } else {
        t.to_string()
    }
}

fn unique_theme_name(base: &str) -> String {
    let root = icons_root();
    if !root.join(base).exists() {
        return base.to_string();
    }
    for i in 2..100 {
        let cand = format!("{base}-{i}");
        if !root.join(&cand).exists() {
            return cand;
        }
    }
    format!("{base}-{}", std::process::id())
}

/// 目录下只有一个子目录且没有散文件时，进入该子目录。
fn descend_single_dirs(dir: PathBuf, max: u32) -> PathBuf {
    let mut cur = dir;
    for _ in 0..max {
        let Ok(rd) = fs::read_dir(&cur) else { break };
        let mut sub = None;
        let mut files = 0;
        for e in rd.flatten() {
            if e.path().is_dir() {
                if sub.is_some() {
                    sub = None;
                    break;
                }
                sub = Some(e.path());
            } else {
                files += 1;
            }
        }
        match sub {
            Some(s) if files == 0 => cur = s,
            _ => break,
        }
    }
    cur
}

fn extract_zip(src: &Path, dest: &Path) -> Result<(), String> {
    let f = fs::File::open(src).map_err(|e| format!("打不开 zip: {e}"))?;
    let mut ar = zip::ZipArchive::new(f).map_err(|e| format!("zip 解析失败: {e}"))?;
    for i in 0..ar.len() {
        let mut zf = ar.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = zf.enclosed_name() else { continue };
        let out = dest.join(&rel);
        if zf.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::new();
        zf.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        fs::write(&out, &buf).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn collect_ext(dir: &Path, depth: u32, exts: &[&str]) -> Vec<PathBuf> {
    if depth > 2 {
        return vec![];
    }
    let mut out = vec![];
    let Ok(rd) = fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(collect_ext(&p, depth + 1, exts));
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| exts.contains(&x.to_lowercase().as_str()))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn copy_tree(src: &Path, dest: &Path) -> Result<(), String> {
    fn walk(src: &Path, dest: &Path, depth: u32) -> Result<(), String> {
        if depth > 4 {
            return Ok(());
        }
        fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        for e in fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
            let from = e.path();
            let to = dest.join(e.file_name());
            if from.is_dir() {
                walk(&from, &to, depth + 1)?;
            } else {
                fs::copy(&from, &to).map_err(|e| format!("复制 {} 失败: {e}", from.display()))?;
            }
        }
        Ok(())
    }
    walk(src, dest, 0)
}

fn write_index_theme(theme_dir: &Path, name: &str) {
    let p = theme_dir.join("index.theme");
    if !p.exists() {
        let _ = fs::write(
            &p,
            format!("[Icon Theme]\nName={name}\nComment=Installed by cursor-manager\n"),
        );
    }
}

// ─────────────────────────── 角色映射 ───────────────────────────

/// 已是 XCursor 标准名的直接认领。
const XC_ROLES: &[&str] = &[
    "left_ptr", "top_left_arrow", "center_ptr", "watch", "left_ptr_watch", "crossed_circle",
    "pointing_hand", "hand1", "hand2", "xterm", "crosshair", "fleur", "sb_v_double_arrow",
    "sb_h_double_arrow", "size_nwse", "size_nesw", "question_arrow", "pencil", "top_side",
    "bottom_side", "left_side", "right_side", "top_left_corner", "top_right_corner",
    "bottom_left_corner", "bottom_right_corner", "move",
];

/// Windows 角色关键词 → XCursor 名。**按关键词长度降序排列**，
/// 避免 "arrow" 抢先匹配 "appstarting"。
const WINDOWS_ROLES: &[(&str, &str)] = &[
    ("appstarting", "left_ptr_watch"),
    ("unavailable", "crossed_circle"),
    ("progress", "left_ptr_watch"),
    ("working", "left_ptr_watch"),
    ("sizenwse", "size_nwse"),
    ("sizenesw", "size_nesw"),
    ("uparrow", "center_ptr"),
    ("sizeall", "fleur"),
    ("sizens", "sb_v_double_arrow"),
    ("sizewe", "sb_h_double_arrow"),
    ("arrow", "left_ptr"),
    ("pointer", "left_ptr"),
    ("busy", "watch"),
    ("wait", "watch"),
    ("hand", "pointing_hand"),
    ("link", "pointing_hand"),
    ("ibeam", "xterm"),
    ("text", "xterm"),
    ("cross", "crosshair"),
    ("help", "question_arrow"),
    ("move", "fleur"),
    ("pen", "pencil"),
    ("no", "crossed_circle"),
];

fn match_role(text: &str) -> Option<&'static str> {
    let t = text.to_lowercase();
    if let Some(x) = XC_ROLES.iter().find(|r| **r == t) {
        return Some(x);
    }
    WINDOWS_ROLES
        .iter()
        .find(|(k, _)| t.contains(k))
        .map(|(_, role)| *role)
}

/// 解析 .inf：把行内出现的每个 .cur/.ani 文件名映射到该行的角色词。
fn parse_inf_roles(text: &str) -> HashMap<String, &'static str> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let lower = line.to_lowercase();
        let mut file_tokens: Vec<String> = vec![];
        let mut role = None;
        for tok in lower.split(|c: char| c == '"' || c == ',' || c.is_whitespace()) {
            let tok = tok.trim().trim_matches('"');
            if tok.is_empty() {
                continue;
            }
            let is_file = tok.ends_with(".cur") || tok.ends_with(".ani");
            if is_file {
                let base = tok.rsplit(['\\', '/']).next().unwrap_or(tok).to_string();
                if !base.is_empty() {
                    file_tokens.push(base);
                }
            } else if role.is_none() {
                role = match_role(tok);
            }
        }
        // 文件名本身也可能是角色词（aero_arrow.cur）
        if role.is_none() {
            for f in &file_tokens {
                let stem = f.trim_end_matches(".cur").trim_end_matches(".ani");
                if let Some(r) = match_role(stem) {
                    role = Some(r);
                    break;
                }
            }
        }
        if let Some(role) = role {
            for f in file_tokens {
                map.entry(f).or_insert(role);
            }
        }
    }
    map
}

// ─────────────────────────── Windows 包 ───────────────────────────

fn install_windows_pack(root: &Path, files: &[PathBuf], theme_dir: &Path) -> Result<String, String> {
    let inf_map = collect_ext(root, 0, &["inf"])
        .first()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|t| parse_inf_roles(&t))
        .unwrap_or_default();

    let mut per_role: Vec<(&'static str, Vec<Frame>)> = vec![];
    let mut skipped: Vec<String> = vec![];
    for f in files {
        let base = f
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        let stem = base
            .trim_end_matches(".ani")
            .trim_end_matches(".cur")
            .to_string();
        let Some(role) = inf_map.get(&base).copied().or_else(|| match_role(&stem)) else {
            skipped.push(base);
            continue;
        };
        let bytes = fs::read(f).map_err(|e| format!("读 {} 失败: {e}", f.display()))?;
        let ext = f.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
        let frames = if ext == "ani" {
            decode_ani(&bytes)?
        } else {
            decode_ico(&bytes)?
        };
        match per_role.iter_mut().find(|(r, _)| *r == role) {
            Some((_, v)) => v.extend(frames),
            None => per_role.push((role, frames)),
        }
    }
    if per_role.is_empty() {
        return Err("没有可识别的光标角色（.inf 缺失且文件名无法识别）".into());
    }

    let mut written = 0;
    let mut roles = vec![];
    for (role, mut frames) in per_role {
        if frames.is_empty() {
            continue;
        }
        frames.sort_by_key(|f| f.width);
        write_xcursor(&frames, &theme_dir.join("cursors").join(role))?;
        written += 1;
        roles.push(role);
    }
    let skip_note = if skipped.is_empty() {
        String::new()
    } else {
        format!("，无法识别: {}", skipped.join(", "))
    };
    Ok(format!(
        "Windows 包：写入 {written} 个角色（{}）{skip_note}",
        roles.join(", ")
    ))
}

// ─────────────────────────── macOS .icns ───────────────────────────

fn install_macos_icons(files: &[PathBuf], theme_dir: &Path) -> Result<String, String> {
    let mut roles = vec![];
    for f in files {
        let stem = f
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        let role = match_role(&stem).unwrap_or("left_ptr");
        let bytes = fs::read(f).map_err(|e| format!("读 {} 失败: {e}", f.display()))?;
        let frames = decode_icns(&bytes, role)?;
        write_xcursor(&frames, &theme_dir.join("cursors").join(role))?;
        roles.push(role);
    }
    Ok(format!("macOS .icns：写入 {}（热点为估算值）", roles.join(", ")))
}

fn decode_icns(bytes: &[u8], role: &str) -> Result<Vec<Frame>, String> {
    let family = icns::IconFamily::read(std::io::Cursor::new(bytes))
        .map_err(|e| format!("icns 解析失败: {e}"))?;
    let mut best: Option<(u64, icns::Image)> = None;
    for t in family.available_icons() {
        // JP2 编码的元素未实现，跳过
        if let Ok(img) = family.get_icon_with_type(t) {
            let img = img.convert_to(icns::PixelFormat::RGBA);
            let area = u64::from(img.width()) * u64::from(img.height());
            if best.as_ref().map(|(a, _)| area > *a).unwrap_or(true) {
                best = Some((area, img));
            }
        }
    }
    let img = best.ok_or("icns 中没有可解码的图标")?.1;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err("icns 图像尺寸为 0".into());
    }
    let (xhot, yhot) = default_hotspot(role, w, h);
    Ok(vec![Frame {
        width: w,
        height: h,
        xhot,
        yhot,
        delay_ms: 50,
        rgba: img.into_data().to_vec(),
    }])
}

/// .icns 没有热点信息：指针类取左上角附近，其余取中心。
fn default_hotspot(role: &str, w: u32, h: u32) -> (u32, u32) {
    if matches!(role, "left_ptr" | "top_left_arrow" | "center_ptr") {
        let v = (w / 16).max(1);
        (v, v)
    } else {
        (w / 2, h / 2)
    }
}

// ─────────────────────────── ICO / CUR 解码 ───────────────────────────

const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn le16(d: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([d[o], d[o + 1]]) as u32
}

fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// 解码 .cur/.ico（所有条目；CUR 的热点取自目录项的 planes/bitcount 字段）。
pub fn decode_ico(bytes: &[u8]) -> Result<Vec<Frame>, String> {
    if bytes.len() < 6 {
        return Err("文件太小".into());
    }
    let count = le16(bytes, 4);
    let mut frames = vec![];
    for i in 0..count as usize {
        let e = 6 + 16 * i;
        if e + 16 > bytes.len() {
            break;
        }
        let bw = u32::from(bytes[e]);
        let bh = u32::from(bytes[e + 1]);
        // CUR：planes = xhot，bitcount = yhot
        let xhot = le16(bytes, e + 4);
        let yhot = le16(bytes, e + 6);
        let size = le32(bytes, e + 8) as usize;
        let off = le32(bytes, e + 12) as usize;
        if off + size > bytes.len() || size == 0 {
            continue;
        }
        let data = &bytes[off..off + size];
        let f = if data.starts_with(&PNG_SIG) {
            decode_png(data, xhot, yhot)?
        } else {
            decode_bmp(data, bw, bh, xhot, yhot)?
        };
        frames.push(f);
    }
    if frames.is_empty() {
        Err("没有图像数据".into())
    } else {
        Ok(frames)
    }
}

fn decode_png(data: &[u8], xhot: u32, yhot: u32) -> Result<Frame, String> {
    let img = image::load_from_memory(data).map_err(|e| format!("PNG 解码失败: {e}"))?;
    let rgba = img.to_rgba8();
    Ok(Frame {
        width: rgba.width(),
        height: rgba.height(),
        xhot: xhot.min(rgba.width().saturating_sub(1)),
        yhot: yhot.min(rgba.height().saturating_sub(1)),
        delay_ms: 50,
        rgba: rgba.into_raw(),
    })
}

/// 解码 ICO 内嵌 BMP（BITMAPINFOHEADER + XOR + AND mask，自底向上）。
fn decode_bmp(data: &[u8], _entry_w: u32, entry_h: u32, xhot: u32, yhot: u32) -> Result<Frame, String> {
    if data.len() < 40 {
        return Err("BMP 数据太小".into());
    }
    let bisize = le32(data, 0) as usize;
    let width = (le32(data, 4) as i32).unsigned_abs();
    let biheight = (le32(data, 8) as i32).unsigned_abs();
    let bitcount = le16(data, 14);
    let clrused = le32(data, 32);
    if width == 0 {
        return Err("BMP 宽度为 0".into());
    }
    // ICO 的 BMP 高度是 XOR+AND 两倍高；个别包只存 XOR（biheight == 单倍）
    let h = if entry_h != 0 && biheight == entry_h {
        biheight
    } else if biheight >= 2 {
        biheight / 2
    } else {
        biheight
    };
    if h == 0 {
        return Err("BMP 高度为 0".into());
    }
    let bpp = match bitcount {
        1 | 4 | 8 | 24 | 32 => bitcount as usize,
        _ => return Err(format!("不支持的位深 {bitcount}")),
    };
    let pal_n = if bpp <= 8 {
        if clrused != 0 {
            clrused as usize
        } else {
            1 << bpp
        }
    } else {
        0
    };
    let pal_off = bisize.min(data.len());
    let xor_off = pal_off.saturating_add(pal_n * 4);
    let xor_stride = ((width as usize * bpp + 31) / 32) * 4;
    let and_stride = ((width as usize + 31) / 32) * 4;
    let and_off = xor_off + xor_stride * h as usize;
    let has_mask = and_off + and_stride * h as usize <= data.len();

    let mut mask = vec![false; (width * h) as usize];
    if has_mask {
        for row in 0..h as usize {
            let line = &data[and_off + row * and_stride..and_off + row * and_stride + and_stride];
            for x in 0..width as usize {
                let bit = (line[x / 8] >> (7 - x % 8)) & 1;
                mask[(h as usize - 1 - row) * width as usize + x] = bit == 1;
            }
        }
    }

    let mut rgba = vec![0u8; (width * h * 4) as usize];
    let mut any_alpha = false;
    for row in 0..h as usize {
        let src_row = xor_off + row * xor_stride;
        let dst_row = (h as usize - 1 - row) * width as usize * 4;
        for x in 0..width as usize {
            let i = dst_row + x * 4;
            match bpp {
                32 => {
                    let s = src_row + x * 4;
                    rgba[i] = *data.get(s + 2).ok_or("像素越界")?;
                    rgba[i + 1] = *data.get(s + 1).ok_or("像素越界")?;
                    rgba[i + 2] = *data.get(s).ok_or("像素越界")?;
                    rgba[i + 3] = *data.get(s + 3).ok_or("像素越界")?;
                }
                24 => {
                    let s = src_row + x * 3;
                    rgba[i] = *data.get(s + 2).ok_or("像素越界")?;
                    rgba[i + 1] = *data.get(s + 1).ok_or("像素越界")?;
                    rgba[i + 2] = *data.get(s).ok_or("像素越界")?;
                    rgba[i + 3] = 255;
                }
                _ => {
                    let pi = read_bits(data, src_row, x, bpp)? as usize * 4;
                    let p = pal_off + pi;
                    if p + 4 <= data.len() {
                        rgba[i] = data[p + 2];
                        rgba[i + 1] = data[p + 1];
                        rgba[i + 2] = data[p];
                    }
                    rgba[i + 3] = 255;
                }
            }
            if rgba[i + 3] != 0 {
                any_alpha = true;
            }
        }
    }
    // 32bpp 但 alpha 全 0：老式光标，透明度在 AND mask 里
    if bpp == 32 && !any_alpha {
        for (i, t) in mask.iter().enumerate() {
            rgba[i * 4 + 3] = if *t { 0 } else { 255 };
        }
    } else if bpp != 32 && has_mask {
        // 调色板图：AND mask 决定透明
        for (i, t) in mask.iter().enumerate() {
            if *t {
                rgba[i * 4 + 3] = 0;
            }
        }
    }

    Ok(Frame {
        width,
        height: h,
        xhot: xhot.min(width - 1),
        yhot: yhot.min(h - 1),
        delay_ms: 50,
        rgba,
    })
}

fn read_bits(data: &[u8], row_off: usize, x: usize, bpp: usize) -> Result<u8, String> {
    let bit_pos = x * bpp;
    let byte = *data.get(row_off + bit_pos / 8).ok_or("像素越界")?;
    let shift = 8 - bpp - (bit_pos % 8);
    Ok((byte >> shift) & ((1u16 << bpp) - 1) as u8)
}

// ─────────────────────────── ANI 解码 ───────────────────────────

pub fn decode_ani(bytes: &[u8]) -> Result<Vec<Frame>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"ACON" {
        return Err("不是 ANI 文件".into());
    }
    let mut icons: Vec<Vec<u8>> = vec![];
    let mut rates: Vec<u32> = vec![];
    let mut jif_rate = 10u32;
    let mut jif_scale = 60u32;

    let end = (le32(bytes, 4) as usize + 8).min(bytes.len());
    let mut pos = 12usize;
    while pos + 8 <= end {
        let id = &bytes[pos..pos + 4];
        let size = le32(bytes, pos + 4) as usize;
        let body = pos + 8;
        if body > bytes.len() {
            break;
        }
        let stop = (body + size).min(bytes.len());
        match id {
            b"anih" => {
                if size >= 32 {
                    jif_rate = le32(bytes, body + 24);
                    jif_scale = le32(bytes, body + 28);
                }
            }
            b"rate" => {
                rates = (0..size / 4).filter_map(|i| {
                    let o = body + i * 4;
                    (o + 4 <= stop).then(|| le32(bytes, o))
                }).collect();
            }
            b"LIST" => {
                if stop >= body + 4 && &bytes[body..body + 4] == b"fram" {
                    let mut sp = body + 4;
                    while sp + 8 <= stop {
                        let sid = &bytes[sp..sp + 4];
                        let ssize = le32(bytes, sp + 4) as usize;
                        let sbody = sp + 8;
                        if sbody > stop {
                            break;
                        }
                        let sstop = (sbody + ssize).min(stop);
                        if sid == b"icon" {
                            icons.push(bytes[sbody..sstop].to_vec());
                        }
                        sp = sbody + ssize + (ssize % 2);
                    }
                }
            }
            _ => {}
        }
        pos = body + size + (size % 2);
    }

    let scale = if jif_scale == 0 { 60 } else { jif_scale };
    let default_ms = ((u64::from(jif_rate) * 1000) / u64::from(scale)) as u32;
    let mut frames = vec![];
    for (i, ico) in icons.iter().enumerate() {
        let mut imgs = decode_ico(ico)?;
        // 每帧取最大的一张
        imgs.sort_by_key(|f| f.width * f.height);
        let mut f = imgs.pop().ok_or("ANI 帧为空")?;
        let ms = rates
            .get(i)
            .copied()
            .map(|r| ((u64::from(r)) * 1000 / u64::from(scale)) as u32)
            .unwrap_or(default_ms);
        f.delay_ms = ms.max(10);
        frames.push(f);
    }
    if frames.is_empty() {
        Err("ANI 中没有帧".into())
    } else {
        Ok(frames)
    }
}

// ─────────────────────────── XCursor 写出 ───────────────────────────

const XCUR_MAGIC: u32 = 0x7275_6358; // 字节序列 "Xcur"（按小端读出）
const CHUNK_IMAGE: u32 = 0xfffd_0002;

/// 写一个 XCursor 文件。名义尺寸取图像宽度；同名义尺寸的多帧按文件
/// 顺序组成动画序列（libXcursor 语义）。
pub fn write_xcursor(frames: &[Frame], path: &Path) -> Result<(), String> {
    if frames.is_empty() {
        return Err("没有帧可写".into());
    }
    let mut out: Vec<u8> = Vec::with_capacity(64 * 1024);
    let n = frames.len() as u32;
    out.extend_from_slice(&XCUR_MAGIC.to_le_bytes());
    out.extend_from_slice(&16u32.to_le_bytes()); // file header size
    out.extend_from_slice(&(1u32 << 16).to_le_bytes()); // version 1.0
    out.extend_from_slice(&n.to_le_bytes());
    let mut pos = 16 + 12 * n;
    for f in frames {
        out.extend_from_slice(&CHUNK_IMAGE.to_le_bytes());
        out.extend_from_slice(&f.width.to_le_bytes()); // subtype = nominal
        out.extend_from_slice(&pos.to_le_bytes());
        pos += 36 + f.width * f.height * 4;
    }
    for f in frames {
        if f.rgba.len() < (f.width * f.height * 4) as usize {
            return Err(format!("帧像素数据不完整（{}x{}）", f.width, f.height));
        }
        let xhot = f.xhot.min(f.width.saturating_sub(1));
        let yhot = f.yhot.min(f.height.saturating_sub(1));
        out.extend_from_slice(&36u32.to_le_bytes());
        out.extend_from_slice(&CHUNK_IMAGE.to_le_bytes());
        out.extend_from_slice(&f.width.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&f.width.to_le_bytes());
        out.extend_from_slice(&f.height.to_le_bytes());
        out.extend_from_slice(&xhot.to_le_bytes());
        out.extend_from_slice(&yhot.to_le_bytes());
        out.extend_from_slice(&f.delay_ms.max(1).to_le_bytes());
        for px in f.rgba.chunks_exact(4) {
            // RGBA(直通) → 文件里的 CARD32 ARGB，小端
            let argb = (u32::from(px[3]) << 24)
                | (u32::from(px[0]) << 16)
                | (u32::from(px[1]) << 8)
                | u32::from(px[2]);
            out.extend_from_slice(&argb.to_le_bytes());
        }
    }
    fs::write(path, &out).map_err(|e| format!("写 {} 失败: {e}", path.display()))?;
    // 往返自校验
    xcursor::parser::parse_xcursor(&out)
        .ok_or_else(|| format!("写入的 {} 未通过自校验", path.display()))?;
    Ok(())
}

// ─────────────────────────── 测试 ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x4 32bpp BMP 光标。BMP 自底向上：存储行 0 = 图像最底行。
    /// alpha 全 0 时靠 AND mask 兜底（存储行 3 的 x0 位 → 图像左上角透明）。
    fn bmp_cursor_bytes(all_alpha_zero: bool) -> Vec<u8> {
        let mut d = vec![];
        // BITMAPINFOHEADER
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&4i32.to_le_bytes());
        d.extend_from_slice(&8i32.to_le_bytes()); // 2 倍高
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        d.extend_from_slice(&[0u8; 20]); // sizeimage, xpels, ypels, clrused, clrimportant
        // XOR：自底向上 4 行，每行 16 字节（BGRA）
        for row in 0..4u8 {
            for x in 0..4u8 {
                let a = if all_alpha_zero { 0 } else { 255 };
                d.extend_from_slice(&[x * 10, row * 10, 200, a]);
            }
        }
        // AND mask：4 行 x 1bpp（每行 4 字节）；bit=1 透明
        for row in 0..4u8 {
            let m = if row == 3 { 0b1000_0000 } else { 0 };
            d.extend_from_slice(&[m, 0, 0, 0]);
        }
        // ICONDIR + entry
        let mut ico = vec![];
        ico.extend_from_slice(&0u16.to_le_bytes());
        ico.extend_from_slice(&2u16.to_le_bytes()); // CUR
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.push(4); // w
        ico.push(4); // h
        ico.push(0);
        ico.push(0);
        ico.extend_from_slice(&2u16.to_le_bytes()); // xhot=2
        ico.extend_from_slice(&1u16.to_le_bytes()); // yhot=1
        ico.extend_from_slice(&(d.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes()); // 6+16
        ico.extend_from_slice(&d);
        ico
    }

    #[test]
    fn decode_bmp_cursor_with_alpha() {
        let frames = decode_ico(&bmp_cursor_bytes(false)).unwrap();
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert_eq!((f.width, f.height), (4, 4));
        assert_eq!((f.xhot, f.yhot), (2, 1));
        // 图像顶行 y=0 = 存储行 3：B=x*10=0, G=30, R=200
        assert_eq!(&f.rgba[0..4], &[200, 30, 0, 255]);
        // 顶行 x=3：B=30
        assert_eq!(&f.rgba[3 * 4..3 * 4 + 4], &[200, 30, 30, 255]);
        // 图像底行 y=3 = 存储行 0：G=0
        assert_eq!(&f.rgba[48..52], &[200, 0, 0, 255]);
    }

    #[test]
    fn decode_bmp_cursor_mask_fallback() {
        // alpha 全 0：透明度由 AND mask 决定（存储行 3 = 图像顶行，x0 透明）
        let frames = decode_ico(&bmp_cursor_bytes(true)).unwrap();
        let f = &frames[0];
        assert_eq!(f.rgba[3], 0); // 图像左上角：mask=1 → 透明
        assert_eq!(f.rgba[7], 255); // 顶行 x1：mask=0 → 不透明
        assert_eq!(f.rgba[48 + 3], 255); // 图像底行：mask=0 → 不透明
    }

    #[test]
    fn ani_roundtrip() {
        let ico = bmp_cursor_bytes(false);
        let mut body = vec![];
        body.extend_from_slice(b"anih");
        body.extend_from_slice(&36u32.to_le_bytes());
        body.extend_from_slice(&36u32.to_le_bytes()); // cbSize
        body.extend_from_slice(&2u32.to_le_bytes()); // nFrames
        body.extend_from_slice(&[0u8; 16]); // nSteps,cx,cy,cBitCount
        body.extend_from_slice(&5u32.to_le_bytes()); // jifRate=5
        body.extend_from_slice(&60u32.to_le_bytes()); // jifScale=60
        body.extend_from_slice(&0u32.to_le_bytes()); // flags
        // LIST fram + 两个 icon
        let mut fram = b"fram".to_vec();
        for _ in 0..2 {
            fram.extend_from_slice(b"icon");
            fram.extend_from_slice(&(ico.len() as u32).to_le_bytes());
            fram.extend_from_slice(&ico);
            if ico.len() % 2 == 1 {
                fram.push(0);
            }
        }
        body.extend_from_slice(b"LIST");
        body.extend_from_slice(&(fram.len() as u32).to_le_bytes());
        body.extend_from_slice(&fram);
        if body.len() % 2 == 1 {
            body.push(0);
        }

        let mut ani = vec![];
        ani.extend_from_slice(b"RIFF");
        ani.extend_from_slice(&(body.len() as u32).to_le_bytes());
        ani.extend_from_slice(b"ACON");
        ani.extend_from_slice(&body);

        let frames = decode_ani(&ani).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].delay_ms, 83); // 5 jiff * 1000 / 60
    }

    #[test]
    fn xcursor_write_roundtrip() {
        let f1 = Frame {
            width: 4,
            height: 4,
            xhot: 2,
            yhot: 1,
            delay_ms: 100,
            rgba: (0..16).flat_map(|i| [i, 0, 0, 255]).collect(),
        };
        let mut f2 = f1.clone();
        f2.width = 8;
        f2.height = 8;
        f2.rgba = vec![7u8, 8, 9, 255].repeat(64);
        let frames = vec![f1, f2];
        let tmp = std::env::temp_dir().join(format!("cm-test-{}.cur", std::process::id()));
        write_xcursor(&frames, &tmp).unwrap();
        let bytes = fs::read(&tmp).unwrap();
        let _ = fs::remove_file(&tmp);
        let imgs = xcursor::parser::parse_xcursor(&bytes).unwrap();
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0].size, 4);
        assert_eq!(imgs[1].size, 8);
        assert_eq!((imgs[0].xhot, imgs[0].yhot), (2, 1));
        // 第一像素：文件里 ARGB BE 语义 → parser 的 pixels_rgba 顺序 R,G,B,A
        assert_eq!(imgs[0].pixels_rgba[0], 0);
        assert_eq!(imgs[0].pixels_rgba[2], 0);
        assert_eq!(imgs[0].pixels_rgba[3], 255);
    }

    #[test]
    fn role_matching() {
        assert_eq!(match_role("aero_arrow.cur"), Some("left_ptr"));
        assert_eq!(match_role("appstarting"), Some("left_ptr_watch"));
        assert_eq!(match_role("left_ptr"), Some("left_ptr"));
        assert_eq!(match_role("sizens"), Some("sb_v_double_arrow"));
        assert_eq!(match_role("whatever"), None);
    }

    #[test]
    fn uninstall_guards() {
        use std::os::unix::fs::symlink;
        let root = home().join(".local/share/icons");
        // 用户目录：可卸载
        let dummy = root.join(format!("Test-CM-Uninstall-{}", std::process::id()));
        fs::create_dir_all(dummy.join("cursors")).unwrap();
        fs::write(dummy.join("cursors/left_ptr"), b"fake").unwrap();
        uninstall(&dummy).unwrap();
        assert!(!dummy.exists());
        // 系统目录：拒绝
        assert!(uninstall(Path::new("/usr/share/icons/Breeze_Light")).is_err());
        // 符号链接：只删链接，目标保留
        let target = root.join(format!("Test-CM-Target-{}", std::process::id()));
        fs::create_dir_all(&target).unwrap();
        let link = root.join(format!("Test-CM-Link-{}", std::process::id()));
        symlink(&target, &link).unwrap();
        uninstall(&link).unwrap();
        assert!(!link.exists());
        assert!(target.exists());
        fs::remove_dir_all(&target).unwrap();
    }
}

