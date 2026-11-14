# 使用说明

## 界面操作

| 操作 | 方式 |
| --- | --- |
| 选择主题 | 点击行，或 `↑` / `↓`（循环）|
| 应用 | 「应用」按钮，或 `Enter` |
| 卸载 | 点「卸载」，再点「确认卸载？」|
| 安装 | 「安装…」选择文件或目录 |
| 撤销上次应用 | 「撤销」按钮，需至少两次应用记录 |
| 导出 | 「导出 zip」→ `~/Downloads/<名称>.zip`，仅限用户目录下的主题 |
| 切换标签 | 点击「本地」/「在线」|
| 刷新主题列表 | 「刷新」|

键盘导航在「在线」标签和搜索框获得焦点时禁用。窗口最小 820×560。

### 预览

选中主题后右侧显示六种形状：箭头、手型、文本、忙碌、移动、禁止。多帧动画自动播放（40 ms 一帧），静态主题不重绘。

尺寸用预设按钮（16 / 24 / 32 / 48 / 64）或滑块（8–128）。滑块拖动时实时更新，松手后重新解码预览图。

### 撤销

只记录主题名，不含尺寸，最多 10 条，存在 `~/.local/state/cursor-manager/history.json`。

撤销会把上一个主题按**当前尺寸**重新应用；如果该主题已被卸载则报错。撤销本身也会写入历史，所以连续撤销是在最后两个主题间来回切换。

## 安装来源

### 压缩包

`.zip` `.tar.xz` `.txz` `.tar.gz` `.tgz` `.tar.bz2` `.tar`

无扩展名或扩展名不识别时按文件头嗅探（gzip / xz / bzip2 / zip / tar）。

zip 在进程内解压并做路径穿越检查；tar 系交给系统 `tar -xf`。

### 目录

直接选目录，或含 `cursors/` 子目录的多主题包。单目录下最多向下探测 3 层。

### 光标文件

- `.cur` / `.ico`：解码全部尺寸，热点从目录项读取，支持 1/4/8/24/32 bpp 内嵌 BMP（含调色板）。32bpp 全零 alpha 时回退用 AND mask 判透明。
- `.ani`：RIFF/ACON 解析，读取 `anih` 的 jifRate / jifScale 与 `rate` 块，每帧取最大图，延迟下限 10 ms。
- `.icns`：取面积最大的图标，单帧 50 ms。**没有热点数据**，`left_ptr` / `top_left_arrow` / `center_ptr` 按 `(宽/16, 宽/16)` 估算，其余取图像中心。JP2 编码的元素跳过。

`.inf` 不能单独安装，只在包内用于映射光标角色。没有 `.inf` 时按文件名关键词匹配，未识别的文件跳过并在结果里列出。

输出为标准 XCursor 二进制，写入后会用 `xcursor` 解析器回读校验。

### 命名与冲突

主题名清洗为字母数字与 `-_.`，空名回退 `Installed-Cursors`。重名追加 `-2` 到 `-99`，仍冲突则加进程号。安装失败会回滚已创建的目录。

## 在线源

「在线」标签有两个子源。

### GNOME-Look（默认）

pling OCS API（`api.pling.com`，分类 107）。支持搜索、按评分 / 最新排序、分页（每页 20）、缩略图。点击条目下载并安装；`↗` 用 `xdg-open` 打开详情页。

### GitHub 策展

固定 11 个仓库：Bibata、macOS、BreezeX、Catppuccin、WhiteSur、Qogir、McMojave、Capitaine、phinger、Nordzy、Volantes。

拉取各自 `releases/latest`，按资产名排序偏好：`.tar.xz` / `.txz` > `.tar.gz` / `.tgz` > `.zip`。含 `windows`、`bitmaps`、`.svg` 的资产被过滤掉，`.deb` / `.rpm` 同样不可选。

### 下载行为

`ureq`，超时 60 秒，UA `cursor-manager`，不带 token。文件存到系统临时目录（`/tmp/cm-dl-<pid>`），**安装后不自动清理**。

## 应用写入的配置

「应用」不检测桌面环境，依次尝试全部六个目标。单个目标失败只记录结果，不中断其余。

### X11 fallback

`~/.icons/default/index.theme`，改写 `Inherits=`。文件不存在时创建，内容为：

```ini
[Icon Theme]
Name=Default
Comment=Default Cursor Theme
Inherits=<主题名>
```

### GTK

