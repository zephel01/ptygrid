# 仕様書: Claude Code Agent Teams / subagent のペイン自動追加

作成日: 2026-07-16 / 状態: ドラフト（サブエージェント調査: Opus 検討・Sonnet 定型調査に基づく）
対象実装基準: Phase 3.9 / 想定リリース: Phase 4.x
実装状況: 6.2.3 の socket RPC 基盤と tmux シムを `src-tauri/teams-backend/` に実装済み
（スタンドアロン crate・app 未配線。契約は CONTRACT.md「Phase 4.x 準備契約」を参照）
関連: [design.md](design.md) · [phase3.md](phase3.md) · [competitive-landscape.md](competitive-landscape.md) · [CONTRACT.md](../CONTRACT.md)

> 表記規約: 本書では **[公式]** = docs.claude.com / code.claude.com/docs で確認できた事実、
> **[観測]** = cmux 等コミュニティ実装で確認された非公式挙動（Claude Code のバージョン変更で
> 壊れうる）を明示的に区別する。参照 URL は 12 章に集約する。

---

## 1. 目的と背景

### 1.1 Claude Code Agent Teams とは

Claude Code の実験的機能。lead セッションが複数の teammate（協調して働く子エージェント）を
起動し、task を分担する。有効化は環境変数 `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`。[公式]

プロセスモデルは 2 種類に分かれ、これが本仕様の核心である。[公式]

| モード | teammate の実体 | 独自 PTY/端末 | 前提 |
|---|---|---|---|
| in-process（既定, v2.1.179以降） | lead と同一プロセス内の独立セッション | **持たない**（lead 端末の agent panel に表示） | 追加不要 |
| split-panes | ペインごとに独立した `claude` プロセス | **持つ**（各ペインがフル端末） | tmux または iTerm2 が必須 |

`teammateMode` の設定値は `in-process`（既定）/ `auto` / `tmux` / `iterm2` の 4 値のみで、
プラガブルな "custom multiplexer" 拡張点は公式には存在しない。[公式]

通常の subagent（`.claude/agents`, Agent/Task ツール）は main プロセス内で動き独自 PTY を
持たない。lifecycle は hook（`SubagentStart` / `SubagentStop`）で観測でき、transcript は
`~/.claude/projects/{project}/{sessionId}/subagents/agent-{agentId}.jsonl` に出力される。[公式]

### 1.2 ptygrid で何を実現するか

lead の Claude Code が teammate / subagent を起動したとき、ptygrid に**自動でペインが増える**
体験を成立させる。理想は「独立 claude プロセスの teammate を ptygrid のネイティブ PTY ペインとして
対話可能にホストする」ことだが、in-process teammate と subagent は PTY を持たないため、
その場合は**読み取り専用の transcript ビュー**で代替する。

### 1.3 cmux との関係

cmux は tmux 互換シムで split-pane teammate を自ペインにホストする方式を実証済み（macOS 専用・
Ghostty ネイティブ）。ptygrid は [competitive-landscape.md](competitive-landscape.md) の方針どおり
**端末プリミティブでは cmux と正面競争しない**。ptygrid の強みは Queen 型協調（config-as-code +
許可リスト spawn + inbox/await）であり、本機能もその軸に沿って「公式 hooks で堅牢に観測 →
opt-in で実 PTY ホスト」の順で設計する。

---

## 2. 検討: 3 方式の比較と採用判断

### 2.1 3 方式

- **方式A: tmux シム方式**（cmux 実証済み・[観測]）。偽 `tmux` バイナリを PATH 先頭に置き、
  Claude Code の split-pane teammate（独立 `claude` プロセス = 実 PTY）を ptygrid のネイティブ
  ペインとしてホストする。対話可能な実端末になるのが最大の利点。非公式でバージョン変更に脆い
  （cmux #6447: Claude Code 2.1.183 が teammate ペイン起動法を変えシムが呼ばれなくなった実例）。
- **方式B: hooks 観測方式**（公式 API・[公式]）。`SubagentStart` / `SubagentStop` /
  `TeammateIdle` / `TaskCreated` / `TaskCompleted` を http hook で Queen（localhost）に POST し、
  ptygrid がペイン（transcript tail の読み取り専用ビュー or ステータス表示）を自動生成する。
  堅牢だが in-process teammate / subagent は PTY を持たず**対話ペインにはならない**。
- **方式C: Queen 自前オーケストレーション拡張**（既存機能の延長）。ptygrid が既に持つ
  `spawn_agent` / `inbox` / `await` を活かし、各ペインで独立 `claude` プロセスを走らせて
  「Teams 風」を公式 API のみで実現する。

### 2.2 比較

