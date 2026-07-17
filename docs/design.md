**日本語** · [English](design.en.md)

# ptygrid 設計・アーキテクチャ

更新日: 2026-07-16 / 実装基準: Phase 3.9

この文書は現在の実装構造と、変更時に守る設計判断をまとめます。初期調査と競合上の
位置づけは[competitive-landscape.md](competitive-landscape.md)、IPCの正確な型と制限は
[CONTRACT.md](../CONTRACT.md)、操作方法は[userguide.md](userguide.md)を参照してください。

## 1. 目的と境界

ptygridは、複数のAI CLIや開発processを最大9個のPTYペインで並行実行し、内蔵MCP
server Queenを通して相互に読み書き・協調させるmacOS向けdesktop applicationです。
Linuxにも同じruntime経路を実装していますが、Phase 3.9時点ではテスト対応（beta）です。

設計上の中心は次の4点です。

- 任意の対話型CLIを通常のPTYとして扱い、特定vendorのprotocolへ依存しない
- session、Git、永続データの境界をproject単位で明確にする
- 同名agentや同時更新を推測で解決せず、誤送信・lost updateを拒否する
- Phase 3を機能単位で独立releaseできる構造に保つ

フルIDE、remote execution、PTY processへのOS再起動後の再接続、dirty worktreeの自動削除は
現時点の対象外です。

## 2. 現在の構成

```text
┌─ Frontend: Svelte 5 / TypeScript / Tauri WebView ──────────────┐
│ App.svelte                                                     │
│ ├ Terminal.svelte × 最大9 (xterm.js + fit addon)               │
│ ├ GitPanel.svelte (status/diff/stage/unstage/commit)            │
│ └ stores.svelte.ts (session/layout/resource/state)              │
└────────────────── Tauri commands / batched events ─────────────┘
                              │
┌─ Backend: Rust ─────────────┴───────────────────────────────────┐
│ commands.rs          IPC boundary                               │
│ session.rs / pty.rs  PTY lifecycle, output ring, name resolution│
│ config.rs            ptygrid.yml parse/watch                      │
│ git_service.rs       installed git invocation                   │
│ worktree.rs          opt-in linked worktree creation/reuse      │
│ project_state.rs     versioned logical session persistence      │
│ resource_monitor.rs  shared process-tree CPU/RSS sampler        │
│ queen.rs             rmcp Streamable HTTP server / 18 tools     │
│ queen_store.rs       SQLite pins/notes/inbox with transactions  │
└─────────────────────────────────────────────────────────────────┘
```

`lib.rs`はserviceを初期化してTauri commandsを登録するだけに留めます。wire formatは
`commands.rs`、processやstorageの実装は個別moduleへ分離し、sessionのhot pathへGitや
SQLite処理を混在させません。

## 3. 技術スタック

| レイヤ | 採用 | 理由 |
|---|---|---|
| desktop | Tauri v2 | Rust backendと軽量なsystem WebView |
| frontend | Svelte 5 + TypeScript + Vite | 小さいreactive stateと複数terminalの描画 |
| terminal | `@xterm/xterm` + `portable-pty` | ANSI terminal UIとnative PTY process |
| async / server | Tokio + Axum + rmcp | QueenのStreamable HTTP transport |
| config | serde_norway + notify | YAML parseと明示Reload用change event |
| Git | installed `git` executable | native hooks、signing、config、worktree semanticsを維持 |
| process monitor | sysinfo | 1 samplerで全session process treeを集計 |
| Queen storage | rusqlite + bundled SQLite | transaction、schema version、project分離 |
| GUI PATH recovery | fix-path-env-rs | Linux/macOSのdesktop起動でもuser導入CLIを解決 |

Gitに`git2`は使用しません。command文字列をshellへ渡さず、構造化した引数でinstalled
`git`を起動します。これによりuserのGit hooksと署名設定を通常のGitと同じように扱います。

### Naming / packaging compatibility

表示名、window title、Rust crate/binary、npm packageは`ptygrid`へ統一しています。bundle iconは
`src-tauri/icons/`のterminal prompt + grid motifです。一方、既存userのTauri app-dataを
引き継ぐためbundle identifier `com.zephel01.multiterminal`は意図的に維持しています。
config filenameは`ptygrid.yml`へ変更済みです。migration設計として旧`mterm.yml`も
引き続き読み込み（両方ある場合は`ptygrid.yml`優先）、watcherは実際に読み込んだ
fileを追跡します。bundle identifierはmigration設計なしにrenameしないでください。

## 4. Session / PTY model

1 sessionはbackend採番の`u32` ID、PTY master/slave、child process、writer、256 KiBの
output ring、起動spec、generationを持ちます。

- `spawn_shell` / `spawn_agent`はPTYを作り、blocking reader threadから`pty-output`をemitする
- frontend入力は`write_pty`、サイズ変更は`resize_pty`を通す
- restart/autorestartは同じsession IDを維持し、generationで古いreaderのeventを無効化する
- manual killはautorestartを発火させない
- `on-failure` / `always`は1秒後に再起動し、連続5回失敗で停止する

