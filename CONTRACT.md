# IPC / Service Contract (backend ⇄ frontend / Queen)

> この文書は段階releaseごとの差分契約を時系列で保持する。前のPhaseと後のPhaseが競合する
> 場合は、後のPhaseの「追加契約」が現在の有効仕様として優先される。現在の実装はPhase 4.0。
> 操作方法と現在仕様の要約は[docs/userguide.md](docs/userguide.md)を参照。

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

## MCP tools（Phase 2時点の基本5種）

| tool | 引数 (JSON Schema相当) | 返り値(text content) | 説明 |
|---|---|---|---|
| `list_agents` | なし | JSON: `{ sessions: SessionInfo[], definitions: [{name, kind:"agent"\|"process", running:boolean}] }` | 実行中セッションと、mterm.yml定義（起動可能なもの）の一覧 |
| `read_output` | `{ agent: string, lines?: int (default 100, 1..1000), raw?: bool (default false) }` | `{ agent, id, text }` のJSON | 指定エージェントの直近出力。`agent` は定義名 or `"#<id>"`。raw=false ならペイン寸法を使ってANSIカーソル移動・消去・alternate screenを再構成したテキスト |
| `send_message` | `{ agent: string, text: string, submit?: bool (default true) }` | `"ok"` | 対象セッションのstdinへ書き込み。submit=true なら末尾に `\r` を付与 |
| `spawn_agent` | `{ name: string }` | `{ id }` のJSON | **mterm.yml で定義された名前のみ**起動可（許可リスト方式）。未定義名はエラー |
| `notify` | `{ title: string, message: string }` | `"ok"` | フロントにトースト通知を出す |

- `agent` の名前解決(Phase 2時点): 定義名→その名前で実行中の最新セッション。`#12`形式は
  id直指定。見つからなければerrorに実行中一覧を含める。この推測規則はPhase 3.6で廃止され、
  現在は`#id`優先かつ名前が一意な場合だけ解決する。
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
2. **名前解決の拡張(当時)**: `read_output` / `send_message`の`agent`は「定義名 →
   session名 → foreground process名 → `#<id>`」へ拡張した。複数match時に最新IDを選ぶ規則は
   Phase 3.6で廃止され、現在は候補IDを返して曖昧errorにする。
3. **read_output のterminal再構成**: ペインのrows/colsを境界に、`\r`、CSIカーソル移動、画面/行消去、save/restore cursor、alternate screenを適用して現在の表示とscrollbackをテキスト化する。色など表示専用のsequenceは無視する。`raw: true`では従来通り無加工。
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

---

# Phase 3.4 追加契約（versioned project state / logical resume）

Phase 3.4 はアプリ終了後にPTYへ再接続する機能ではない。前回のproject、pane順、
layoutと論理session参照を復元し、現在の`mterm.yml`から新しいPTY processを起動する。

## mterm.yml拡張

```yaml
agents:
  - name: codex
    cmd: codex
    resume: codex resume --last  # 任意。logical resume時だけ使用
```

`AgentDef`へ`resume?: string`を追加する。省略時のlogical resumeは`cmd`を再実行する。
通常起動、manual restart、autorestartは従来どおり、そのsessionが保持するcommandを使う。

## Tauri Commands

| command | args | returns | 説明 |
|---|---|---|---|
| `save_project_state` | `{ state: ProjectState }` | `()` | 検証後にproject別stateとlast-project pointerをatomic保存 |
| `load_project_state` | `{ dir?: string }` | `ProjectState \| null` | 指定project、または最後に保存したprojectのstateを読む |
| `resume_logical_session` | `{ session, cols, rows }` | `number` | 現在のconfig定義を再解決して新規PTYを起動 |

```ts
type LogicalSession =
  | { kind: "definition"; name: string; worktree?: WorktreeInfo }
  | { kind: "shell" };

type ProjectState = {
  version: 1;
  configDir: string;
  layoutMode: "auto" | "1" | "2" | "3";
  sessions: LogicalSession[];       // pane順、最大9件
  maximizedIndex?: number;
};
```

`ConfigInfo`には解決済みconfig directoryを示す`dir: string`を追加する。

## 保存・復元セマンティクス

- 保存先はTauri app-data配下の`project-state/projects/<canonical-dir hash>.json`。
  最終projectは`project-state/last-project.json`で参照する。repository内には書かない。