| 比較軸 | 方式A（tmux シム） | 方式B（hooks 観測） | 方式C（Queen 拡張） |
|---|---|---|---|
| ユーザー価値 | 実 PTY・**対話可能**ペイン | 読み取り専用ビュー / 状態表示 | 実 PTY・対話可能（ただし ptygrid 主導で起動） |
| 公式サポート度 | 非公式・[観測] | **公式・[公式]** | **公式・[公式]**（既存 API のみ） |
| 壊れやすさ | 高（Claude Code のペイン起動法に完全依存, #6447） | 低（hooks は公開契約） | 低（自前制御） |
| 実装コスト | 高（シムバイナリ + socket RPC + フォールバック） | 中（hook 受信 + 新ペイン種別） | 低〜中（既存 tool の presets 化） |
| 許可リスト spawn との整合 | 緊張あり（teammate は allowlist 外・別枠が必要） | 整合（観測のみ・spawn しない） | **完全整合**（allowlist spawn そのもの） |
| project 境界 / 推測拒否との整合 | 要配慮（lead cwd 継承・socket token で限定） | 整合（read-only・宛先は agent_id） | 整合 |
| cmux との差別化 | cmux と同じ土俵（端末プリミティブ） | Queen に lifecycle を取り込む → 協調軸で差別化 | Queen そのもの → **最も差別化** |

### 2.3 採用判断

推奨方向を妥当と判断し、**B 先行 → A を opt-in 実験機能 → C を並走**で採用する。根拠:

1. **B を最初に置く**のは、公式・低リスクで「lead が teammate/subagent を起動したらペインが増える」
   という**体験の骨格**を確実に成立させられるからである。Phase 3 で確立した「未知セッションの
   ペイン自動生成」「9 面上限バナー」「Queen ステータス」の資産をそのまま延長できる。
2. **A は opt-in 実験機能**として後追いする。実 PTY teammate ペインは価値が高い一方、非公式で
   壊れやすい（#6447）。既定オフ・明示 opt-in（worktree 分離と同じ思想）とし、壊れたら B に
   自動フォールバックする**ハイブリッド**にすることで、脆さのコストをユーザーに転嫁しない。
3. **C は A/B と独立**して並走できる。既存 tool のみで完結し Claude Code の内部に一切依存しないため、
   ptygrid 単独の「Teams 風」プリセットとして価値がある。Queen 型協調という ptygrid の勝ち筋を
   最も直接的に強化する。

**ハイブリッド方針（採用）**: host モード（A）を有効化した lead では、A と B の両チャネルを同時に
起動する。シムの `split-window` が実際に呼ばれれば実 PTY ペインでホストし、hook で teammate 起動を
検知したのにシム RPC が来なければ（= #6447 型の破損）、B の読み取り専用ペインへフォールバックし
ユーザーへ通知する。詳細は 6.3。

---

## 3. スコープ / 非スコープ

### 3.1 スコープ

- 方式B: Queen への hook 受信エンドポイント、teammate/subagent lifecycle の toast、
  read-only transcript tail ペイン（新ペイン種別）の自動生成。
- 方式A: ptygrid 同梱の tmux 互換シム、専用 Unix socket 経由の実 PTY teammate ペインホスト、
  フォールバック検知。既定オフの opt-in 実験機能。
- 方式C: mterm.yml の team preset と、既存 `spawn_agent` を使ったグループ起動の ergonomics。
- settings.json への hooks 登録の案内 / 半自動化。
- 9 ペイン上限との整合、teammate ペインの UX、mterm.yml スキーマ拡張。

### 3.2 非スコープ

- cmux と競う汎用フル端末エミュレータ / tmux の完全実装。
- Windows での host モード（A）。split-pane は Windows Terminal で公式非対応。[公式] observe（B）は
  Windows でも成立しうるが本 Phase では macOS / Linux を対象とする。
- iTerm2 ネイティブ split のホスト（`it2` 依存）。ptygrid は tmux 面のみを模倣する。
- Claude Code の team config（`~/.claude/teams/*/config.json`）を直接編集する運用。[公式] 手動編集不可・
  上書きされる、と明記されているため read/watch のみに留める。
- 未文書の内部プロトコル（shutdown_request 等）の再現。観測対象は tmux サブコマンド面と hook 面に限定。

---

## 4. UX 仕様: ペイン自動追加の見え方

### 4.1 ペインの自動追加と 9 ペイン上限

teammate / subagent が起動されると、ptygrid は種別に応じたペインを自動追加する。上限は既存の
グリッド上限（最大 9 ペイン）を厳守する。

- **空きスロットがある場合**: 自動でグリッドに追加配置する（Phase 2 の「未知セッションのペイン
  自動生成」と同じ経路）。
- **9 面が埋まっている場合**: セッション（実 PTY teammate も含む）自体は動き続け、ペインは追加せず
  日本語バナーで通知する（Phase 2 の既存挙動を踏襲）。paneless セッションは `list_agents` と
  teammate パネルに現れ、既存ペインを閉じるとユーザーが手動でグリッドへ昇格できる。
- グローバル上限 `teammates.global_max_panes`（既定 6, ≤9）と lead ごとの `teams.max_panes`
  （既定 3）で、teammate によるグリッド占有を制御する。人間が使うペインを teammate が食い潰さない
  ための既定値である。

### 4.2 teammate ペインのヘッダーと状態ドット

| ペイン種別 | ヘッダー表示例 | 状態ドット | 操作 |
|---|---|---|---|
| host PTY teammate（A） | `claude·team #7 ▸reviewer` | running / exited + code（既存 PTY と同じ） | ⟳restart / ✕close / ⤢maximize |
| observe transcript（B） | `claude·sub #7 ▸reviewer 📖RO` | active / idle / stopped（hook 由来） | ✕close / ⤢maximize（restart 不可） |

