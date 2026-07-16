# 調査レポート1: Claude Code Agent Teams / subagents / hooks（2026年7月, claude-code-guideエージェント調査）

以下、公式ドキュメント (docs.claude.com / code.claude.com/docs) で確認できた事実とコミュニティ観測を区別して記載。バージョンは v2.1.178〜v2.1.208 相当。

## 1. Claude Code「Agent teams（teammates）」機能

### 1-1. 有効化・設定（公式）
- 実験的機能でデフォルト無効。環境変数 `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` を設定して有効化（シェル環境変数 or settings.json の env ブロック）。無いと team ディレクトリも作られず teammate も spawn されない。
- v2.1.178 以降、teammate の spawn に事前セットアップ不要。旧 `TeamCreate`/`TeamDelete` ツールは廃止。Agent ツールの `team_name` 入力は受理されるが無視。hook payload の `team_name` は session 由来名で deprecated。
- 出典: https://code.claude.com/docs/en/agent-teams

### 1-2. プロセスモデル・PTY・表示モード（公式）— 設計の核心

| モード | 各 teammate は | 独自 PTY/端末 | 要件 |
|---|---|---|---|
| in-process（デフォルト, v2.1.179以降） | lead の同一プロセス内で動く独立セッション | 持たない（lead ターミナル内 agent panel に表示、↑↓/Enter で切替） | 追加不要・任意の端末で動作 |
| split-panes | 独立した `claude` プロセスが各ペインで走る（独自 PTY を持つ） | 持つ（各ペインがフル端末ビュー） | tmux または iTerm2 が必須 |

- in-process が同一プロセスの根拠（公式）: 「in-process teammate の background subagent は不可。teammate の background 作業は lead のプロセスより長生きできないため」。
- split-pane は別プロセスの根拠: team config が「session IDs and tmux pane IDs」を保持。孤立 tmux session が残ることがある（`tmux ls` / `tmux kill-session` で掃除）と公式記載。

teammateMode 設定値（公式）:
- `"in-process"`（デフォルト）
- `"auto"`: すでに tmux session 内 or 端末が iTerm2 かつ `it2` CLI 導入済みなら split-pane、そうでなければ in-process にフォールバック
- `"tmux"`: split-pane 有効化。tmux か iTerm2 かを端末から自動判定
- `"iterm2"`（v2.1.186以降）: iTerm2 ネイティブ split。`it2` CLI（https://github.com/mkusaka/it2）必須

設定箇所: `~/.claude/settings.json` の `"teammateMode": "auto"`、またはセッション単位フラグ `claude --teammate-mode auto`。

tmux split-pane モードのサポート端末（公式）: tmux（macOS が最も安定）、iTerm2（`tmux -CC` 推奨）。非対応と明記: VS Code 統合ターミナル、Windows Terminal、Ghostty。

### 1-3. サードパーティ端末が teammate を自ペインでホストする方法
- 公式にプラガブルな "custom multiplexer" 拡張点は存在しない。選択肢は in-process/auto/tmux/iterm2 の4値のみ。`CLAUDE_CODE_TEAMMATE` のような teammate 専用環境変数は公式ドキュメントに記載なし。
- コミュニティ実証済み手法 = cmux の「tmux シム」方式:
  1. `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` を設定
  2. 偽の `tmux` バイナリ（シム）を PATH の先頭に置く。Claude Code は tmux があると誤認
  3. シムが Claude Code の発行する tmux サブコマンドを自前 API に変換: `split-window` → ペイン分割、`send-keys` → テキスト送信、`capture-pane` → ペイン内容読み取り、`select-pane` → フォーカス
  4. teammate が cmux のネイティブペインとして表示される
  - つまり「Claude Code は split-pane モードで tmux を CLI サブプロセスとして駆動している」→ ptygrid も同じシム方式で teammate を自前ペインにホスト可能。
  - 出典: https://cmux.com/blog/cmux-claude-teams
