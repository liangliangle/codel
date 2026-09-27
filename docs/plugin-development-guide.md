# Codel 插件开发手册

本手册面向希望为 Codel 开发插件（Plugin）或钩子（Hook）的开发者。阅读完本手册后，你将能够独立完成插件的设计、开发、测试和发布。

---

## 目录

1. [概述](#1-概述)
2. [核心概念](#2-核心概念)
3. [插件目录结构](#3-插件目录结构)
4. [插件清单 (plugin.json)](#4-插件清单-pluginjson)
5. [Hooks 系统](#5-hooks-系统)
   - 5.1 [事件类型](#51-事件类型)
   - 5.2 [Hook 配置文件格式](#52-hook-配置文件格式)
   - 5.3 [脚本通信协议](#53-脚本通信协议)
   - 5.4 [决策协议（Gate Hooks）](#54-决策协议gate-hooks)
   - 5.5 [Stop Gate 协议](#55-stop-gate-协议)
   - 5.6 [Matcher 匹配规则](#56-matcher-匹配规则)
   - 5.7 [环境变量](#57-环境变量)
   - 5.8 [超时与错误处理](#58-超时与错误处理)
6. [Skills（技能）](#6-skills技能)
7. [Agents（代理定义）](#7-agents代理定义)
8. [MCP 服务器](#8-mcp-服务器)
9. [插件发现与加载](#9-插件发现与加载)
10. [信任与安全模型](#10-信任与安全模型)
11. [配置参考](#11-配置参考)
12. [Marketplace 发布](#12-marketplace-发布)
13. [完整示例](#13-完整示例)
14. [调试与排错](#14-调试与排错)
15. [FAQ](#15-faq)

---

## 1. 概述

Codel 的插件系统允许开发者扩展 AI 代理的能力。一个插件可以包含以下一种或多种组件：

| 组件 | 说明 |
|------|------|
| **Hooks** | 在特定事件（工具调用前后、会话开始/结束等）时执行自定义脚本 |
| **Skills** | 为代理提供领域知识和操作指南（Markdown 文件） |
| **Agents** | 自定义代理角色/人设定义（Markdown 文件） |
| **MCP Servers** | 通过 Model Context Protocol 暴露自定义工具 |
| **LSP Servers** | 语言服务器集成 |
| **Commands** | 自定义斜杠命令 |

插件系统是**基于文件系统**的：无需编译，无需注册 API，只需按照约定的目录结构放置文件即可。

---

## 2. 核心概念

### 2.1 插件 vs 钩子

- **插件（Plugin）**：一个完整的目录包，包含清单文件和多种组件。
- **钩子（Hook）**：插件的一个组件，也可以独立存在（直接放在 `~/.codel/hooks/` 目录）。

### 2.2 作用域（Scope）

插件按发现位置分为四个作用域，优先级从高到低：

| 作用域 | 位置 | 信任 |
|--------|------|------|
| `CliOverride` | `--plugin-dir` CLI 参数 | 始终信任 |
| `Project` | `<project>/.codel/plugins/` 或 `.claude/plugins/` | 需要用户信任 |
| `User` | `~/.codel/plugins/` 或 `~/.claude/plugins/` | 始终信任 |
| `ConfigPath` | `[plugins].paths` 配置项 | 视位置而定 |

### 2.3 插件 ID

每个插件有唯一 ID，格式为 `<scope>/<hex8>/<name>`，例如：

```
user/a1b2c3d4/my-awesome-plugin
```

其中 `hex8` 是插件根目录绝对路径的 SHA-256 前 8 位十六进制。

---

## 3. 插件目录结构

一个完整的插件目录结构如下：

```
my-plugin/
├── plugin.json              # 插件清单（可选，无则按约定发现）
├── skills/                  # 技能目录
│   ├── code-review/
│   │   └── SKILL.md
│   └── deploy/
│       └── SKILL.md
├── agents/                  # 代理定义
│   ├── reviewer.md
│   └── architect.md
├── commands/                # 自定义命令
│   └── deploy.md
├── hooks/                   # 钩子定义
│   ├── hooks.json           # 钩子配置
│   └── bin/
│       ├── pre-check.sh
│       └── post-action.py
├── .mcp.json                # MCP 服务器配置
└── .lsp.json                # LSP 服务器配置（可选）
```

**最小插件**只需要一个目录和一个组件文件即可工作。例如，一个只有 skill 的插件：

```
my-skill-plugin/
└── skills/
    └── hello/
        └── SKILL.md
```

---

## 4. 插件清单 (plugin.json)

清单文件是可选的。如果不存在，系统按约定目录结构自动发现组件，插件名取自目录名。

### 4.1 完整字段

```json
{
  "name": "my-plugin",
  "version": "1.2.0",
  "description": "A plugin that does amazing things",
  "author": {
    "name": "Your Name",
    "email": "you@example.com",
    "url": "https://github.com/you"
  },
  "homepage": "https://github.com/you/my-plugin",
  "repository": "https://github.com/you/my-plugin",
  "license": "MIT",
  "keywords": ["testing", "ci", "automation"],
  "skills": "skills",
  "commands": "commands",
  "agents": "agents",
  "hooks": "hooks/hooks.json",
  "mcpServers": ".mcp.json",
  "lspServers": ".lsp.json"
}
```

### 4.2 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | ✅ | 插件名，kebab-case，1-64 字符，小写字母+数字+连字符 |
| `version` | string | ❌ | 语义化版本号 |
| `description` | string | ❌ | 一句话描述 |
| `author` | object | ❌ | 作者信息 |
| `homepage` | string | ❌ | 主页 URL |
| `repository` | string | ❌ | 仓库 URL |
| `license` | string | ❌ | 许可证标识 |
| `keywords` | string[] | ❌ | 关键词（用于市场搜索） |
| `skills` | string \| string[] | ❌ | 技能目录路径（默认 `skills/`） |
| `commands` | string \| string[] | ❌ | 命令目录路径（默认 `commands/`） |
| `agents` | string \| string[] | ❌ | 代理目录路径（默认 `agents/`） |
| `hooks` | string \| object | ❌ | 钩子文件路径或内联 JSON（默认 `hooks/hooks.json`） |
| `mcpServers` | string \| object | ❌ | MCP 配置路径或内联 JSON（默认 `.mcp.json`） |
| `lspServers` | string \| object | ❌ | LSP 配置路径或内联 JSON（默认 `.lsp.json`） |

### 4.3 名称规则

- 仅允许小写字母 `a-z`、数字 `0-9`、连字符 `-`
- 不能以连字符开头或结尾
- 最长 64 字符
- 示例：`code-review-helper`、`k8s-deployer`、`my-plugin-2`

### 4.4 内联 Hooks 和 MCP

`hooks` 和 `mcpServers` 字段支持直接内联 JSON 对象，无需外部文件：

```json
{
  "name": "inline-hook-plugin",
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "bin/check.sh", "timeout": 5 }
        ]
      }
    ]
  }
}
```

### 4.5 清单查找顺序

系统按以下顺序查找清单文件：

1. `<plugin-root>/plugin.json`（首选）
2. `<plugin-root>/.codel-plugin/plugin.json`
3. `<plugin-root>/.claude-plugin/plugin.json`

---

## 5. Hooks 系统

Hooks 是插件系统中最强大的组件——它允许你在代理生命周期的关键节点插入自定义逻辑。

### 5.1 事件类型

| 事件 | 说明 | Gate 类型 | Matcher |
|------|------|-----------|---------|
| `SessionStart` | 会话启动时 | Observe | 匹配 source（`startup`/`resume`） |
| `UserPromptSubmit` | 用户提交提示时 | Observe | 忽略 |
| `PreToolUse` | 工具执行前 | **Tool**（可阻止） | 匹配工具名 |
| `PostToolUse` | 工具执行后 | Observe | 匹配工具名 |
| `PostToolUseFailure` | 工具执行失败后 | Observe | 匹配工具名 |
| `PermissionDenied` | 权限被拒绝时 | Observe | 匹配工具名 |
| `Stop` | 代理准备结束回合时 | **Stop**（可阻止/强制停止） | 忽略 |
| `StopFailure` | 因 API 错误结束时 | Observe | 匹配错误类型 |
| `Notification` | 通知事件 | Observe | 匹配通知类型 |
| `SubagentStart` | 子代理启动时 | Observe | 匹配子代理类型 |
| `SubagentStop` | 子代理结束时 | **Stop**（可阻止） | 匹配子代理类型 |
| `PreCompact` | 压缩前 | Observe | 匹配 source（`manual`/`auto`） |
| `PostCompact` | 压缩后 | Observe | 匹配 source |
| `SessionEnd` | 会话结束时 | Observe | 匹配 reason |

**Gate 类型说明：**

- **Observe**：仅观察，输出被记录但不影响代理行为。
- **Tool**：可以阻止工具执行（`PreToolUse` 专用）。
- **Stop**：可以阻止代理停止（让它继续工作），或强制停止。

### 5.2 Hook 配置文件格式

Hook 配置使用 JSON 格式，兼容 Claude Code 的 settings 格式：

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "bin/safe-shell-guard.sh",
            "timeout": 5
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "bun run format || true"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bin/stop-verify.sh",
            "timeout": 300
          }
        ]
      }
    ]
  }
}
```

**字段说明：**

| 字段 | 类型 | 说明 |
|------|------|------|
| `matcher` | string | 正则表达式，匹配工具名/事件源。省略则匹配所有 |
| `hooks` | array | 处理器数组 |
| `hooks[].type` | string | `"command"` 或 `"http"` |
| `hooks[].command` | string | 脚本路径（相对于 hook 文件目录）或内联 shell 命令 |
| `hooks[].url` | string | HTTP 端点 URL（type 为 http 时） |
| `hooks[].timeout` | number | 超时秒数（默认 5 秒，Stop 事件默认 600 秒） |
| `hooks[].env` | object | 额外环境变量（不可覆盖系统保留变量） |

### 5.3 脚本通信协议

Hook 脚本通过 **stdin/stdout** 与 Codel 通信：

**输入（stdin）**：JSON 格式的事件信封

```json
{
  "hookEventName": "PreToolUse",
  "sessionId": "abc-123-def",
  "cwd": "/Users/dev/project",
  "workspaceRoot": "/Users/dev/project",
  "timestamp": "2026-08-24T10:30:00Z",
  "transcriptPath": "/Users/dev/.codel/sessions/.../transcript.jsonl",
  "permissionMode": "default",
  "toolName": "run_terminal_cmd",
  "toolUseId": "tool_abc123",
  "toolInput": {
    "command": "rm -rf /tmp/build"
  },
  "toolInputTruncated": false
}
```

**输出（stdout）**：JSON 格式的决策（仅 Gate 类型事件需要）

### 5.4 决策协议（Gate Hooks）

#### PreToolUse（工具门控）

**允许执行：**
```json
{"decision": "allow"}
```

**拒绝执行：**
```json
{"decision": "deny", "reason": "此命令会删除系统文件，已被安全策略阻止"}
```

**退出码约定：**

| 退出码 | 含义 |
|--------|------|
| `0` | 允许（或无决策） |
| `2` | 拒绝（stderr 作为原因） |
| 其他 | 失败，fail-open（不阻止） |

> 当 stdout 有有效 JSON 决策时，JSON 优先于退出码。

#### 完整示例脚本

```bash
#!/bin/sh
# bin/safe-shell-guard.sh — 阻止危险 shell 命令

INPUT=$(cat)

# 提取命令内容
COMMAND=$(echo "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | sed 's/"command":"//;s/"$//')

if [ -z "$COMMAND" ]; then
  echo '{"decision":"allow"}'
  exit 0
fi

LOWER_CMD=$(echo "$COMMAND" | tr '[:upper:]' '[:lower:]')

case "$LOWER_CMD" in
  *"rm -rf /"*|*"sudo rm -rf"*)
    echo '{"decision":"deny","reason":"Blocked: destructive rm command"}'
    exit 2
    ;;
  *"mkfs"*|*"dd if=/dev/zero of=/dev"*)
    echo '{"decision":"deny","reason":"Blocked: disk operation not allowed"}'
    exit 2
    ;;
esac

echo '{"decision":"allow"}'
exit 0
```

### 5.5 Stop Gate 协议

`Stop` 和 `SubagentStop` 事件使用扩展协议，可以：

1. **阻止停止**（让代理继续工作）
2. **注入上下文**（给代理额外信息）
3. **强制停止**（覆盖其他 block）

**阻止停止（让代理继续）：**
```json
{"decision": "block", "reason": "请先运行测试套件确认所有测试通过"}
```

**注入额外上下文：**
```json
{
  "hookSpecificOutput": {
    "hookEventName": "Stop",
    "additionalContext": "提醒：部署前需要更新 CHANGELOG.md"
  }
}
```

**强制停止：**
```json
{"continue": false, "stopReason": "已达到最大迭代次数，强制结束"}
```

**组合使用：**
```json
{
  "decision": "block",
  "reason": "还有未完成的任务",
  "hookSpecificOutput": {
    "additionalContext": "剩余任务：更新文档、运行 lint"
  }
}
```

**Stop 事件输入额外字段：**

```json
{
  "hookEventName": "Stop",
  "reason": "end_turn",
  "stopHookActive": false,
  "lastAssistantMessage": "我已经完成了代码修改...",
  "backgroundTasks": [
    {
      "id": "task-001",
      "type": "shell",
      "status": "running",
      "command": "npm run dev"
    }
  ],
  "sessionCrons": [
    {
      "id": "cron-001",
      "schedule": "every 5 minutes",
      "recurring": true,
      "prompt": "check build status"
    }
  ]
}
```

> `stopHookActive` 为 `true` 时表示本轮已经被 Stop hook 阻止过一次。建议检查此字段避免无限循环。系统内置上限为 8 次连续 continuation。

### 5.6 Matcher 匹配规则

Matcher 是正则表达式，用于过滤 hook 的触发条件：

```json
{"matcher": "Bash"}
```

**匹配目标（按事件类型）：**

| 事件 | 匹配值 |
|------|--------|
| `PreToolUse` / `PostToolUse` / `PostToolUseFailure` / `PermissionDenied` | 工具名（如 `run_terminal_cmd`、`read_file`） |
| `SessionStart` / `PreCompact` / `PostCompact` | source（如 `startup`、`manual`） |
| `SessionEnd` | reason |
| `Notification` | notificationType |
| `SubagentStart` / `SubagentStop` | subagentType |
| `Stop` / `UserPromptSubmit` | 忽略 matcher（始终触发） |

**兼容性扩展**：Claude 风格的工具名会自动扩展匹配 Codel 工具名：

- `Bash` → 同时匹配 `run_terminal_cmd`
- `Read` → 同时匹配 `read_file`
- `Edit` / `Write` → 同时匹配 `search_replace`

**正则示例：**

```json
{"matcher": "Write|Edit"}
{"matcher": "run_terminal_cmd|Bash"}
{"matcher": "startup|resume"}
```

### 5.7 环境变量

Hook 脚本执行时，以下环境变量**始终可用**：

| 变量 | 说明 |
|------|------|
| `CODEL_HOOK_EVENT` | 当前事件名（如 `pre_tool_use`） |
| `CODEL_HOOK_NAME` | Hook 完整名称 |
| `CODEL_SESSION_ID` | 当前会话 ID |
| `CODEL_WORKSPACE_ROOT` | 工作区根目录 |
| `CLAUDE_PROJECT_DIR` | 同 `CODEL_WORKSPACE_ROOT`（兼容别名） |

这些变量由运行器注入，**不可被用户 `env` 配置覆盖**。

**自定义环境变量**（通过 `env` 字段）：

```json
{
  "type": "command",
  "command": "bin/check.sh",
  "env": {
    "MY_API_KEY": "xxx",
    "CHECK_LEVEL": "strict"
  }
}
```

### 5.8 超时与错误处理

| 场景 | 行为 |
|------|------|
| 脚本超时 | 进程被 kill，视为失败，**fail-open**（不阻止） |
| 脚本崩溃（非 0/2 退出） | 视为失败，fail-open |
| stdout 非法 JSON | 回退到退出码判断 |
| 命令不存在 | 视为失败，fail-open |
| 环境变量未设置 | 命令不执行，报错 |

**默认超时：**
- 普通事件：5 秒
- Stop/SubagentStop 事件：600 秒（因为可能运行构建/测试）

**设计原则：Fail-Open**——hook 的任何故障都不会阻止用户正常工作。

---

## 6. Skills（技能）

Skills 是 Markdown 文件，为代理提供领域知识和操作指南。

### 目录结构

```
skills/
└── deploy-k8s/
    └── SKILL.md
```

### SKILL.md 格式

```markdown
---
name: deploy-k8s
description: 部署应用到 Kubernetes 集群
triggers:
  - "deploy"
  - "kubernetes"
  - "k8s"
---

# Kubernetes 部署指南

当用户要求部署应用时，按以下步骤操作：

1. 确认 Dockerfile 存在
2. 构建镜像并推送到 registry
3. 使用 kubectl apply 部署
...
```

Skills 通过触发词（triggers）或用户显式 `/skill` 命令激活。

---

## 7. Agents（代理定义）

Agents 是 Markdown 文件，定义自定义代理角色。

```
agents/
└── code-reviewer.md
```

### 格式

```markdown
---
name: code-reviewer
description: 专注于代码审查的代理
model: grok-4
tools:
  - read_file
  - grep
  - list_dir
---

# Code Reviewer

你是一个严格的代码审查专家。审查代码时关注：
- 安全漏洞
- 性能问题
- 代码风格一致性
...
```

---

## 8. MCP 服务器

插件可以通过 `.mcp.json` 文件暴露 MCP（Model Context Protocol）工具。

### .mcp.json 格式

```json
{
  "mcpServers": {
    "my-database": {
      "command": "node",
      "args": ["./mcp-server/index.js"],
      "env": {
        "DB_URL": "postgres://localhost/mydb"
      }
    },
    "my-api": {
      "command": "python3",
      "args": ["-m", "my_mcp_server"],
      "env": {}
    }
  }
}
```

MCP 服务器在插件被信任后自动启动，其工具会出现在代理的可用工具列表中。

---

## 9. 插件发现与加载

### 9.1 发现顺序

系统按以下优先级扫描插件：

1. CLI `--plugin-dir <path>` 指定的目录
2. `<project>/.codel/plugins/*/`（项目级）
3. `<project>/.claude/plugins/*/`（项目级，兼容）
4. `~/.codel/plugins/*/`（用户级）
5. `~/.claude/plugins/*/`（用户级，兼容）
6. `~/.codel/installed-plugins/*/`（市场安装）
7. `[plugins].paths` 配置中的路径

### 9.2 去重与冲突

- 按规范化路径去重（symlink 解析后）
- 同名插件：高优先级作用域覆盖低优先级
- 冲突时，被覆盖的插件会记录 `conflict` 警告

### 9.3 启用/禁用

在 `~/.codel/config.toml` 中：

```toml
[plugins]
paths = ["/extra/plugin/dir"]
disabled = ["problematic-plugin"]
enabled = ["project-plugin-that-needs-explicit-enable"]
```

> 项目级插件（`.codel/plugins/`）默认**禁用**，需要在 `enabled` 列表中显式启用。

---

## 10. 信任与安全模型

### 10.1 信任规则

| 来源 | 默认信任 | 说明 |
|------|----------|------|
| CLI `--plugin-dir` | ✅ | 用户显式指定 |
| `~/.codel/plugins/` | ✅ | 用户自己安装 |
| `<project>/.codel/plugins/` | ❌ | 可能来自不可信仓库 |
| Marketplace 安装 | ✅ | 用户主动安装 |

### 10.2 项目插件信任

项目级插件需要用户通过 `/hooks-trust` 命令或 TUI 界面显式信任后才会执行 hooks 和 MCP 服务器。

### 10.3 安全限制

- Hook 脚本的 `env` 字段不能覆盖系统保留变量
- 清单中的路径不能逃逸插件根目录（`..` 被拒绝）
- Hook 进程与控制终端隔离（防止 TUI 干扰）
- stdout/stderr 输出限制为 64 KB
- stdin payload 中 `toolInput`/`toolResult` 限制为 128 KB

---

## 11. 配置参考

### 11.1 config.toml 中的插件配置

```toml
[plugins]
# 额外插件目录
paths = ["/path/to/shared/plugins"]
# 禁用的插件（ID 或名称）
disabled = ["noisy-plugin"]
# 显式启用的项目插件
enabled = ["team-standards"]
```

### 11.2 Hook 独立配置

Hooks 可以独立于插件存在，放置在：

- `~/.codel/hooks/`（全局）
- `<project>/.codel/hooks/`（项目级，需信任）

每个 `.json` 文件定义一组 hooks。

### 11.3 禁用特定 Hook

在 `~/.codel/disabled-hooks` 文件中按名称禁用：

```
global/safe-shell:pre_tool_use[0].hooks[0]
```

---

## 12. Marketplace 发布

### 12.1 Marketplace 源配置

在 `~/.codel/config.toml` 中注册市场源：

```toml
[[marketplace.sources]]
name = "My Company Plugins"
git = "https://github.com/my-org/codel-plugins.git"

[[marketplace.sources]]
name = "Local Dev"
path = "~/dev/my-plugins"
```

### 12.2 市场仓库结构

```
codel-plugins/                    # Git 仓库根目录
├── .codel-plugin/
│   └── marketplace.json          # 市场索引（首选）
├── plugins/
│   ├── plugin-a/
│   │   ├── plugin.json
│   │   ├── skills/
│   │   └── hooks/
│   └── plugin-b/
│       ├── plugin.json
│       └── .mcp.json
└── README.md
```

### 12.3 marketplace.json 索引

```json
{
  "name": "My Company Plugin Marketplace",
  "description": "Internal plugins for engineering team",
  "owner": {
    "name": "Platform Team",
    "email": "platform@company.com"
  },
  "plugins": [
    {
      "name": "deploy-helper",
      "version": "2.1.0",
      "description": "Automated deployment workflows",
      "category": "devops",
      "author": { "name": "Platform Team" },
      "source": {
        "path": "plugins/deploy-helper"
      },
      "homepage": "https://wiki.company.com/deploy-helper",
      "tags": ["deploy", "ci-cd"],
      "keywords": ["deploy", "release", "rollback"],
      "domains": ["devops", "infrastructure"]
    }
  ]
}
```

### 12.4 安装与更新

用户通过 TUI 的 `/plugins` 命令或 marketplace UI 安装：

- **安装**：克隆/拉取仓库，复制到 `~/.codel/installed-plugins/`
- **更新**：`/plugins update` 拉取最新版本
- **卸载**：`/plugins uninstall <plugin-id>`

### 12.5 SHA 固定

生产环境建议启用 SHA 固定：

```toml
[marketplace]
require_sha = true
```

或设置环境变量 `CODEL_MARKETPLACE_REQUIRE_SHA=1`。

---

## 13. 完整示例

### 13.1 示例：代码格式化 Hook 插件

```
auto-formatter/
├── plugin.json
├── hooks/
│   ├── hooks.json
│   └── bin/
│       └── format-on-write.sh
└── skills/
    └── formatting/
        └── SKILL.md
```

**plugin.json:**
```json
{
  "name": "auto-formatter",
  "version": "1.0.0",
  "description": "Automatically format code after write operations"
}
```

**hooks/hooks.json:**
```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write|Edit|search_replace",
        "hooks": [
          {
            "type": "command",
            "command": "bin/format-on-write.sh",
            "timeout": 30
          }
        ]
      }
    ]
  }
}
```

**hooks/bin/format-on-write.sh:**
```bash
#!/bin/sh
# 在文件写入后自动运行格式化工具

INPUT=$(cat)

# 提取文件路径
FILE=$(echo "$INPUT" | grep -o '"target_file":"[^"]*"' | head -1 | sed 's/"target_file":"//;s/"$//')

if [ -z "$FILE" ]; then
  exit 0
fi

# 根据扩展名选择格式化工具
case "$FILE" in
  *.rs)
    rustfmt "$FILE" 2>/dev/null
    ;;
  *.py)
    black "$FILE" 2>/dev/null
    ;;
  *.ts|*.js)
    prettier --write "$FILE" 2>/dev/null
    ;;
  *.go)
    gofmt -w "$FILE" 2>/dev/null
    ;;
esac

exit 0
```

### 13.2 示例：CI 验证 Stop Gate

```
ci-gate/
├── plugin.json
└── hooks/
    ├── hooks.json
    └── bin/
        └── verify-build.sh
```

**hooks/hooks.json:**
```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bin/verify-build.sh",
            "timeout": 300
          }
        ]
      }
    ]
  }
}
```

**hooks/bin/verify-build.sh:**
```bash
#!/bin/sh
# 在代理结束前验证构建是否通过

INPUT=$(cat)

# 检查是否已经被阻止过（避免无限循环）
STOP_ACTIVE=$(echo "$INPUT" | grep -o '"stopHookActive":true')
if [ -n "$STOP_ACTIVE" ]; then
  # 已经阻止过一次了，这次放行
  exit 0
fi

# 运行构建
cd "$CODEL_WORKSPACE_ROOT"
if cargo build 2>&1; then
  # 构建通过，允许停止
  exit 0
else
  # 构建失败，阻止停止并告知原因
  echo '{"decision":"block","reason":"cargo build 失败，请修复编译错误后再结束"}'
  exit 0
fi
```

### 13.3 示例：HTTP Hook

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "run_terminal_cmd",
        "hooks": [
          {
            "type": "http",
            "url": "https://hooks.mycompany.com/validate-command",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

HTTP hook 接收相同的 JSON 信封作为 POST body，响应格式与 command hook 的 stdout 相同。

### 13.4 示例：会话审计日志

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          { "type": "command", "command": "bin/session-log.sh" }
        ]
      }
    ],
    "SessionEnd": [
      {
        "hooks": [
          { "type": "command", "command": "bin/session-log.sh" }
        ]
      }
    ]
  }
}
```

**bin/session-log.sh:**
```bash
#!/bin/sh
INPUT=$(cat)
EVENT=$(echo "$INPUT" | grep -o '"hookEventName":"[^"]*"' | sed 's/.*:"//;s/"//')
SESSION=$(echo "$INPUT" | grep -o '"sessionId":"[^"]*"' | sed 's/.*:"//;s/"//')
CWD=$(echo "$INPUT" | grep -o '"cwd":"[^"]*"' | sed 's/.*:"//;s/"//')
TS=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

echo "$TS | $EVENT | session=$SESSION | cwd=$CWD" >> ~/.codel/session-audit.log
exit 0
```

---

## 14. 调试与排错

### 14.1 调试模式

设置环境变量启用 hook 调试日志：

```bash
export CODEL_HOOK_DEBUG=1
```

这会输出每个 hook 的 stdin payload 和 stdout 响应。

### 14.2 常见问题

| 问题 | 原因 | 解决方案 |
|------|------|----------|
| Hook 不触发 | 文件不在正确目录 | 确认在 `~/.codel/hooks/` 或 `<project>/.codel/hooks/` |
| Hook 不触发 | 项目 hooks 未信任 | 运行 `/hooks-trust` |
| Hook 不触发 | 被禁用 | 检查 `~/.codel/disabled-hooks` |
| 命令找不到 | 相对路径解析错误 | 路径相对于 hook JSON 文件所在目录 |
| 超时 | 脚本执行太慢 | 增加 `timeout` 值 |
| 决策无效 | JSON 格式错误 | 确保 stdout 只输出有效 JSON |
| 环境变量空 | 未设置 | 使用 `${VAR:-default}` 或确保变量已导出 |

### 14.3 测试 Hook

手动测试 hook 脚本：

```bash
echo '{
  "hookEventName": "PreToolUse",
  "sessionId": "test",
  "cwd": "/tmp",
  "workspaceRoot": "/tmp",
  "timestamp": "2026-01-01T00:00:00Z",
  "toolName": "run_terminal_cmd",
  "toolUseId": "test-1",
  "toolInput": {"command": "rm -rf /"},
  "toolInputTruncated": false
}' | ./bin/safe-shell-guard.sh
```

预期输出：
```json
{"decision":"deny","reason":"Blocked: destructive rm command"}
```

### 14.4 查看已加载的 Hooks

在 TUI 中使用 `/hooks` 命令查看所有已加载的 hooks 及其状态。

---

## 15. FAQ

**Q: Hook 脚本可以用什么语言？**

A: 任何可执行文件都可以——Shell、Python、Node.js、Go 二进制等。只要能从 stdin 读取 JSON 并向 stdout 写入 JSON 即可。

**Q: 多个 Hook 匹配同一事件时执行顺序是什么？**

A: 全局 hooks 先于项目 hooks 执行。同一来源内按文件名字母序、文件内按定义顺序执行。

**Q: 一个 PreToolUse hook deny 后，后续 hook 还会执行吗？**

A: 不会。第一个 deny 决策立即生效，后续 hook 被跳过。

**Q: Hook 可以修改工具输入吗？**

A: 不可以。Hook 只能 allow/deny，不能修改工具参数。如需修改行为，考虑使用 MCP 服务器包装工具。

**Q: 插件支持热重载吗？**

A: 支持。在 TUI 中使用 `/plugins reload` 或 `/hooks reload` 重新发现和加载。

**Q: 如何为特定项目禁用全局 hook？**

A: 目前不支持按项目禁用全局 hook。可以在 hook 脚本内检查 `$CODEL_WORKSPACE_ROOT` 自行跳过。

**Q: HTTP hook 和 command hook 有什么区别？**

A: 协议相同（JSON in/out），但 HTTP hook 通过 POST 请求发送信封到指定 URL，响应体作为决策。适合集中式策略服务。

---

## 附录：事件信封完整字段参考

### 通用字段（所有事件）

```json
{
  "hookEventName": "string",
  "sessionId": "string",
  "cwd": "string",
  "workspaceRoot": "string",
  "timestamp": "ISO-8601",
  "transcriptPath": "string | null",
  "clientIdentifier": "string | null",
  "promptId": "string | null",
  "permissionMode": "default | auto | plan | bypassPermissions"
}
```

### PreToolUse 额外字段

```json
{
  "toolName": "run_terminal_cmd",
  "toolUseId": "tool_xxx",
  "toolInput": {},
  "toolInputTruncated": false,
  "subagentType": "explore | null"
}
```

### PostToolUse 额外字段

```json
{
  "toolName": "string",
  "toolUseId": "string",
  "toolInput": {},
  "toolResult": {},
  "toolInputTruncated": false,
  "toolResultTruncated": false,
  "durationMs": 1234,
  "isBackgrounded": false,
  "subagentType": "string | null"
}
```

### Stop 额外字段

```json
{
  "reason": "end_turn",
  "stopHookActive": false,
  "lastAssistantMessage": "string | null",
  "backgroundTasks": [{"id": "", "type": "shell|monitor|subagent", "status": "running", "description": "", "command": "", "agentType": ""}],
  "sessionCrons": [{"id": "", "schedule": "every 5 minutes", "recurring": true, "prompt": ""}]
}
```

### SessionStart 额外字段

```json
{
  "source": "startup | resume | clear",
  "modelId": "string | null",
  "agentType": "string | null"
}
```

### SessionEnd 额外字段

```json
{
  "reason": "string",
  "turnCount": 42,
  "toolCallCount": 128
}
```