- `▸<role>` は hook payload の `agent_type`（subagent 定義名）を表示する。取得できない場合は省略。
- host PTY teammate は既存 PTY ペインとして扱い、状態ドットは Phase 1 の running/exited/restarting を
  再利用する。observe ペインは PTY を持たないため専用の `📖RO`（read-only）バッジと、hook から
  導出した論理状態（active/idle/stopped）を表示する。
- 親 lead との関係を示すため、ヘッダーに lead の `#id` を薄字で併記する（例: `↳#3`）。

### 4.3 閉じたときの挙動

- **host PTY teammate ペインの close**: 実プロセスを kill する破壊的操作のため、作業中は確認
  ダイアログを出す（推測で kill しない原則）。kill 後、シムには tmux `kill-pane` 相当を返し、
  Claude Code 側の team state と齟齬が出ないようにする。
- **observe ペインの close**: transcript の tail を停止するだけで、実際の subagent / teammate は
  影響を受けない（read-only ビューであることを明示）。lead 側で subagent が終了すると `SubagentStop`
  で自動的に `stopped` へ遷移し、ペインは残置（ユーザーが閉じるまで最終状態を表示）。
- **lead 自体が終了**した場合、host teammate PTY は孤立しうる（tmux の孤立 session と同様[公式]）。
  ptygrid は lead セッションの `pty-exit` を検知し、その lead に紐づく host teammate PTY を一括で
  停止候補として通知（既定は自動 kill せず、確認 UI で掃除）。

### 4.4 通知

- teammate 起動 / idle / stop、task 作成 / 完了を toast（`queen-notify` 系）で通知する。
  competitive-landscape が「次に取る UX」に挙げた**通知リング / 要承認ハイライト**の初出実装として、
  teammate が permission を要求している状態（lead に prompt が出ている状態）をヘッダーで
  ハイライトする。
- 通知の粒度は `teammates.hook_notifications`（既定 true）で全体制御する。

### 4.5 設定 UI

- ツールバーに **「Teammates」バッジ**を追加する（Queen バッジと同列）。クリックで
  (1) hooks 登録スニペットのコピー / 半自動登録、(2) 現在の teammate 一覧（paneless 含む）、
  (3) host モードの有効・フォールバック状態、を表示する。
- host モードは mterm.yml の opt-in が前提で、UI からの一時有効化は行わない（config-as-code 原則）。

---

## 5. mterm.yml スキーマ拡張案

グローバル既定を `teammates:` ブロック、agent 単位の挙動を `agents[].teams` に置く。すべて任意で、
ブロックを省略すれば従来と完全に同じ挙動（既定はすべて無効）。

```yaml
project: my-app

# teammate 機能のグローバル既定（任意・ブロックごと省略可）
teammates:
  enabled: false            # 既定 false。全体マスタースイッチ。false なら hook 受信もシムも無効
  hook_notifications: true  # 既定 true。teammate/task lifecycle を toast する
  global_max_panes: 6       # teammate ペインがグリッドを占有できる合計上限（<=9）。既定 6
  hooks_scope: user         # user | project。settings.json 自動登録の書き込み先。既定 user

agents:
  - name: claude
    cmd: "claude"
    cwd: "."
    autostart: true
    teams:
      enabled: true         # 既定 false。この lead で teammate ペイン化を行う
      mode: observe         # observe | host。既定 observe
      max_panes: 3          # この lead が生む teammate ペイン上限。既定 3
      transcript_tail: true # observe 時: read-only transcript ペインを自動生成。既定 true
      # --- host（方式A, 実験機能）でのみ使用 ---
      teammate_binaries:    # split-window で PTY 起動を許可する argv0 の allowlist。既定 [claude]
        - claude
      fallback_to_observe: true  # host が使われなかった場合に observe へ自動降格。既定 true

  - name: codex
    cmd: "codex"
    # teams ブロック省略 = teammate 機能オフ（従来どおり）
```

型（TypeScript 相当・CONTRACT.md へ追記する）:

```ts
type TeammatesConfig = {
  enabled?: boolean;              // default false
  hookNotifications?: boolean;    // default true
  globalMaxPanes?: number;        // default 6, clamp 1..9
  hooksScope?: "user" | "project";// default "user"
};
type AgentTeamsConfig = {
  enabled?: boolean;              // default false
  mode?: "observe" | "host";      // default "observe"
  maxPanes?: number;              // default 3
  transcriptTail?: boolean;       // default true
  teammateBinaries?: string[];    // host only, default ["claude"]
  fallbackToObserve?: boolean;    // host only, default true
};
// Config へ teammates?: TeammatesConfig、AgentDef へ teams?: AgentTeamsConfig を追加
```

---

## 6. アーキテクチャ仕様

### 6.1 方式B: hook 受信と read-only transcript ペイン

#### 6.1.1 受信エンドポイント（Queen Axum サーバー上の HTTP パス）

Claude Code の http 型 hook は URL へ POST する。[公式] MCP tool 型 hook でも実現可能だが、
http 型の方が hook 定義が単純で settings.json だけで完結するため http を採用する。既存の Queen は
`127.0.0.1:<port>/mcp` で rmcp を提供する Axum サーバーであり、**同一 Axum app に hook 用パスを増設**する
（MCP と分離）。