- JSONは`version: 1`必須。未知version、未知layout、10件以上のsession、不正な
  `maximizedIndex`は拒否する。
- 保存対象はconfig directory、layout、pane順、定義名、worktree参照だけ。
  `cmd`、terminal output、展開前後のenv値、`QUEEN_URL`は保存しない。
- 起動時はlast-projectを読み、現在の`mterm.yml`をloadしてからpane順に再起動する。
  stateがない場合だけ従来のautostartを適用する。
- state破損、project消失、未知versionでは復元エラーを表示し、通常startupへfallbackする。
- 定義が削除されたなど一部sessionだけ失敗した場合、残りのsession復元を継続する。
- shell参照はdefault shellを新規起動する。以前のshell processやscrollbackには再接続しない。
- 保存worktreeが存在し同じgit common-dirに属する場合はsetupを再実行せず再利用する。
  消失していれば現在のworktree定義から新規作成し、別repositoryを指す既存pathは拒否する。

---

# Phase 3.5 追加契約（process-tree resource monitoring）

全sessionを1つの共有samplerで監視し、PTY直下のchild processだけでなく、その全子孫の
CPU使用率とresident memoryをsession単位で合算する。

## Event

| event | payload | 発火タイミング |
|---|---|---|
| `session-resources` | `SessionResourcesPayload` | 1秒ごとに全running sessionを1 batchでemit |

```ts
type SessionResourceUsage = {
  id: number;
  cpuPercent: number;
  memoryBytes: number;
  processCount: number;
};
type SessionResourcesPayload = {
  sampledAtMs: number;
  sessions: SessionResourceUsage[];
};
```

## Sampling semantics

- `sysinfo::System`はアプリ全体で1 instanceだけ作り、1秒間隔で再利用する。
  sessionごとのsampler threadや`ps` subprocessは作らない。
- process情報は各tickで一度だけrefreshし、parent PID graphからPTY childをrootとする
  全descendantを走査する。CPU、memory、process countはroot自身を含めて合算する。
- `cpuPercent`は1 coreを100%とするprocess CPUの合計で、multi-core workloadでは100を超え得る。
- `memoryBytes`は各processのresident memory byte数の合計。共有pageの重複排除はしない。
- CPU deltaを有効にするため初回refreshはprime専用とし、1 interval後からemitする。
- sample中に消失したroot、取得不能なprocess treeはそのbatchから省略する。
  frontendはbatchにない古い値を削除する。
- samplerはsession map lock中にPID snapshotだけを取り、OS refreshやtree集約中はlockを保持しない。

## Frontend

- 各running paneのheaderに`CPU n.n% · n MiB/GiB`を表示する。
- tooltipに集約process countとbyte数を表示する。
- toolbar右側に、最新batch内の全sessionを合算した
  `Σ CPU n.n% · n MiB/GiB`を表示する。追加samplingは行わない。
- batch eventごとにresource mapを1回だけ置換し、exit/restart/close時は対象値を削除する。

---

# Phase 3.6 追加契約（durable Queen pins / notes）

Phase 3.6はQueen MCPを13 toolsへ拡張し、読み込まれたprojectに紐づく短い共有値と
共有noteを永続化する。Phase 2/2.1の「同名なら最新IDを選ぶ」規則は廃止し、曖昧な
宛先への誤送信を拒否する。

## Queen MCP tools

| tool | args | returns | 説明 |
|---|---|---|---|
| `set_pin` | `{ key, value, expectedRevision? }` | `{ pin }` | 新規pinを作成、またはrevision一致時だけ更新 |
| `list_pins` | `{}` | `{ pins }` | project内pinをkey順で返す |
| `delete_pin` | `{ key, expectedRevision }` | `{ deleted: true, key }` | revision一致時だけ削除 |
| `create_note` | `{ title, body, tags? }` | `{ note }` | noteを作成 |
| `list_notes` | `{ query?, limit? }` | `{ notes }` | title/body/tagsを任意検索し、更新日時の降順で返す |
| `get_note` | `{ id }` | `{ note }` | project内noteを取得 |
| `update_note` | `{ id, expectedRevision, title?, body?, tags? }` | `{ note }` | 指定fieldをrevision一致時だけ更新 |
| `delete_note` | `{ id, expectedRevision }` | `{ deleted: true, id }` | revision一致時だけ削除 |

