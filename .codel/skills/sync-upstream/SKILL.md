---
name: sync-upstream
description: >
  从上游仓库同步最新 commit 到 codel：识别真实分叉点、按 crate/标识符映射规则做
  三方合并、清理上游重新引入的登录与遥测代码，并验证编译。
  触发词："sync upstream"、"同步上游"、"merge upstream"。
metadata:
  short-description: "从上游仓库同步代码到 codel"
---

# 同步上游仓库 → codel

codel 是上游仓库（本机路径 `/Users/zhangqiong/Desktop/Project/grok-build`，
GitHub 上叫 `xai-org/grok-build`）的衍生分支：去掉了 x.ai 登录与遥测，
并把所有包/标识符改名为 `codel`。本 skill 把上游新 commit 合入 codel。

## 关键事实

- **分叉点不要信 `SOURCE_REV`**。上游会重写历史，`SOURCE_REV` 里的 monorepo
  sha 在当前上游历史里未必对应同一个树。要用**内容比对**确定真实分叉点：

  ```bash
  # 对每个上游 commit，统计 codel 有多少文件在改名后与它逐字节相同，取峰值
  python3 /tmp/score.py        # 见下方“分叉点打分”
  ```

  实测：`SOURCE_REV` 指向 `d71f6e0c`（匹配率 56%），而真实分叉点是
  `b41c75a5`（匹配率 82%）。用错基准会把 44% 的文件当成“codel 改过”而产生大量假冲突。

- **命名映射**（路径与内容都要做，顺序敏感，长匹配优先）：

  | 上游 | codel |
  | --- | --- |
  | `xai-grok-X` / `xai-X` | `codel-X` |
  | `xai_grok_X` / `xai_X` | `codel_X` |
  | `xai-grok-telemetry` | `codel-logging`（遥测已折叠为本地日志） |
  | `grok_build` | `codel_build` |
  | `grok` / `Grok` / `GROK` | `codel` / `Codel` / `CODEL` |
  | `x.ai` | `codel.dev` |
  | `x.ai/...`（ACP 方法名） | `codel/...` |
  | `grok.com` | `codel.dev` |
  | `XAI` / `xAI` | `CODEL` / `Codel` |

  规则实现见 `/tmp/xform.py` 与 `/tmp/mappath.py`。

## 执行步骤

### 1. 拉取上游并确定分叉点

```bash
cd /Users/zhangqiong/Desktop/Project/grok-build && git pull
git rev-parse HEAD            # 记录新 HEAD，分支名取其后 6 位
```

然后用 `score.py` 的内容比对找到分叉点 commit（峰值），记作 `<base>`。

### 2. 在 codel 建分支

```bash
cd /Users/zhangqiong/Desktop/Project/codel
git checkout -b sync/<HEAD 后 6 位>
```

### 3. 三方合并（带路径与标识符映射）

不要用 `git apply` 打 patch：上游与 codel 的行差异太大，上下文匹配会碎成
大量 `.rej`。改用**逐文件三方合并**（`/tmp/sync.py`）：

- `ancestor` = 改名后的上游 `<base>` 版本
- `theirs`   = 改名后的上游 `HEAD` 版本
- `ours`     = codel 当前文件

`ours == ancestor` 时直接取 `theirs`；三方都不同时用 `git merge-file --diff3`
留冲突标记（`<<<<<<< CODEL(ours)` / `||||||| UPSTREAM-BASE` / `>>>>>>> UPSTREAM-HEAD`）。

```bash
python3 /tmp/sync.py <base> <HEAD>            # 先 --dry-run 看冲突规模
```

### 4. 解决冲突

冲突块默认取 `UPSTREAM-HEAD`（同步方向），随后把 codel 的“去能力”重新施加。

```bash
python3 /tmp/theirs_resolve.py $(cat /tmp/conflict_remaining.txt)
```

### 5. 重建 codel 的能力删除

上游会把 codel 删掉的东西加回来，逐类清理（每类都检查调用链，不要只删声明）：

1. **遥测**：`codel-logging` 保留 API 但传输置空（`client::is_enabled()` 恒 false、
   `track()`/`init()` 空实现；`external::build_handle` 直接返回 `None`；
   `sentry::init` 空实现；`codel-otel` 的 `build_otel_layer` 不建 exporter）。
   Mixpanel / Sentry / OTLP 导出不得存在。
2. **hub 遥测捐赠**：`donate_pump` / `log_donate` / `metric_donate` / `trace_donate`
   整链删除 —— SDK 模块与 `ToolServer` 的 `*_donation_*` 方法、
   `codel-workspace` handle 入口、`codel-workspace-daemon` 的 metrics scraper、
   `codel-shell` leader 的 pump、`workspace-server` 的 wiring、
   `codel-tool-protocol` 的 `*Donate` 帧与方法。参考 `/tmp/excise_donation.py`。
3. **登录/登出**：删除 `/login` `/logout` slash 命令、OIDC / device-code /
   external-auth 流程；鉴权只保留 API key。
4. **订阅套餐**：删除 `SuperCodel` 各档位、upsell 文案与 `codel.dev/supercodel`
   链接、JWT tier 映射；保留用量/配额这类功能性记账。
5. **内置模型名**：`crates/codegen/codel-models/default_models.json` 保持空目录
   `{"models": []}`，`default_model()` 返回 `Option`；模型只能来自配置。
6. **关键字**：全仓（含注释、常量、环境变量、文档）不得出现 `grok` / `xai` /
   `x.ai`；环境变量统一 `CODEL_*`，不考虑兼容旧名。

### 6. 编译与验证

```bash
cargo check --workspace --keep-going --message-format=short
./build.sh --debug          # 绝不要用 release.sh，会破坏本地环境
./target/debug/codel-pager --version
```

编译报错时**不要猜**：去上游对照（`git show <HEAD>:<上游路径>`）确认后再改。
常见模式：

- 新文件引用已删除的 crate → 补齐模块声明或删掉整条调用链
- `mod.rs` 声明了 codel 没有的文件 → 该文件是 codel 的删除项，删声明并清理引用
- crate 缺依赖 → 与上游同名 crate 的 `Cargo.toml` 对齐后再补
- 形如 `x_codel_conv_id` / `cost_usd_ticks` 的未定义标识符 → 上游新增字段，
  按上游补齐（或按 codel 删除项整块去掉）

### 7. 更新文档并提交

- `crates/codegen/codel-pager/docs/user-guide/*.md`（中文用户手册）随新配置/
  环境变量更新；`crates/codegen/codel-pager/src/docs.rs` 里的文件名/标题数组
  与手册文件必须一一对应（它用 `include_str!` 编译期内联）。
- 提交到 `sync/<...>` 分支。

## 踩坑记录

1. **`SOURCE_REV` 不可信** —— 见上，必须内容比对定基准。
2. **上游会重构 crate**：`xai-grok-shell/src/auth/` 被抽成 `xai-grok-login`，
   pager 的 PTY 测试抽成 `xai-grok-pager-pty-harness`。codel 侧相应建
   `codel-login`、`codel-pager-pty-harness`，并在 `codel-shell/src/lib.rs`
   里 `pub mod auth { pub use codel_login::*; }` 保持 `crate::auth::` 路径可用。
3. **`--keep-going`**：cargo 默认在一个 crate 失败后不再检查其下游，
   加 `--keep-going` 才能一次拿到全部错误。
4. **macOS 无 `timeout`**，用 `gtimeout` 或不设超时。
5. **不要用 `release.sh`**。
6. **改动源码时别同时跑 cargo**，会读到半成品文件产生假错误。