| メソッド | パス | 対応 hook | payload（抜粋） |
|---|---|---|---|
| POST | `/hooks/v1/subagent-start` | SubagentStart | `session_id, agent_id, agent_type, transcript_path, cwd` |
| POST | `/hooks/v1/subagent-stop` | SubagentStop | `session_id, agent_id, agent_type` |
| POST | `/hooks/v1/teammate-idle` | TeammateIdle | `session_id, agent_id, agent_type, team_name(deprecated)` |
| POST | `/hooks/v1/task-created` | TaskCreated | `task_id, task_name, agent_type, team_name` |
| POST | `/hooks/v1/task-completed` | TaskCompleted | `task_id, task_name, status, team_name` |

- 応答は常に `200 {"decision":"allow"}`（ブロックしない）。ptygrid は観測専用であり、exit code 2 相当の
  ブロッキング（idle/作成/完了の阻止）は**行わない**。[公式] のブロック機能はサポート対象外とする。
- payload はスキーマ検証し、未知フィールドは無視。必須欠落は 400 で拒否しログのみ。

#### 6.1.2 セキュリティ（127.0.0.1 bind + トークン）

- bind は既存どおり `127.0.0.1` のみ。
- **hook トークン**: アプリ起動ごとにランダムな 256bit を生成し `QUEEN_HOOK_TOKEN` として保持。
  hook 登録スニペットの `headers` に `Authorization: Bearer <token>` を埋め込み、エンドポイントは
  一致しないリクエストを 401 で拒否する。localhost であっても、同一マシンの他プロセスやブラウザ由来の
  誤爆・悪用を防ぐ。
- `Content-Type: application/json` と `POST` を強制。CORS は許可しない（ブラウザからの誤 POST 遮断）。

#### 6.1.3 settings.json への hooks 登録

Queen MCP 登録（README のバッジ方式）と同じ思想で、**既定はスニペットのコピー**、追加で**半自動
マージ**を提供する。ptygrid が勝手にユーザーの設定を書き換えない原則を守る。

- 「Teammates」バッジ → 「hooks 設定をコピー」で以下を生成（token 埋め込み済み）:

```json
{
  "hooks": {
    "SubagentStart":  [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:39237/hooks/v1/subagent-start",  "headers": { "Authorization": "Bearer <token>" } }] }],
    "SubagentStop":   [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:39237/hooks/v1/subagent-stop",   "headers": { "Authorization": "Bearer <token>" } }] }],
    "TeammateIdle":   [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:39237/hooks/v1/teammate-idle",   "headers": { "Authorization": "Bearer <token>" } }] }],
    "TaskCreated":    [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:39237/hooks/v1/task-created",    "headers": { "Authorization": "Bearer <token>" } }] }],
    "TaskCompleted":  [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:39237/hooks/v1/task-completed",  "headers": { "Authorization": "Bearer <token>" } }] }]
  }
}
```

- **半自動マージ**（`register_teammate_hooks` command）: `teammates.hooks_scope` に従い
  `~/.claude/settings.json`（user）または `<project>/.claude/settings.json`（project）へ、
  既存内容を保ったままマージする。実行前に `settings.json.ptygrid-backup-<ts>` を作成し、
  トークンが変わる（アプリ再起動でトークン再生成）たびに URL/token を更新する。project スコープの
  場合はユーザー repository へ `.claude/settings.json` を作るため、明示確認を必須にする
  （repository へ暗黙 file を作らない原則）。
- **トークン再生成問題**: トークンはアプリ起動ごとに変わるため、user スコープ登録は起動時に
  `register_teammate_hooks` を自動再実行して URL/token を最新化する（差分がある時だけ書き込み）。
  これを避けたいユーザー向けに、`teammates` に固定トークンを設定する将来拡張は 11 章で言及。

#### 6.1.4 subagent transcript tail ペイン（新ペイン種別）

`SubagentStart` 受信で、read-only の transcript tail ペインを新設する。既存 PTY ペインとの違いを
吸収するため、**フロントに新ペイン種別 `transcript` を導入**する。

- **transcript path 解決**: hook payload の `transcript_path` を最優先で使う[公式]。欠落時のみ
  `~/.claude/projects/{projectSlug}/{sessionId}/subagents/agent-{agentId}.jsonl` を構築する（[観測] の
  ディレクトリ規約に依存するため、構築経路はフォールバックであり失敗時はステータス表示のみに縮退）。
- **tail 実装**: backend が `notify` でファイルを watch（or 200ms ポーリング）し、追記された JSONL 行を
  パースして表示用テキストへ整形、`transcript-output` イベントで emit する。既存 PTY の `pty-output`
  とは別イベント。PTY・writer・reader thread・output ring は割り当てない（read-only のため
  `write_pty` / `resize_pty` / `restart_session` は対象外）。
- **セッション表現**: backend session map に「PTY を持たない論理セッション」として u32 id + generation を
  割り当て、`SessionInfo.kind = "transcript"` とする。`list_agents` にも現れるが `read_output` は
  transcript テキストを返し `send_message` は拒否する（宛先にできない）。
- JSONL の描画は「role: text」を時系列で連結する簡易ビューとし、ツール呼び出しは要約表示。全文の
  完全再現は目指さない（read_output のターミナル再構成とは別系統）。

### 6.2 方式A: tmux 互換シム（opt-in 実験機能）

