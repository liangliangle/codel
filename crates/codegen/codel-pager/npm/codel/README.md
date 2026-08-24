# Codel

Bring Codel into your terminal. Fast, flicker-free CLI built for plans, subagents, and parallel work.

**[Homepage](https://codel.dev/cli)** | **[Documentation](https://docs.codel.dev/build/overview)**

## Install

```bash
curl -fsSL https://codel.dev/cli/install.sh | bash
```

Or install with npm:

```bash
npm i -g @codel-official/codel
```

## Get Started

```bash
# Launch the interactive TUI
codel

# Run a single task
codel -p "Explain this codebase"
```

On first launch, Codel opens your browser to authenticate. For CI or headless environments, use an API key from [console.codel.dev](https://console.codel.dev):

```bash
export CODEL_API_KEY="codel-..."
```

## Update

```bash
codel update
```

Or if installed via npm:

```bash
npm i -g @codel-official/codel@latest
```

## Supported Platforms

| Platform | Architecture |
|---|---|
| macOS | Apple Silicon (arm64) |
| Linux | x86_64, arm64 |
| Windows | x86_64 |

## Documentation

For full documentation including configuration, MCP servers, custom models, headless mode, agent mode, and more, visit [docs.codel.dev/build/overview](https://docs.codel.dev/build/overview).

## Feedback

Run `/feedback` inside Codel to report issues or send feedback directly.
