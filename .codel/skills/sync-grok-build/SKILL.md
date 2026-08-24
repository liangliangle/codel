---
name: sync-grok-build
description: >
  从 grok-build 单仓同步最新 commit 到 codel，自动完成路径/crate 名映射，
  跳过 login/logout 和 telemetry 相关变更。
  触发词："sync grok-build"、"merge grok-build"、"同步 grok-build"、"合并 grok-build"。
metadata:
  short-description: "从 grok-build 同步代码到 codel"
---

# 同步 grok-build → codel

codel 是 grok-build 的衍生分支。本 skill 拉取 grok-build 新 commit，分类变更，
应用 crate 名映射后合入 codel，并验证编译和运行。

## 基本信息

- **grok-build 仓库**：`/Users/liangliang/Project/Code/grok-build`
- **codel 仓库**：`/Users/liangliang/Project/liliangliang.lll/codel`
- **crate 名映射**：`xai-grok-*` → `codel-*`，`xai-*` → `codel-*`
  （如 `xai_grok_shell` → `codel_shell`，`xai_grok_config` → `codel_config`）
- **跳过类别**：login/logout/认证流程代码、telemetry 代码
  （`xai-grok-telemetry` crate、`session/telemetry.rs`、OTel/fastrace）

## 执行步骤

### 1. 拉取 grok-build 并识别新 commit

```bash
cd /Users/liangliang/Project/Code/grok-build
git pull
git rev-parse HEAD   # 记录新 HEAD
```

找同步基准——两个仓库共有的最新 commit：

```bash
cd /Users/liangliang/Project/liliangliang.lll/codel
git log --oneline -1   # codel 的 HEAD 应该对应 grok-build 中某个 commit
```

列出新 commit：

```bash
cd /Users/liangliang/Project/Code/grok-build
git log --oneline <基准>..HEAD
```

### 2. 在 codel 创建同步分支

```bash
cd /Users/liangliang/Project/liliangliang.lll/codel
git checkout -b sync/<HEAD后6位>
```

### 3. 分类变更文件

```bash
cd /Users/liangliang/Project/Code/grok-build
git diff --name-status <基准>..HEAD
```

按以下规则分类：
- **跳过-认证**：路径含 `auth/flow`、`auth/storage`、`auth/error`、`auth/mod`、
  `auth/manager`、`auth/refresh`、`auth/credential`、`extensions/auth`、`login`、`logout`
- **跳过-遥测**：路径含 `telemetry`、`mixpanel`、`xai-grok-telemetry`
- **跳过-元数据**：`SOURCE_REV`、`Cargo.lock`
- **需同步**：其余所有文件

### 4. 生成 patch 并应用（带路径映射）

```bash
cd /Users/liangliang/Project/Code/grok-build
git diff <基准>..HEAD \
  -- ':!SOURCE_REV' ':!Cargo.lock' \
  -- ':!crates/codegen/xai-grok-telemetry' \
  -- ':!crates/codegen/xai-grok-shell/src/auth/' \
  -- ':!crates/codegen/xai-grok-shell/src/extensions/auth.rs' \
  -- ':!crates/codegen/xai-grok-shell/src/session/telemetry.rs' \
  | sed 's|xai-grok-|codel-|g; s|xai-file-utils|codel-file-utils|g; s|xai-computer-hub-sdk|codel-computer-hub-sdk|g; s|xai-tool-protocol|codel-tool-protocol|g; s|xai-workflow|codel-workflow|g' \
  > /tmp/sync.patch

cd /Users/liangliang/Project/liliangliang.lll/codel
git apply --reject --whitespace=fix /tmp/sync.patch
```

### 5. 解决 .rej 冲突

按 crate 分组派并行 subagent 处理 `.rej` 文件：
- Subagent 1：`codel-shell` 的 .rej 文件
- Subagent 2：`codel-pager*` 的 .rej 文件
- Subagent 3：`codel-workspace` + `codel-config` + `codel-hooks` + 其他

每个 subagent 的处理流程：
1. 读 `.rej` 文件，理解 grok-build 想做什么改动
2. 读 codel 当前源文件，了解现状
3. 手动应用等价改动，适配 codel 的差异
4. 解决后删除 `.rej` 文件