#### 6.2.1 シムバイナリの設計

ptygrid が同梱する小型 Rust バイナリ `ptygrid-tmux-shim`。host モードの lead を spawn する際、
実行時に app-data 配下 `teams/bin/tmux` としてシムを配置（またはハードリンク）し、その dir を
**PATH 先頭に注入**して lead を起動する。Claude Code は `tmux` の存在を検出し split-pane モードで
tmux サブコマンドを CLI サブプロセスとして駆動する。[観測]

シムはステートレスな 1-shot プロセスで、受け取ったサブコマンドを Unix socket RPC に変換して
ptygrid backend に転送し、tmux 互換の stdout / exit code を返すだけに徹する。

#### 6.2.2 対応すべき tmux サブコマンド

| サブコマンド | Claude Code の用途[観測] | シムの動作 → backend RPC |
|---|---|---|
| `new-session` / `has-session` | tmux 環境の存在確認 | 成功を返す（TMUX 環境変数で「内部にいる」と応答） |
| `list-sessions` (`ls`) | session 列挙 | ptygrid が持つ論理 team session を tmux 形式で返す |
| `split-window [-d] [-h/-v] '<cmd>'` | teammate プロセスを新ペインで起動 | `teammate.spawn { cmd, cwd }` → backend が PTY 割当・grid 追加。pane id を返す |
| `send-keys -t <pane> '<text>'` | teammate へ入力送出 | `teammate.write { paneId, data }`（既存 write_pty 相当） |
| `capture-pane -t <pane> -p` | teammate 出力の読み取り | `teammate.capture { paneId }` → read_output のテキスト再構成を返す |
| `select-pane -t <pane>` | フォーカス移動 | `teammate.focus { paneId }`（フロントで該当ペインを強調） |
| `kill-pane -t <pane>` | teammate 終了 | `teammate.kill { paneId }`（PTY kill・autorestart 発火させない） |
| `display-message` / `set-option` / `show-options` | メタ情報照会 | 既知キーは固定値、未知は無害な成功を返す |

対応外のサブコマンドは「成功だが no-op」を返す（tmux の欠損機能で Claude Code が異常終了しない
ことを優先）。ただし `split-window` が観測されないことは破損シグナルとして扱う（6.3）。

#### 6.2.3 シム ⇄ backend の IPC: Queen HTTP か専用 Unix socket か

| 観点 | Queen HTTP（/mcp と同居） | 専用 Unix socket（採用） |
|---|---|---|
| 目的適合 | MCP 用途。tmux 制御を載せると責務が混在 | teammate 制御専用で分離できる |
| 遅延 / 頻度 | send-keys / capture が高頻度 → HTTP は重い | ローカル socket で低遅延 |
| 認可 | localhost の誰でも到達（token 必要） | ファイル権限 + token で lead の子孫に限定できる |
| Windows | 動くが host 自体が非対象 | named pipe 相当（本 Phase 非対象） |
| 前例 | — | cmux も `CMUX_SOCKET_PATH` の socket 方式[観測] |

**専用 Unix socket を選定**する。cmux と同型で、責務分離・低遅延・認可の限定が容易。socket は
app-data 配下 `teams/run/lead-<leadSessionId>.sock`（dir 0700 / socket 0600）に作り、lead へ次を注入:

- `PTYGRID_TEAMS_SOCK=<socket path>`
- `PTYGRID_TEAMS_TOKEN=<per-lead random token>`（RPC ハンドシェイクで検証）
- `TMUX=<sock>,<pid>,0` / `TMUX_PANE=%0`（tmux 内と誤認させる）[観測]
- `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`[公式]
- `teammateMode` は `auto`（tmux を自動判定させる）で起動。[公式]

RPC は **JSON-RPC 2.0 / NDJSON** とし、公式化が提案されている CustomPaneBackend protocol
（anthropics/claude-code#26572 draft, protocol_version `"1"`）に準拠する: `initialize`
handshake の後、`spawn_agent` / `write`(base64) / `capture` / `kill` / `list` / `get_self_id` を
提供し、push event `context_exited` を配る。ptygrid 拡張は `ptygrid/focus`（tmux
`select-pane` 対応）と `initialize.auth_token`（`PTYGRID_TEAMS_TOKEN` の検証）の 2 点のみで、
いずれも名前空間・追加フィールドとして分離する。**提案が上流採用された場合は、同一 socket を
`CLAUDE_PANE_BACKEND_SOCKET` として広告するだけでシム無しの公式経路へ移行できる。**
詳細な method 表・error code・shim 対応表は CONTRACT.md「Phase 4.x 準備契約」に記載する。
シムは応答の `stdout`/`exit` をそのまま tmux 出力として中継する。

#### 6.2.4 実 PTY teammate ペインの生成

`split-window` の `<cmd>`（Claude Code が渡す teammate 起動コマンド）を backend が**既存の PTY 起動
基盤で spawn** する。teammate プロセス（独立 `claude`）は ptygrid の PTY child として動くため、
teammate ペインは**ネイティブな対話 PTY ペイン**になる。send-keys → write、capture-pane → read_output、
kill-pane → kill を既存経路にマップする。これにより teammate は resize / スクロールバック /
Queen 接続まで通常ペインと同等に扱える。

