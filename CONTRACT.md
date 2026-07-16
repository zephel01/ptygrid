# IPC Contract (backend ⇄ frontend)

> Phase 0 の契約はそのまま有効。Phase 1 の追加分は末尾のセクション参照。

## Phase 0 (基本PTY)

両エージェントはこの契約に**厳密に**従うこと。変更禁止。

## Tauri Commands (frontend → backend, `invoke`)

| command | args | returns | 説明 |
|---|---|---|---|
| `spawn_shell` | `{ cols: number, rows: number, cmd?: string }` | `number` (session id) | PTYでシェルを起動。cmd省略時は `$SHELL`（fallback: `/bin/bash`、Windowsは`powershell.exe`）。作業dirはユーザーHOME |
| `write_pty` | `{ id: number, data: string }` | `void` | キー入力をPTY stdinへ |
| `resize_pty` | `{ id: number, cols: number, rows: number }` | `void` | PTYリサイズ |
| `kill_pty` | `{ id: number }` | `void` | セッション終了 |

- 引数名は camelCase で invoke する（Tauri v2 デフォルト変換。Rust側は snake_case で受ける）。
- エラーは `Err(String)` で返す。

## Tauri Events (backend → frontend, `emit`)

| event | payload | 説明 |
|---|---|---|
| `pty-output` | `{ id: number, data: string }` | PTY出力（UTF-8 lossy変換した文字列。チャンク単位） |
| `pty-exit` | `{ id: number, code: number \| null }` | プロセス終了 |

## 技術固定事項

- Backend: Rust, tauri = "2", portable-pty = "0.9", tokio (features full)。PTY readerは専用スレッド（`std::thread`）で回し、`AppHandle::emit` で送る（portable-ptyのreaderはblocking I/Oのため）。
- Frontend: Svelte 5 + TypeScript + Vite, `@tauri-apps/api@^2`, `@xterm/xterm@^5`, `@xterm/addon-fit@^0.10`。
- 識別子: session id は backend が採番する連番 u32。
- Phase 0 は 1ペインのみ。ただしbackendは複数セッションを保持できる設計（HashMap<u32, PtySession>）にしておく（Phase 1 準備）。

---

# Phase 1 追加契約 (グリッド + mterm.yml)

両エージェントはこの契約に**厳密に**従うこと。Phase 0 のコマンド/イベントは互換維持（`spawn_shell` は `cwd?: string` 引数を追加拡張してよい）。

## mterm.yml スキーマ

プロジェクトディレクトリ直下の `mterm.yml`:

```yaml
project: my-app            # 任意
agents:
  - name: claude           # 必須・一意
    cmd: "claude"          # 必須（shell経由で実行: sh -c 相当）
    cwd: "."               # 任意。相対はmterm.ymlのあるdir基準。省略時はそのdir
    env:                   # 任意。値の ${VAR} はホスト環境変数で展開
      ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}"
    autostart: true        # 任意。default false
    autorestart: never     # 任意。never | on-failure | always。default never
    instructions: "..."    # 任意。Phase 1 では保持のみ（Phase 2 でQueenが使用）
processes:                 # 任意。エージェントでない常駐プロセス。フィールドはagentsと同じ（instructionsなし）
  - name: web
    cmd: "npm run dev"
    autostart: false
```

- Rust側の型: `Config { project: Option<String>, agents: Vec<AgentDef>, processes: Vec<AgentDef> }`（processes省略時は空Vec）。
- YAML crate は **serde_norway**（serde_yaml後継fork）を使用。
- parseエラーは `Err(String)`（行番号含むメッセージそのまま）で返す。

## 追加 Tauri Commands

