# マルチエージェント統合ターミナル 設計ドキュメント（v0.1）

> HiveTerm 相当の「複数 AI エージェント（Claude Code / Codex / Grok CLI）をスプリットペインで並行実行する統合ワークスペース」を自作するための、OSS 調査・fork判断・スタック選定・アーキ設計・段階的MVP計画。
> 対象環境: macOS（Apple Silicon）中心、Linux/Windows もクロスプラットフォームで視野。作者スキル: Rust / TypeScript / ローカルLLM運用。

作成日: 2026-07-15 ／ プロジェクト仮称: **multi-terminal**

> **2026-07-16 追記**: 製品名を **ptygrid**(pty + grid)に決定(GitHub / npm / crates.io の重複調査済み、[競合調査](competitive-landscape.md) と命名調査に基づく)。公開先: `github.com/zephel01/ptygrid`。本文中の「multi-terminal」は旧仮称。

---

## 0. 結論（先に要点だけ）

- **HiveTerm 本体は OSS ではなく有料クローズド製品**（$99/年、Rust + Tauri、~20MB ネイティブ）。クローンの「実装参考」にはできても、コードを流用することはできない。
- OSS の類似ツールは複数あるが、**方向性が2系統**に割れる:
  1. **tmux + git worktree 系 TUI**（Claude Squad など） … 実装は軽いが「ネイティブGUI + グリッドペイン + MCP相互通信」という HiveTerm の体験には遠い。
  2. **デスクトップ GUI 系**（Crystal / cmux / Parallel Code など） … 体験は近いが、多くが Electron/独自設計で、あなたの理想（Tauri + MCP "Queen" + mterm.yml（HiveTermのhive.yml相当））とは細部が違う。
- **推奨は「fork ではなく自作」**。理由は後述（ライセンス・設計思想・学習価値の3点）。ただし **Claude Squad と Crystal を"読む対象"として最大活用**する。
- **スタックは Tauri v2 + Rust バックエンド + Svelte フロント + `portable-pty` + 公式 MCP SDK `rmcp`** を推奨。HiveTerm と同じ土俵で、あなたの Rust 経験を活かせる。
- 進め方は **4フェーズの段階的MVP**（Phase 0: 単一PTY表示 → Phase 1: マルチペイン+config → Phase 2: MCPサーバー "Queen" → Phase 3: Git/監視/通知）。まず「動くもの」を Phase 1 まで最短で作る。

---

## 1. 既存OSS調査（2026-07 時点）

### 1.1 参照元 HiveTerm（クローズド）

| 項目 | 内容 |
|---|---|
| 形態 | プロプライエタリ（無料枠: 1プロジェクト/2エージェント、Pro $99/年） |
| スタック | Rust + Tauri（Electron不使用、~20MBネイティブ） |
| コア機能 | スプリットペイングリッド、エージェント間オーケストレーション、mterm.yml（config-as-code）、Queen（MCPサーバー、20ツール）、Git diff/inline commit、内蔵エディタ、Pins & Notes、音声プロンプト（Pro） |
| Queen（MCP） | サブエージェント生成 / エージェント間メッセージ / 共有出力の読み取り / 通知 をプロジェクト単位で分離 |
| mterm.yml | エージェント・プロセスをコードで定義（env / 作業dir / auto-start/restart / 起動時システムプロンプト注入） |
| 対応 | macOS(AS/Intel) / Windows / Linux |

→ **「作りたいもの」の仕様書として最良のリファレンス。** ただしコード非公開なので中身は真似できない。

### 1.2 OSS 類似ツール比較

| ツール | Stars(概算) | スタック | 分離方式 | ライセンス | 体験の近さ | メモ |
|---|---|---|---|---|---|---|
| **Claude Squad** | ~7.5k | Go | tmux + git worktree | **AGPL-3.0** | △(TUI) | 最も定番。設定は config.json。MCP相互通信は無し。fork時 AGPL 感染に注意 |
| **Crystal** | ~3.1k | デスクトップ(Electron系) | git worktree | 要確認 | ◎ | Codex/Claude Code を worktree で並行。GUI体験が近い |
| **cmux** | ~17.3k | デスクトップ | 並行セッション | 要確認 | ◎ | スター最多。並行マルチエージェント特化 |
| **Superset** | ~10.7k | ターミナル | 並行セッション | 要確認 | ○ | 「エージェント向けに作られたターミナル」 |
| **Parallel Code** | ~633 | デスクトップ | git worktree | OSS | ○ | Claude/Codex/Gemini を worktree 分離で同時実行 |
| **amux** | ~186 | Python 単一ファイル | tmux | MIT | △(web) | Webダッシュボード+watchdog+kanban。スマホ操作 |
| **ntm** | ~314 | Python + tmux | tmux pane | MIT | △(TUI) | command palette で複数エージェント調整 |
| **wmux** | 小 | (Windows向け) | 分割端末 | OSS | △ | WSL不要、Windowsネイティブ志向、MCPブラウザ自動化 |

