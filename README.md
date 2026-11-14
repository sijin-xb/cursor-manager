# cursor-manager

基于 [GPUI Kit](https://gpui-kit.com) 的 Linux 光标主题管理器。预览、应用、安装、卸载、在线下载 XCursor 光标主题，一次同步六处配置。

![tech](https://img.shields.io/badge/GPUI%20Kit-0.9-blue) ![license](https://img.shields.io/badge/license-MIT-green)

## 功能

- **预览**：六种形状（箭头/手型/文本/忙碌/移动/禁止）实时渲染，支持多帧动画、热点标记、16–64 尺寸切换
- **应用**：一次同步 default 继承 / gsettings(GTK) / environment.d / niri / Hyprland / Xresources，niri 下自动热重载（含 socket 失效回退）
- **安装**：zip / tar.xz / tar.gz / 目录 / Windows `.cur` `.ani` `.inf`（自动解码含 AND-mask 兜底）/ macOS `.icns`（热点估算）；GitHub 多主题包（如 Bibata 全家桶）整包安装
- **在线**：策展的 GitHub 光标主题目录（Bibata、macOS、Catppuccin、WhiteSur、Capitaine 等），一键拉取最新 release
- **卸载**：仅限用户目录，正在使用时拦截，符号链接只删链接本身
- **其它**：搜索过滤、键盘导航（↑↓ 选择 / Enter 应用 / Delete 卸载）、撤销上次应用、形状角色清单、导出 zip、行内指针预览

## 构建

```bash
./install.sh   # cargo build --release + 安装到 ~/.local/bin + 注册 .desktop
```

## niri 用户必读

gpui 的 Wayland 后端假设只有一个输入座位（wl_seat），而 niri 会额外暴露一个
`niri-agent` 座位，导致应用绑定到幽灵座位、指针事件全部丢失（渲染正常、点击无响应）。

解决办法：给应用设置 `app_id = "cursor-manager"`（已内置），并在 niri 配置中：

```kdl
agent {
    hide-seat-from "noctalia" "xwayland-satellite" "cursor-manager"
}
```

## 配置覆盖范围

| 来源 | 路径 |
| --- | --- |
| X11 fallback | `~/.icons/default/index.theme` |
| GTK | `gsettings org.gnome.desktop.interface cursor-*` |
| systemd/dbus | `~/.config/environment.d/cursor.conf` |
| niri | `~/.config/niri/**/*.kdl`（自动发现含 `xcursor-theme` 的文件）|
| Hyprland | `~/.config/hypr/custom/env.lua` + `~/.cache/cursor_theme` |
| Xresources | `~/.Xresources`（已有条目时改写并 `xrdb -merge`）|

## License

MIT
