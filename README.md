---

<div align="center">

<h1>
  Codel (<code>codel</code>)
</h1>

**Codel** is a terminal-based AI coding agent. It runs as a
full-screen TUI that understands your codebase, edits files, executes shell
commands, searches the web, and manages long-running tasks — interactively,
headlessly for scripting/CI, or embedded in editors via the Agent Client
Protocol (ACP).

[Building from source](#building-from-source) ·
[Documentation](#documentation) ·
[Repository layout](#repository-layout) ·
[Development](#development) ·
[Contributing](#contributing) ·
[License](#license)

This repository contains the Rust source for the `codel` CLI/TUI and its agent
runtime.

A small `SOURCE_REV` file at the root records the full monorepo commit SHA
for the version of the code present in this tree.

</div>

---

## Building from source

Requirements:

- **Rust** — the toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml);
  `rustup` installs it automatically on first build.
- **[DotSlash](https://dotslash-cli.com)** — required so hermetic tools under
  [`bin/`](bin/) (notably [`bin/protoc`](bin/protoc)) can download and run.
  Install it and ensure `dotslash` is on your `PATH` **before** building:

  ```sh
  cargo install dotslash
  # or: prebuilt packages — https://dotslash-cli.com/docs/installation/
  /usr/bin/env dotslash --help   # sanity check
  ```

- **protoc** — proto codegen resolves [`bin/protoc`](bin/protoc) via DotSlash,
  or falls back to a `protoc` on `PATH` / `$PROTOC`.
- macOS and Linux are supported build hosts; Windows builds are best-effort
  and not currently tested from this tree.

```sh
cargo run -p codel-pager-bin              # build + launch the TUI
cargo build -p codel-pager-bin --release  # release binary: target/release/codel-pager
cargo check -p codel-pager-bin            # fast validation
```

The binary artifact is named `codel-pager`; official installs ship it as
`codel`.

## Authentication

Codel uses API key authentication. Set the `CODEL_API_KEY` environment variable:

```sh
export CODEL_API_KEY="your-api-key"
```

## ACP Integration

Codel implements the [Agent Client Protocol (ACP)](https://agentclientprotocol.com) v1,
allowing it to be embedded in any ACP-compatible editor or client (Zed, JetBrains, etc.).

### Starting the ACP server

```sh
# stdio mode (recommended — standard ACP transport)
codel agent stdio

# WebSocket server mode
codel agent serve --bind 127.0.0.1:2419 --secret <token>
```

### Client configuration

**Zed IDE** (`~/.config/zed/settings.json`):

```json
{
  "agent_servers": {
    "Codel": {
      "type": "custom",
      "command": "codel",
      "args": ["agent", "stdio"],
      "env": {
        "CODEL_API_KEY": "your-api-key"
      }
    }
  }
}
```

**Generic ACP client** — spawn `codel agent stdio` as a subprocess and
communicate over stdin/stdout using JSON-RPC 2.0.

### Connection flow

1. `initialize` → negotiate protocol version, receive capabilities and auth methods
2. `authenticate` → authenticate with API key or session token
3. `session/new` → create a new session (or `session/load` / `session/resume`)
4. `session/prompt` → send a prompt, receive streaming `session/update` notifications
5. `session/cancel` → cancel an in-flight prompt

### Supported ACP capabilities

| Capability | Status | Notes |
|------------|--------|-------|
| `initialize` | ✅ | Returns protocol version, capabilities, auth methods |
| `authenticate` | ✅ | API key and session-based auth |
| `session/new` | ✅ | Create session with cwd + MCP servers |
| `session/load` | ✅ | Load session with full history replay |
| `session/resume` | ✅ | Resume session without history replay |
| `session/list` | ✅ | List sessions with cwd filter and cursor pagination |
| `session/close` | ✅ | Close an active session |
| `session/prompt` | ✅ | Streaming prompt with content blocks |
| `session/cancel` | ✅ | Cancel in-flight prompt |
| `session/set_mode` | ✅ | Switch session mode |
| `session/set_model` | ✅ | Switch model mid-session |
| `session/update` | ✅ | Streaming notifications (text, tool calls, diffs, plans) |
| `session/request_permission` | ✅ | Tool permission prompts |
| `read_text_file` / `write_text_file` | ✅ | Client filesystem access |
| `terminal/*` | ✅ | Create, output, release, wait, kill terminals |
| Slash commands | ✅ | Advertised via `available_commands_update` |
| MCP servers | ✅ | HTTP, SSE, and stdio transports |
| Agent plan | ✅ | Plan content blocks in session updates |
| Tool calls | ✅ | Tool call lifecycle in session updates |
| `session/delete` | ⚠️ | Via ext_method (`codel/session/delete`) |
| `session/config_options` | 🔜 | Planned (model, mode, reasoning effort selectors) |
| `elicitation` | 🔜 | Planned (structured user input) |
| `agentInfo` | 🔜 | Planned |

### Extension methods

Codel exposes additional functionality via ACP `ext_method`:

| Method | Description |
|--------|-------------|
| `codel/mcp/list` | List MCP servers and their status |
| `codel/mcp/auth_trigger` | Trigger MCP OAuth authentication |
| `codel/mcp/toggle` | Enable/disable an MCP server |
| `codel/session/list` | List sessions (extended format with facets) |
| `codel/session/close` | Close a session |
| `codel/session/delete` | Delete a session from history |
| `codel/session/info` | Get session details |
| `codel/hooks/*` | Hook management |
| `codel/plugins/*` | Plugin management |

## Documentation

The user guide ships with the pager crate:
[`crates/codegen/codel-pager/docs/user-guide/`](crates/codegen/codel-pager/docs/user-guide/)
— getting started, keyboard shortcuts, slash commands, configuration, theming,
MCP servers, skills, plugins, hooks, headless mode, sandboxing, and more.

## Repository layout

| Path | Contents |
|------|----------|
| `crates/codegen/codel-pager-bin` | Composition-root package; builds the `codel-pager` binary |
| `crates/codegen/codel-pager` | The TUI: scrollback, prompt, modals, rendering |
| `crates/codegen/codel-shell` | Agent runtime + leader/stdio/headless entry points |
| `crates/codegen/codel-tools` | Tool implementations (terminal, file edit, search, ...) |
| `crates/codegen/codel-workspace` | Host filesystem, VCS, execution, checkpoints |
| `crates/codegen/...` | The rest of the CLI crate closure (config, MCP, markdown, sandbox, ...) |
| `crates/common/`, `crates/build/`, `prod/mc/` | Small shared leaf crates pulled in by the closure |

> [!IMPORTANT]
> The root `Cargo.toml` (workspace members, dependency versions, lints,
> profiles) is **generated** — treat it as read-only. Prefer editing per-crate
> `Cargo.toml` files.

## Development

```sh
cargo check -p <crate>        # always target specific crates; full-workspace builds are slow
cargo test -p codel-config    # per-crate tests
cargo clippy -p <crate>       # lint config: clippy.toml at the repo root
cargo fmt --all               # rustfmt.toml at the repo root
```

## Contributing

> [!NOTE]
> External contributions are not accepted. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

First-party code in this repository is licensed under the **Apache License,
Version 2.0** — see [`LICENSE`](LICENSE).

Third-party and vendored code remains under its original licenses. See:

- [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES) — crates.io / git dependencies,
  bundled UI themes, and **in-tree source ports** (including openai/codex and
  sst/opencode tool implementations)
- [`crates/codegen/codel-tools/THIRD_PARTY_NOTICES.md`](crates/codegen/codel-tools/THIRD_PARTY_NOTICES.md)
  — crate-local notice for the codex and opencode ports (license texts +
  Apache §4(b) change notice)