| command | args | returns | 説明 |
|---|---|---|---|
| `load_config` | `{ dir?: string }` | `ConfigInfo` | `dir`（省略時: 前回のdir、初回はカレント）の `mterm.yml` を読む。成功時にnotifyでそのファイルのwatch開始（既存watchは置換）。ファイルが無い場合は `Err("not_found: <path>")` |
| `spawn_agent` | `{ name: string, cols: number, rows: number }` | `number` (session id) | ロード済みconfigの該当agent/processを起動。cwd/env/autorestartを適用。TERM=xterm-256color |
| `restart_session` | `{ id: number }` | `void` | 該当セッションをkill→**同じidで**再spawn（定義から起動したものはその定義で、adhoc shellは同じcmd/cwdで） |
| `list_sessions` | なし | `SessionInfo[]` | 現在の全セッション |

```ts
type ConfigInfo = { path: string; config: Config };
type Config = { project?: string; agents: AgentDef[]; processes: AgentDef[] };
type AgentDef = { name: string; cmd: string; cwd?: string; env?: Record<string,string>;
                  autostart?: boolean; autorestart?: "never"|"on-failure"|"always"; instructions?: string };
type SessionInfo = { id: number; name?: string; cmd: string; state: SessionState; code?: number|null };
type SessionState = "starting"|"running"|"exited"|"restarting";
```

## 追加 Tauri Events

| event | payload | 説明 |
|---|---|---|
| `session-state` | `SessionInfo` | 状態遷移のたびにemit（spawn直後=running、exit時=exited+code、autorestart時=restarting→running） |
| `config-changed` | `{ path: string }` | watch中の mterm.yml が変更された（backendは自動では何もしない。frontendがリロードUIを出す） |

- Phase 0 の `pty-output` / `pty-exit` は従来通りemitする（exitedのsession-stateと重複してよい）。

## セマンティクス（重要）

- **同一ID再起動**: restart/autorestart ではセッションidを維持する（フロントのペイン⇄idマッピングを崩さない）。PtySessionの中身（master/writer/child/reader thread）だけ差し替える。古いreader threadは自然にEOF終了させ、世代カウンタで古いスレッドからのemitを抑止する。
- **autorestart**: `on-failure`=exit code≠0のとき、`always`=常に、1秒後に再spawn。手動 `kill_pty` されたセッションはautorestartしない。連続再起動は5回で打ち切り（exited扱い）。
- **autostart はbackendが勝手にspawnしない**。フロントが load_config 後に autostart=true の定義に対して spawn_agent を呼ぶ（ペイン生成の主導権はフロント）。
- **cmd はシェル経由**（`/bin/sh -c "<cmd>"`、Windowsは `powershell -Command`）で起動する。PATHや引数分解の面倒を避ける。

## Frontend 追加仕様

- グリッド: **svelte-splitpanes**（MIT）で列×行のリサイズ可能スプリット。ペイン数に応じ 1 / 1x2 / 2x2 / 2x3 / 3x3 に自動配置（手動リサイズ可）。最大9。
- 各ペイン: ヘッダー（名前 or "shell #id"、状態ドット、⟳restart / ✕close / ⤢maximizeトグル）+ xterm.js本体。
- ツールバー: 「+ Shell」（adhoc zsh）、「Open mterm.yml」dirはとりあえずテキスト入力+Loadボタン（Phase 1ではネイティブdialog不使用）、configのagents/processes一覧から「▶起動」。
- 起動時: カレントdirで load_config を試行→成功なら autostart 分を spawn_agent し1ペインずつ配置。失敗（not_found）なら zsh を1枚だけ開く（Phase 0 と同じ見た目）。
- フォント: `fontFamily: "'MesloLGS NF','Hack Nerd Font Mono','JetBrainsMono Nerd Font Mono','Symbols Nerd Font Mono',Menlo,monospace"`（Nerd Fontグリフ□対策）。
- `pty-output` 購読はペインごとにid でフィルタ。`session-state` でヘッダーの状態表示を更新。

---

# Phase 2 追加契約 (Queen: 内蔵MCPサーバー)

両エージェントはこの契約に**厳密に**従うこと。Phase 0/1 は互換維持。

## Queen 概要

