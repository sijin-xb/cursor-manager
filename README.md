# cursor-manager

Linux 桌面光标主题管理器（GUI）。预览、应用、安装、卸载、在线下载 XCursor 主题。Rust + GPUI Kit，适用于 niri、Hyprland、GNOME 等 Wayland/X11 桌面。

![GPUI Kit](https://img.shields.io/badge/GPUI%20Kit-0.9-blue) ![license](https://img.shields.io/badge/license-MIT-green)

## 安装

需要 Rust 工具链（edition 2024，Rust ≥ 1.85）。

```bash
git clone https://github.com/sijin-xb/cursor-manager.git
cd cursor-manager
./install.sh
```

`install.sh` 构建 release 版本，装到 `~/.local/bin/cursor-manager`，并注册 `.desktop` 入口。之后从应用菜单启动，或直接运行：

```bash
cursor-manager
```

程序不接受命令行参数，启动即打开窗口。

## 快速开始

1. 左侧「本地」列出 `~/.local/share/icons`、`~/.icons`、`/usr/share/icons` 下的光标主题。
2. 点击主题，右侧预览六种形状（箭头 / 手型 / 文本 / 忙碌 / 移动 / 禁止），多帧动画自动播放。
3. 选尺寸：预设 16 / 24 / 32 / 48 / 64，或拖动滑块选 8–128。
4. 点「应用」或按 Enter。

## 用法

| 操作 | 方式 |
| --- | --- |
| 选择主题 | 点击行，或 ↑ / ↓ |
| 应用 | 「应用」按钮，或 Enter |
| 卸载 | 点「卸载」，再点「确认卸载？」 |
| 安装 | 「安装…」选择文件或目录 |
| 撤销上次应用 | 「撤销」按钮（需至少两次应用记录） |
| 导出 | 「导出 zip」→ `~/Downloads/<名称>.zip`，仅限用户目录下的主题 |

键盘导航在「在线」标签和搜索框获得焦点时禁用。

### 安装来源

- 压缩包：`.zip` `.tar.xz` `.txz` `.tar.gz` `.tgz` `.tar.bz2` `.tar`
- 目录：含 `cursors/` 的目录或主题包（自动识别多主题包）
- 单文件：`.cur` `.ani` `.icns`
- 无扩展名时按文件头嗅探格式

Windows 格式的 `.inf` 不单独安装，只在包内用于映射光标角色。`.icns` 无热点数据，热点为估算值。未识别角色的文件会被跳过并列出。

### 在线源

「在线」标签有两个子源：

- **GNOME-Look**（默认）：pling OCS API，支持搜索、按评分 / 最新排序、分页、缩略图预览。
- **GitHub 策展**：11 个主题仓库（Bibata、macOS、BreezeX、Catppuccin、WhiteSur、Qogir、McMojave、Capitaine、phinger、Nordzy、Volantes），拉取各自最新 release 中可安装的资产。

在线安装的下载文件在系统临时目录，安装后不自动清理。

## 应用写入的配置

「应用」会依次尝试以下全部位置，不检测当前桌面环境；单个目标失败只记录结果，不中断其余目标。

| 目标 | 路径 |
| --- | --- |
| X11 fallback | `~/.icons/default/index.theme`（改写 `Inherits=`）|
| GTK | `gsettings org.gnome.desktop.interface cursor-theme` / `cursor-size` |
| systemd / dbus | `~/.config/environment.d/cursor.conf` |
| niri | `~/.config/niri/**/*.kdl` 中含 `xcursor-theme` / `XCURSOR_THEME` 的文件 |
| Hyprland | `~/.config/hypr/custom/env.lua`、`~/.config/hypr/hyprland/execs.lua`、`~/.cache/cursor_theme` |
| Xresources | `~/.Xresources`（仅当已有 `Xcursor.theme` 条目时改写，并执行 `xrdb -merge`）|

niri 配置有改动时自动执行 `niri msg action load-config-file`；继承的 `NIRI_SOCKET` 失效时，回退扫描 `$XDG_RUNTIME_DIR` 下最新的 `niri.*.sock` 重试。

## 外部依赖

按需调用，缺失时对应目标标记为跳过：

| 命令 | 用途 |
| --- | --- |
| `tar` | 解压 tar 系压缩包 |
| `gsettings` | GTK 主题 |
| `niri` | niri 配置热重载 |
| `xrdb` | Xresources 合并 |
| `xdg-open` | 打开主题详情页 |

## 常见问题

**niri 下窗口能渲染但点击无响应**

gpui 的 Wayland 后端假设只有一个输入座位，而 niri 额外暴露 `niri-agent` 座位，应用会绑定到幽灵座位。应用已内置 `app_id = "cursor-manager"`，在 niri 配置中隐藏该座位即可：

```kdl
agent {
    hide-seat-from "noctalia" "xwayland-satellite" "cursor-manager"
}
```

**应用后当前会话光标没变**

`environment.d` 只对新会话生效。GTK 应用由 gsettings 立即生效；niri 会热重载；Hyprland 需重启或重载配置。

**`.Xresources` 没被修改**

文件不存在或没有 `Xcursor.theme` 条目时跳过，避免创建无意义配置。

**系统主题卸载失败**

只允许卸载 `~/.local/share/icons` 和 `~/.icons` 下的主题，系统包请用包管理器。正在使用的主题会被拦截。符号链接只删除链接本身。

## 文件位置

| 用途 | 路径 |
| --- | --- |
| 撤销历史 | `~/.local/state/cursor-manager/history.json`（最多 10 条，仅主题名，不含尺寸）|
| 缩略图缓存 | `~/.cache/cursor-manager/thumbs/` |
| 安装目标 | `~/.local/share/icons` |
| 导出 | `~/Downloads/<名称>.zip` |

## License

MIT