```ts
type Pin = {
  key: string;
  value: string;
  revision: number;
  createdAtMs: number;
  updatedAtMs: number;
};

type Note = {
  id: number;
  title: string;
  body: string;
  tags: string[];
  revision: number;
  createdAtMs: number;
  updatedAtMs: number;
};
```

## 保存と競合制御

- 保存先はTauri app-data配下の`queen/queen.sqlite3`。repository内には書かない。
- project scopeは読み込まれた`mterm.yml`のcanonical directory pathとする。
  project未読込時、pins/notes toolsはerrorにする。
- schemaは`PRAGMA user_version = 1`。未知の新しいversionは起動時に拒否する。
- 1つの`Mutex<Connection>`でprocess内writeを直列化し、各mutationはSQLiteの
  `BEGIN IMMEDIATE` transactionとして実行する。永続DBはWAL、busy timeout 5秒とする。
- 新規pinは`expectedRevision`を省略する。既存pinの更新、pin/noteの削除、noteの更新は
  現在の`revision`と一致する正の`expectedRevision`を必須とする。
- 成功した更新は`revision`を1増やす。同じrevisionを複数agentが同時更新しても1件だけが
  成功し、後続は`conflict`でrollbackする。stale update/deleteによるlost updateを許さない。
- note IDはDB内で一意かつ安定。異なるprojectのnoteはIDを知っていても取得・変更できない。
- 上限はprojectごとにpin 256件、note 10,000件。key 128 bytes、value 16 KiB、
  title 256 bytes、body 64 KiB、tag 32件・各64 bytes。`list_notes`は最大200件。

## Session宛先の識別

- pane headerは名前がある場合も常に`<name> #<id>`を表示する。
- `agent: "#<id>"`はsession IDを厳密指定し、最優先で解決する。
- 定義/session名、次にforeground process名は完全一致かつ候補が1つの場合だけ解決する。
- 同名のCodex/Claude/Grok等が複数ある場合、最新IDを推測してはならない。曖昧errorに
  全候補を`#<id>`形式で含め、callerに厳密指定を求める。
- session IDは現在のapp実行中の識別子であり、app再起動後は`list_agents`で再取得する。

---

# Phase 3.7 追加契約（durable Queen inbox / reply）

Phase 3.7はQueen MCPを17 toolsへ拡張し、live PTYへ直接入力する`send_message`とは別に、
project-scopedな永続inboxを提供する。messageは追記専用で、安定ID、acknowledgement、
reply correlationを持つ。

## Queen MCP tools

| tool | args | returns | 説明 |
|---|---|---|---|
| `send_inbox` | `{ sender, recipient, subject, body }` | `{ message }` | mailboxへroot messageを作成 |
| `list_inbox` | `{ mailbox, afterId?, includeAcknowledged?, limit? }` | `{ messages, nextCursor }` | ID昇順でmailboxを読む |
| `ack_inbox` | `{ id, recipient }` | `{ message }` | 宛先本人としてmessageをidempotentにacknowledge |
| `reply_inbox` | `{ id, sender, body }` | `{ message }` | 元宛先から元送信者へcorrelated replyを作り、元messageもacknowledge |

```ts
type InboxMessage = {
  id: number;
  sender: string;
  recipient: string;
  subject: string;
  body: string;
  inReplyToId?: number;
  rootMessageId: number;
  acknowledgedAtMs?: number;
  createdAtMs: number;
};
```

## Inbox semantics

- mailbox名はtrim後1〜128 bytesの安定した論理名とする。app再起動で変わる`#<id>`形式は拒否する。
  定義には`codex-review`、`claude-impl`等、役割を含む一意な名前を推奨する。
- sender/recipientは明示引数とする。Phase 3.7ではMCP client identityとの暗黙bindingや認証は行わない。
- subjectは1〜256 bytes、bodyは最大64 KiB。projectごとに最大50,000 messages。
- root messageは`inReplyToId`なし、`rootMessageId == id`。replyは
  `inReplyToId == 元message.id`かつ元messageの`rootMessageId`を継承する。