**关键适配规则：**
- 跳过所有引用 telemetry/OTel/fastrace/login 的代码块
- `GrokAuth` → `CodelAuth`，`GrokComConfig` → `CodelComConfig`
- `grok_home` → `codel_home`
- `xai_grok_*` crate 引用 → `codel_*`
- 如果函数返回类型变了（如 `Option<T>` → 新枚举），加适配调用如 `.into_option()`

### 6. 批量修复 crate 名引用

解决 .rej 后，批量替换残留的 `xai_grok_*` 引用：

```bash
cd /Users/liangliang/Project/liliangliang.lll/codel
find . -name '*.rs' -not -path './target/*' -exec grep -l 'xai_grok_' {} \; \
  | xargs sed -i '' \
    -e 's/xai_grok_shell/codel_shell/g' \
    -e 's/xai_grok_pager/codel_pager/g' \
    -e 's/xai_grok_config/codel_config/g' \
    -e 's/xai_grok_hooks/codel_hooks/g' \
    -e 's/xai_grok_agent/codel_agent/g' \
    -e 's/xai_grok_tools/codel_tools/g' \
    -e 's/xai_grok_workspace_types/codel_workspace_types/g' \
    -e 's/xai_grok_workspace/codel_workspace/g' \
    -e 's/xai_grok_version/codel_version/g' \
    -e 's/xai_grok_sampling_types/codel_sampling_types/g' \
    -e 's/xai_grok_test_support/codel_test_support/g' \
    -e 's/xai_grok_voice/codel_voice/g' \
    -e 's/xai_tty_utils/codel_tty_utils/g'
```

同时检查 `codel_telemetry` 引用（该 crate 在 codel 中不存在），全部移除。

### 7. 处理重构的模块

如果 grok-build 把某模块拆分了（如 `models.rs` → `models/` 目录），
直接从 grok-build 复制新文件再适配：

```bash
cp /Users/liangliang/Project/Code/grok-build/crates/codegen/xai-grok-shell/src/agent/models.rs \
   crates/codegen/codel-shell/src/agent/models.rs
# 复制子模块文件后 sed 替换 crate 名
```

### 8. 迭代修复编译错误

```bash
./build.sh --debug 2>&1 | grep '^error' | sort | uniq -c | sort -rn
```

常见错误模式及修复：
- `cannot find function user_grok_home` → 改为 `user_codel_home`
- `unresolved import xai_grok_config` → 改为 `codel_config`
- `cannot find type GrokAuth` → 改为 `CodelAuth`
- `mismatched types: expected Option, found SettingsFetch` → 加 `.into_option()`
- `cannot find function apply_otel_config` → 删除该调用（OTel 已移除）
- `could not find TurnOutcomeLabel in events` → 删除遥测发射代码

反复 构建 → 修复 直到零错误。

### 9. 验证非交互运行

```bash
./target/debug/codel-pager -p "echo hello"
```

应输出 `hello`，exit 0，无 panic。

### 10. 提交并推送

```bash
git add -A
git commit -m "Sync grok-build (<首个commit>..<末尾commit>)

- 同步 N 个 commit
- 跳过 login/logout 和 telemetry 变更
- 修复所有 crate 名映射 (xai-grok-* → codel-*)
- 编译零错误"
git push origin main
```

## 踩坑记录

1. **SOURCE_REV 可能不存在于 grok-build** — monorepo 同步会重写历史，
   用两个仓库共有的最新 commit 作为实际基准。
2. **zsh 中 `status` 是只读变量** — shell 脚本里别用 `read status file`，
   改用 `read -r st fl`。
3. **macOS 没有 `timeout` 命令** — 用 `gtimeout`（coreutils）或直接不设超时。
4. **patch 冲突是常态** — codel 已分叉（删了 telemetry、OTel、feedback），
   预计 30-40% 的文件需要手动解决。
5. **新文件可能引用已删除的 crate** — 应用 patch 后务必 grep `codel_telemetry`、
   `xai_grok_telemetry`，全部清除。
6. **`git apply --reject` 只应用能应用的部分** — 应用后检查 `.rej` 数量
   来评估手动工作量。
7. **macOS 代码签名问题** — 如果构建后直接运行被 SIGKILL（exit 137），
   原因是 linker-signed 签名被 taskgated 拒绝。修复：
   `xattr -cr <binary> && codesign --force --sign - <binary>`。
   `release.sh` 已内置此修复。