```bash
gsettings set org.gnome.desktop.interface cursor-theme <主题名>
gsettings set org.gnome.desktop.interface cursor-size <尺寸>
```

`gsettings` 不存在时跳过。

### systemd / dbus

`~/.config/environment.d/cursor.conf`，改写 `XCURSOR_THEME=` / `XCURSOR_SIZE=`。文件不存在时创建。

**只对新会话生效。**

### niri

扫描 `~/.config/niri`（深度 ≤ 3，跳过 `backup` 目录与含 `.bak` / `.pre-inir` 的文件），只处理文本含 `xcursor-theme` 或 `XCURSOR_THEME` 的 `*.kdl`。

改写这些形式：

```kdl
xcursor-theme "主题名"
xcursor-size 32
XCURSOR_THEME="主题名"
XCURSOR_SIZE=32
XCURSOR_THEME "主题名"
XCURSOR_SIZE "32"
hl.env("XCURSOR_THEME", "主题名")
hl.env("XCURSOR_SIZE", "32")
```

`hl.env` 第二参数不是字面量时保持原样，不猜。

有文件被改动时执行 `niri msg action load-config-file`。继承的 `NIRI_SOCKET` 失效（niri 重启过）时，扫描 `$XDG_RUNTIME_DIR` 下最新的 `niri.*.sock` 重试。

### Hyprland

改写 `~/.config/hypr/custom/env.lua` 与 `~/.config/hypr/hyprland/execs.lua`，并写入 `~/.cache/cursor_theme`（内容为 `主题名\n`）。

### Xresources

仅当 `~/.Xresources` 已存在**且**含 `Xcursor.theme` 条目时改写 `Xcursor.theme:` / `Xcursor.size:`，随后执行 `xrdb -merge`。`xrdb` 不可用时只报告，不失败。

### 原子写入

所有配置改写先写 `<文件名>.cursor-manager.tmp` 再 `rename`，避免写一半留下坏文件。

## 外部依赖

按需调用，缺失时对应目标标记为跳过。

| 命令 | 用途 |
| --- | --- |
| `tar` | 解压 tar 系压缩包 |
| `gsettings` | GTK 主题 |
| `niri` | niri 配置热重载 |
| `xrdb` | Xresources 合并 |
| `xdg-open` | 打开主题详情页 |

## 卸载

只允许卸载 `~/.local/share/icons` 和 `~/.icons` 下的主题，系统包提示用包管理器。

正在使用的主题（从 gsettings 读 `cursor-theme`，回退到 `Inherits=`，`-alt` 后缀归一化）会被拦截。

符号链接只删除链接本身，不碰目标。普通目录 `remove_dir_all`，无备份、无回收站。

## 排障

### niri 下窗口能渲染但点击无响应

gpui 的 Wayland 后端假设只有一个输入座位，niri 额外暴露 `niri-agent` 座位，应用会绑定到幽灵座位，指针事件全部丢失。

应用已内置 `app_id = "cursor-manager"`，在 niri 配置中隐藏该座位：

```kdl
agent {
    hide-seat-from "noctalia" "xwayland-satellite" "cursor-manager"
}
```

### 应用后当前会话光标没变

- `environment.d` 只对新会话生效。
- GTK 应用由 gsettings 立即生效。
- niri 会热重载。
- Hyprland 需重启或重载配置。

### `.Xresources` 没被修改

文件不存在或没有 `Xcursor.theme` 条目时跳过，避免创建无意义配置。

### 卸载按钮点了没反应

需要点两次：第一次是「卸载」，第二次是「确认卸载？」。

### 主题列表里看不到某个主题

`index.theme` 里 `Hidden=true` 的主题会被排除。

## 文件位置

| 用途 | 路径 |
| --- | --- |
| 撤销历史 | `~/.local/state/cursor-manager/history.json` |
| 缩略图缓存 | `~/.cache/cursor-manager/thumbs/<id>.img`（不自动清理）|
| 安装目标 | `~/.local/share/icons` |
| 导出 | `~/Downloads/<名称>.zip` |
| 安装暂存 | `/tmp/cursor-manager-install-<pid>/`（完成后删除）|
| 在线下载 | `/tmp/cm-dl-<pid>`（**不删除**）|

## 调试

| 环境变量 | 作用 |
| --- | --- |
| `CURSOR_MGR_DEBUG=1` | 向 stderr 打印鼠标事件 |
| `CURSOR_MGR_PROBE=1` | 额外开一个诊断窗口（偏移 700px）|

两个都只用于排查 gpui / Wayland 座位问题。