- `reply_inbox`のsenderは元messageのrecipientと完全一致しなければ拒否する。新messageのrecipientは
  元messageのsenderとし、subjectは元messageから継承する。reply作成と元messageのacknowledgeは
  同じ`BEGIN IMMEDIATE` transactionでcommitする。
- `ack_inbox`のrecipientは元messageのrecipientと完全一致必須。acknowledgementは単調な
  `null -> timestamp`遷移で、同じrecipientによる再実行は同じmessageを返すidempotent operationとする。
- `list_inbox`は`id > afterId`をID昇順で返す。defaultはunacknowledgedだけ、limit 50、最大200。
  `nextCursor`は返却messageの最大ID、0件なら入力`afterId`を返す。
- message本文は更新・削除しない。誤送信訂正・retention policyはPhase 3.7の対象外。
- storageは既存`queen/queen.sqlite3`をschema version 2へtransactional migrationする。
  project scope、WAL、busy timeout、repositoryへfileを作らない規則はPhase 3.6を継承する。

---

# Phase 3.8 追加契約（cancellable Queen `await`）

Phase 3.8はQueen MCPを18 toolsへ拡張し、Inbox messageをbusy pollingせず待つread-only tool
`await`を追加する。

## Queen MCP tool

| tool | args | returns | 説明 |
|---|---|---|---|
| `await` | `{ mailbox, afterId?, includeAcknowledged?, limit?, timeoutMs? }` | `{ messages, nextCursor, timedOut }` | cursorより後のmessage到着、timeout、MCP cancellationのいずれかまで待つ |

## Await semantics

- mailbox、afterId、includeAcknowledged、limitのfilterは`list_inbox`と同一。defaultは
  `afterId: 0`、`includeAcknowledged: false`、`limit: 50`。
- `timeoutMs`は1〜300,000、default 30,000。無制限waitは提供しない。
- 呼出時点ですでに一致messageがあれば待たずに返す。messageがある場合は
  `timedOut: false`、`nextCursor`は返却最大ID。
- deadlineまでにmessageがなければ`messages: []`、入力afterIdを`nextCursor`、
  `timedOut: true`として正常returnする。timeoutはerrorにしない。
- Queen Storeはprocess内のInbox generationを`tokio::sync::watch`で通知する。
  waiterは通知をsubscribeしてから初回queryすることで、queryとwait開始の間のlost wakeupを防ぐ。
- `send_inbox` / `reply_inbox`はDB transaction commit後にgenerationを1増やす。
  rollback、ackだけの変更では新規message通知を出さない。
- rmcpがrequestに付与する`CancellationToken`をtool handlerへextractし、MCP
  `notifications/cancelled`受信時はwaitを直ちに終了してcancellation errorを返す。
  cancellationはInboxをack/updateせず、cursorも永続化しない。
- 同時にmessage、timeout、cancellationがreadyの場合はcancellationを優先する。
- wait中はSQLite connection mutexやsession map lockを保持しない。通知ごとの短いqueryだけを行う。

---

# Phase 4.0 追加契約（teammate hooks 受信基盤）

Phase 4.0 は Claude Code 等の CLI が発火する teammate ライフサイクル hook を受信する
HTTP 基盤を追加する。受信専用でノンブロッキング（常に `200 {"decision":"allow"}`）。
Phase 4.1 の transcript ペインや `agents[].teams` は含まない。Phase 0〜3.8 は互換維持。

## HTTP endpoints（Queen と同居 / bind は 127.0.0.1 のみ）

Queen の Axum アプリ（`/mcp`）と同じ 127.0.0.1:<port> サーバー上に `/hooks/v1/*` を増設する。
全 endpoint は **POST + `Content-Type: application/json` + `Authorization: Bearer <token>` 必須**。
CORS は許可しない（preflight 用の OPTIONS/CORS ヘッダを付けない）。

| method | path | kind | 追加の必須field |
|---|---|---|---|
| POST | `/hooks/v1/subagent-start` | `subagent-start` | `agent_id` |
| POST | `/hooks/v1/subagent-stop` | `subagent-stop` | `agent_id` |
| POST | `/hooks/v1/teammate-idle` | `teammate-idle` | （なし） |
| POST | `/hooks/v1/task-created` | `task-created` | `task_id` |
| POST | `/hooks/v1/task-completed` | `task-completed` | `task_id` |

### payload（受信 JSON、snake_case）

