<div align="center">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://media.codel/v1/website/spacecodel-symbol-white-transparent-0c31957f.png">
    <source media="(prefers-color-scheme: light)" srcset="https://media.codel/v1/website/spacecodel-symbol-black-transparent-6435cf42.png">
    <img alt="SpaceCODEL logo" src="https://media.codel/v1/website/spacecodel-symbol-black-transparent-6435cf42.png" width="96">
  </picture>
  <br>
  Codel Build (<code>codel</code>)
</h1>

**Codel Build** is SpaceCODEL's terminal-based AI coding agent. It runs as a
full-screen TUI that understands your codebase, edits files, executes shell
commands, searches the web, and manages long-running tasks — interactively,
headlessly for scripting/CI, or embedded in editors via the Agent Client
Protocol (ACP).

[Installing the released binary](#installing-the-released-binary) ·
[Building from source](#building-from-source) ·
[Documentation](#documentation) ·
[Repository layout](#repository-layout) ·
[Development](#development) ·
[Contributing](#contributing) ·
[License](#license)

![Codel Build TUI](https://media.codel/v1/website/universe-tui-screenshot-6f7a0837.png)

**Learn more about Codel Build at [codel/cli](https://codel/cli)**

This repository contains the Rust source for the `codel` CLI/TUI and its agent
runtime. It is synced periodically from the SpaceCODEL monorepo.

A small `SOURCE_REV` file at the root records the full monorepo commit SHA
for the version of the code present in this tree.

</div>

---

## Installing the released binary

Prebuilt binaries are published for macOS, Linux, and Windows:

```sh
curl -fsSL https://codel/cli/install.sh | bash   # macOS / Linux / Git Bash
irm https://codel/cli/install.ps1 | iex          # Windows PowerShell
codel --version
```

See the [changelog](https://codel/build/changelog) for the latest fixes,
features, and improvements in each release.

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
`codel`. On first launch it opens your browser to authenticate — see the
[authentication guide](crates/codegen/codel-pager/docs/user-guide/02-authentication.md).

## Documentation

Full online documentation is available at
[docs.codel/build/overview](https://docs.codel/build/overview).

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
| `third_party/` | Vendored upstream source (Mermaid diagram stack) — see below |

> [!IMPORTANT]
> The root `Cargo.toml` (workspace members, dependency versions, lints,
> profiles) is **generated** — treat it as read-only. Prefer editing per-crate
> `Cargo.toml` files.

## Development

```sh
cargo check -p <crate>        # always target specific crates; full-workspace builds are slow
cargo test -p codel-config # per-crate tests
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
- [`third_party/NOTICE`](third_party/NOTICE) — vendored Mermaid-stack index