アプリ終了後はPTYへ再接続しません。logical resumeは保存した定義参照を現在の
`ptygrid.yml`（legacy: `mterm.yml`）で解決し、新しいPTY processを起動します。`resume`があれば再開用command、
なければ通常の`cmd`を使用します。

## 5. Session addressing

pane headerは定義sessionを`codex #3`、adhoc sessionを`shell #3`のように表示します。
shell内で手動起動したCLIはforeground processとして別途検出します。

Queenの`agent`解決順は次のとおりです。

1. `#<id>`による厳密指定
2. 一意な定義/session名の完全一致
3. 一意なforeground process名の完全一致

候補が複数なら`#3, #5`のような候補を含むambiguous errorを返します。最新IDなどを
推測して送信しません。session IDは現在のapp実行中だけ有効で、app再起動後は
`list_agents`またはpane headerから取得し直します。

## 6. Queen

Queenは`127.0.0.1`だけにbindするrmcp Streamable HTTP serverです。default portは39237、
使用中なら39246まで順に試します。全sessionへ実際のURLを`QUEEN_URL`として渡します。

Phase 3.8時点の18 tools:

- live session: `list_agents`, `read_output`, `send_message`, `spawn_agent`, `notify`
- durable pins: `set_pin`, `list_pins`, `delete_pin`
- durable notes: `create_note`, `list_notes`, `get_note`, `update_note`, `delete_note`
- durable inbox: `send_inbox`, `list_inbox`, `ack_inbox`, `reply_inbox`
- wait: `await`

`spawn_agent`は現在の`ptygrid.yml`定義名だけを許可します。Pins/Notes/Inboxは読み込まれた
canonical config directoryでscopeし、project未読込時は使用できません。

Inboxはlive PTYへ書く`send_message`と分離した追記専用channelです。app再起動で変わる
session `#id`ではなくstable mailbox名を使います。replyは元送受信者を反転し、root IDを
継承してthreadを形成します。reply作成は元messageのacknowledgementと同じtransactionで行います。

`await`はInbox generationを`tokio::sync::watch`で購読してから初回queryするため、queryと
sleep開始の間に届いたmessageを取りこぼしません。wait中はDB mutexを保持せず、message commit時の
通知で短いqueryを再実行します。rmcpのrequest cancellation token、最大5分のdeadline、既存messageの
即時returnを同じselect loopで扱います。

## 7. 永続化と同時更新

runtime管理データはTauri app-data配下に置き、user repositoryへ管理fileを作りません。

```text
app-data/
├ project-state/           versioned JSON / last project pointer
├ worktrees/               opt-in linked worktrees
└ queen/queen.sqlite3      project-scoped pins, notes and inbox
```

Queen Storeは1つの`Mutex<Connection>`でprocess内accessを直列化し、mutationを
`BEGIN IMMEDIATE` transactionで行います。各pin/noteは単調増加する`revision`を持ち、
更新・削除は`expectedRevision`一致時だけ成功します。同じrevisionから複数agentが同時に
書いても最初の1件だけがcommitされ、残りはconflictとしてrollbackされます。

DBはWAL、busy timeout 5秒、`PRAGMA user_version = 2`です。v1からv2はtransactionalに
migrationし、未知の新しいschema versionは
黙って開きません。

## 8. Git / Worktree safety

Git panelはstatus、working/staged diff、明示pathのstage/unstage、現在のindexだけを対象にした
commitを提供します。file選択やcommitで暗黙stageは行いません。diffにはsize/file-count上限を
設け、pathはrepository内に限定します。

worktree isolationはagent定義ごとのopt-inです。ptygridはapp-data配下へlinked worktreeと
`ptygrid/<agent>/...` branchを作りlockします。restartでは同じworktreeを再利用し、setupは
初回だけ実行します。失敗時もworktreeを保持し、dirty contentを自動削除しません。

## 9. Resource monitoring

全sessionで1つのsysinfo samplerを共有し、1秒ごとにprocess情報を1回refreshします。
PTY childをrootとして全descendantのCPU、resident memory、process countを合算し、1つの
`session-resources` eventとしてfrontendへ送ります。

CPUは1 coreを100%とするためmulti-core workloadは100%を超えます。memoryは各processのRSS
合計で、shared pageの重複排除はしません。pane値の合計をtoolbarへ表示し、合計用の追加samplingは
行いません。

## 10. Release status

Phase 0〜2.1とPhase 3.0〜3.9は実装済みです。3.9でLinuxテスト対応としてnative build、
Ubuntu CI、`.deb` / AppImage packaging、GUI起動時のPATH復元を追加しました。
詳細なgateは[phase3.md](phase3.md)を参照してください。

## 11. 変更時の原則

- IPCやMCP schema変更は先に[CONTRACT.md](../CONTRACT.md)へ追記する
- project dataをrepositoryへ暗黙作成しない
- destructive Git/worktree操作を推測で行わない
- 同名sessionやstale revisionを自動選択・自動上書きしない
- backendは`cargo test` / `cargo check`、frontendは`svelte-check` / production buildを通す
- macOS / Linux betaのCIを維持し、platform固有process取得をfallback可能に保つ
