#!/usr/bin/env bash
# 构建并安装到 ~/.local/bin，注册 .desktop 入口
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"
cp target/release/cursor-manager "$HOME/.local/bin/cursor-manager"
cat > "$HOME/.local/share/applications/cursor-manager.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Cursor Manager
Name[zh_CN]=光标管理器
Comment=Preview, apply, install and download XCursor cursor themes
Exec=$HOME/.local/bin/cursor-manager
Icon=input-mouse
Terminal=false
Categories=Utility;Settings;
StartupWMClass=cursor-manager
DESKTOP
echo "installed: $HOME/.local/bin/cursor-manager"
