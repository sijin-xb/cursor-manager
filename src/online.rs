//! 在线主题目录：策展的 GitHub 光标主题仓库 + Releases API + 下载。
//!
//! v1 只做 GitHub（资产规整、无需登录）；GNOME-Look / KDE Store 走
//! 「安装」功能吃手动下载的 zip/tar。

use std::path::Path;

pub struct OnlineTheme {
    pub name: &'static str,
    pub repo: &'static str,
    pub desc: &'static str,
}

pub const CATALOG: &[OnlineTheme] = &[
    OnlineTheme { name: "Bibata", repo: "ful1e5/Bibata_Cursor", desc: "最流行的现代光标，多种配色与形态" },
    OnlineTheme { name: "macOS", repo: "ful1e5/apple_cursor", desc: "macOS 原生风格光标" },
    OnlineTheme { name: "BreezeX", repo: "ful1e5/BreezeX_Cursor", desc: "KDE Breeze 的现代重制版" },
    OnlineTheme { name: "Catppuccin", repo: "catppuccin/cursors", desc: "Catppuccin 全 flavor × accent 配色" },
    OnlineTheme { name: "WhiteSur", repo: "vinceliuice/WhiteSur-cursors", desc: "macOS Big Sur 风格" },
    OnlineTheme { name: "Qogir", repo: "vinceliuice/Qogir-cursors", desc: "扁平化 Win10 风格" },
    OnlineTheme { name: "McMojave", repo: "vinceliuice/McMojave-cursors", desc: "macOS Mojave 风格" },
    OnlineTheme { name: "Capitaine", repo: "keeferrourke/capitaine-cursors", desc: "macOS 灵感的描边光标" },
    OnlineTheme { name: "phinger", repo: "phisch/phinger-cursors", desc: "简约高定制光标" },
    OnlineTheme { name: "Nordzy", repo: "guillaumeboehm/Nordzy-cursors", desc: "Nord 配色光标" },
    OnlineTheme { name: "Volantes", repo: "varlesh/volantes-cursors", desc: "经典飞镖风格" },
];

pub struct ReleaseAssets {
    pub tag: String,
    /// (资产名, 下载 URL)，已过滤并按「Linux 优先」排序
    pub assets: Vec<(String, String)>,
}

/// 拉取仓库最新 release 的资产列表。
pub fn latest_assets(repo: &str) -> Result<ReleaseAssets, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let text = ureq::get(&url)
        .set("User-Agent", "cursor-manager")
        .call()
        .map_err(|e| format!("请求失败: {e}"))?
        .into_string()
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("JSON 解析失败: {e}"))?;
    if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
        return Err(format!("GitHub: {msg}"));
    }
    let tag = v["tag_name"].as_str().unwrap_or("?").to_string();
    let mut assets: Vec<(String, String)> = Vec::new();
    if let Some(list) = v["assets"].as_array() {
        for a in list {
            let name = a["name"].as_str().unwrap_or("").to_string();
            let url = a["browser_download_url"].as_str().unwrap_or("").to_string();
            if !name.is_empty() && !url.is_empty() {
                assets.push((name, url));
            }
        }
    }
    assets.sort_by_key(|(name, _)| asset_rank(name));
    Ok(ReleaseAssets { tag, assets })
}

/// 排序键：越小越优先。Windows 版/位图源码包沉底，tar.xz > tar.gz > zip，
/// 普通版优先于 -Right 左右手变体。
fn asset_rank(name: &str) -> u32 {
    let l = name.to_lowercase();
    let mut rank = 0u32;
    if l.contains("windows") {
        rank += 1000;
    }
    if l.contains("bitmaps") || l.ends_with(".svg") {
        rank += 900;
    }
    if l.ends_with(".tar.xz") || l.ends_with(".txz") {
        rank += 0;
    } else if l.ends_with(".tar.gz") || l.ends_with(".tgz") {
        rank += 10;
    } else if l.ends_with(".zip") {
        rank += 20;
    } else {
        rank += 500; // deb/rpm/其它一律沉底
    }
    if l.contains("-right") {
        rank += 3;
    }
    rank
}

/// 是否是本程序能处理的包格式（交给 install 模块）。
pub fn is_installable_asset(name: &str) -> bool {
    let l = name.to_lowercase();
    let rank = asset_rank(name);
    rank < 500
        && (l.ends_with(".tar.xz")
            || l.ends_with(".txz")
            || l.ends_with(".tar.gz")
            || l.ends_with(".tgz")
            || l.ends_with(".zip"))
}

/// 流式下载到目标文件，返回字节数。
pub fn download_to(url: &str, dest: &Path) -> Result<u64, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "cursor-manager")
        .call()
        .map_err(|e| format!("下载失败: {e}"))?;
    let mut reader = resp.into_reader();
    let mut f = std::fs::File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;
    let n = std::io::copy(&mut reader, &mut f).map_err(|e| format!("写入失败: {e}"))?;
    Ok(n)
}