- GitHub Issue（manaflow-ai/cmux #123）: 公式 multiplexer 登録の upstream 提案、`CMUX_WORKSPACE_ID` 等の自前環境変数で「自分の中で動いているか」判定する設計案。`CLAUDE_CODE_TEAMMATE` 環境変数・teammate session への公式 attach 機構は「言及なし」。
- 結論: (A) tmux シムで split-pane 出力を横取り（実証済み・最有力だが非公式で脆い）、(B) headless で teammate 相当を自前 spawn し Queen MCP + hooks で観測（公式 API のみで完結）。

### 1-4. team 状態のディスク上の場所（公式）
session 由来名 `session-<sessionIdの先頭8文字>` の下:
- team config: `~/.claude/teams/{team-name}/config.json` — members 配列（name / agent ID / agent type）、session IDs、tmux pane IDs 等。セッション終了時に削除。手動編集不可（上書きされる）。
- mailbox: `~/.claude/teams/{team-name}/inboxes/{agent-name}.json`
- task list: `~/.claude/tasks/{team-name}/` — ローカル永続化。resume で保持。`cleanupPeriodDays` に従う。ファイルロックで claim 競合防止。
- プロジェクトレベルの team config は存在しない。
- 外部ツールはこれらのファイル watch + hooks（TeammateIdle/TaskCreated/TaskCompleted）で lifecycle を観測可能。

### 1-5. teammate 関連の CLI・API（公式）
- CLI: `claude --teammate-mode <in-process|auto|tmux|iterm2>`。`--teammate` や `--team-name` という専用フラグは存在しない。
- teammate の spawn は自然言語で lead に依頼（専用サブコマンド無し）。内部的には Agent ツール（旧 Task ツール）経由。
- 内部ツール `SendMessage`（エージェント間メッセージ）。team プロトコル専用メッセージ（shutdown_request, plan_approval_response）は teams 有効時のみ。
- lead ターミナル UI: agent panel で ↑↓ 選択 / Enter で transcript / Esc で中断 / x で停止 / Ctrl+T で task list。
- per-teammate モデル指定（公式）: spawn プロンプトで指定（"Use Sonnet for each teammate"）。デフォルトは /config の "Default teammate model"。teammate のモデルは spawn 時に固定。
- subagent 定義（.claude/agents）を teammate ロールとして再利用可: `tools` allowlist と `model` を尊重。ただし frontmatter の `skills` と `mcpServers` は teammate 実行時には適用されない。

### 1-6. teammate の既知の制約（公式）
- in-process teammate は /resume・/rewind で復元されない
- 1 セッション = 1 team、ネスト不可（teammate は teammate を spawn できない）
- lead 固定、リーダー移譲不可
- 権限は spawn 時に lead から継承。teammate の permission prompt は lead セッションに出る
- split-pane は VS Code 端末 / Windows Terminal / Ghostty 非対応

## 2. 通常の subagent（Agent/Task ツール, .claude/agents）
- subagent は main Claude Code プロセス内で動き、独自の PTY/端末を持たない。独自 context window は持つ。
- frontmatter フィールド: name, description, tools, disallowedTools, model, permissionMode, maxTurns, skills, mcpServers, hooks, memory, background, effort, isolation(worktree), color, initialPrompt。
- v2.1.198 以降 subagent はデフォルト background。`CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1` で全 background 無効。
- hooks で観測可能（公式）: `SubagentStart`（matcher=agent type 名）と `SubagentStop`。payload に `agent_id` / `agent_type`。Task ツール名への PreToolUse matcher は無い。
- subagent transcript: `~/.claude/projects/{project}/{sessionId}/subagents/agent-{agentId}.jsonl` — ペインに映せるのはこの tail（非対話ビュー）。
- 出典: https://code.claude.com/docs/en/sub-agents

