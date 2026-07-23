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

- Rust側の型: `Config { project: Option<String>, agents: Vec<AgentDef>, processes: Vec<AgentDef> }`（`agents`・`processes` とも省略時は空Vec。`queen:` / `processes:` / `teammates:` のみの config も有効）。
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
- **Queen が spawn した（= mterm.yml 定義由来の）セッションにも、アプリが spawn する全セッションにも、env `QUEEN_URL=http://127.0.0.1:<port>/mcp?token=<token>` を注入する**（エージェントが自分の接続先を知れるように）。トークンについては後述の「Queen 認証」を参照。

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
- セキュリティ: bind は 127.0.0.1 のみ。spawn は許可リスト（config定義名）のみ。**認証は当初 Phase 2 では無しだったが、Finding S1 対応で token + Host/Origin 検証を追加した（下記「Queen 認証」）。**

## Backend 追加実装

- **出力リングバッファ**: 各セッションの reader thread が emit と同時に slot 内バッファ（上限 256 KiB、超過分は先頭から破棄）へ追記。restart でクリアしない（世代をまたいで連続、`— restarted —` 等の区切りは不要）。
- **ANSIストリップ**: CSI/OSC/単独ESCシーケンス除去のユーティリティ + 単体テスト。
- 新 Tauri command: `queen_status` → `{ enabled: bool, running: bool, port?: number, url?: string, token: string, error?: string }`（`url` は token 抜きの表示用、`token` は登録 URL 組み立て用。「Queen 認証」参照）
- 新 event: `queen-notify` `{ title: string, message: string }`（notifyツール呼び出し時にemit）
- rmcp の API 使用パターンは **スタンドアロン検証 crate（mcp-server-check/、pty-core-checkと同方式）** で実証してから本体に組み込む。tokio runtime は `tauri::async_runtime` を利用。

## Frontend 追加実装

- **未知セッションのペイン自動生成**: `session-state` で未知の id が来たら（= Queen の spawn_agent 由来）自動でペインを追加。9面上限なら日本語バナーで通知（セッション自体は動き続ける）。
- **Queenステータス**: ツールバー右側に `● Queen :39237`（running=緑/停止=赤/無効=灰）。クリックで登録コマンド `claude mcp add -s user --transport http queen http://127.0.0.1:<port>/mcp?token=<token>` をクリップボードにコピーし「コピーしました（再起動ごとに再登録が必要）」トースト。ツールチップには token 抜きの URL と「再登録が必要」注記。
- **queen-notify** 受信 → タイトル+本文のトースト（自動消滅5秒、複数スタック可）。
- `queen_status` は起動時と config 再読込後に取得。

## Queen 認証（token + Host/Origin 検証, Finding S1）

`/mcp` は無認証だと 127.0.0.1 に到達できる同一ホストの別プロセス/別ユーザーや、DNS
リバインディングした悪意ある Web ページから全ツール（`send_message` 経由の任意ペイン
stdin 書込 → RCE 含む）を呼べた。これを塞ぐため以下を契約に追加する。

- **token 生成**: アプリ起動ごとに 256bit ランダム（lowercase hex）を 1 つ生成し、`QueenStatus`
  が保持する。**非永続**（毎回変わる・ディスクに書かない）。teammate hooks の Bearer トークン
  とは**別トークン**（用途別）。生成方式は共通（`getrandom` / OS CSPRNG）。
- **`/mcp` 検証（axum middleware）**: リクエストを以下の順に検証する。
  1. **Host allow-list**: `Host` ヘッダが loopback ホスト（`127.0.0.1` / `localhost`）であり、
     ポートを含む場合は bind 済みポートと一致すること。Host 欠落・非 loopback は **403**。
  2. **Origin allow-list**: `Origin` ヘッダが存在する場合、`http(s)://` の loopback オリジンで
     あること。非 loopback（実 Web オリジン・`null`）は **403**（DNS リバインディング対策）。
  3. **token**: URL クエリ `?token=<hex>`（MCP クライアントは URL をそのまま使うだけでよい）
     **または** `Authorization: Bearer <hex>` のどちらかが一致すること。両方欠落/不一致は **401**。
  - token 比較は**定数時間**（`teams_hooks::constant_time_eq`）。同関数で hooks 側 Bearer 比較も
    定数時間化した（Finding S7/L12c）。
- **middleware のスコープ**: layer は `/mcp` ルーターにのみ適用し、`/hooks/v1/*`（既存 Bearer）や
  teams-backend Unix ソケット（既存 token）には非回帰。
- **`QUEEN_URL` 形式変更**: セッション env へ注入する URL は
  `http://127.0.0.1:<port>/mcp?token=<token>`（token 込み）。表示用 URL（tooltip・`queen_status.url`）
  は従来どおり token 抜き。
- **既存ユーザーへの影響**: 登録 URL に token が入るため、公開済みの旧手順で登録済みのユーザーは
  **再登録が必要**。さらに token は再起動ごとに変わるため、**アプリを再起動したら都度再登録**する。

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
/** Phase 4.4.2/4.4.3: running PTY session の foreground プロセス名（解決できた
 * session のみ）。detail は表示用の補足で、現状は foreground が `ssh` のときの
 * 接続先（argv の最初の非オプション引数。`user@host` / ssh_config alias /
 * `ssh://` authority。`-l user` は宛先に `@` が無いとき `user@dest` に畳み込む）。
 * 無い場合は field ごと省略。 */