`session_id`（全 endpoint 共通で必須）, `agent_id`, `agent_type`, `transcript_path`,
`cwd`, `task_id`, `task_name`, `status`, `team_name`。未知fieldは無視する。

### ステータスセマンティクス

- **401**: `Authorization: Bearer <token>` が欠落 / 不一致。emit しない。
- **400**: token は正しいが Content-Type が `application/json` でない、JSON parse 失敗、
  または必須field欠落。ログ（stderr）のみ出力し emit しない。
- **200** `{"decision":"allow"}`: 正常。ブロッキング判断は返さない。
  `teammates.enabled: false`（デフォルト）の間は token 検証のみ行い、
  イベント emit と toast をスキップして 200 allow を返す。

## token（非永続）

- アプリ起動ごとにランダム 256bit を生成（`getrandom`、lowercase hex）。
- ディスクに保存しない。`register_teammate_hooks` が書く settings.json のスニペットは
  当該起動中のみ有効。アプリ再起動後は `teammate_hooks_info` で再取得し再登録が必要。

## mterm.yml 拡張（グローバル `teammates:` ブロック、すべて任意）

```yaml
teammates:
  enabled: false            # default false。true で hook の emit / toast を有効化
  hook_notifications: true  # default true。teammate-lifecycle 受信時の toast 可否
  global_max_panes: 6       # default 6、1..9 に clamp（Phase 4.1 で使用）
  hooks_scope: user         # "user" | "project"、default "user"
```

- 未知fieldは無視、欠落はデフォルト補完。`agents[].teams` は Phase 4.1 のため今回は未対応。

## 追加 Tauri Commands

| command | args | returns | 説明 |
|---|---|---|---|
| `teammate_hooks_info` | なし | `TeammateHooksInfo` | frontend がスニペット生成に使う設定・ポート・token |
| `register_teammate_hooks` | `{ scope: "user" \| "project" }` | `{ written: boolean, path: string }` | settings.json へ hooks 定義をマージ |

```ts
type TeammateHooksInfo = {
  enabled: boolean;
  hookNotifications: boolean;
  port: number;      // Queen の実ポート（bind 済みならその値、未 bind なら設定ポート）
  token: string;
  hooksScope: "user" | "project";
};
```

- `register_teammate_hooks`:
  - `user` → `~/.claude/settings.json`、`project` → `<project>/.claude/settings.json`
    （project 未読込時はエラー）。
  - 既存内容を保ってマージ。`hooks` 内の各イベント配列に、`type: "http"` /
    実ポートの URL / `headers.Authorization: "Bearer <token>"` を持つグループを追加する。
  - 古い ptygrid エントリ（URL が `http://127.0.0.1:` + 任意 port + `/hooks/v1/` に一致）は置換。
  - 書込前に同 dir へ `settings.json.ptygrid-backup-<unix秒>` を作成。
  - 結果が既存と同一（意味的に等価）なら書き込まず backup も作らず `written: false`。

## 追加 Tauri Event

| event | payload | 説明 |
|---|---|---|
| `teammate-lifecycle` | 下記 | hook を正規化して frontend へ emit（`teammates.enabled: true` 時のみ） |

```ts
type TeammateLifecyclePayload = {
  kind: "subagent-start" | "subagent-stop" | "teammate-idle"
      | "task-created" | "task-completed";
  sessionId?: string;
  agentId?: string;
  agentType?: string;
  taskId?: string;
  taskName?: string;
  status?: string;
  cwd?: string;
};
```

## Frontend

- ツールバーに **Teammates バッジ**（Queen バッジと同列）。`teammates.enabled` を反映し、
  無効時はグレー。クリックでポップオーバー: (a) hooks 設定スニペット（token 埋め込み JSON）の
  コピー、(b) user スコープの settings.json 登録ボタン、(c) 直近の teammate-lifecycle 一覧（最大10件）。
- `teammate-lifecycle` 受信時、`hook_notifications` が有効なら既存 notice/toast 機構で
  日本語短文を表示する。

---

# Phase 4.1 追加契約（observe: read-only transcript ペイン）