## 3. Hooks リファレンス（2026年）
イベント名（公式）:
- セッション: SessionStart, Setup, SessionEnd
- ターン: UserPromptSubmit, UserPromptExpansion, Stop, StopFailure
- ツール: PreToolUse, PostToolUse, PostToolUseFailure, PostToolBatch, PermissionRequest, PermissionDenied
- エージェント/チーム: SubagentStart, SubagentStop, TeammateIdle, TaskCreated, TaskCompleted
- ファイル/設定: FileChanged, CwdChanged, ConfigChange, InstructionsLoaded
- MCP: Elicitation, ElicitationResult
- 圧縮/表示: PreCompact, PostCompact, MessageDisplay
- worktree: WorktreeCreate, WorktreeRemove
- 通知: Notification

共通 JSON 入力（stdin または HTTP hook の POST body）: session_id, prompt_id, transcript_path, cwd, permission_mode, hook_event_name, effort。ツール系は tool_name/tool_input(/tool_result)。subagent 動作中は agent_id/agent_type 付与。

チーム系 payload:
- TeammateIdle: { session_id, hook_event_name, team_name(deprecated), agent_type, agent_id }
- TaskCreated: { hook_event_name, task_id, task_name, team_name, agent_type }
- TaskCompleted: { hook_event_name, task_id, task_name, status, team_name }
- exit code 2 で TeammateIdle=idle 阻止、TaskCreated=作成阻止、TaskCompleted=完了阻止。

hook 型: command（任意シェル、curl で localhost に POST 可）/ http（直接 POST: { "type":"http", "url":"http://localhost:8080/...", "headers": {...} }）/ mcp tool / prompt(LLM) / agent(subagent)。
exit code: 0=成功、2=ブロッキング、他=非ブロッキングエラー。
matcher: tool_name の exact/正規表現。SubagentStart/SubagentStop の matcher は agent type 名。
出典: https://code.claude.com/docs/en/hooks

## 4. Headless / プログラム制御（公式）
- `claude -p` / `--print`: 非対話実行。Agent SDK（Python claude-agent-sdk / TS @anthropic-ai/claude-agent-sdk）の CLI 面。
- `--output-format`: text / json / stream-json（NDJSON。system/init イベントがセッションメタを報告、最終行が result）。
- `--input-format stream-json` + `--resume <session_id>` / `--continue` で会話継続。
- `--bare`: hooks/skills/plugins/MCP/CLAUDE.md の自動探索スキップ。
- `--model`、`--agents '<json>'`（セッション限定 subagent 定義、model 指定可）、`--append-system-prompt(-file)`、`--append-subagent-system-prompt`、`--mcp-config`、`--settings`、`--add-dir`、`--allowedTools`/`--disallowedTools`、`--permission-mode`。
- subagent モデル解決順 = `CLAUDE_CODE_SUBAGENT_MODEL` 環境変数 → per-invocation model → frontmatter → main 会話。
- agent teams を headless で制御する公式手段は無し（teammate spawn は対話 lead への自然言語依頼が前提）。
- 出典: https://code.claude.com/docs/en/headless

## 5. まとめ: 実現の3方式
- (A) tmux シム方式（cmux 実証）: teammateMode tmux/auto + PATH 先頭に偽 tmux。split-pane teammate は独立 claude プロセス＝独自 PTY → 実 PTY ペインとして対話可能に映せる。非公式・脆い。
- (B) hooks 観測方式: SubagentStart/TeammateIdle/TaskCreated を localhost(Queen) に POST → ペイン生成。in-process teammate / subagent は独自 PTY を持たないため、映せるのは transcript tail（非対話）。公式・堅牢だが受動観測。
- (C) 自前オーケストレーション: ptygrid が各ペインで `claude -p --output-format stream-json`（または対話 claude）を spawn し、Queen MCP でペイン間協調。公式 API のみで "Teams風" を自作。最も制御可能。

注記: tmux シム、CMUX_* 検出、Claude Code が発行する具体的 tmux サブコマンド（split-window/send-keys/capture-pane）はコミュニティ観測であり、Claude Code のバージョン変更で壊れうる。公式保証は teammateMode 4値・hooks・headless フラグ・team/task ファイル配置。