アプリ内に MCP サーバー（**rmcp** / streamable HTTP transport）を常駐させ、PTY内で動くエージェントCLI（まずは Claude Code）が MCPクライアントとして接続し、他ペインの読み取り・指示・起動・通知を行えるようにする。

- URL: `http://127.0.0.1:<port>/mcp`
- デフォルトポート: **39237**。使用中なら +1 ずつ最大 39246 まで試行。
- mterm.yml 拡張（任意）:
  ```yaml
  queen:
    enabled: true      # default true
    port: 39237        # default 39237
  ```
- アプリ起動時に自動起動。load_config でポートが変わった場合のみ再起動。
- **Queen が spawn した（= mterm.yml 定義由来の）セッションにも、アプリが spawn する全セッションにも、env `QUEEN_URL=http://127.0.0.1:<port>/mcp` を注入する**（エージェントが自分の接続先を知れるように）。

## MCP tools（5種、名前・引数は固定）

| tool | 引数 (JSON Schema相当) | 返り値(text content) | 説明 |
|---|---|---|---|
| `list_agents` | なし | JSON: `{ sessions: SessionInfo[], definitions: [{name, kind:"agent"\|"process", running:boolean}] }` | 実行中セッションと、mterm.yml定義（起動可能なもの）の一覧 |
| `read_output` | `{ agent: string, lines?: int (default 100, 1..1000), raw?: bool (default false) }` | `{ agent, id, text }` のJSON | 指定エージェントの直近出力。`agent` は定義名 or `"#<id>"`。raw=false ならANSIエスケープ除去済みテキスト |
| `send_message` | `{ agent: string, text: string, submit?: bool (default true) }` | `"ok"` | 対象セッションのstdinへ書き込み。submit=true なら末尾に `\r` を付与 |
| `spawn_agent` | `{ name: string }` | `{ id }` のJSON | **mterm.yml で定義された名前のみ**起動可（許可リスト方式）。未定義名はエラー |
| `notify` | `{ title: string, message: string }` | `"ok"` | フロントにトースト通知を出す |

- `agent` の名前解決: 定義名→その名前で実行中の最新セッション。`#12` 形式はid直指定。見つからなければエラーメッセージに実行中一覧を含める。
- セキュリティ: bind は 127.0.0.1 のみ。spawn は許可リスト（config定義名）のみ。認証はPhase 2では無し（localhost限定で許容）。

## Backend 追加実装

- **出力リングバッファ**: 各セッションの reader thread が emit と同時に slot 内バッファ（上限 256 KiB、超過分は先頭から破棄）へ追記。restart でクリアしない（世代をまたいで連続、`— restarted —` 等の区切りは不要）。
- **ANSIストリップ**: CSI/OSC/単独ESCシーケンス除去のユーティリティ + 単体テスト。
- 新 Tauri command: `queen_status` → `{ enabled: bool, running: bool, port?: number, url?: string, error?: string }`
- 新 event: `queen-notify` `{ title: string, message: string }`（notifyツール呼び出し時にemit）
- rmcp の API 使用パターンは **スタンドアロン検証 crate（mcp-server-check/、pty-core-checkと同方式）** で実証してから本体に組み込む。tokio runtime は `tauri::async_runtime` を利用。

## Frontend 追加実装

- **未知セッションのペイン自動生成**: `session-state` で未知の id が来たら（= Queen の spawn_agent 由来）自動でペインを追加。9面上限なら日本語バナーで通知（セッション自体は動き続ける）。
- **Queenステータス**: ツールバー右側に `● Queen :39237`（running=緑/停止=赤/無効=灰）。クリックで登録コマンド `claude mcp add --transport http queen http://127.0.0.1:<port>/mcp` をクリップボードにコピーし「コピーしました」トースト。ツールチップに URL。
- **queen-notify** 受信 → タイトル+本文のトースト（自動消滅5秒、複数スタック可）。
- `queen_status` は起動時と config 再読込後に取得。

---

# Phase 2.1 追補 (ドッグフーディングのフィードバック反映)