Phase 4.1 は方式B（hooks 観測）の中核を実装する。`SubagentStart` hook を受けて **PTY を持たない
論理セッション（新種別 `transcript`）** を自動生成し、subagent の transcript JSONL を read-only で
tail する。Phase 0〜4.0 は互換維持（既存 PTY 経路・Queen MCP 契約・IPC に非回帰）。方式A（host, 実 PTY）
は Phase 4.2。**`agents[].teams.mode: host` は 4.1 では observe と同一挙動**（read-only transcript ペイン）
とし、実 PTY ホストは 4.2 で実装する。

## mterm.yml 拡張（`agents[].teams` ブロック、すべて任意）

```yaml
agents:
  - name: claude
    cmd: claude
    teams:
      enabled: false         # default false。この lead で teammate ペイン化を行う
      mode: observe          # "observe" | "host"、default observe（4.1 では host も observe と同挙動）
      max_panes: 3           # この lead が生む teammate ペイン上限、default 3、1..9 に clamp
      transcript_tail: true  # default true。false なら subagent はステータス（lifecycle）のみ、ペイン非生成
```

- 未知field（`teammate_binaries` / `fallback_to_observe` 等の host 用 4.2 field 含む）は無視、欠落はデフォルト補完。
- ブロック省略 = teammate ペイン化オフ（従来どおり）。

```ts
type AgentTeamsConfig = {
  enabled?: boolean;              // default false
  mode?: "observe" | "host";      // default "observe"（4.1 では host == observe）
  maxPanes?: number;              // default 3, clamp 1..9
  transcriptTail?: boolean;       // default true
};
// AgentDef へ teams?: AgentTeamsConfig を追加
```

## 新ペイン種別 `transcript`（PTY を持たない論理セッション）

- backend の session map に、PTY・writer・reader thread・output ring（tail 整形テキストは
  既存 output ring を流用）を **spawn しない**論理セッションを追加する。`u32` id + generation は
  既存 PTY と同じ採番（`next_id` / `generations`）を共有する。
- `SessionInfo` を additive に拡張（既存fieldは不変）:

```ts
type SessionInfo = {
  /* 既存field... */
  kind?: "pty" | "transcript";     // 既存セッションは常に "pty"
  teammate?: {                     // transcript セッションにのみ付与
    role?: string;                 // hook payload の agent_type
    leadId: number;                // 親 lead セッションの #id
    mode: "observe";               // 4.1 では常に observe（host も observe と表示）
  };
};
```

- `list_agents`（Queen MCP）の `sessions[]` に kind 付きで現れる。
- `read_output`（Queen MCP）は transcript セッションに対し、**整形済みテキストをそのまま返す**
  （ANSI 画面再構成は行わない。`raw` 指定は無視して整形済みを返す）。
- `send_message`（Queen MCP）は transcript 宛先を **`invalid_params` で拒否**する
  （read-only・stdin なし。「session #id is a read-only teammate transcript and cannot receive messages」）。
- `restart_session` は transcript セッションを拒否する（respawn 対象の PTY spec を持たない）。
- close（`kill_pty`）は論理セッションを map から除去するのみ。subagent プロセスには一切影響しない。

## SubagentStart → transcript セッション自動生成

`teammates.enabled: true` かつ以下の帰属・上限判定を満たすとき、`SubagentStart` 受信で transcript
セッションを生成する（`teammates.enabled: false` の間は Phase 4.0 どおり emit/生成なし）。

### lead 帰属

- **lead 候補** = 実行中（running）の PTY セッションのうち、mterm.yml 定義に `teams.enabled: true` を持つもの。
- hook payload の `cwd` を正規化パス（canonicalize、失敗時は入力パス）で lead 候補の cwd と比較し、
  **一致する候補**へ帰属（複数一致時は最小 id）。
- cwd 一致が無い場合、lead 候補が **ちょうど1つ**ならそれへ帰属。
- それも無ければ **ペインを作らずログ（stderr）のみ**。

### 上限（超過時はセッションを作らず日本語バナー通知のみ）

- lead ごと `teams.max_panes`（該当 lead の transcript セッション数）。
- 全体 `teammates.global_max_panes`（transcript セッション総数）。
- グリッド 9 面（全セッション総数）。
- いずれか超過時は **transcript 論理セッションを生成せず**、`teammate-banner` イベントでバナー通知する
  （Phase 2 の 9 面上限バナー経路 = `ui.errorBanner` を踏襲）。transcript は paneless にしても tail コストが
  無駄なため、4.1 では「作らない」を採用する。