#### 6.2.5 9 ペイン上限超過時の挙動（host）

`split-window` 要求が上限（`teams.max_panes` / `teammates.global_max_panes` / グリッド 9）を超える
場合でも、backend は teammate PTY セッションを**生成する**（作業を止めない）。ただしグリッドには
配置せず paneless とし、日本語バナーで通知する（4.1 と同一ポリシー）。シムには pane id を返すため
Claude Code の team state は成立する。ユーザーは空きが出たら paneless teammate をグリッドへ昇格できる。

### 6.3 フォールバック検知（#6447 型破損の検知と通知）

host モードでは A と B の両チャネルを同時起動する。teammate 起動を hook（`SubagentStart` /
`TeammateIdle`）で検知したにもかかわらず、対応する `split-window` RPC がシムから来ない場合、
Claude Code が in-process へ静かにフォールバックした（= シムのコードパスが通っていない・#6447 型）と
判断する。

- **相関**: hook の `agent_id` と、直近の `split-window` RPC を時間窓で突き合わせる。ある `agent_id` の
  `SubagentStart`/`TeammateIdle` 受信後 **2 秒**以内にシム RPC が無ければ「シム未使用」と確定。
- **フォールバック動作**（`fallback_to_observe: true` の場合）: その agent を observe（read-only
  transcript ペイン）として自動生成し、toast で通知する:
  「teammate をネイティブペインにホストできませんでした（Claude Code のバージョン変更の可能性）。
  読み取り専用ビューにフォールバックしました。」
- 起動直後に一度も `split-window` を観測しないままセッションが進む場合も同様に host 無効と判断し、
  Teammates バッジに「host: フォールバック中」を表示する。

---

## 7. 既存設計原則との整合

### 7.1 許可リスト spawn との関係

`spawn_agent` は mterm.yml 定義名だけを許可する厳格な allowlist である。teammate ペイン（host）は
Claude Code が渡す任意の `claude` 起動コマンドを spawn するため allowlist の外にある。これを
**別枠の「teammate spawn チャネル」**として明確に分離し、次の 3 段で制約する:

1. **config opt-in**: `agents[].teams.mode: host` を明示した lead でのみ、そのシム socket が有効化される
   （worktree 分離と同じ「明示 opt-in」思想）。
2. **socket token**: `PTYGRID_TEAMS_TOKEN` を持つ lead の子孫プロセスのみが RPC を発行できる。
3. **binary allowlist**: `split-window` の argv0 を `teams.teammate_binaries`（既定 `[claude]`）で
   検証し、一致しないコマンドの PTY 起動を拒否する。任意コマンド実行の踏み台化を防ぐ。

`spawn_agent`（Queen MCP）自体の allowlist セマンティクスは変更しない。teammate spawn は spawn_agent を
経由せず、専用チャネルを通ることを CONTRACT.md に明記する。

### 7.2 session ID / generation

host teammate PTY / observe transcript の双方に、backend 採番の `u32` id + generation を割り当てる。
generation で古い reader / watcher からの emit を無効化する既存規律を踏襲する。`SessionInfo` に
種別と teammate メタを追加する（7.5）。

### 7.3 resume での扱い

teammate / subagent セッションは ephemeral であり **logical resume の対象外**とする。根拠:
in-process teammate は Claude Code 自身が `/resume` で復元しない[公式]、tmux 孤立 session は掃除対象[公式]。
`ProjectState.sessions` には teammate / transcript セッションを**保存しない**（Phase 3.4 の
「保存対象は定義名・worktree 参照だけ」を継承）。resume 後は lead の再起動により teammate が
改めて生成される。

### 7.4 project 境界

- host teammate PTY は lead の解決済み cwd（worktree opt-in 時はその worktree）で動く。
- observe の transcript は `~/.claude/projects/...` 由来で、ptygrid の project scope（読み込まれた
  mterm.yml の canonical dir）とは別系統である。Queen の pins/notes/inbox の project scope は不変。
- hook のうち project を跨ぐ情報（team_name 等）は表示メタとしてのみ扱い、Queen storage の scope 判定には
  用いない。

### 7.5 CONTRACT.md に追記が必要な項目（列挙）

- mterm.yml: `teammates` ブロック、`agents[].teams` ブロックのスキーマと既定値。
- 型追加: `TeammatesConfig`, `AgentTeamsConfig`、`SessionInfo.kind: "pty" | "transcript"`、
  `SessionInfo.teammate?: { role?: string; leadId: number; mode: "host" | "observe"; transcriptPath?: string }`。
- Queen HTTP hook エンドポイント `/hooks/v1/*`（メソッド・パス・payload・token・応答）。MCP tool ではない
  ことを明記。
- 新 Tauri command: `register_teammate_hooks { scope }`, `teammate_status`,
  `open_transcript_pane { ... }`, `close_transcript_pane { id }`, `promote_teammate_pane { id }`。
- 新 Tauri event: `teammate-lifecycle`（hook 由来の状態遷移）、`transcript-output { id, text }`、
  `teammate-fallback { agentId, reason }`。
- シム ⇄ backend の Unix socket RPC プロトコル（内部契約: op 一覧・token・エラー形）。
- ProjectState から teammate / transcript セッションを除外する旨。
- 環境変数注入: `QUEEN_HOOK_TOKEN`, `PTYGRID_TEAMS_SOCK`, `PTYGRID_TEAMS_TOKEN`, `TMUX`, `TMUX_PANE`,
  `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`。