実運用（docs/troubleshooting.md）で判明した問題への対応。

1. **フォアグラウンドプロセスの可視化**: `SessionInfo` に `foreground?: string` を追加（そのPTYのフォアグラウンドプロセス名。取得不能時は省略）。`list_agents` の sessions に含める。ペイン内で手動起動された CLI（zsh の中の codex 等）を発見可能にする。
2. **名前解決の拡張**: `read_output` / `send_message` の `agent` は「定義名 → セッション名 → **フォアグラウンドプロセス名**（完全一致、複数マッチ時は最新ID）→ `#<id>`」の順で解決する。
3. **read_output のCR処理**: ANSI除去後、各行について `\r` で上書きされた部分を畳み込み、最終状態のみ返す（TUIスピナー残骸対策）。`raw: true` では従来通り無加工。
4. **send_message の説明文強化**: MCPツールのdescriptionに「対話型TUIのcomposerに未送信テキストが残っている場合があるため、送信前に read_output で状態確認を推奨。text を空にして submit=true でEnterのみ送出可能」と明記（挙動自体は不変）。

---

# Phase 3.0 追加契約（段階リリース基盤）

Phase 3 は一括変更せず、[docs/phase3.md](docs/phase3.md) の順序で独立して
リリースする。Phase 3.0 は内部構造とテスト基盤のみを変更し、Phase 0–2.1
の IPC、設定スキーマ、UI挙動を一切変更しない。

- Tauri command の引数・返り値・command名は従来どおり。
- command handler は `commands.rs` をIPC境界とし、サービス実装から分離する。
- foreground processの名前解決は注入可能にし、OSの `ps` / `/proc` 権限に
  依存せず名前解決規則をテストできること。
- Phase 3.1以降の契約は、各リリースの実装着手前に本ファイルへ追記する。

---

# Phase 3.1 追加契約（読み取り専用 Git status / diff）

Phase 3.1 はGitリポジトリを読み取るだけで、index・worktree・refsを変更しない。
Git操作はshellを介さず、インストール済みの `git` へ構造化した引数を渡す。

## Tauri Commands

| command | args | returns | 説明 |
|---|---|---|---|
| `git_status` | `{ dir?: string }` | `GitStatusInfo` | porcelain形式で変更ファイル、branch、HEADを取得 |
| `git_diff` | `{ dir?: string, path?: string, staged?: boolean }` | `GitDiffInfo` | unified diffを取得。`staged` default false |

`dir` の省略時はロード済み `mterm.yml` のディレクトリ、それも無い場合は
アプリのカレントディレクトリを使用する。Gitリポジトリ外なら
`Err("not_a_git_repository: ...")` を返す。

```ts
type GitFileStatus = {
  path: string;
  originalPath?: string;
  indexStatus: string;
  worktreeStatus: string;
};
type GitStatusInfo = {
  repoRoot: string;
  branch?: string;
  head: string;
  files: GitFileStatus[];
  truncated: boolean;
};
type GitDiffInfo = {
  repoRoot: string;
  path?: string;
  staged: boolean;
  text: string;
  truncated: boolean;
};
```

## 制限とセマンティクス

- statusは最大10,000ファイル。超えた場合は `truncated: true`。
- diffは最大2 MiB。超えた場合は末尾に切り詰め表示を追記し
  `truncated: true`。
- `path` の前には必ずGitの `--` separatorを付ける。
- external diff、textconv、pagerは無効化する。
- Phase 3.1ではuntracked file本文、stage、unstage、commitは対象外。

---

# Phase 3.2 追加契約（Git stage / unstage / inline commit）

Phase 3.2 は明示的に選択されたpathだけをstage/unstageし、commitは呼出時点の
indexだけを対象にする。ファイル選択やcommit操作による暗黙stageは禁止する。

## Tauri Commands