- `transcript_tail: false` の lead は、ペインを生成せず lifecycle イベント（ステータス）のみ。

### SubagentStop

- `SubagentStop` 受信で、該当 `agent_id` の transcript セッションを `stopped`（state=exited）へ遷移し、
  tail を停止する（generation bump）。ペインは残置し最終状態を表示する。

## transcript tail と path 検証

- **path 検証（セキュリティ）**: hook payload の `transcript_path` は、**絶対パスかつ `$HOME/.claude/`
  配下**（`..` 成分を含まない）のみ tail を許可する。それ以外はステータス表示のみに縮退する
  （任意ファイル読み出しの踏み台化を防止）。`transcript_path` 欠落時も **フォールバック path 構築は行わず**
  （[観測] 依存を避ける）ステータスのみ。
- **tail 実装**: `notify` crate でファイル（親 dir）を watch し、失敗時・および常時 300ms のポーリングへ
  フォールバックする。追記された JSONL の完全行のみをパースし、`role: text` を時系列連結した簡易テキストへ整形
  （tool 呼び出しは `[tool_use: <name>]` の1行要約）。パース不能行・表示要素の無い行は **生表示せずスキップ**
  してカウントする。
- generation により stale watcher の emit・append を無効化する（既存 PTY reader と同じ規律）。
- 新モジュール `src-tauri/src/transcript.rs` に隔離し、session hot path に混ぜない。

## 追加 Tauri Event

| event | payload | 説明 |
|---|---|---|
| `transcript-output` | `{ id: number, text: string }` | tail が整形した **追記分のみ**（既存 `pty-output` とは別イベント）。generation で stale 抑止 |
| `teammate-banner` | `{ message: string }` | ペイン上限超過時のバナー（frontend は `ui.errorBanner` に表示） |

## ProjectState

- transcript セッションは ephemeral であり **`ProjectState.sessions` に保存しない**（Phase 3.4 継承・
  logical resume の対象外）。`LogicalSession` に transcript variant は存在せず、frontend は保存前に
  transcript ペインを除外する（backend `project_state::persistable_pane_ids` が同じ不変条件を単体テスト可能に保持）。
  resume 後は lead の再起動により subagent が改めて生成される。

## Frontend

- transcript ペインは xterm ではなく **read-only のスクロールビュー**（等幅・追記で自動スクロール・
  ユーザーが上へスクロール中は追従停止）。ヘッダーは `claude·sub #<id> ▸<role> 📖RO` + 親 lead 併記 `↳#<leadId>`、
  状態ドット active（running）/ stopped（exited）。**restart ボタンなし**・close 可・maximize 可。
- close は「tail 停止のみ・subagent には影響しない」旨を tooltip で明示する。
- 9 面上限バナー・toast は既存経路（`ui.errorBanner` / notices）を流用する。

---

# 設定ファイル名の変更（ptygrid.yml / legacy mterm.yml）

config filenameを`mterm.yml`から`ptygrid.yml`へ変更する。migration設計として
legacy filenameを引き続き受理し、wire formatは変更しない。

## `load_config` の解決順

1. `<dir>/ptygrid.yml` があればそれを読む
2. 無ければ `<dir>/mterm.yml`（legacy）を読む
3. 両方無ければ `not_found: <dir>/ptygrid.yml (also tried legacy <dir>/mterm.yml)` エラー

- 両方存在する場合は`ptygrid.yml`が勝つ。file watcherは**実際に読み込んだfile**だけを
  追跡する（legacyを読み込んだ場合、後から置いた`ptygrid.yml`は次のload_configまで
  検知しない）。
- `ConfigInfo.path`は実際に読み込んだfileのpathを返す（既存フィールド、意味の明確化のみ）。
- スキーマ・`config-changed` event・Reload挙動は不変。

## 影響範囲

- Queen MCP toolのdescription文字列中の`mterm.yml`表記を`ptygrid.yml`へ更新
  （tool名・引数・返り値は不変）。
- UI文言・README・userguide・exampleを`ptygrid.yml`表記へ更新。注釈付き全項目
  サンプルは`ptygrid.example.yml`（旧`mterm.example.yml`）、用途別スターターは
  `example/{basic,multi-agent,web-dev,worktree,teammates}/ptygrid.yml`に配置。
