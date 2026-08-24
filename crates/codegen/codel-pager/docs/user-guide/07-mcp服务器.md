# MCP 服务器

MCP（Model Context Protocol，模型上下文协议）服务器通过外部工具集成拓展了 Codel 的能力。它们使 Codel 能够与任何实现了 MCP 标准的第三方服务进行交互。

---

## 什么是 MCP 服务器？

MCP 服务器是一个通过标准化协议向 Codel 暴露工具的进程。当你配置一个 MCP 服务器后，其提供的工具将与 Codel 的内置工具一同对大模型可用。模型可以在会话期间自动发现并调用这些工具。

例如，一个 GitHub MCP 服务器可能会暴露像 `create_issue`、`list_pull_requests` 和 `search_code` 这样的工具。一个数据库 MCP 服务器可能会暴露 `query`、`list_tables` 和 `describe_schema`。

参阅 [MCP 规范说明](https://modelcontextprotocol.io)了解协议的具体细节。

---

## 配置说明

MCP 服务器在 `~/.codel/config.toml` 的 `[mcp_servers.<name>]` 部分中配置。

### stdio 传输（本地进程方式）

Codel 启动一个本地进程并基于 stdin/stdout 进行通信：

```toml
[mcp_servers.my-server]
command = "/path/to/server"           # 服务器可执行文件路径
args = ["--flag", "value"]            # 命令参数列表
env = { API_KEY = "sk-..." }          # 环境变量
enabled = true                        # 启用或禁用该服务器 (默认: true)
startup_timeout_sec = 30              # 服务器启动超时时间，秒 (默认: 30)
tool_timeout_sec = 6000               # 单工具调用的退回超时时间，秒 (默认: 6000)
tool_timeouts = { slow_op = 120 }     # 指定工具的超时覆盖时间，秒
```

> **全局启动超时时间覆盖：** 无需为每个服务器单独设置 `startup_timeout_sec`，你可以通过环境变量 `MCP_TIMEOUT`（毫秒，兼容 Claude Code）或 `CODEL_MCP_STARTUP_TIMEOUT_SECS`（秒）修改所有服务器的默认超时时间。服务器专属的 `startup_timeout_sec` 优先级仍然高于这两者。在首次启动时需要下载软件包的冷启动 `npx`/`uvx` 服务器通常需要增大该值；默认值为 30 秒。
>
> **MCP 工具结果大小限制：** 较大的 MCP / `use_tool` 输出结果会被内联截断（完整载荷保存在会话的 `mcp/` 文件夹下）。默认限制为 **20,000 字节**。可以通过以下方式覆盖：
>
> - 环境变量 `CODEL_MAX_MCP_OUTPUT_BYTES` 或 `MAX_MCP_OUTPUT_BYTES`（字节；两者同时设置时 Codel 原生变量优先；为 Claude 风格的变量名，但我们按**字节**而非 Token 进行限制）
> - `config.toml` — 用户级（`~/.codel/config.toml`）**或仓库级**（从当前工作目录向上至 git 根目录链条上的任意 `.codel/config.toml`；层级最深的文件优先，且仓库级数值仅在文件夹受信任后生效）：
>
> ```toml
> [mcp]
> max_output_bytes = 40000
> ```
>
> 优先级：requirements.toml > 环境变量 > 仓库级 `.codel/config.toml` > 用户/托管配置 > 默认值。仓库级的配置修改会通过热重载直接应用到该目录下正在运行的会话中。

### HTTP/SSE 传输（远程服务器方式）

对于通过 HTTP 访问的远程 MCP 服务器：

```toml
[mcp_servers.remote-api]
url = "https://mcp.example.com/api"
headers = { "Authorization" = "Bearer token" }
```

### 带会话 ID 的可流式传输 HTTP

```toml
[mcp_servers.my-streamable-server]
url = "https://mcp.example.com/api/mcp"
headers = { "x-mcp-session-id" = "{{session_id}}" }
```

---

## CLI 命令行管理

无需编辑配置文件，直接在命令行管理 MCP 服务器：

```bash
# 列出已配置的 MCP 服务器
codel mcp list
codel mcp list --json          # 机器可读的 JSON 输出

# 添加一个 stdio 服务器。-- 之后的所有内容均作为服务器的命令，因此像 -y 这样的标志会传给服务器而非被 codel 解析。
codel mcp add filesystem -- npx -y @modelcontextprotocol/server-filesystem /path/to/dir

# 添加带环境变量的 stdio 服务器 (-e 参数可多次重复使用)
codel mcp add postgres -e DATABASE_URL=postgres://localhost/mydb -- npx -y @modelcontextprotocol/server-postgres

# 添加一个远程 HTTP 服务器
codel mcp add --transport http sentry https://mcp.sentry.dev/mcp

# 添加带身份验证请求头的远程服务器 (--header 参数可多次重复使用)
codel mcp add --transport http api https://mcp.example.com/mcp --header "Authorization: Bearer YOUR_TOKEN"

# 添加一个远程 SSE 服务器
codel mcp add --transport sse linear https://mcp.linear.app/sse

# 移除一个服务器
codel mcp remove github

# 诊断服务器配置与网络连通性
codel mcp doctor               # 检查所有配置的服务器
codel mcp doctor github        # 检查指定的单个服务器
codel mcp doctor --json        # 机器可读的 JSON 输出
```

传输方式默认值为 `stdio`；远程服务器请传入 `--transport http` 或 `--transport sse`。

默认情况下 `codel mcp add` 将配置写入 `~/.codel/config.toml`（`--scope user`）。使用 `--scope project` 可将其写入当前目录下的 `.codel/config.toml` 中，以便提交并与团队共享（参阅[项目级作用域的 MCP 服务器](#项目级作用域的-mcp-服务器)）。请求头和环境变量的值会原样保存，因此请使用 `${VAR}` 引用敏感词，而非将其直接贴入已提交的项目配置中（参阅[配置示例](#配置示例)）。`codel mcp list` 会展示两个作用域内的服务器，并使用 `(project)` 标记项目级作用域的服务器。

`codel mcp remove` 会同时搜索两个作用域，并在成功移除服务器后退出 0。当找不到名称，或者名称同时在用户和项目作用域中定义时退出 1 —— 此时需要传入 `--scope` 明确要删除哪一个。

相比早期版本的破坏性变更：`--env` 现在每次仅接受一个 `KEY=value` 格式（请使用 `-e A=1 -e B=2`，而非 `--env A=1 B=2`），且服务器名称仅能包含字母、数字、连字符和下划线。

---

## 项目级作用域的 MCP 服务器

可以通过在仓库中放置 `.codel/config.toml` 来按项目单独配置 MCP 服务器：

```
my-project/
  .codel/
    config.toml
  src/
  ...
```

```toml
# .codel/config.toml
[mcp_servers.linear]
url = "https://mcp.linear.app/mcp"
enabled = true
```

当服务器暴露了原生的 HTTP/SSE 终端节点时，相比包裹在类似 `npx mcp-remote <url>` 的 stdio 代理中，更推荐使用 `url` 形式。Codel 会直接处理 HTTP/SSE 与 OAuth，因此原生形式可以避免在每个会话中额外启动一个子进程，且会将 Codel 自有的 OAuth 客户端注册到服务提供商。

Codel 会从当前目录向上遍历至 git 仓库根目录，并在每个层级加载 `.codel/config.toml`：

| 位置 | 作用域 | 优先级 |
|----------|-------|----------|
| `~/.codel/config.toml` | 所有项目 | 最低 |
| `<repo-root>/.codel/config.toml` | 当前仓库 | 中等 |
| `<cwd>/.codel/config.toml` | 当前目录 | 最高 |

如果项目定义的服务器与全局服务器同名，项目版本的配置将完全替换全局配置（属性不会合并）。

项目级配置文件仅贡献 `[mcp_servers]`、`[plugins]` 与 `[permission]` 条目。Codel 仅从 `~/.codel/config.toml` 中读取绝大多数其他配置块。

---

## 工具命名规范

MCP 工具使用服务器名称作为命名空间，以防冲突：

- 服务器 `filesystem` 提供的工具 `read_file` 变为 `filesystem__read_file`
- 服务器 `github` 提供的工具 `create_issue` 变为 `github__create_issue`

---

## 运行时动态开关服务器

你可以在会话运行期间随时启用或禁用 MCP 服务器，而无需重启 Codel。

### /mcps 模态框

在 TUI 界面中打开 MCP 服务器模态框：

- 运行 `/mcps` 斜杠指令
- 或按下 `Ctrl+L`（非 VS Code 系列）并导航到 MCP Servers 标签页；在 VS Code 系列中请使用 `/plugins` 或 `/mcp` 并打开 MCP Servers 标签页

在模态框中你可以：

- 查看每个服务器的来源、启用状态及工具数量
- 使用 `Space`（空格键）启用或禁用服务器
- 展开服务器以查看其提供的工具列表
- 在修改 `config.toml` 后按 `r` 键刷新列表
- 按 `i` 键对 OAuth 服务器进行身份验证
- 按 `a` 添加服务器，或按 `x` 移除本地服务器（模态框会要求确认；按小写 `y` 确认删除，按其他按键取消）

### 工具自动发现机制

模型可以使用两个内置工具来配合 MCP 服务器使用：

- `search_tool` — 在所有启用的 MCP 服务器中搜索可用的集成工具。使用此工具可通过名称或描述查找工具。
- `use_tool` — 调用通过 `search_tool` 发现的集成工具。需指定完整的限定工具名称（例如 `github__create_issue`）。

---

## 兼容性说明

为了实现兼容性，Codel 可以从多个来源加载 MCP 服务器配置：

| 来源 | 格式 | 位置 | 是否可配置 |
|--------|--------|----------|-------------|
| `config.toml` | Codel 原生配置格式 | `~/.codel/config.toml`, `.codel/config.toml` | 始终开启 |
| `.claude.json` | Claude Code 格式 | `~/.claude.json` | `[compat.claude] mcps` |
| `.cursor/mcp.json` | Cursor 格式 | `~/.cursor/mcp.json`, `<project>/.cursor/mcp.json` | `[compat.cursor] mcps` |
| `.mcp.json` | MCP 标准格式 | 项目根目录 (从 cwd 到 git root) | 默认加载，除非你已导入或忽略了 Claude 导入提示（已设置导入标记） |

所有来源均按优先级顺序合并：config.toml > Claude > Cursor > `.mcp.json`。当名称发生冲突时，较高优先级来源的服务器优先。

Claude 与 Cursor 的 MCP 来源默认会被自动扫描。若要禁用针对特定厂商的自动扫描，请在 `~/.codel/config.toml` 中设置 `[compat.<vendor>] mcps = false` 或设置对应的环境变量（`CODEL_CURSOR_MCPS_ENABLED`、`CODEL_CLAUDE_MCPS_ENABLED`）。参阅[系统配置](05-系统配置.md#harness-工具链兼容性)获取详情。使用 `codel inspect` 可以查看已加载的 MCP 服务器及其厂商来源（`[cursor]`、`[claude]`）。

---

## MCP OAuth 身份验证

对于需要 OAuth 身份验证的 MCP 服务器，Codel 会自动处理凭据授权流程。当 MCP 服务器请求 OAuth 凭据时，Codel 会打开基于浏览器的授权流程，并保存返回的 Token 以供后续使用。

---

## 配置示例

托管的 MCP 服务请使用 `url` 形式，本地 stdio 工具请使用 `command` / `args` 形式。

### 原生 HTTP（托管服务）

在能够使用基于 OAuth 的 MCP 服务器前，你必须对其进行身份验证。Codel 会将返回的 Token 保存在 `~/.codel/mcp_credentials.json` 下作为仅允许所有者访问（Unix 上权限为 `0600`）的本地明文文件。建议在主机上开启全盘加密。修改 `config.toml` 后，在 `/mcps` 模态框中按 `r` 刷新服务器列表。

```toml
[mcp_servers.linear]
url = "https://mcp.linear.app/mcp"
enabled = true

[mcp_servers.sentry]
url = "https://mcp.sentry.dev/mcp"
enabled = true

[mcp_servers.mixpanel]
url = "https://mcp.mixpanel.com/mcp"
enabled = true
```

对于使用静态 Bearer Token 而非 OAuth 进行身份验证的内部或私有化部署服务器，请显式设置 `Authorization` 请求头：

```toml
[mcp_servers.internal-tools]
url = "https://mcp.internal.example.com/mcp"
enabled = true

[mcp_servers.internal-tools.headers]
Authorization = "Bearer <token>"
```

为了避免在配置文件中暴露敏感信息，可使用 `${VAR}`（或 `${VAR:-default}`）引用环境变量。Codel 会在加载时对 `[mcp_servers.*]` 中的字符串字段 —— 包括 `url`、`command`、`args` 以及 `env` 和 `headers` 中的值 —— 进行展开：

```toml
[mcp_servers.internal-tools]
url = "https://mcp.internal.example.com/mcp"
enabled = true
headers = { "Authorization" = "Bearer ${INTERNAL_MCP_TOKEN}" }
```

### 本地 stdio 模式

对于必须在本地运行的工具（文件系统访问、本地数据库、内部服务器等），请使用 stdio 传输模式。

```toml
# 限定在特定目录下的文件系统访问
[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed/directory"]

# 本地 Postgres 数据库
[mcp_servers.postgres]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-postgres", "postgresql://user:pass@localhost/db"]

# 包含更长启动超时和针对单工具超时调优的自定义服务器
[mcp_servers.my-tools]
command = "/usr/local/bin/my-mcp-server"
args = ["--config", "/etc/my-mcp.json"]
startup_timeout_sec = 30
tool_timeout_sec = 120
tool_timeouts = { slow_analysis = 300, quick_lookup = 10 }
```

在 Windows 系统上，npm 会将 `npx`、`npm`、`pnpm` 和 `yarn` 等启动程序安装为 `.cmd` 批处理文件（并不存在 `npx.exe`）。Codel 在启动前会将像 `npx` 这样的裸 `command` 解析为在 `PATH` 上真实存在的启动器路径（遵循 `PATHEXT`），因此无需手动将它们包裹在 `cmd /c` 中。作为绝对路径或包含路径分隔符的 `command` 会按原样直接使用。

---

## 常用 MCP 服务器清单

下面是通过上述 `url` 或 `command` 形式配置的常见 MCP 服务器的部分清单。在使用前请向服务提供商确认最新的终端节点或软件包名称：

| 服务器名称 | 传输模式 | 终端节点 / 软件包名称 |
|--------|-----------|--------------------|
| Linear | HTTP (OAuth) | `https://mcp.linear.app/mcp` |
| Sentry | HTTP (OAuth) | `https://mcp.sentry.dev/mcp` |
| Mixpanel | HTTP (OAuth) | `https://mcp.mixpanel.com/mcp` |
| Filesystem | stdio | `@modelcontextprotocol/server-filesystem` |
| Git | stdio | `@modelcontextprotocol/server-git` |
| GitHub | stdio | `@modelcontextprotocol/server-github` |
| GitLab | stdio | `@modelcontextprotocol/server-gitlab` |
| PostgreSQL | stdio | `@modelcontextprotocol/server-postgres` |
| SQLite | stdio | `@modelcontextprotocol/server-sqlite` |
| Puppeteer | stdio | `@modelcontextprotocol/server-puppeteer` |

参阅 [MCP 服务器注册表](https://github.com/modelcontextprotocol/servers)了解社区服务器的完整列表，以及参阅 [MCP 规范说明](https://modelcontextprotocol.io)了解协议详情。

---

## 故障排查

### 服务器无法启动

```bash
# 手动测试服务器命令
npx -y @modelcontextprotocol/server-filesystem /path

# 增加启动超时时间
# 在 config.toml 中配置：
[mcp_servers.filesystem]
startup_timeout_sec = 30
```

对于 stdio 服务器，Codel 会将进程的标准错误输出捕获到 `~/.codel/logs/mcp/<server>.stderr.log` 中（每次启动时截断）。当服务器启动但握手失败时，请检查该日志文件：

```bash
tail -f ~/.codel/logs/mcp/filesystem.stderr.log
```

### 查看服务器状态

使用 `codel inspect` 查看所有已加载的 MCP 服务器及其来源：

```bash
codel inspect          # 人类可读格式
codel inspect --json   # 机器可读格式
```

### 调试日志记录

```bash
RUST_LOG=debug CODEL_LOG_FILE=/tmp/codel.log codel
tail -f /tmp/codel.log
```

查找包含 `mcp` 的日志条目，以跟踪服务器启动、工具发现及工具调用执行过程。