> ライセンスの「要確認」は、fork判断の前に各リポジトリの LICENSE を直接確認すべき項目。特に **AGPL（Claude Squad）はネットワーク配布でもソース公開義務**が及ぶため、将来クローズド化・有償化する可能性があるなら流用は避ける。

### 1.3 分かったこと

- OSS 群の大半は **tmux か git worktree を土台**にしている。「PTYを自前で管理してネイティブGUIに描画」という HiveTerm 型は OSS では少数派。
- つまり **HiveTerm の差別化ポイント = ①ネイティブGUIのグリッドペイン ②MCP "Queen" による相互通信 ③mterm.yml**。ここが自作の価値になる。
- Grok CLI や Codex を含め、各エージェントは結局「CLIをPTYで起動」なので、**PTY抽象化さえ作れば任意のエージェントを載せられる**（HiveTerm と同じ発想）。

---

## 2. fork vs 自作 の判断

| 観点 | fork（Claude Squad等） | 自作 |
|---|---|---|
| 初速 | ◎ すぐ動く | △ ゼロから |
| 理想体験(GUIグリッド/Queen) | △ TUI/設計思想が違い改造が重い | ◎ 思い通り |
| ライセンス自由度 | ✕ AGPL感染リスク | ◎ 自分で選べる |
| 学習価値(Rust/PTY/MCP) | △ 他人の設計を追う | ◎ 深く理解できる |
| 記事化(note/Zenn)の映え | △ | ◎「ガチ自作」シリーズに最適 |
| 保守 | 上流追従が必要 | 自分次第 |

**推奨: 自作。** ただし完全独力ではなく、**Claude Squad（PTY/セッション管理の実装）と Crystal（worktree並行のGUI体験）をコードリーディングの教材**にする。「読んで理解 → 自分の設計で書き直す」が最短で質も高い。ライセンス上も、他コードをコピーせず自分で書けば MIT/Apache-2.0 など好きに選べる。

---

## 3. 技術スタック選定

要求（マルチPTY / ネイティブ軽量 / MCPサーバー内蔵 / ローカルLLM統合 / クロスプラットフォーム）に対する選定。

### 3.1 推奨スタック

| レイヤ | 採用 | 根拠 / 代替 |
|---|---|---|
| アプリ枠 | **Tauri v2** | HiveTerm と同じ。Rust バックエンド + Web フロント、~数MB〜、あなたの Rust 力が活きる。代替: Wails(Go), Electron(重い) |
| バックエンド言語 | **Rust** | PTY/非同期/MCPを1言語で。`tokio` 前提 |
| PTY | **`portable-pty`**（wezterm製） | クロスプラットフォームで実績最多。代替: `pty-process` |
| 非同期ランタイム | **`tokio`** | PTY読み書き・MCP・監視すべて非同期で統一 |
| フロント | **Svelte(Kit) + TypeScript** | 軽量・リアクティブがシンプル。ペイン多数でも軽い。代替: React（エコシステム最大） |
| 端末描画 | **xterm.js**（+ `@xterm/addon-fit`） | デファクト。ANSI/カラー/リサイズ対応。WebGL addon で高速化可 |
| ペインレイアウト | Svelte 製 split-pane（自作 or 既存） | リサイズ可能グリッド |
| Config | **`serde` + YAML crate** | ⚠️`serde_yaml` は**非推奨(0.9.34+deprecated)**。**`serde_yml` か `serde_norway` を採用** |
| ファイル監視 | **`notify`** | mterm.yml 変更で auto-restart |
| MCP サーバー | **`rmcp`（公式 Rust SDK, v1.8系）** | Queen 相当。stdio/SSE 対応。代替: Python公式SDKで別プロセス |
| Git | **`git2`**（libgit2 bindings） | diff/commit/worktree |
| リソース監視 | **`sysinfo`** | CPU/メモリ/プロセス |
| ローカルLLM統合 | CodeRouter / Ollama / vLLM を **HTTP経由**でエージェントとして接続 | 既存資産(OpenClaw/Hermes)と連携 |

### 3.2 なぜ Svelte を React より推すか（今回に限る）

ペインが増えるほど再描画コストが効くため、リアクティブが軽い Svelte が相性良い。ただし **エコシステム/求人/AIによる生成しやすさは React が上**。「情報量重視」なら React でも全く問題ない。→ ここは好みで差し替え可（設計は同じ）。

### 3.3 代替案（参考）