| command | args | returns | 説明 |
|---|---|---|---|
| `git_stage` | `{ dir?: string, paths: string[] }` | `GitStatusInfo` | 指定pathをstageし、更新後statusを返す |
| `git_unstage` | `{ dir?: string, paths: string[] }` | `GitStatusInfo` | 指定pathをunstageし、更新後statusを返す |
| `git_commit` | `{ dir?: string, message: string }` | `GitCommitInfo` | 現在のindexをcommitする |

```ts
type GitCommitInfo = {
  repoRoot: string;
  oid: string;
  summary: string;
  output: string;
};
```

## 制限とセマンティクス

- `paths` は1件以上、最大1,000件。空文字は禁止。
- path引数の前には必ず `--` separatorを置き、shellは使用しない。
  `GIT_LITERAL_PATHSPECS=1` によりpathspec magicとして解釈しない。
- stageは `git add -- <paths...>`。削除とuntracked fileも対象にできる。
- unstageはHEADがあれば `git restore --staged`、unborn repositoryでは
  `git rm --cached --ignore-unmatch` を使う。worktree本文は変更しない。
- commit messageはコマンド引数にせずstdinから `git commit --file=-` へ渡す。
- 空メッセージは禁止。`--no-verify` / `--no-gpg-sign` は付けず、Git hooksと
  署名設定を尊重する。
- commitはblocking Git processをTauriのblocking taskで実行し、UIスレッドを
  直接ブロックしない。
- untracked fileのdiffは、Gitが返した完全一致pathだけを対象にし、canonical pathが
  repository外へ出る場合は拒否する。

---

# Phase 3.3 追加契約（opt-in agent worktree isolation）

Phase 3.3 は定義ごとに明示的に有効化した場合だけlinked worktreeを作成する。
`worktree` 未指定または `enabled: false` の既定動作は、従来どおり共有cwdを使う。

## mterm.yml拡張

```yaml
agents:
  - name: codex
    cmd: codex
    cwd: packages/app
    worktree:
      enabled: true       # default false
      base: HEAD          # default HEAD。branch/tag/commitも可
      setup: npm install  # 任意。worktree作成後、agent cwdで一度実行
```

```ts
type WorktreeConfig = {
  enabled?: boolean;
  base?: string;
  setup?: string;
};
type WorktreeInfo = {
  name: string;
  repoRoot: string;
  path: string;
  branch: string;
  base: string;
  locked: boolean;
};
```

`AgentDef` に `worktree?: WorktreeConfig`、`SessionInfo` に
`worktree?: WorktreeInfo` を追加する。既存fieldは変更しない。

## 作成セマンティクス

- 定義の解決済みcwdからGit repository rootとgit common-dirを検出する。
- 元cwdがrepository外、存在しない、またはGit repositoryでない場合はエラー。
  共有cwdへ暗黙fallbackしない。
- 保存先はTauri app-data配下の
  `worktrees/<common-dir hash>/<agent slug>-<unique suffix>`。
- branch名は `ptygrid/<agent slug>/<unique suffix>`。`base` から新規作成する。
- `git worktree add --lock --reason "ptygrid active session" -b ...` を使用する。
- 元cwdがrepository内のサブディレクトリなら、linked worktree内でも同じ相対cwdを使う。
- `setup` はworktree作成後、agent cwdでshell経由で一度だけ実行し、定義の展開済みenvを渡す。
- setup失敗、cwd不足、後続のPTY spawn失敗ではworktreeとbranchを削除せず、復旧用pathをエラーに含める。
- manual restart / autorestartは同じworktreeとbranchを再利用し、追加worktreeを作らない。
- Phase 3.3では自動削除しない。dirty worktreeを黙って削除しないことを優先し、明示cleanup UIは後続リリースとする。

## Frontend

- worktree sessionのペインヘッダーにbranch名を表示し、ツールチップにpathを表示する。
- 実行中sessionにworktreeがある場合、Gitパネルにworkspace selectorを表示する。
- selectorでworktreeを選ぶと、Phase 3.1/3.2のstatus/diff/stage/unstage/commitは
  そのworktree pathを `dir` として使用する。
