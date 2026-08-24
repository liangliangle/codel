#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# build.sh — 编译 codel 可执行产物
# 用法:
#   ./build.sh              # release 构建（默认）
#   ./build.sh --debug      # debug 构建（更快）
#   ./build.sh --all        # 构建所有二进制（含 workspace-server）
# ============================================================================

PROFILE="release"
BUILD_ALL=false

for arg in "$@"; do
    case "$arg" in
        --debug) PROFILE="debug" ;;
        --all)   BUILD_ALL=true ;;
        -h|--help)
            echo "Usage: ./build.sh [--debug] [--all]"
            echo "  --debug   使用 debug profile（编译更快，产物更大）"
            echo "  --all     构建所有二进制（codel-pager + workspace-server）"
            exit 0
            ;;
        *) echo "Unknown option: $arg"; exit 1 ;;
    esac
done

cd "$(dirname "$0")"

# --- 前置检查 ---------------------------------------------------------------

# 1. Rust toolchain
if ! command -v cargo &>/dev/null; then
    echo "❌ 未找到 cargo，请先安装 Rust: https://rustup.rs"
    exit 1
fi

echo "🔧 Rust: $(rustc --version)"

# 2. protoc（proto 编译需要）
#    优先级: $PROTOC > bin/protoc (dotslash) > 系统 protoc
if [[ -z "${PROTOC:-}" ]]; then
    if [[ -x "bin/protoc" ]]; then
        if bin/protoc --version &>/dev/null; then
            export PROTOC="$(pwd)/bin/protoc"
            echo "🔧 protoc: $PROTOC (dotslash wrapper)"
        else
            echo "⚠️  bin/protoc 存在但无法执行（可能缺少 dotslash），尝试系统 protoc..."
            if command -v protoc &>/dev/null; then
                export PROTOC="$(command -v protoc)"
                echo "🔧 protoc: $PROTOC (system)"
            else
                echo "❌ 未找到可用的 protoc。请安装: brew install protobuf"
                exit 1
            fi
        fi
    elif command -v protoc &>/dev/null; then
        export PROTOC="$(command -v protoc)"
        echo "🔧 protoc: $PROTOC (system)"
    else
        echo "❌ 未找到可用的 protoc。请安装: brew install protobuf"
        exit 1
    fi
else
    echo "🔧 protoc: $PROTOC (env)"
fi

# --- 构建 -------------------------------------------------------------------

CARGO_FLAGS=()
if [[ "$PROFILE" == "release" ]]; then
    CARGO_FLAGS+=(--release)
fi

echo ""
echo "📦 构建 codel-pager ($PROFILE)..."
cargo build ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} -p codel-pager-bin

if [[ "$BUILD_ALL" == "true" ]]; then
    echo "📦 构建 codel-workspace-server ($PROFILE)..."
    cargo build ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} -p codel-workspace --bin codel-workspace-server
fi

# --- 输出产物路径 -----------------------------------------------------------

echo ""
echo "✅ 构建完成！产物位于:"

TARGET_DIR="target/${PROFILE}"
echo "   ${TARGET_DIR}/codel-pager"

if [[ "$BUILD_ALL" == "true" ]]; then
    echo "   ${TARGET_DIR}/codel-workspace-server"
fi

echo ""
echo "运行: ./${TARGET_DIR}/codel-pager --help"