---

## 8. セキュリティ考慮

- **bind**: Queen / hook エンドポイントは `127.0.0.1` のみ。既存原則を維持。
- **hook token**: `Authorization: Bearer` を必須化し、未一致は 401。CORS 不許可・POST/JSON 強制で
  ブラウザ経由の誤爆を遮断。トークンは起動ごとに再生成し settings.json を更新。
- **socket 認可**: socket dir 0700 / socket 0600、`PTYGRID_TEAMS_TOKEN` ハンドシェイクで lead 子孫に
  限定。socket path は app-data 配下（world-readable な /tmp を避ける、あるいは `$XDG_RUNTIME_DIR`）。
- **任意コマンド実行の抑止**: teammate spawn は binary allowlist（既定 `[claude]`）で argv0 を検証。
  シム経由の任意プロセス起動を拒否。
- **repository 汚染防止**: project スコープの hooks 登録（`.claude/settings.json` 作成）は明示確認を
  必須とし、既定は user スコープ。runtime 生成物（socket・shim バイナリ・backup）は app-data 配下のみ。
- **秘密の非永続化**: token・socket path・展開後 env は ProjectState に保存しない（Phase 3.4 継承）。
- **破壊的操作の非推測**: host teammate ペインの close/kill と lead 終了時の孤立掃除は確認を挟む。
- **hook のブロッキング不使用**: exit code 2 相当の lifecycle 阻止は行わず、常に allow を返す。
  ptygrid が Claude Code の実行制御に介入しない（観測に徹する）。

---

## 9. 段階リリース計画（Phase 4.x）

phase3.md の流儀（各リリースは独立 release、既存契約を壊さない、CONTRACT 先行、両チェック通過）に従う。

| Release | スコープ | Completion gate |
|---|---|---|
| 4.0 | hook 受信基盤: Queen Axum に `/hooks/v1/*` 増設、token 認可、teammate/task lifecycle の toast、Teammates バッジ（スニペットコピー + user スコープ半自動登録） | hooks が token 検証付きで受信され toast 化。MCP `/mcp` 契約に非回帰。新ペイン種別は未導入 |
| 4.1 | observe: 新ペイン種別 `transcript`、`SubagentStart` での read-only tail ペイン自動生成、9 面上限バナー、close で subagent 非影響 | transcript が read-only で描画・close 安全・9 面上限順守。`send_message` 宛先にならない。既存 PTY 経路に非回帰 |
| 4.2 | host（実験・既定オフ）: `ptygrid-tmux-shim` 同梱、Unix socket RPC、env/PATH 注入、実 PTY teammate ペイン、フォールバック検知 → observe 降格 | 互換 Claude Code で teammate がネイティブ対話ペイン化。シム未使用時は observe へ降格 + 通知。opt-in 無しでは一切起動しない。binary allowlist 強制 |
| 4.3 | Queen team preset（方式C・A/B と独立）: mterm.yml の team preset と、既存 `spawn_agent` を使ったグループ起動の ergonomics | 公式 API のみ・allowlist spawn のみを使用。Claude Code teams 内部に非依存。既存 Queen tool 契約に非回帰 |

各リリース共通: (1) CONTRACT.md へ差分契約を先に追記、(2) 新ロジックを `lib.rs` と session hot path の
外に置く、(3) parse/state 遷移の unit test と外部プロセス挙動の integration test、(4)
`cargo test` / `cargo check` / `npm run check` / `npm run build` 通過、(5) 該当挙動のみ userguide 更新。

---

## 10. テスト計画

### 10.1 cargo test（ユニット / 結合）

- **config parse**: `teammates` / `agents[].teams` の既定値補完、`global_max_panes` の 1..9 clamp、
  未知フィールド無視、`mode` の enum 検証。
- **hook payload**: 各 `/hooks/v1/*` の JSON デシリアライズ、必須欠落で 400、未知 status の受理、
  token 不一致で 401、Content-Type / method 検証。
- **transcript 解決 / tail**: payload の `transcript_path` 優先、欠落時の path 構築フォールバック、
  JSONL 追記 → 整形テキストの差分 emit、generation による stale watcher 抑止。
- **teammate session 種別**: `SessionInfo.kind`、`read_output` が transcript テキストを返し
  `send_message` を拒否、ProjectState から teammate/transcript を除外。
- **9 面上限**: 上限超過で paneless セッション生成 + バナー、空き発生時の promote。
- **binary allowlist**: `split-window` の argv0 検証、非許可コマンド拒否。
- **フォールバック相関**: hook 受信後 2 秒窓で split-window 不在 → observe 降格イベント emit。

### 10.2 シムのスモークテスト

- `pty-core-check/` / `mcp-server-check/` と同方式のスタンドアロン crate `teams-shim-check/` を新設。
- socket サーバーの stub を立て、`ptygrid-tmux-shim` に `split-window` / `send-keys` /
  `capture-pane` / `select-pane` / `kill-pane` / `ls` を実行させ、RPC 変換・stdout・exit code を検証。
- token 不一致・未知サブコマンド no-op・socket 不在時の graceful degrade を確認。

