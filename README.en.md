<div align="center">

# ptygrid

**English** · [日本語 (Japanese)](README.md)

**A lightweight, native terminal for running and coordinating multiple AI agent CLIs side by side on a single screen**

Run Claude Code / Codex / Grok simultaneously in split panes, and let the agents "read, instruct, and launch each other" through the built-in MCP server, **Queen**.

[![Tauri](https://img.shields.io/badge/Tauri-v2-24C8D8?logo=tauri&logoColor=white)](https://v2.tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-backend-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Svelte](https://img.shields.io/badge/Svelte-5-FF3E00?logo=svelte&logoColor=white)](https://svelte.dev/)
[![MCP](https://img.shields.io/badge/MCP-built--in%20server-8A2BE2)](https://modelcontextprotocol.io/)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20beta-lightgrey?logo=linux)](#system-requirements)
[![Status](https://img.shields.io/badge/status-v0.4.7-brightgreen)](#roadmap)

[User Guide](docs/userguide.en.md) · [Design Document](docs/design.en.md) · [Linux / Windows Porting](docs/porting.en.md) · [Competitive Analysis](docs/competitive-landscape.en.md) · [Troubleshooting](docs/troubleshooting.en.md)

<img src="docs/screenshot-phase0.4.5.png" width="1100" alt="ptygrid v0.4.5 running Claude Code ×2, Codex, and Grok across four panes. The left dock shows a per-pane semantic status list (Status / Git tabs); the bottom footer shows Queen, Teammates, CPU/memory, and pane count." />

</div>

---

> [!NOTE]
> The project was renamed from its old working title **multi-terminal** to **ptygrid** (pty + grid). The config file name has also changed to `ptygrid.yml` (the old `mterm.yml` is still loaded for compatibility, and when both exist, `ptygrid.yml` takes precedence).

## ✨ Features

- 🪟 **Split grid (up to 9 panes)** — freely resizable. Per-pane restart / close / maximize, plus a status dot (running / exited / restarting + exit code)
- 📝 **config-as-code (`ptygrid.yml`)** — define agents and processes in YAML. Launch them all at once with autostart, and reload on change via file watching
- 👑 **Queen (built-in MCP server)** — agent CLIs use MCP tools to read, write to, launch, and notify other panes
- 📌 **Shared Pins / Notes** — per-project persistent memos shared through Queen. Revision-conflict detection prevents concurrent updates from silently overwriting each other
- 📬 **Durable Inbox / Reply** — asynchronous agent-to-agent messaging with stable message IDs, acknowledgements, and thread correlation
- ⏳ **Cancellable Await** — wait for new Inbox arrivals past a cursor without busy polling. Supports timeouts and MCP cancellation
- 🔒 **Allowlist-based spawn** — `spawn_agent` can only launch names defined in ptygrid.yml, and it binds to 127.0.0.1 only
- 🎯 **Unambiguous addressing** — every pane shows a `#id`. When multiple panes run the same CLI, target one precisely with `agent: "#3"`; sends that rely on guessing a name are rejected
- 🔁 **autorestart** — never / on-failure / always (aborts after 5 consecutive attempts). A restart preserves the pane and its session ID
- 🌿 **Git / Worktree** — status, diff, stage, unstage, and commit, plus optional linked-worktree isolation per definition
- 📊 **Resource monitoring** — per-pane process-tree CPU/RSS, plus an all-sessions total in the toolbar
- 💾 **Logical session restore** — saves the project, pane order, layout, and definitions, and restarts via a `resume` command of your choosing
- 🧹 **Readable output sharing** — `read_output` returns text reconstructed to match the pane's dimensions, resolving ANSI cursor moves, screen clears, and the alternate screen (handling TUI full-screen redraws and leftover spinner artifacts)
- 🌐 **English/Japanese UI** — switch via the ⚙ settings menu: Auto/English/日本語 (defaults to the OS language)
- 🪶 **Native and lightweight** — no Electron. Rust + Tauri v2 + portable-pty

## 🏗️ How it works

```
┌─ ptygrid ────────────────────────────────────────────┐
│  ┌─────────┐  ┌─────────┐  ┌─────────┐               │
│  │ claude  │  │ codex   │  │ grok    │ ← each pane = │
│  └────┬────┘  └────┬────┘  └────┬────┘    PTY + xterm.js
│       │            │            │                     │
│  ┌────┴────────────┴────────────┴──────────────────┐ │
│  │ Session Manager / Monitor / Git / State         │ │
│  │ portable-pty · sysinfo · installed git          │ │
│  ├─────────────────────────────────────────────────┤ │
│  │ 👑 Queen — MCP server (rmcp, streamable HTTP)   │ │
│  │    list_agents / read_output / send_message /   │ │
│  │    spawn / notify / pins / notes / inbox / await│ │
│  │    durable data: SQLite (project-scoped)        │ │
│  └─────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────┘
         ▲ MCP (http://127.0.0.1:39237/mcp)
         └─ each agent CLI inside a pane calls Queen as a tool
```

Ask the Claude Code in one pane to "**read codex's output and summarize it**," and it actually happens through Queen. When multiple panes run the same CLI, address one by the session ID shown in its header, for example "**ask `#3` for a review**."

## 🚀 Quick Start

Prerequisites: Rust (rustup), Node.js 20+, Git, and the OS-specific Tauri dependencies.

- macOS: Xcode Command Line Tools
- Ubuntu / Debian: `libwebkit2gtk-4.1-dev` and others (see [Linux setup](docs/userguide.en.md#linuxubuntu--debian) for details)

```bash
git clone https://github.com/zephel01/ptygrid.git
cd ptygrid
npm install
npm run tauri dev    # first run takes a few minutes for the Rust build
```

A window opens and `$SHELL` (zsh, etc.) starts in a single pane.

### Defining agents

Enter your target folder (e.g. `~/works/hoge`; a leading `~` is allowed) in the toolbar's "Working folder" field to load it. The config file `ptygrid.yml` is searched for in the order **the working folder → the app's launch folder → `~/.ptygrid/`**, so you can place it directly in the project, or put settings shared across multiple projects in `~/.ptygrid/ptygrid.yml` (the old name `mterm.yml` is loaded for compatibility only from within the working folder) ([annotated sample](ptygrid.example.yml) / [samples by use case](example/README.md)):

```yaml
project: my-app

agents:
  - name: claude
    cmd: "claude"
    cwd: "."
    autostart: true
  - name: codex
    cmd: "codex"

processes:
  - name: web
    cmd: "npm run dev"
    autorestart: on-failure
```

The first time you load a project (a `ptygrid.yml` from the working folder or the launch folder), a **"Do you trust this folder?" confirmation** appears exactly once, so that autostart commands aren't run without your consent. Choosing "Trust and launch" remembers that folder from then on, and the confirmation no longer appears (the global config in `~/.ptygrid` is always trusted). For details, see "Trust confirmation" in [docs/userguide.en.md](docs/userguide.en.md).

### Registering Queen with each CLI

Click the "● Queen :39237" badge on the right of the toolbar to copy a registration command that includes the auth token. `/mcp` is protected by token plus Host/Origin validation, and the URL carries a `?token=<token>`. **The token changes every time the app starts (it is not persistent), so re-register after a restart.** The `<token>` below is a placeholder — use the copy from the badge in practice.

```bash
# Claude Code (-s user is required; watch out: the local scope is limited to the directory)
claude mcp add -s user --transport http queen "http://127.0.0.1:39237/mcp?token=<token>"

# Grok CLI
grok mcp add -s user -t http queen "http://127.0.0.1:39237/mcp?token=<token>"
```

```toml
# Codex CLI (~/.codex/config.toml)
[mcp_servers.queen]
url = "http://127.0.0.1:39237/mcp?token=<token>"
```

For detailed usage, see the **[User Guide](docs/userguide.en.md)**, and for common pitfalls, see [Troubleshooting](docs/troubleshooting.en.md).

## 🧰 Tech Stack

| Layer | Technology |
|---|---|
| App shell | Tauri v2 (Rust backend + WebView) |
| PTY | portable-pty 0.9 (by wezterm) |
| MCP server | rmcp (official Rust SDK) / streamable HTTP |
| Frontend | Svelte 5 (runes) + @xterm/xterm + svelte-splitpanes + Vite 6 |
| Config | serde_norway (YAML) + notify (file watching) |
| Git | runs the installed `git` directly, without going through a shell |
| Resource monitoring | sysinfo (aggregates the process tree via a shared sampler) |
| Queen durable data | rusqlite + bundled SQLite (WAL) |

## ✅ Verified

- `pty-core-check/` (portable-pty standalone smoke test): output capture, resize, and kill confirmed by actual runs
- `mcp-server-check/` (rmcp standalone smoke test): initialize → tools/list → tools/call confirmed by actual runs
- `cargo check` + `cargo test` (PTY/session, Git, worktree, state, resource, Queen; 61 tests): passing
- `npm run build` + `svelte-check`: 0 errors / 0 warnings
- GitHub Actions: validates the frontend, Rust tests, clippy, and the Tauri native build on macOS 14 / Ubuntu 22.04

### Checks during development

```bash
npm run check
npm run build
cd src-tauri
cargo check
cargo test
cargo clippy --all-targets --all-features
```

When you change the IPC / MCP schema, update [CONTRACT.md](CONTRACT.md) in the same change; for release progress, update [docs/phase3.md](docs/phase3.md); and for user-visible behavior, update the [User Guide](docs/userguide.en.md) as well.

## 🗺️ Roadmap

- [x] **Phase 0** — single PTY pane
- [x] **Phase 1** — multi-pane + config-as-code (now `ptygrid.yml`, autostart/restart)
- [x] **Phase 2** — Queen (built-in MCP server: 5 core tools)
- [x] **Phase 3.0–3.8** — Git diff/commit, worktree isolation, logical resume, resource monitoring, Queen pins/notes/inbox/reply/await (18 tools)
- [x] **Phase 3.9** — Linux test support (PATH restoration, Ubuntu CI, `.deb` / AppImage packaging)

For the background on this direction, see the [Competitive Analysis](docs/competitive-landscape.en.md) (we chose the "coordinate on one screen" approach rather than a worktree-isolation one). Phase 3 proceeds feature by feature while preserving compatibility, following the [phased release plan](docs/phase3.md).

## 📚 Documentation

| Document | Contents |
|---|---|
| [docs/userguide.en.md](docs/userguide.en.md) | Installation, reading the UI, ptygrid.yml reference, and how to use Queen |
| [docs/design.en.md](docs/design.en.md) | Design document (OSS research, stack selection, architecture) |
| [docs/competitive-landscape.en.md](docs/competitive-landscape.en.md) | Competitive analysis of similar tools and positioning |
| [docs/troubleshooting.en.md](docs/troubleshooting.en.md) | Pitfalls found through real-world dogfooding, and their fixes |
| [docs/phase3.md](docs/phase3.md) | The independent phased release plan for Phase 3, and its progress |
| [docs/plan.md](docs/plan.md) | Work plan (current-status summary, next tasks, versioning conventions) |
| [docs/porting.en.md](docs/porting.en.md) | Linux test-support status, Linux build/package steps, and the Windows porting plan |
| [CONTRACT.md](CONTRACT.md) | The backend ⇄ frontend IPC contract (for developers) |

## System Requirements

**macOS is the primary target**, and Linux has **test support (beta)**. The Linux build and distribution path is based on x86_64 Ubuntu 22.04 / Debian 12 equivalents, and can produce both `.deb` and AppImage artifacts. The AppImage is built on Ubuntu 22.04 CI for glibc compatibility, but stable operation across the various Linux environments will be confirmed through further testing on real hardware and user feedback. Windows has portable-pty/ConPTY branches, but foreground-process detection and testing on real hardware are not yet complete.

## License

Undecided (to be determined at public release)
