#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# release.sh — 构建 codel release 版本并安装到本地，可在任意目录直接 `codel` 运行
# 用法:
#   ./release.sh                  # release 构建并安装到 ~/.local/bin/codel
#   ./release.sh --prefix /usr/local/bin   # 安装到指定目录（可能需要 sudo）
#   ./release.sh --all            # 同时构建 workspace-server
#   ./release.sh --no-install     # 只构建，不安装
# ============================================================================

PREFIX="${HOME}/.local/bin"
INSTALL_NAME="codel"
BUILD_ALL=false
DO_INSTALL=true

while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix)
            PREFIX="$2"; shift 2 ;;
        --prefix=*)
            PREFIX="${1#*=}"; shift ;;
        --all)
            BUILD_ALL=true; shift ;;
        --no-install)
            DO_INSTALL=false; shift ;;
        -h|--help)
            echo "Usage: ./release.sh [--prefix DIR] [--all] [--no-install]"
            echo "  --prefix DIR   安装目录（默认 ~/.local/bin，需在 PATH 中）"
            echo "  --all          同时构建 codel-workspace-server"
            echo "  --no-install   只构建，不安装到本地"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

cd "$(dirname "$0")"

# --- 复用 build.sh 做 release 构建 -----------------------------------------

echo "🚀 开始 release 构建..."
if [[ "$BUILD_ALL" == "true" ]]; then
    ./build.sh --all
else
    ./build.sh
fi

BINARY="target/release/codel-pager"
if [[ ! -x "$BINARY" ]]; then
    echo "❌ 构建产物不存在: $BINARY"
    exit 1
fi

echo ""
echo "📦 构建产物: $BINARY ($(du -h "$BINARY" | cut -f1))"

# --- 安装 -------------------------------------------------------------------

if [[ "$DO_INSTALL" != "true" ]]; then
    echo "⏭️  跳过安装（--no-install）"
    exit 0
fi

mkdir -p "$PREFIX"
DEST="${PREFIX}/${INSTALL_NAME}"

# 若目标目录不可写，提示用 sudo 重试
USE_SUDO=false
if [[ ! -w "$PREFIX" ]]; then
    echo "⚠️  目录 $PREFIX 不可写，尝试使用 sudo 安装..."
    USE_SUDO=true
    sudo cp "$BINARY" "$DEST"
    sudo chmod +x "$DEST"
else
    cp "$BINARY" "$DEST"
    chmod +x "$DEST"
fi

# --- macOS 代码签名修复 -----------------------------------------------------
# cp 复制的二进制保留 linker-signed 签名，macOS taskgated 会判定无效并 SIGKILL。
# 清除扩展属性（quarantine/provenance/EDR 标记）并重新做正式 adhoc 签名以通过校验。
if [[ "$(uname)" == "Darwin" ]]; then
    echo "🔏 修复 macOS 代码签名..."
    if [[ "$USE_SUDO" == "true" ]]; then
        sudo xattr -cr "$DEST" 2>/dev/null || true
        sudo codesign --force --sign - "$DEST" 2>/dev/null || true
    else
        xattr -cr "$DEST" 2>/dev/null || true
        codesign --force --sign - "$DEST" 2>/dev/null || true
    fi
fi

echo ""
echo "✅ 已安装到: $DEST"

# --- PATH 检查 --------------------------------------------------------------

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$PREFIX"; then
    echo ""
    echo "⚠️  注意: $PREFIX 不在 PATH 中。请将以下行加入你的 shell 配置（~/.zshrc 或 ~/.bashrc）:"
    echo "    export PATH=\"$PREFIX:\$PATH\""
fi

echo ""
echo "🎉 现在可以在任意目录运行:"
echo "    codel --help"
