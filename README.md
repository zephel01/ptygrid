<div align="center">

# ptygrid

**複数の AI エージェント CLI を1画面で並行実行・協調させる、軽量ネイティブターミナル**

Claude Code / Codex / Grok をスプリットペインで同時に走らせ、内蔵 MCP サーバー **Queen** で
エージェント同士が「他ペインを読む・指示する・起動する」を実現します。

[![Tauri](https://img.shields.io/badge/Tauri-v2-24C8D8?logo=tauri&logoColor=white)](https://v2.tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-backend-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Svelte](https://img.shields.io/badge/Svelte-5-FF3E00?logo=svelte&logoColor=white)](https://svelte.dev/)
[![MCP](https://img.shields.io/badge/MCP-built--in%20server-8A2BE2)](https://modelcontextprotocol.io/)
[![Platform](https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple)](#動作環境)
[![Status](https://img.shields.io/badge/status-Phase%203.6-brightgreen)](#ロードマップ)

[ユーザーガイド](docs/userguide.md) · [設計ドキュメント](docs/design.md) · [競合調査](docs/competitive-landscape.md) · [トラブルシューティング](docs/troubleshooting.md)

<img src="docs/screenshot-phase3.6.png" width="1100" alt="ptygrid Phase 3.6: Claude Code、Codex、Grok、shellを4ペインで実行し、ペイン別と全体のCPU・メモリ、Queenの稼働状態を表示している画面" />

</div>

---

> [!NOTE]
> 旧仮称 **multi-terminal** から **ptygrid**(pty + grid)に改名しました。設定ファイル名 `mterm.yml` は当面互換のためそのままです。

## ✨ 特徴

- 🪟 **スプリットグリッド(最大9ペイン)** — リサイズ自由。ペインごとに restart / close / maximize、状態ドット(running / exited / restarting + exit code)
- 📝 **config-as-code(`mterm.yml`)** — エージェントとプロセスを YAML で定義。autostart で一斉起動、変更を監視して Reload
- 👑 **Queen(内蔵 MCP サーバー)** — エージェント CLI が MCP ツールとして他ペインを読む・書く・起動する・通知する
- 📌 **共有Pins / Notes** — project単位の永続メモをQueen経由で共有。revision競合検出で同時更新の上書き消失を防止
- 🔒 **許可リスト方式の spawn** — `spawn_agent` は mterm.yml で定義された名前しか起動できない。bind は 127.0.0.1 のみ
- 🎯 **曖昧でない宛先指定** — 全ペインに`#id`を表示。同名CLIが複数なら`agent: "#3"`で厳密指定し、名前の推測送信を拒否
- 🔁 **autorestart** — never / on-failure / always(連続5回で打ち切り)。restart してもペインとセッション ID を維持
- 🌿 **Git / Worktree** — status・diff・stage・unstage・commitと、定義ごとの任意linked-worktree分離
- 📊 **リソース監視** — ペインごとのprocess tree CPU/RSSと、ツールバーの全セッション合計
- 💾 **論理セッション復元** — project・ペイン順・layout・定義を保存し、任意の`resume` commandで再起動
- 🧹 **読みやすい出力共有** — `read_output` は ANSI エスケープ除去 + `\r` 上書きの畳み込み済みテキストを返す(TUI スピナー残骸対策)
- 🪶 **ネイティブで軽量** — Electron 不使用。Rust + Tauri v2 + portable-pty

## 🏗️ 仕組み

```
┌─ ptygrid ────────────────────────────────────────────┐
│  ┌─────────┐  ┌─────────┐  ┌─────────┐               │
│  │ claude  │  │ codex   │  │ grok    │  ← 各ペイン = │
│  └────┬────┘  └────┬────┘  └────┬────┘    PTY + xterm.js
│       │            │            │                     │
│  ┌────┴────────────┴────────────┴──────────────────┐ │
│  │ Session Manager / Monitor / Git / State         │ │
│  │ portable-pty · sysinfo · installed git          │ │
│  ├─────────────────────────────────────────────────┤ │
│  │ 👑 Queen — MCP server (rmcp, streamable HTTP)   │ │
│  │    list_agents / read_output / send_message /   │ │
│  │    spawn_agent / notify / pins / notes          │ │
│  │    durable data: SQLite (project-scoped)        │ │
│  └─────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────┘
         ▲ MCP (http://127.0.0.1:39237/mcp)
         └─ ペイン内の各エージェント CLI が Queen をツールとして呼ぶ
```

ペイン内の Claude Code に「**codex の出力を読んで要約して**」と頼むと、Queen 経由で実際に動きます。
同じCLIが複数ある場合は「**`#3`にレビューを依頼して**」のように、ヘッダーに表示された
session IDを指定します。

## 🚀 クイックスタート

前提: Rust (rustup), Node.js 20+, Xcode Command Line Tools

```bash
git clone https://github.com/zephel01/ptygrid.git
cd ptygrid
npm install
npm run tauri dev    # 初回は Rust ビルドで数分
```

ウィンドウが開き、`$SHELL`(zsh 等)が1ペインで起動します。

### エージェントを定義する

プロジェクトルートに `mterm.yml` を置いてツールバーから読み込みます([サンプル](mterm.example.yml)):

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

### Queen を各 CLI に登録する

ツールバー右の「● Queen :39237」バッジをクリックすると登録コマンドがコピーされます。

```bash
# Claude Code(-s user 必須。local スコープはディレクトリ限定になる罠あり)
claude mcp add -s user --transport http queen http://127.0.0.1:39237/mcp

# Grok CLI
grok mcp add -s user -t http queen http://127.0.0.1:39237/mcp
```

```toml
# Codex CLI (~/.codex/config.toml)
[mcp_servers.queen]
url = "http://127.0.0.1:39237/mcp"
```

詳しい使い方は **[ユーザーガイド](docs/userguide.md)** を、ハマりどころは [トラブルシューティング](docs/troubleshooting.md) を参照してください。

## 🧰 技術スタック

| レイヤ | 採用技術 |
|---|---|
| アプリ枠 | Tauri v2(Rust バックエンド + WebView) |
| PTY | portable-pty 0.9(wezterm 製) |
| MCP サーバー | rmcp(公式 Rust SDK)/ streamable HTTP |
| フロント | Svelte 5 (runes) + @xterm/xterm + svelte-splitpanes + Vite 6 |
| 設定 | serde_norway(YAML)+ notify(ファイル監視) |
| Git | インストール済み`git`をshellを介さず実行 |
| リソース監視 | sysinfo(process treeを共有samplerで集計) |
| Queen永続データ | rusqlite + bundled SQLite(WAL) |

## ✅ 検証済み

- `pty-core-check/`(portable-pty 単体スモークテスト): 出力キャプチャ・resize・kill を実走確認
- `mcp-server-check/`(rmcp 単体スモークテスト): initialize → tools/list → tools/call を実走確認
- `cargo check` + `cargo test`(PTY/session・Git・worktree・state・resource・Queen、53 tests): 合格
- `npm run build` + `svelte-check`: 0 errors / 0 warnings

### 開発時のチェック

```bash
npm run check
npm run build
cd src-tauri
cargo check
cargo test
cargo clippy --all-targets --all-features
```

IPC / MCP schemaを変更する場合は[CONTRACT.md](CONTRACT.md)、release進捗は
[docs/phase3.md](docs/phase3.md)、user-visibleな操作は[ユーザーガイド](docs/userguide.md)も
同じ変更で更新します。

## 🗺️ ロードマップ

- [x] **Phase 0** — 単一 PTY ペイン
- [x] **Phase 1** — マルチペイン + `mterm.yml`(config-as-code, autostart/restart)
- [x] **Phase 2** — Queen(内蔵 MCP サーバー: 基本5 tools)
- [x] **Phase 3.0–3.6** — Git diff/commit・worktree分離・logical resume・リソース監視・Queen pins/notes(13 tools)
- [ ] **Phase 3.7** — Queen inbox/reply
- [ ] **Phase 3.8** — cancellable Queen `await`

方向性の背景は [競合調査](docs/competitive-landscape.md) を参照(worktree 隔離系ではなく「同一画面で協調する系」を選んでいます)。
Phase 3 は [段階リリース計画](docs/phase3.md) に沿って、互換性を保ちながら機能単位で進めます。

## 📚 ドキュメント

| ドキュメント | 内容 |
|---|---|
| [docs/userguide.md](docs/userguide.md) | インストール・画面の見方・mterm.yml リファレンス・Queen の使い方 |
| [docs/design.md](docs/design.md) | 設計ドキュメント(OSS 調査・スタック選定・アーキテクチャ) |
| [docs/competitive-landscape.md](docs/competitive-landscape.md) | 類似ツールの競合調査とポジショニング |
| [docs/troubleshooting.md](docs/troubleshooting.md) | 実際のドッグフーディングで判明した罠と対処 |
| [docs/phase3.md](docs/phase3.md) | Phase 3の段階独立リリース計画と進捗 |
| [CONTRACT.md](CONTRACT.md) | backend ⇄ frontend の IPC 契約(開発者向け) |

## 動作環境

現在は **macOS(Apple Silicon 中心)** で開発・検証しています。Linux / Windows対応は今後の課題です。

## License

未定(公開時に決定予定)