type SessionForeground = { id: number; name: string; detail?: string };
type SessionResourcesPayload = {
  sampledAtMs: number;
  sessions: SessionResourceUsage[];
  foreground?: SessionForeground[];
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
- foreground名は同じtickに同乗して解決する（追加pollingなし。Phase 4.4.2）。detail（接続先）は
  foreground名が許可リスト（`ssh` `sftp` `scp` `mosh` `mosh-client` `telnet` `kubectl` `docker`）に
  一致したときだけ argv を追加取得して抽出する（kubectl/docker は exec 系 subcommand のときのみ
  `"<subcommand> <target>"`、kubectl は namespace を `ns/target` に畳み込み）
  （Linux: `/proc/<pid>/cmdline`、macOS: `ps -o command=`。Phase 4.4.3、additive・後方互換）。

## Frontend

- 各running paneのheaderに`CPU n.n% · n MiB/GiB`を表示する。
- tooltipに集約process countとbyte数を表示する。
- toolbar右側に、最新batch内の全sessionを合算した
  `Σ CPU n.n% · n MiB/GiB`を表示する。追加samplingは行わない。
- batch eventごとにresource mapを1回だけ置換し、exit/restart/close時は対象値を削除する。
- `foreground[].name` は `ui.sessions[id].foreground` を毎tick更新し、`detail` はサイドバーの行名と
  paneヘッダーで `ssh user@host` のように name の後ろへ表示する（定義名がある session は定義名優先で
  detail は付けない）。detail が来ないtickで保持値を消し、exit/close時も削除する（Phase 4.4.3）。

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
  teammate_binaries: [claude]  # default ["claude"]。手打ち起動を暗黙 observe lead 扱いにする
                               # フォアグラウンドプロセス名の許可リスト（空リストは既定へ縮退）
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

- **lead 候補（明示）** = 実行中（running）の PTY セッションのうち、mterm.yml 定義に `teams.enabled: true` を持つもの。
- **lead 候補（暗黙 / フォアグラウンド）** = 明示 lead が **1つも無い**ときのフォールバック。running な PTY セッションの
  フォアグラウンドプロセス名が teammate 対象バイナリ（`teammates.teammate_binaries`、既定 `["claude"]`）に一致するものを
  **observe 専用**の暗黙 lead として候補に含める。これにより shell ペインで **手打ち起動した `claude`**（`spec.name = None`）でも
  observe が動く。暗黙 lead は agent 定義に紐づかないため設定は observe 既定（`max_panes=3`、`transcript_tail=true`、`is_host=false`）で、
  グローバル `teammates.enabled: true` が前提（`false`／未設定なら候補化しない）。**明示 lead が存在すれば常にそれを優先し、暗黙 lead は使わない**。
  host 経路には波及しない（host は明示 opt-in の named lead のみ）。
- hook payload の `cwd` を正規化パス（canonicalize、失敗時は入力パス）で lead 候補の cwd と比較し、
  **一致する候補**へ帰属（複数一致時は最小 id）。
- cwd 一致が無い場合、lead 候補が **ちょうど1つ**ならそれへ帰属。
- それも無ければ **ペインを作らずログ（stderr）のみ**。加えて `teammates.enabled: true` のときは
  `teammate-banner` で「サブエージェントを検知したが teams 有効な lead が見つからない」旨をトースト通知する
  （▶ チップからの起動 or `teammates.enabled` 確認を促す。`teammates.enabled: false` の間は黙る）。

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
| `teammate-banner` | `{ message: string }` | ペイン上限超過時、および lead 未マッチ時（`teammates.enabled: true` のみ）のバナー（frontend は `ui.errorBanner` に表示） |

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

# Phase 4.2 追加契約（host モード: 実 PTY teammate ペイン）

Phase 4.2 は方式A（tmux シム）を **既定オフの opt-in 実験機能**として実装する。`agents[].teams.mode:
host` を指定した lead を、ptygrid 同梱の tmux 互換シム + per-lead Unix socket サーバとともに起動し、
Claude Code の split-pane teammate（独立 `claude` プロセス）を **ネイティブな対話 PTY ペイン**として
ホストする。Phase 0〜4.1 は互換維持（既存 PTY 経路・Queen MCP 契約・observe transcript に非回帰）。
socket RPC プロトコル本体は `src-tauri/teams-backend/`（「Phase 4.x 準備契約」）。

## 有効化条件（completion gate）

- `agents[].teams.enabled: true` **かつ** `mode: host` の lead でのみ有効（`AgentTeamsConfig::is_host()`）。
- opt-in なしでは **env 注入も socket サーバ起動もシム配置も一切行わない**。global `teammates.enabled`
  には依存しない（host は per-agent opt-in）。
- host は unix のみ（Windows は非対応: 何もせず通常セッションとして起動）。

## ptygrid.yml 拡張（`agents[].teams` の host 用 field、すべて任意）

```yaml
agents:
  - name: claude
    cmd: claude
    teams:
      enabled: true
      mode: host                 # observe | host。host で 4.2 の実 PTY ホスト
      max_panes: 3               # この lead の teammate ペイン上限、1..9 clamp
      teammate_binaries:         # split-window で PTY 起動を許可する argv0 basename。既定 ["claude"]
        - claude
      fallback_to_observe: true  # host 未使用時に observe へ自動降格。既定 true
```

```ts
type AgentTeamsConfig = {
  enabled?: boolean; mode?: "observe" | "host"; maxPanes?: number; transcriptTail?: boolean;
  teammateBinaries?: string[];   // host only, default ["claude"]（空配列は既定へ collapse）
  fallbackToObserve?: boolean;   // host only, default true
};
```

- 未知 field は無視。`teammate_binaries` が空/未指定なら `["claude"]`。

## lead PTY への env 注入（host lead spawn 時のみ）

| env | 値 |
|---|---|
| `PTYGRID_TEAMS_SOCK` | `app-data/teams/run/lead-<sessionId>.sock` |
| `PTYGRID_TEAMS_TOKEN` | per-lead ランダム 256bit（lowercase hex、非永続） |
| `TMUX` | `<sock>,<ptygrid pid>,0` |
| `TMUX_PANE` | `%0`（lead 自身） |
| `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` | `1` |
| `PATH` | 先頭に `app-data/teams/bin` を付加（シム `tmux` を優先） |

## シムのアプリ内蔵（cmux 方式・シムバイナリは同梱しない）

- host lead 起動時、`app-data/teams/bin/tmux` に実行スクリプトを冪等生成（0755）:
  `#!/bin/sh\nexec "<ptygrid 実行ファイル絶対パス>" __tmux-compat "$@"\n`（`std::env::current_exe()`）。
- ptygrid は `argv[1] == "__tmux-compat"` で起動されると、残り引数を tmux サブコマンドとして
  `teams_backend::shim`（parse/execute）+ 内蔵 blocking `SocketClient` で処理し、**GUI を初期化せず即 exit**。
  NoOp / `list-sessions` は socket 不要。他は `PTYGRID_TEAMS_SOCK`/`TOKEN` で socket に接続。

## socket サーバのライフサイクル

- host lead ごとに `teams_backend::server::{bind_socket, serve}`（`ServerConfig { auth_token }`）を
  spawn（socket 0600 / 親 dir 0700）。`initialize` の `auth_token` 検証必須。
- lead の pty-exit（セッションが Running/Restarting/Starting を外れる）を監視タスクが検知したら
  サーバ task を abort し socket file を削除。
- restart_session（同一 id 再起動）ではサーバを張り直さず既存 socket/env を再利用する。

## PaneHost セマンティクス（socket RPC → 実 PTY）

- **context_id**: lead = `%0`、teammate = `%<sessionId>`（`%0` は書込/capture 時 lead に解決）。
- `spawn_agent`: argv0 basename を `teammate_binaries` で検証（違反は `SPAWN_DENIED`）。lead の解決済み
  cwd（worktree 含む）を既定 cwd に、lead env + RPC env をマージして **既存 PTY spawn 基盤**で teammate
  セッションを生成（`SessionKind::Pty` + teammate メタ `{ leadId, mode: "host", role }`。role は
  `metadata.name`→`role`）。
- **上限（host）**: `teams.max_panes` / `teammates.global_max_panes` / グリッド 9 のいずれか到達時も
  **セッションは生成する**（作業を止めない）。グリッドには載せず paneless とし `teammate-banner` を emit
  （spec 6.2.5）。
- `write`(base64 decode 済みバイト) → 既存 stdin 書込。`capture`(lines?) → `read_output` と同じ ANSI
  再構成テキストの末尾 N 行。`kill` → autorestart を発火させない kill。`focus` → `teammate-focus { id }`。
- `list` / `get_self_id`: lead=`%0`、teammate=`%<sessionId>`。
- teammate セッション終了は `context_exited { context_id, exit_code }`（push）で socket へ broadcast。

## フォールバック検知（spec 6.3・2 秒窓）

- host lead で `SubagentStart` により teammate を検知してから **2 秒**以内に、その lead の socket へ
  `spawn_agent` RPC（= `split-window`）が来なければ「シム未使用（in-process フォールバック）」と判定。
  相関は時間窓ベースの純関数（`teams_host::correlate_fallback`）。
- host lead では `SubagentStart` で **即座に transcript ペインを作らず**、この 2 秒窓を待つ。
- 判定が fallback のとき: `fallback_to_observe: true` なら 4.1 の observe（read-only transcript ペイン）を
  生成、`teammate-fallback { leadId, agentId, reason }` を emit、lead を「fallback 中」状態にする。

## 追加 Tauri Event / Command

| event | payload | 説明 |
|---|---|---|
| `teammate-focus` | `{ id: number }` | tmux `select-pane` 相当。該当ペインを強調（frontend は次段） |
| `teammate-fallback` | `{ leadId: number, agentId: string, reason: string }` | host 未使用で observe 降格したとき |
| `teammate-banner` | `{ message: string }` | host 上限超過時にも emit（4.1 と同一経路） |

| command | args | returns | 説明 |
|---|---|---|---|
| `teams_host_status` | なし | `{ leads: [{ id, mode, fallback, teammates: number[] }] }` | 稼働中 host lead と live teammate 一覧 |

## `SessionInfo.teammate`（additive・host teammate にも付与）

```ts
type SessionInfo = {
  /* 既存 */ kind?: "pty" | "transcript";
  teammate?: { role?: string; leadId: number; mode: "observe" | "host" };
};
```

- host teammate は `kind: "pty"` + `teammate.mode: "host"`。observe transcript は
  `kind: "transcript"` + `teammate.mode: "observe"`。

## ProjectState 除外

- teammate セッション（observe transcript と host teammate PTY の双方）は ephemeral であり
  `ProjectState.sessions` に **保存しない**。除外は kind ではなく **teammate メタ**で掛かる
  （`project_state::persistable_pane_ids` は `(id, is_teammate)` で不変条件を保持・単体テスト可能）。
  frontend は保存前に `SessionInfo.teammate` を持つペインを除外する。

## 許可リスト spawn との関係（spec 7.1・3 段）

teammate spawn は Queen MCP `spawn_agent`（config 定義名の allowlist）を経由せず、専用チャネルを通る:
(1) `teams.mode: host` の config opt-in、(2) `PTYGRID_TEAMS_TOKEN` の socket ハンドシェイク、
(3) `teammate_binaries`（既定 `["claude"]`）による argv0 basename 検証。Queen `spawn_agent` の
allowlist セマンティクスは不変。

## Frontend（Phase 4.2 で実装した UI 挙動）

backend 契約（上記イベント/コマンド/型）は不変。frontend は以下の挙動でこれらを消費する:

- **host teammate ペイン**（`kind: "pty"` + `teammate.mode: "host"`）は通常 PTY ペインとして
  xterm でホストし対話可能。ヘッダーは `claude·team #<id> ▸<role>` + lead 併記 `↳#<leadId>`
  （role/lead は欠落時省略）、状態ドットは既存 PTY の running/exited を流用。⟳restart / ⤢maximize 可。
- **close は確認付き**（実プロセス kill の破壊的操作）。インライン確認「teammate を停止しますか？」を
  挟み、確定時のみ `kill_pty`。observe transcript の close（tail 停止のみ）とは別扱い。
- `teammate-focus { id }` 受信で該当ペインを約 1.6 秒ハイライト（枠のアクセントリング）。
- `teammate-fallback` 受信で toast 通知 + `teams_host_status` を再取得。lead が `fallback: true` の間、
  Teammates バッジは赤ドット + 「host: フォールバック中」を表示する。
- **Teammates パネル**は開時に `teams_host_status` を取得し、host lead ごとに mode / fallback /
  teammate 一覧を表示。paneless teammate（`teammate` を持つが `ui.panes` に無い session）には
  「グリッドへ表示」ボタン（9 面上限内で pane 追加）。lead が host 一覧から消え（終了）かつ teammate
  PTY が生存している場合は「lead 終了済み（孤立 teammate）」として列挙し「停止」ボタン（`kill_pty`）を出す。
- **ProjectState 除外**: 保存対象ペインは `SessionInfo.teammate` を持つ session（observe transcript /
  host teammate の双方）を除外する（kind ではなく teammate メタで判定。CONTRACT の不変条件と一致）。

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

---

# app settings と projects root（cd補助）

作業フォルダ入力欄のサジェスト向けに、プロジェクトの**置き場所（projects root）**をアプリ全体の
設定として永続化する。project stateと違いprojectには紐付かない、app-global設定。

## ファイル位置とスキーマ

- 保存先はTauri app-data直下の`app-settings.json`（`project-state/`とは別階層）。
  repository内には書かない。
- `version: 1`必須。未知versionは黙って開かず、明確なエラーを返す（`project_state`と
  同じ流儀）。書き込みはtemp file + renameのatomic。

```jsonc
{ "version": 1, "projectsRoot": "~/works/project" }
```

- `projectsRoot`は**ユーザーが入力したままの文字列**（先頭`~`を含む）で保存する。
  移植性のため`~`はファイル内で展開しない。未設定なら当該キーは省略され、
  `get_projects_root`は`null`を返す。
- `~` / `~/...` の展開は**実際にpathへ触れる時だけ**（検証・一覧取得）ホームディレクトリへ
  展開する。`~name`（named home）は特別扱いせずそのまま扱う。

## Tauri Commands

| command | args | returns | 説明 |
|---|---|---|---|
| `get_projects_root` | `()` | `{ root: string \| null }` | 保存済みルート（verbatim）。未設定は`null` |
| `set_projects_root` | `{ root: string }` | `{ root: string \| null }` | `~`展開後に「存在するディレクトリ」であることを検証してから保存し、保存値を返す |
| `list_project_dirs` | `()` | `{ root, dirs: string[], truncated: boolean }` | 保存済みルート直下の**非隠しディレクトリ名**をソートして返す |

## 検証とエラー

- `set_projects_root`: trim後に空文字は拒否。`~`展開後のpathが存在しない／
  ディレクトリでない場合は明確なエラー文字列（例: `... is not a directory`）を返し、
  保存しない。
- `list_project_dirs`: ルート未設定は`projects root is not set`、保存済みルートが
  消失／アクセス不可なら明確なエラー。

## 一覧の上限とフィルタ

- ルート直下のみ（再帰しない）。**dotで始まる名前**（隠しディレクトリ・ファイル）と
  **通常ファイル**は除外し、ディレクトリ名だけをOSソートで返す。symlinkは実体が
  ディレクトリなら含める。
- 返す件数の上限は**200件**。超過分は切り捨て、`truncated: true`を立てる。

## Frontend（作業フォルダ入力のサジェスト）

独立した「cd…」ボタン／ポップオーバーは撤去した（「読み込み」が cd 相当の動作を担うため）。
projects root は UI で手動編集せず、下記のとおり自動で保存・利用する。コマンド3つの契約は不変。

- **自動保存**: 手動の「読み込み」成功時のみ、読み込んだ作業フォルダ（`ConfigInfo.dir`）の
  **親ディレクトリ**を`set_projects_root`で保存する（best-effort・失敗は無視・トーストなし）。
  親が`/`またはホームディレクトリそのものの場合は、置き場所として広すぎるため保存しない。
  保存形式はbackendの既存挙動どおり（存在するディレクトリかの検証はbackend側）。
- **サジェスト**: ルート設定済みなら、アプリ起動時と作業フォルダ入力欄フォーカス時に
  `list_project_dirs`を呼び、`<root>/<name>`の一覧を`<datalist>`候補として提示する。
  rootはverbatim（先頭`~`を保持）のまま連結する。ルート未設定・取得失敗時は候補なし
  （エラー表示もしない）。

---

# 作業フォルダと設定ファイル置き場所の分離

`load_config`の`dir`引数の意味を**作業フォルダ（working folder / プロジェクト境界）**に
変更し、設定ファイル`ptygrid.yml`の探索場所を作業フォルダから分離する。ツールバーの
「作業フォルダ」欄には`~/works/hoge`のような作業フォルダのパスを入れ、設定ファイルは
グローバル（`~/.ptygrid/`）やアプリ起動フォルダにも置けるようにする。wire formatは不変。

## `load_config` の意味変更

- `dir` = **作業フォルダ**。先頭`~` / `~/`はホームディレクトリへ展開してから使う
  （`~name`は特別扱いしない）。展開後のパスが存在しない／ディレクトリでない場合は明確な
  エラー（`working folder <path> is not a directory` 等）。
- `dir`省略時の既定は従来どおり（前回の作業フォルダ→初回はカレントディレクトリ）。
- 作業フォルダは設定ファイルを含まなくてよい（設定は別途下記の順で探索する）。

## 設定ファイルの探索順（新 resolve ロジック）

1. `<作業フォルダ>/ptygrid.yml`、無ければ `<作業フォルダ>/mterm.yml`
   （**legacy互換は作業フォルダ内のみ**）→ `origin: "project"`
2. `<アプリ起動フォルダ>/ptygrid.yml`（legacyフォールバックなし）→ `origin: "launch"`
   - アプリ起動フォルダ = プロセス起動時のカレントディレクトリ。**起動シーケンスの最初**
     （`fix_path_env::fix()`より前）に`capture_launch_dir`で一度だけ捕捉し、`OnceLock`で固定。
   - 起動フォルダが作業フォルダと同一パスの場合は重複試行しない。
3. `~/.ptygrid/ptygrid.yml`（グローバル設定）→ `origin: "global"`
- どこにも無い場合の扱いは`allow_default`引数で分岐する（下記「設定なしフォールバック」）。
- file watcherは**実際に読み込んだファイルの親ディレクトリ**を監視する（既存挙動の一般化）。
  グローバル設定を読んだ場合は`~/.ptygrid`を、起動フォルダ設定を読んだ場合は起動フォルダを
  監視することになる。`config-changed` event・Reload挙動は不変。

## 設定なしフォールバック（`allow_default`）

`load_config`はoptionalな`allow_default: bool`引数を取る（省略時 `false`。JS側は camelCase
`allowDefault`）。

- `allow_default: true`（**手動読み込み**）で3か所すべてに設定が無い場合、エラーにせず
  **組み込みの既定設定**（`Config::default()` = `project: None` / `agents: []` /
  `processes: []` / `queen: None`（＝既定でenabled・既定ポート）/ `teammates: None`）で成功し、
  `origin: "default"`を返す。
  - この場合の`path`は空文字ではなく、**読み込む予定だった第一候補 `<作業フォルダ>/ptygrid.yml`**
    （実在しないパス）。後からユーザーがそこに yml を作成した際、watcherが検出できるようにするため。
  - watcherは`<作業フォルダ>`を監視し、`<作業フォルダ>/ptygrid.yml`の出現/変更で`config-changed`を
    emitする（既存のReloadトーストで反映）。legacy `mterm.yml`の出現までは追わない。
- `allow_default: false`（**起動時の自動load**）で3か所すべてに設定が無い場合は従来どおり、
  試した全candidateを列挙した`not_found:`エラーを返す（frontendの起動時fallback＝adhocシェル1枚を
  開く挙動を壊さない）。実在ファイルのparse/readエラーは`allow_default`に関係なく常にエラー。

## プロジェクト境界のセマンティクス（重要）

- cwd解決の基準（`resolve_cwd`）、Queenのproject scope（pins/notes/inbox）、Gitパネル、
  `project_state`の`dir`は、**設定ファイルの場所に関係なく常に作業フォルダ**を使う。
  `ConfigManager::current()`が返す`dir`は作業フォルダであり、②/③から設定を読んでも
  設定ファイル側のディレクトリにはならない。

## `ConfigInfo` 拡張（additive）

```ts
type ConfigOrigin = "project" | "launch" | "global" | "default";
type ConfigInfo = { path: string; dir: string; origin: ConfigOrigin; config: Config };
```

- `path` = 実際に読み込んだ設定ファイル、`dir` = 作業フォルダ、`origin` = `path`の由来。
  既存の`path` / `dir`フィールドは意味を明確化しただけで型は不変、`origin`に`"default"`のみ追加。
  `origin: "default"`のときは設定ファイルが実在せず、`path`は上記のとおり第一候補パス。

## Frontend

- ツールバーの「プロジェクト」欄を「作業フォルダ」に変更。placeholderは
  `作業フォルダ（例: ~/works/hoge。先頭 ~ 可）`、tooltipで探索順を説明。
- 読み込み成功後、読み込みボタン付近に**origin バッジ**（`設定: プロジェクト内 / 起動フォルダ /
  ~/.ptygrid / 既定`）を表示。hoverで実pathと作業フォルダ、`既定`のときは「設定ファイルなし。
  `<作業フォルダ>/ptygrid.yml`を作成すると自動で読み込みます」を表示。
- **手動読み込み成功時は `cd` と同じ動き**: 開いているシェルのペイン（既定対象＝
  `selectCdTargets(sessions, false)`、CLI実行中ペインは除外）へ `cd '<作業フォルダ>'` + Enter を
  送信する。対象0件でもエラーにしない。成功トーストは設定元と cd 件数を統合して1つ
  （例:「作業フォルダ: ~/works/notemake（設定: 既定） / 2ペインに cd を送信」）。
  **起動時の自動loadでは cd を送信しない**（`allow_default`も付けない）。
- 既存のReload / config-changed挙動は不変。

---

# Security 追加契約 (config trust 境界 / CSP)

両エージェントはこの契約に**厳密に**従うこと。既存のコマンド/イベント/`ConfigInfo`は
互換維持（本節はいずれも additive）。背景は
[docs/inside/security-review-2026-07-16.md](docs/inside/security-review-2026-07-16.md)
Finding 2（S2）/ Finding 4（S4）。

## config trust 境界（S2）

repo 同梱の `ptygrid.yml`（`origin: "project" / "launch"`）は `cmd` / `resume` /
`worktree.setup` を定義でき、これらはコマンド自動実行に繋がる（`${VAR}` 展開で host
環境変数の持ち出しも可能）。悪意あるリポジトリを `cd` して読み込んだだけで発火するのを
防ぐため、**自動コマンド実行を「信頼済みフォルダ」ゲートの配下**に置く。

### 信頼状態の永続化

- 信頼済み作業フォルダの**正規化済み絶対パス集合**を Tauri app-data 直下の
  `trusted-folders.json` に保存する（`app-settings.json` と同階層・同流儀）。

  ```json
  { "version": 1, "folders": ["/home/user/works/project"] }
  ```

- `app_settings` と同じ on-disk 規律: version 付き JSON、未知の version は開かず拒否、
  書き込みは atomic（temp + rename）。
- パスは保存・比較の前に `canonicalize`（`~` 展開＋symlink/`.`/`..` 解決）する。
  symlink 経由でのゲート回避を防ぐため。

### origin ごとの信頼扱い

- `origin: "global"`（`~/.ptygrid`）と `origin: "default"`（組み込み既定）は
  **常に信頼済み**扱い（ユーザー自身のグローバル設定）。
- `origin: "project" / "launch"` は、その作業フォルダが信頼集合に**在るときのみ**信頼済み。

### `ConfigInfo.trusted`（additive）

```ts
type ConfigInfo = {
  path: string; dir: string; origin: ConfigOrigin;
  trusted: boolean; config: Config;
};
```

- `trusted` = この config を**自動コマンド実行に使ってよいか**。上記 origin ルールで決まる。
- **`load_config` 自体は `trusted` の値に関わらず成功する**（設定の閲覧・ペイン表示・
  エージェントチップからの手動起動は常に可能）。`trusted` は frontend のゲート判定用。

### 新コマンド

| command | args | returns | 説明 |
|---|---|---|---|
| `trust_working_folder` | `{ dir: string }` | `{ trusted: boolean }` | `dir`（`~`展開＋canonicalize）を信頼集合へ追加（冪等）。常に `{ trusted: true }` |
| `is_working_folder_trusted` | `{ dir: string }` | `{ trusted: boolean }` | 集合内メンバシップ（フォルダ単位、origin 非依存）を返す |

### ゲート挙動（frontend）

- 起動時の autostart ループは **`ConfigInfo.trusted === true` のときのみ**実行する。
  `false` かつ autostart 対象がある場合は、**確認バナー**
  （「このフォルダ（&lt;dir&gt;）の設定は未確認です。定義されたコマンドを自動起動しますか？」）
  を出し、autostart は保留する。
- ユーザーが「信頼して起動」を押すと `trust_working_folder` を呼び、以後そのフォルダは
  信頼済み。押した後に保留していた autostart を実行する。
- **手動起動（エージェント/プロセスチップの ▶）はゲート対象外**（ユーザーの明示操作）。
  ゲートするのは autostart と worktree.setup を伴う**自動**実行のみ。未信頼フォルダで
  worktree.setup を伴う spawn を手動で行う場合の扱いは将来検討（最小方針: 未信頼では
  autostart を止める、が主目的）。

## Content Security Policy（S4）

`tauri.conf.json` の `app.security.csp` を `null` から明示値へ:

```
default-src 'self'; script-src 'self'; connect-src 'self' ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; img-src 'self' data:
```

- `script-src 'self'`: Vite ビルド成果物のみ（inline script なし、`{@html}` 不使用）。
- `connect-src`: Tauri IPC（`ipc:` / `http://ipc.localhost`）。
- `style-src 'unsafe-inline'`: Svelte / xterm.js / svelte-splitpanes のスタイル注入に必要。
- `img-src 'self' data:`: `data:` 画像許可（ビルド後の CSS に `data:` フォントは無し）。
- 多層防御目的（現状ライブな XSS シンクは未検出）。webview 実機確認は別途。

---

# Security 追加契約 (認証トークンの永続化)

両エージェントはこの契約に**厳密に**従うこと。既存の `/mcp` 認証・`/hooks/v1/*` Bearer・
`QUEEN_URL` 形式（`?token=`）・teams-backend は互換維持（本節は「トークンの出所」を
非永続→永続へ替えるだけで、生成方式・検証ロジック・URL 形式は不変）。背景は
[docs/inside/evaluation-2026-07-16.md](docs/inside/evaluation-2026-07-16.md) の
「hook token 固定化」。

## 動機

Phase 2「Queen 認証」と Phase 4.0「token（非永続）」は、`/mcp` トークンと teammate hooks
Bearer トークンを**起動ごとに再生成**し、ディスクに書かなかった。その結果、アプリを再起動
（や再ビルド）するたびに `~/.claude/settings.json` の登録済み hook トークンと MCP の
`QUEEN_URL` トークンがずれ、**再登録するまで observe ペインが出ず Queen も 401** になっていた。
これを解消するため、両トークンを app-data に**永続化**し、起動ごとに変わらないようにする。

## トークンストア

- 両トークンを Tauri app-data 直下の `auth-tokens.json` に version 付き JSON で保存する
  （`app-settings.json` / `trusted-folders.json` と同階層・同流儀）。

  ```json
  { "version": 1, "hookToken": "<64-hex>", "queenToken": "<64-hex>" }
  ```

- `app_settings` / `trust` と同じ on-disk 規律: version 付き JSON、未知の version は
  開かず拒否、書き込みは atomic（temp + rename）。
- **ファイル権限は Unix で `0600`**（所有者読み書きのみ）。秘密値のため、rename 前の
  temp ファイルに設定してから公開する（一瞬でも world-readable にしない）。
- 生成方式は従来どおり 256bit ランダム・lowercase hex・`getrandom`（OS CSPRNG）。生成関数と
  定数時間比較（`teams_hooks::{generate_token, constant_time_eq}`）は不変で共有。

## 起動時ロード / 生成

- アプリ setup（Queen サーバ bind の**前**）で `token_store::load_or_create` を一度呼び、
  ストアに保存済みトークンがあればロード、無ければ両トークンを生成して保存する。
- ロードした値で `QueenStatus` / `TeamsHooks` を構築する。両トークンは起動時に一度だけ確定し、
  以降 env 注入（`QUEEN_URL` / hook Bearer）・middleware 検証・snippet 生成が同じ値を使う。
- 実行中トークンは共有ハンドル（`TokenHandle` = `Arc<Mutex<String>>`）で保持し、bind 済みの
  `/mcp` middleware と `/hooks/v1/*` receiver はそのハンドルを**キャプチャ**する
  （スナップショットしない）。これにより再生成が**再 bind なし**で反映される。

## 再生成コマンド（ローテーション）

| command | args | returns | 説明 |
|---|---|---|---|
| `regenerate_auth_tokens` | `{ which?: "hook" \| "queen" \| "all" }` | `{ regenerated: string[] }` | 対象トークンを再生成 + ストア保存 + 実行中 state へ反映 |

- `which` 省略/空/`"all"` は両方、`"hook"` / `"queen"` は片方。未知値はエラー。
- 新トークンを生成してストアに保存し、`QueenStatus` / `TeamsHooks` の `TokenHandle` を更新する。
  Queen は再 bind 不要（middleware が同ハンドルを見る）。hooks も同様。
- 戻り値 `regenerated` は再生成したトークンのラベル（`"hook"` / `"queen"`）。
- **再生成後は settings.json / MCP 登録がずれる**ため、frontend は再登録を促す通知を出す。

## Frontend

- Teammates パネルに「hook トークン再生成」「Queen トークン再生成」ボタンを追加
  （`regenerate_auth_tokens`）。再生成後は `teammate_hooks_info` / `queen_status` を再取得し、
  再登録を促すトースト（hook→settings.json 登録 / Queen→登録コマンドコピー）を出す。
  Queen バッジは単体ボタン（クリックで登録コマンドコピー）で専用パネルを持たないため、
  Queen トークンの再生成ボタンも Teammates パネルに集約する。
- 既存の「settings.json へ登録」「Queen 登録 URL コピー」は永続トークンを使う（値がストア由来に
  なるだけで、呼び出し側は不変）。
- ツールチップ/注記の文言を「初回のみ登録が必要。トークンは保存され再起動後も有効
  （再生成したときだけ再登録）」に更新する。

## セキュリティのトレードオフ

- 永続化により、**app-data を読める同一ユーザープロセス**がトークンを取得可能になる
  （従来は子プロセスの environ 経由でのみ露出）。app-data は `0600` でユーザー専用のため、
  対象脅威モデル（同一ユーザーローカル）では**実質同等**。漏洩時は `regenerate_auth_tokens` で
  ローテーション可能。
- DNS リバインディング / cross-origin / env に触れないプロセスへの防御（Phase 2「Queen 認証」の
  Host/Origin allow-list・token 検証）は**不変**。bind は 127.0.0.1 のみ、検証は定数時間のまま。

---

# Phase 4.4.0 追加契約（agent-status: セマンティック状態検出の基盤 + イベント）

Phase 4.4.0 は、生きている（`running`）PTY セッションの端末出力から**意味的状態**
`working | blocked | done | idle`（+ `unknown`）を推定し、`agent-status` イベントで frontend へ
運ぶ**検出基盤**を実装する。UI バッジ（ヘッダー）は本 Phase の frontend で、ステータスサイドバー
（4.4.1）と blocked 通知（4.4.2）は後続。Phase 0〜4.2 はすべて互換維持（本節は additive）。

## AgentStatus は SessionState とは別レイヤ（重要）

- `AgentStatus`（`working|blocked|done|idle|unknown`）は、既存 `SessionState`
  （`starting|running|exited|restarting`＝**プロセス生死**の事実）とは**別レイヤの推定（意見）**である。
- **意味的状態は `SessionState == running` のセッションにのみ付与する**。`exited`/`restarting`/`starting`
  は対象外。`AgentStatus` は `SessionState` を**上書きしない**（`SessionState` enum / `session-state`
  イベント / `SessionInfo` の既存 field は**一切不変**）。
- `unknown` はルールセット未割当／評価前（UI ではバッジ非表示）。

## ptygrid.yml 拡張（グローバル `agent_status:` ブロック、すべて任意）

```yaml
agent_status:
  enabled: true          # default true。false で検出・イベントを停止
  tail_lines: 24         # default 24、4..=200 に clamp。検出に使う再構成末尾行数
  debounce_ms: 250       # default 250、100..=2000 に clamp。評価デバウンス間隔
  done_linger_ms: 6000   # default 6000、0..=60000 に clamp。done を保持して idle へ減衰（0で done 無効）

  # ルールセット定義。キー = agent 定義名 or フォアグラウンドプロセス名（+ opt-in の "*"）。
  patterns:
    claude:                    # 内蔵既定へ merge（既定）: 内蔵 + ユーザーを連結（順序: 内蔵→ユーザー）
      blocked: ['Do you want to proceed\?']
      working: ['esc to interrupt']
    codex:
      replace: true            # 内蔵既定を破棄し、以下だけを使う
      blocked: ['\[y/N\]']
    "*":                       # opt-in の generic フォールバック（既定では存在しない）
      blocked: ['\[y/N\]\s*$']
```

- **既定値と clamp**: 上記コメントの通り。`enabled` の既定は **true**（省略で検出 on）。
- **merge / replace セマンティクス**:
  - `patterns.<key>` は同名の内蔵既定があれば**既定で merge**（各カテゴリ blocked/working/done ごとに
    **内蔵配列 + ユーザー配列**を連結、重複除去なし、順序は内蔵→ユーザー）。
  - `replace: true` でそのキーの内蔵既定を**破棄**し、ユーザー定義だけを使う。
  - カテゴリ省略時: merge では内蔵配列をそのまま使用／replace では空。
  - 内蔵に無いキーは常に新規追加。`"*"` は 3.1 でルールセット未割当だった running PTY にのみ
    **opt-in で**適用する generic ルール（既定では未定義）。
- **内蔵既定パターン**はバイナリに**コンパイル時同梱**（`src-tauri/src/agent_status_defaults.yml` を
  `include_str!`）。初期同梱キー: `claude` / `codex` / `grok` / `aider`。**これらは各 CLI の UI 変更で
  陳腐化しうる**ため、リリースごとの更新 + ユーザー上書きで保守する。
- **不正な正規表現**はその**1本だけをスキップ**して backend ログに警告を出し、残りは有効化する
  （設定全体を失敗させない／config reload の非破壊性）。
- 未知 field は無視、欠落はデフォルト補完（既存 config ブロックと同流儀）。パターンは既定で
  case-insensitive + multiline の部分一致（`(?-i)` 等のインラインフラグで個別上書き可）。

## 検出方式（backend、hot path 分離）

- 対象は `SessionState == running` の PTY セッション。決定順は **blocked > working > done > idle**、
  どれもマッチしなければ `idle`（ルールセットあり）/ `unknown`（ルールセット無し）。
  **blocked は保守的**（既知の承認/権限/選択 UI にマッチした時だけ。未知プロンプト・シェル復帰・空は
  `idle`）。
- ルールセット選択は評価のたびに遅延解決: ① agent 定義名 → ② フォアグラウンドプロセス名 →
  ③ opt-in `"*"` → ④ 無ければ `unknown`。
- 検出入力は **`output_snapshot` → `ansi::render_terminal`（現在画面再構成）→ 末尾 N 行**。生バイト列には
  掛けない。`done` は `done_linger_ms` 保持後に `idle` へ減衰（working→プロンプト復帰も done 扱い）。
- **hot path（reader thread）には regex/重処理を置かない**。reader は当該セッションを dirty マークする
  だけ。別の**単一デバウンス評価タスク**（既定 250ms）が dirty セッションのみ評価する。regex は起動時／
  config reload 時に一度コンパイルしてキャッシュ（hot path でコンパイルしない）。

## 新イベント `agent-status`（backend → frontend）

| event | payload | 説明 |
|---|---|---|
| `agent-status` | `AgentStatusPayload` | 意味的状態が**変化したときだけ** emit（`agent_status.enabled: true` 時のみ） |

```ts
type AgentStatus = "working" | "blocked" | "done" | "idle" | "unknown";
type AgentStatusPayload = {
  id: number;
  status: AgentStatus;
  matchedRule?: string;   // マッチしたルール id（＝正規表現ソース。tooltip/デバッグ、任意）
  ruleSet?: string;       // 適用したルールセットキー（claude / codex / "*" 等、任意）
};
```

- **emit は状態変化時のみ**（`blocked → blocked` は emit しない）。`agent_status.enabled: false` の間は
  一切 emit しない。
- **`exited` クリア規約**: セッションが running でなくなった時点で backend は tracker から当該
  セッションを破棄する（**専用のクリアイベントは出さない**）。frontend は `session-state: exited`
  で `ui.agentStatus[id]` を削除する。
- 本 Phase では MCP 面（`list_agents` / `read_output` / `SessionInfo.agentStatus?`）へ意味的状態は
  **同梱しない**（7.4 の任意項目・将来）。イベントのみで UI は成立する。

## 非回帰（本節はすべて additive）

- 既存 `session-state` / `SessionState` / `SessionInfo`（全 field）/ `pty-output` / `pty-exit` /
  `read_output` / `list_agents` は**不変**。
- reader hot path の追加は「dirty マーク（atomic + unbounded channel send）」のみ。`agent_status` 未 manage
  の経路（session 単体テスト等）では dirty マークは no-op。

# Phase 4.3 追加契約（Queen team preset: 一括起動）

複数エージェントの名前付きチーム構成を `ptygrid.yml` に宣言し、1操作で一括起動する。
実体は**既存 allowlist spawn 経路の薄いラッパー**であり、新しい信頼境界・新しい
プロトコルを導入しない。設計背景は docs/spec-team-presets.md。

## ptygrid.yml 拡張（トップレベル `team_presets:` ブロック、任意）

```yaml
team_presets:
  daily:                       # preset 名（map キー、一意・非空）
    lead: local                # 任意: kickoff の宛先。省略時は最初の非 standby メンバー
    members:                   # 必須: 1 件以上
      - agent: local           # 必須: agents: の定義名への参照のみ（processes: は不可）
        standby: true          # 任意（default false）: チーム起動時に立ち上げない待機層
        instructions: "..."    # 任意: inbox で配送される役割指示
    kickoff: "..."             # 任意: 起動後に lead の inbox へ投函する初回メッセージ
```

- Rust: `Config.team_presets: Option<BTreeMap<String, TeamPreset>>`。
  `TeamPreset { lead?, members: Vec<TeamMember>, kickoff? }`、
  `TeamMember { agent, standby?, instructions? }`（`effective_standby()` default false）。
- **検証は config ロード時（parse に含む）**。他の 4.x ブロックと異なり、以下は
  `parse_config` を **エラーで失敗**させる（preset は起動対象の宣言であり、壊れた宣言を
  黙って読み込むと allowlist の意味が崩れるため）:
  - `members[].agent` が `agents:` に存在しない（`processes:` の名前は不可）
  - `members` が空 / 非 standby メンバーが 0 件
  - `lead` が members に無い、または standby メンバーを指す
  - 同一 preset 内で同じ `agent` を重複宣言
  - preset 名が空文字列
- 未知キーは従来どおり無視（forward compat）。`team_presets:` ブロック自体の省略は常に有効。

## 一括起動セマンティクス（backend `team_presets::start_team`）

1. 非 standby メンバーを**宣言順に逐次起動**する。spawn は既存の
   `ConfigManager::resolve_def` + `PtyManager::spawn_agent`（= Queen `spawn_agent` と
   同一経路・同一 allowlist）。
2. **冪等 skip**: 同じ定義名の**生存セッション**（`starting` / `running` / `restarting`。
   直前に spawn されたばかりの `starting` も含む）が既にあるメンバーは起動せず
   `skipped`（既存セッション id を報告）。
3. **ペイン上限**: 起動前に現在のセッション総数（kind 不問）が 9 以上なら、その
   メンバー以降は spawn せず `failed`（`error: "pane limit"`）。部分起動を許し、
   全体を事前拒否しない（frontend の 9 面グリッドの近似。上限は今後の contract 変更なしに
   frontend と同期して見直しうる）。
4. **instructions / kickoff の配送は Queen 永続 inbox に統一**（`send_inbox` と同一の
   検証・上限に従う）。CLI 引数への埋め込みは行わない。
   - 宛先 mailbox = **定義名**（inbox の mailbox は論理名であり `#id` は使えない、
     Phase 3.7 契約に従う）。sender mailbox = `queen:preset/<preset名>`。
   - 配送は **この呼び出しで `started` が 1 件以上あるとき（=チームが実際に起動したとき）
     のみ**行う: `started` メンバー各自の `instructions` → standby メンバーの
     `instructions`（durable なので後から起動しても読める）→ 最後に `kickoff` を
     effective lead へ。全 skip の呼び出しは何も配送しない（冪等な no-op）。
   - subject は `preset:<preset名> instructions` / `preset:<preset名> kickoff` 固定。
5. 起動レポート（下記 wire）を戻り値で返す。配送失敗は起動自体を失敗させず、
   レポートの `kickoffDelivered` / member `error` に反映する。

## Wire: `TeamStartReport`（camelCase）

```jsonc
{
  "preset": "daily",
  "lead": "local",                    // effective lead（lead 省略時は先頭非 standby）
  "members": [
    { "agent": "local", "standby": false, "status": "started", "id": 3 },
    { "agent": "opus",  "standby": true,  "status": "standby" },
    { "agent": "grok",  "standby": false, "status": "skipped", "id": 2 },
    { "agent": "gpt",   "standby": false, "status": "failed", "error": "pane limit" }
  ],
  "kickoffDelivered": true            // kickoff あり かつ started>0 かつ送信成功
}
```

`status`: `started` | `skipped`（既 running、id=既存）| `failed`（error 必須）|
`standby`（起動対象外）。

## Tauri Command（additive）

- `spawn_team(preset: string, cols: number, rows: number) -> TeamStartReport`
  - Queen tool と**同一の backend 関数**を呼ぶ。エラー（preset 不明・config 未ロード）は
    文字列 reject。

## Queen MCP tool（additive・19 本目）

- `spawn_team { preset: string }` → `TeamStartReport` の JSON。
  spawn サイズは既存 Queen spawn と同じ 120x30。preset 不明は invalid_params。

## Frontend

- ツールバー: `team_presets` があるとき preset チップ（👥）を表示。クリックで
  `spawn_team` を呼び、レポートをバナー表示（started/skipped/failed の要約）。
  standby メンバーの個別起動は既存のエージェントチップ（▶）をそのまま使う。
- `started` セッションのペイン追加は既存の「Queen spawn の未知セッション自動ペイン化」
  と同じ経路（session-state イベント）に乗る。frontend 側の新規イベントは無い。

## 非回帰（本節はすべて additive）

- `team_presets:` の無い config のパース・挙動は不変。`spawn_agent`（command / Queen tool）
  ・inbox 系 tool・`SessionInfo` は不変。
- 検証エラーは `team_presets:` を書いた config にのみ発生しうる。

---

# Phase 5.0 追加契約（Orchestrated & Remembering）

> 状態: **5.0.0 (MVO) 実装済み（2026-07-22）** / 5.0.1 以降は設計のみ。
> 本仕様は [docs/spec-phase5-0.md](docs/spec-phase5-0.md) を参照。
> 5.0.0 実装範囲: `workflows:` スキーマ + parse 検証（循環検出・allowlist）、
> pipeline / fan-out の spawn と DAG 進行ドライバ（300ms ポーリング、完了判定は
> PTY exit(0) / AgentStatus==done の2経路、fail-fast で下流 Skipped）、
> Queen MCP tools 3本（19→22: `spawn_workflow` / `join_workflow` /
> `cancel_workflow`。join は 200ms ポーリング・timeout 既定 600000ms・
> clamp 1000..=3600000・`{"timedOut": bool, "run": {...}}` 返却）、
> Tauri commands 3本（`spawn_workflow` / `cancel_workflow` /
> `list_workflow_runs`）+ `workflow-state` イベント（payload = WorkflowRun,
> camelCase）、WorkflowPanel + 🔀 チップ UI。
> fan-out は copies>=2 のとき常に新規 spawn（冪等 reuse は copies==1 のみ）。
> 追補（同日）: 終了ペイン自動クローズ — `agents[].close_on_exit` /
> `workflows.<name>.autoClose`（ともに `success | always | never`、既定 never）。
> workflow 由来セッションは autoClose が優先。success は exit 0 のみ、
> maximized 中は閉じない、3 秒遅延 + 発火時再判定。判定・実行は frontend、
> Rust 側はパースのみ。
> supervisor / handoff / retry / timeout 実行・join_on: reply|N は 5.0.4 送り
> （パースは通り、spawn 時に typed error）。
> 対象 patch: 5.0.0 MVO / 5.0.1 Memory FTS5 / 5.0.2 Memory embedding / 5.0.3 Provider / 5.0.4 Orchestrator supervisor+handoff / 5.0.5 Arena view。
> SQLite: 5.0.1（2026-07-23 実装済み）で `workflow_runs` テーブル + **user_version 2 → 3**。
> `WorkflowRegistry::put` から write-through 永続化。`load_config` 成功時に state='running'
> の残存 run を検出し `workflow-resume-pending`（WorkflowRun[]）を emit、frontend の
> Y/N バナーから `resume_workflow`（running step→pending に戻し既存ドライバが続行）/
> `abandon_workflow`（DB 上 cancelled 化・再プロンプト防止）。memory 系テーブルは 5.0.2+。

## 5.0.1 ptygrid.yml スキーマ追加（予約）

- `workflows:` ブロック — pipeline / fan-out / supervisor / handoff の 4 パターン、`steps[].agent` は既存 `agents:` allowlist 参照のみ。
- `providers:` ブロック — `local:*` / `cloud:*` 宣言、`embedding:` サブブロック（provider + dimension）。
- `agents[].provider` フィールド — env 自動注入のトリガ。
- `queen.memory:` サブブロック — `enabled` / `encrypted` / `max_bytes` / `ttl_default_ms`。

## 5.0.2 新 Tauri Command / Event（予約）

- Commands: `spawn_workflow` / `cancel_workflow` / `list_workflow_runs` / `memory_status` / `provider_status` / `arena_open`。
- Events: `workflow-state` / `provider-status` / `arena-open` / `memory-changed`。

## 5.0.3 新 Queen MCP tools（予約）

- Orchestrator: `spawn_workflow` / `join_workflow` / `cancel_workflow`（**5.0.0 実装済み**）
- Memory: `memory.remember` / `memory.recall` / `memory.forget` / `memory.list` / `memory.reindex`
- Arena: `arena.vote` / `arena.list_votes`
- Provider: `provider.status`

計 **11 tools 追加**。既存の 18〜19 tools は不変。

## 5.0.4 非回帰

- 既存 `session-state` / `agent-status` / `queen-notify` / `session-resources` は不変。
- 既存 `spawn_agent` / `list_agents` / `send_message` / `read_output` / `notify` / Pins / Notes / Inbox / `await` / `spawn_team` は引数・返り値ともに不変。
- `queen.sqlite3` v2 → v3 migration は transactional、v2 のままの起動時に `memory*` テーブル + provider ident pragma を追加。

---

# Phase 5.5 追加契約（Observable & Standards-Compliant）

> 状態: 未実装（設計のみ）。実装時に本節へ具体的な wire 契約を書き足す。
> 本仕様は [docs/spec-phase5-5.md](docs/spec-phase5-5.md) を参照。
> 対象 patch: 5.5.0 MCP RC 両立ルータ / 5.5.1 OTel + SQLite / 5.5.2 Cost + agent-cost / 5.5.3 Status Rings / 5.5.4 Waterfall + Dashboard。

## 5.5.1 Queen（MCP）契約拡張

- **HTTP ヘッダ受理**: `Mcp-Method`（値 = リクエストボディの `method` と一致必須）、`Mcp-Name`（tool 呼び出しなら `params.name`）。不一致は 400。`Mcp-Session-Id` は受理はする（旧経路互換）／発行しない（RC 経路）。
- **JSON-RPC メタ**: `params._meta.traceparent` / `.tracestate` / `.baggage` を受理、応答の `result._meta` に自サーバ側 traceparent を返す。
- **`initialize` 系メソッド**: RC 経路は no-op 200。旧経路は従来通り `Mcp-Session-Id` を発行して 200。
- **Deprecation ヘッダ**: 旧経路応答のみに付与。
- 既存 18+1 tools の I/O 定義は無変更。`queen.rs` の各 handler は無改修、`queen_compat.rs` がヘッダとメタだけ翻訳する。

## 5.5.2 新 Tauri Command（予約）

- `query_spans({ sessionId?, traceId?, sinceNs?, limit? }) -> Span[]` — Waterfall / cost breakdown 用の read-only SQLite クエリ。
- `set_capture_content({ enabled }) -> void` — 実行時に prompt 本体キャプチャを toggle（永続化なし）。

## 5.5.3 新 Tauri Event（予約）

- `agent-cost` — `{ id, model, costUmicro, inputTok, outputTok, ttftMs? }`。`gen_ai.chat` span 終了時に1回 emit。
- `trace-updated` — `{ traceId, sessionId? }`。新しい root span が SQLite に書かれたとき。

## 5.5.4 非回帰

- Phase 3.x〜4.4 の全 CONTRACT 断面は追加のみで維持。
- 旧 MCP クライアントは `mcp.legacy_2025_06: true` の間は無改修で動く。
- `observability.enabled: false`（既定）では新規テーブルの作成も event の emit も一切起きない。

---

# Phase 6.0 追加契約（Secure & Auditable）

> 状態: 未実装（設計のみ）。実装時に本節へ具体的な wire 契約を書き足す。
> 本仕様は [docs/spec-phase6-0.md](docs/spec-phase6-0.md) を参照。
> 対象 patch: 6.0.0 Foundation / 6.0.1 Sandbox filesystem-only / 6.0.2 Sandbox strict / 6.0.3 Secrets keychain / 6.0.4 Secrets derived + proxy / 6.0.5 Replay UI + Export。
> SQLite `PRAGMA user_version` を **3 → 4** へ bump（6.0.0 で `replays` / `secrets_audit` / `sandbox_events` 追加）。

## 6.0.1 ptygrid.yml スキーマ追加（予約）

- `sandbox:` ブロック — `default_profile` / `warm_pool` / `linux_engine` / `fail_mode` / `strict.{memory_mb, cpu, workspace_mount, kernel}` / `proxy.{enabled, service_rules}`。
- `secrets:` ブロック — `vault` / backend 固有設定 / `entries[].{name, kind: static|short_lived|derived, ttl, scope}`。
- `replay:` ブロック — `enabled` / `storage_dir` / `retention_days` / `redact.{patterns, include_secret_names}`。
- `agents[].{sandbox, secrets, record}` フィールド — agent 単位の profile 上書き。
- workflow step 単位の `{sandbox, secrets, record}` 上書き（Phase 5.0 の `spawn_workflow` 引数拡張）。

## 6.0.2 新 Queen MCP tools（予約・5本）

- `secrets.get({ name, scope }) -> { value, expires_at, jti }` — 短命 token 発行。error `-32011` SecretNotAllowed / `-32012` SecretVaultUnavailable / `-32013` SecretLeaseExhausted。
- `secrets.revoke({ jti }) -> void` — 即時破棄。
- `sandbox.info({}) -> { profile, engine, workspace_mount, network_via_proxy, queen_channel }` — self introspection。
- `sandbox.exec_side({ cmd, args }) -> { stdout, stderr }` — strict プロファイル内で追加コマンド実行、頭 64KB のみ返す。
- `replay.mark({ label, kind }) -> void` — asciicast に `m` イベント差込、Phase 5.5 の OTel span からも AddEvent。

## 6.0.3 新 Tauri Command（予約）

- `replay_list(session_id) -> ReplayMeta[]`
- `replay_open(replay_id) -> { asciicast_url, span_root_id }`
- `replay_export(replay_id, fmt: "cast"|"mp4"|"json") -> { path }`
- `sandbox_status(pane_id) -> SandboxStatus`
- `secrets_audit_tail(limit) -> AuditEntry[]`

## 6.0.4 非回帰

- Phase 5.0 / 5.5 の全契約は不変。
- 既存 `session-state` / `agent-status` / `pty-output` / `pty-exit` は不変。
- `sandbox.default_profile: filesystem-only`（既定）で新 pane は起動、`off` は `unsafe: true` 明示時のみ。
- `queen.sqlite3` v3 → v4 migration は transactional。