- **手っ取り早くプロトタイプ**したいだけなら **Go + Wails**：`creack/pty` が枯れていて PTY が最短。ただし MCP は Go SDK or 別プロセス。
- **既存資産最優先**なら、Hermes/OpenClaw に **Web UI(マルチペイン)を後付け**して MCP を足すルートも有力（“ターミナルアプリ”ではなく“エージェント基盤の可視化”になる）。

---

## 4. アーキテクチャ設計

### 4.1 全体構成

```
┌───────────────────────────────────────────────────────────┐
│  Frontend (Svelte + TS, Tauri WebView)                     │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐   グリッド/リサイズ    │
│  │ Pane A  │ │ Pane B  │ │ Pane C  │  各ペイン = xterm.js   │
│  │ Claude  │ │ Codex   │ │ Grok    │                        │
│  └────┬────┘ └────┬────┘ └────┬────┘                        │
│       │  IPC(Tauri commands / events, stdin/stdout stream)  │
└───────┼───────────┼───────────┼────────────────────────────┘
        ▼           ▼           ▼
┌───────────────────────────────────────────────────────────┐
│  Backend (Rust + tokio)                                    │
│  ┌──────────────────────────────────────────────────────┐ │
│  │ Session Manager  … PTY 群のライフサイクル管理          │ │
│  │  ├ PtySession(A)  portable-pty + reader/writer task   │ │
│  │  ├ PtySession(B)                                       │ │
│  │  └ PtySession(C)                                       │ │
│  ├──────────────────────────────────────────────────────┤ │
│  │ Config Loader    … mterm.yml を serde でparse + notify  │ │
│  │ Process Monitor  … sysinfo で CPU/mem, 死活監視/restart│ │
│  │ Git Service      … git2 で diff/commit/worktree        │ │
│  ├──────────────────────────────────────────────────────┤ │
│  │ Queen (MCP Server, rmcp)                               │ │
│  │  tools: spawn_agent / send_message / read_output /     │ │
│  │         list_agents / notify …                         │ │
│  └──────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────┘
        ▲ MCP (stdio/SSE)
        │
  各エージェント CLI（Claude Code / Codex / Grok）= PTY の中で動くプロセス
  → Queen をMCPサーバーとして登録し、エージェント自身が spawn/相互通信を呼ぶ
```

### 4.2 コンポーネント詳細

**(1) Session Manager / PtySession（コアの肝）**
各エージェントは「mterm.yml で定義されたコマンドを PTY 内で起動したプロセス」。1セッション = 1 PTY pair（master/slave）。
- master 側の reader を tokio task で回し、出力を Tauri event でフロントの xterm.js へストリーム。
- フロントの入力（キー入力/リサイズ）は Tauri command 経由で writer / `resize()` に流す。
- 状態: `Starting / Running / Exited(code) / Restarting`。auto-restart 対象なら Exit を検知して再起動。

```rust
// 概念コード（portable-pty）
let pty_system = native_pty_system();
let pair = pty_system.openpty(PtySize { rows, cols, ..Default::default() })?;
let mut cmd = CommandBuilder::new("claude"); // or "codex" / "grok"
cmd.cwd(workdir);
for (k, v) in env { cmd.env(k, v); }
let mut child = pair.slave.spawn_command(cmd)?;
let mut reader = pair.master.try_clone_reader()?;
// tokio::task で reader → frontに emit、writer は master.take_writer()
```

**(2) Config Loader（mterm.yml 相当）**
最小スキーマ例:
```yaml
project: my-app
agents:
  - name: claude
    cmd: "claude"
    cwd: "."
    env: { ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}" }
    autostart: true
    autorestart: on-failure
    instructions: "あなたは実装担当。設計はcodexに従う"
  - name: codex
    cmd: "codex"
    autostart: true
processes:            # 通常プロセス（devサーバー等）も同居可
  - name: web
    cmd: "npm run dev"
```
`notify` で mterm.yml を watch → 差分に応じて該当セッションのみ restart。

**(3) Queen（MCP サーバー）— 差別化の中心**
`rmcp` で MCP サーバーをアプリ内に立て、各エージェント CLI にこの Queen を MCP サーバーとして登録させる。エージェントが「別のエージェントを起動」「他ペインの出力を読む」「通知を出す」を**ツール呼び出し**として実行できる。
最小ツール群:
| tool | 役割 |
|---|---|
| `list_agents` | 現在のエージェント一覧と状態 |
| `read_output(agent, tail_n)` | 指定エージェントの直近出力を取得 |
| `send_message(agent, text)` | 別エージェントの stdin にメッセージ投入 |
| `spawn_agent(cmd, cwd)` | サブエージェントを新ペインで起動 |
| `notify(title, body)` | デスクトップ通知 |

> セキュリティ: `spawn_agent` は任意コマンド実行に等しい。プロジェクト単位でサンドボックス/許可リストを設ける（HiveTerm も「プロジェクト分離」を明記）。