### 10.3 Claude Code 実機での手動検証手順

1. `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` と host モード opt-in で lead を起動。
2. lead に「サブエージェントを 2 体立てて分担して」と依頼し、ネイティブ teammate ペインが 2 枚
   自動追加されることを確認（send-keys で入力が届く / capture で出力が読める）。
3. observe モードで通常 subagent を起動し、read-only transcript ペインが tail されることを確認。
4. Claude Code を意図的に in-process にして（`teammateMode: in-process`）、host が observe へ
   フォールバックし toast が出ることを確認（#6447 相当の擬似再現）。
5. 9 面を埋めた状態で追加 teammate が paneless + バナーになること、空きを作って promote できることを確認。
6. lead を終了し、孤立 teammate PTY の掃除確認 UI が出ることを確認。
7. hook token を無効化したリクエストが 401 で弾かれることを `curl` で確認。
8. macOS と Linux（Ubuntu 22.04）双方で 2〜3 を実施。

---

## 11. リスクと未解決事項

- **Claude Code バージョン依存（host）**: tmux サブコマンド面は [観測] であり、cmux #6447 のように
  ペイン起動法が変わればシムが呼ばれなくなる。フォールバック検知で被害は限定するが、host は
  常に「壊れうる実験機能」と位置づける。
- **CustomPaneBackend 提案のウォッチ（機会）**: anthropics/claude-code#26572（2026-02 起票・
  open・公式返答なし）が採用されると、tmux シム無しで `CLAUDE_PANE_BACKEND_SOCKET` に
  socket を広告するだけの公式経路になる。6.2.3 の RPC はこの draft に準拠して実装済みのため、
  採用時の移行コストはシム撤去とフラグ切替に限られる。issue の動向を定期確認する。
- **実験フラグ廃止の可能性**: `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` が将来仕様変更・廃止される
  可能性。observe（hooks）も含め、フラグ / hook 名の変更に追随できる設定点を残す。
- **hook token の再生成**: 起動ごとにトークンが変わり settings.json を毎回更新する運用は摩擦がある。
  固定トークン（`teammates.hook_token` の明示指定 or app-data 永続化）は将来検討の**未解決事項**。
- **transcript path の非公式規約**: 構築フォールバック経路（`~/.claude/projects/.../subagents/...`）は
  [観測] のディレクトリ規約に依存する。payload の `transcript_path` があれば不要だが、無い hook 種別で
  縮退する可能性を残す。
- **iTerm2 モード非対応**: ptygrid は tmux 面のみを模倣する。`teammateMode: iterm2` を使うユーザーの
  host は本 Phase では成立しない（observe は成立）。
- **Linux での挙動**: split-pane の安定性は macOS 基準[公式]。Linux での実機検証は Phase 3.9 と同じく
  継続課題。socket 権限・`$XDG_RUNTIME_DIR` の扱いは環境差がある。
- **teammate と Queen の二重接続**: host teammate は独立 `claude` プロセスのため Queen MCP へ別途
  接続しうる。teammate に `QUEEN_URL` を渡すか（協調に組み込む）、渡さないか（lead 経由に限定するか）は
  **未解決事項**。既定は「渡さない」を暫定採用（teammate の宛先曖昧化を避ける）。
- **paneless teammate の上限**: 上限超過で作業は続くが paneless セッションが増え続ける懸念。
  paneless の総数上限とハードストップ挙動は未決。
- **方式C の具体像**: team preset の YAML 形と、グループ起動 tool（例 `spawn_team`）の要否は 4.3 で
  詰める。既存 `spawn_agent` の逐次呼び出しで代替可能なため、ergonomics 追加の投資対効果を要検討。

---

## 12. 参考資料（URL 一覧）

公式（[公式]）:
- Agent Teams: https://code.claude.com/docs/en/agent-teams
- Sub-agents: https://code.claude.com/docs/en/sub-agents
- Hooks: https://code.claude.com/docs/en/hooks
- Headless / プログラム制御: https://code.claude.com/docs/en/headless

コミュニティ観測（[観測]・非公式）:
- cmux Claude Teams（tmux シム方式の解説）: https://cmux.com/blog/cmux-claude-teams
- cmux リポジトリ: https://github.com/manaflow-ai/cmux
- Issue #6447（Claude Code 2.1.183 でシムが呼ばれなくなった破損例）: https://github.com/manaflow-ai/cmux/issues/6447
- Issue #123（公式 multiplexer 登録の upstream 提案）: https://github.com/manaflow-ai/cmux/issues/123
- CustomPaneBackend protocol 提案（本仕様 6.2.3 が準拠する draft）: https://github.com/anthropics/claude-code/issues/26572
- Issue #2618（サブエージェントペイン自動生成の挙動）: https://github.com/manaflow-ai/cmux/issues/2618
- it2（iTerm2 CLI, iterm2 モード用）: https://github.com/mkusaka/it2

ptygrid 内部:
- [docs/design.md](design.md) / [docs/phase3.md](phase3.md) /
  [docs/competitive-landscape.md](competitive-landscape.md) / [CONTRACT.md](../CONTRACT.md)
- 調査レポート: [research/claude-code-teams-research.md](research/claude-code-teams-research.md),
  [research/cmux-research.md](research/cmux-research.md)