**(4) Process Monitor**：`sysinfo` で各 PID の CPU/メモリを定期取得しフッターに表示。死活監視で autorestart をトリガ。

**(5) Git Service**：`git2` で `status`/`diff`/`commit`。worktree でエージェントごとにブランチ分離する設計（Claude Squad/Crystal と同じ発想）は Phase 3 以降で。

### 4.3 スレッド/非同期モデル
- 1 PTY につき reader task 1本（+ 必要なら writer）。tokio の `mpsc` でイベント集約 → Tauri の `emit` はメインへ。
- MCP サーバーは独立 task。ツール実行時に Session Manager を共有（`Arc<Mutex<..>>` or actor パターン）。

---

## 5. 段階的MVP計画

「まず動くもの」を最短で。各フェーズ末に `cargo tauri dev` で触れる状態を作る。

### Phase 0 — 単一PTYが画面に出る（土台の検証, 数日）
- Tauri v2 プロジェクト初期化、Svelte + xterm.js を1ペイン表示。
- `portable-pty` で `bash`（or `claude`）を1つ起動し、入出力が往復することを確認。
- **達成条件**: 1ペインで Claude Code が普通に操作できる。

### Phase 1 — マルチペイン + mterm.yml（MVPの本体, 1〜2週）
- リサイズ可能なグリッドで N ペイン。
- mterm.yml を読んで定義通りにエージェントを autostart。
- start/stop/restart ボタン、状態表示。
- **達成条件**: mterm.yml に Claude/Codex/Grok を書くと3ペインで同時に立ち上がる。← ここで「使える道具」になる。

### Phase 2 — Queen（MCP相互通信, 1〜2週）
- `rmcp` で MCP サーバーを内蔵し、`list_agents`/`read_output`/`send_message` から実装。
- 各エージェントに Queen を MCP 登録し、エージェントが他ペインを読む/指示する動作を確認。
- `spawn_agent` と許可リスト、`notify`。
- **達成条件**: Claude Code に「codexの出力を読んで要約して」と頼むと Queen 経由で動く。

### Phase 3 — 実戦機能（継続）
- Git diff / inline commit（git2）、worktree 分離。
- Process Monitor（sysinfo）フッター、resume session。
- ファイルツリー/検索、（余力で）音声入力・モバイル連携。

> 記事化するなら Phase 0→1 で「①Tauri+PTYでマルチターミナルを作る」、Phase 2 で「②MCPでエージェントを協調させる」と2本立てにすると読み応えが出る。

---

## 6. リスク・注意点

- **serde_yaml は非推奨**。新規採用しない（`serde_yml`/`serde_norway`）。地味だが最初に踏みやすい罠。
- **PTY のクロスプラットフォーム差**（特に Windows の ConPTY）。まず macOS で固め、Windows は Phase 3 で対応するのが安全。
- **MCP の spawn 系はセキュリティ境界**。任意コマンド実行になり得るので、許可リスト/作業ディレクトリ制限を最初から設計に入れる。
- **各エージェント CLI の差異**（Grok CLI/Codex の対話プロトコル、認証、MCP対応可否）。「PTYで動く任意CLI」として抽象化しておけば吸収できるが、Queen を「使わせる」には各CLIが MCP クライアント対応している必要がある（Claude Code は対応。他は要確認）。
- **AGPL コードを読む際の線引き**：Claude Squad 等はコピペせず、設計理解のみに留めれば自作物のライセンスは自由。

---

## 7. 参考リンク

- HiveTerm（参照仕様）: https://hiveterm.com/ ／ Agents: https://hiveterm.com/agents/ ／ Changelog: https://hiveterm.com/changelog/
- Claude Squad（Go/tmux/worktree, AGPL）: https://github.com/smtg-ai/claude-squad
- awesome-cli-coding-agents（landscape）: https://github.com/bradAGI/awesome-cli-coding-agents
- awesome-agent-orchestrators: https://github.com/andyrewlee/awesome-agent-orchestrators
- 公式 Rust MCP SDK（rmcp）: https://github.com/modelcontextprotocol/rust-sdk ／ crate: https://crates.io/crates/rmcp
- portable-pty（wezterm）: https://docs.rs/portable-pty
- Tauri v2: https://v2.tauri.app/
- serde_yaml 非推奨の議論: https://users.rust-lang.org/t/serde-yaml-deprecation-alternatives/108868

---

### 次の一手（提案）
このドキュメントで方向が良ければ、**Phase 0 の実際のコード（Tauri v2 + Svelte + xterm.js + portable-pty で1ペインPTY）を `multi-terminal/` にscaffold** します。「動く最小構成」を置くところから始めるのが一番早いです。
