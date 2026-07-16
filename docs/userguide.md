# ptygrid ユーザーガイド

ptygrid のインストールから、`mterm.yml` の書き方、Queen(内蔵 MCP サーバー)を使った
エージェント間協調までを一通り説明します。

## 目次

1. [ptygrid とは](#ptygrid-とは)
2. [インストールと起動](#インストールと起動)
3. [画面の見方](#画面の見方)
4. [ペイン操作](#ペイン操作)
5. [Git status / diff](#git-status--diff)
6. [mterm.yml リファレンス](#mtermyml-リファレンス)
7. [Worktree 分離](#worktree-分離)
8. [セッション復元](#セッション復元)
9. [Queen のセットアップ](#queen-のセットアップ)
10. [Queen ツールリファレンス](#queen-ツールリファレンス)
11. [実践レシピ: エージェント間協調](#実践レシピ-エージェント間協調)
12. [保存データと安全性](#保存データと安全性)
13. [困ったときは](#困ったときは)

---

## ptygrid とは

複数の AI エージェント CLI(Claude Code / Codex / Grok など)をスプリットペインで
並行実行する統合ターミナルです。ただ並べるだけでなく、内蔵 MCP サーバー **Queen** を通じて
ペイン内のエージェント自身が「他のペインを読む・指示を送る・エージェントを起動する」ことができます。

## インストールと起動

前提ツール:

- Rust(rustup でインストール)
- Node.js 20+
- Xcode Command Line Tools(macOS)
- Git

```bash
git clone https://github.com/zephel01/ptygrid.git
cd ptygrid
npm install
npm run tauri dev    # 初回は Rust ビルドで数分かかります
```

ウィンドウが開き、`$SHELL`(zsh 等)が1ペインで動きます。

> ブラウザ単体(`npm run dev`)で開いた場合は PTY が無いため、ローカルエコーのデモ表示になります。

## 画面の見方

- **ツールバー左**: 「+ Shell」ボタン(ペイン追加)、mterm.yml で定義したエージェントのチップ(クリックで起動)
- **ツールバー右**: Gitパネルのボタン、全ペインのCPU/メモリ合計、「● Queen :39237」バッジ、ペイン数
  - 🟢 緑 = 稼働中 / 🔴 赤 = 停止 / ⚪ 灰 = 無効(`queen.enabled: false`)
  - クリックで Claude Code 用の登録コマンドをクリップボードにコピー
- **各ペイン**: ヘッダーに`<name> #<id>`(adhocは`shell #<id>`)、状態ドット、process tree全体のCPU/メモリ使用量、restart / close / maximizeボタン
- **トースト通知**: mterm.yml の変更検知(Reload)、Queen の `notify` ツール呼び出しなどが右上に表示(5秒で自動消滅)

## ペイン操作

| 操作 | 方法 |
|---|---|
| シェルペインを追加 | ツールバーの「+ Shell」 |
| エージェントを起動 | ツールバーのエージェントチップをクリック(または mterm.yml で `autostart: true`) |
| 再起動 | ペインヘッダーのrestart。**ペインとsession IDを保ったまま**同一設定で再起動 |
| 閉じる | ペインヘッダーの close |
| 最大化/復帰 | ペインヘッダーの maximize |

- ペインは**最大9面**。Queen の `spawn_agent` で起動されたセッションも自動でペインが追加されます(上限到達時はバナーで通知され、セッション自体は動き続けます)。
- session IDは現在のアプリ実行中の識別子です。アプリを終了してlogical resumeした後は、
  新しく採番されるためheaderまたは`list_agents`で確認し直してください。
- 出力はセッションごとにリングバッファ(256 KiB)へ保存され、restart をまたいで連続します。
- CPU/メモリ表示は1秒ごとに更新されます。CPUは1 coreを100%として合算するため、
  複数coreを使うsessionでは100%を超える場合があります。メモリはPTY childと全子孫の
  resident memory合計です。ツールバー右側の`Σ CPU`表示は、現在監視できている
  全running sessionの合計です。

## Git status / diff

ツールバー右の「Git」を押すと、現在のプロジェクトの変更ファイルとunified diffを
右側パネルに表示します。ファイルを選択し、`Working tree` / `Staged` を切り替えられます。

- `mterm.yml` 読込済みなら、そのファイルがあるディレクトリのリポジトリを使用します。
- 未読込なら、ptygridを起動したカレントディレクトリを使用します。
- external diff、textconv、pagerは実行しません。
- diff表示は2 MiB、status表示は10,000ファイルで打ち切ります。
- untracked fileも選択すると新規ファイルdiffを表示します。

stage/unstageするには、対象ファイルのチェックボックスを選び、`Stage` または
`Unstage` を押します。ファイル行を開くだけではindexは変更されません。

commit欄へメッセージを入力して `Commit staged changes` を押すと、現在stage済みの
変更だけをcommitします。未stageのファイルを暗黙に追加することはありません。
リポジトリのpre-commit / commit-msgなどのhooksと署名設定は通常の`git commit`と
同様に適用され、失敗した場合はGitのエラーがパネルに表示されます。

## mterm.yml リファレンス

プロジェクトルートに `mterm.yml` を置き、アプリのツールバーから読み込みます。
ファイルは監視されており、変更すると「Reload」トーストから再読込できます。
サンプル: [mterm.example.yml](../mterm.example.yml)

```yaml
project: my-app

queen:            # 省略可(丸ごと省略でデフォルト動作)
  enabled: true   # デフォルト true。false で Queen を停止
  port: 39237     # デフォルト 39237。使用中なら +1 を 39246 まで試す

agents:           # 対話型 AI CLI
  - name: claude
    cmd: "claude"
    cwd: "."                                   # mterm.yml のあるディレクトリ基準の相対パス可
    env:
      ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}"  # ${VAR} はホスト環境変数を展開
    autostart: false
    autorestart: never                          # never | on-failure | always

processes:        # 通常の常駐プロセス(dev サーバー等)。フィールドは agents と同じ
  - name: web
    cmd: "npm run dev"
    autorestart: on-failure
```

### フィールド一覧

| フィールド | 必須 | デフォルト | 説明 |
|---|---|---|---|
| `project` | - | - | プロジェクト名(表示用) |
| `queen.enabled` | - | `true` | Queen(内蔵 MCP サーバー)の有効/無効 |
| `queen.port` | - | `39237` | Queen の待受ポート。使用中なら +1 を 39246 まで自動試行 |
| `agents[].name` / `processes[].name` | ✅ | - | 表示名。Queen の宛先名・`spawn_agent` の許可リストにもなる |
| `.cmd` | ✅ | - | 起動コマンド |
| `.cwd` | - | mterm.yml の場所 | 作業ディレクトリ。相対パスは mterm.yml 基準で解決 |
| `.env` | - | - | 環境変数。値の `${VAR}` はホスト環境から展開(未定義は空文字) |
| `.autostart` | - | `false` | 設定読込時に自動起動 |
| `.autorestart` | - | `never` | `never` / `on-failure` / `always`。連続5回失敗で打ち切り |
| `.resume` | - | `.cmd` | アプリ再起動後のlogical resume時に使うcommand |
| `.worktree.enabled` | - | `false` | 定義の起動ごとにlinked worktreeと専用branchを作る |
| `.worktree.base` | - | `HEAD` | worktree branchの起点となるbranch/tag/commit |
| `.worktree.setup` | - | - | worktree作成後、agent cwdで一度だけ実行するsetup command |

> すべてのセッションには環境変数 `QUEEN_URL`(例: `http://127.0.0.1:39237/mcp`)が注入されます。
> ペイン内で接続先を確認したいときは `echo $QUEEN_URL` を実行してください。

## Worktree 分離

同じrepositoryで複数agentが同時編集すると競合する場合、定義ごとにworktree分離を
有効化できます。既定は無効で、従来どおり全agentが同じworkspaceを共有します。

```yaml
agents:
  - name: codex
    cmd: codex
    cwd: packages/app
    worktree:
      enabled: true
      base: HEAD
      setup: npm install
```

有効な定義を起動すると、app-data配下に一意なlinked worktreeと
`ptygrid/codex/...` branchを作り、ペインヘッダーにbranch名を表示します。
`cwd` がrepository内のサブディレクトリなら、worktree内でも同じ相対位置から
起動します。restart/autorestartでは同じworktreeを再利用します。実行中の
worktreeはGitパネル上部の`Workspace`から選び、そのbranchのdiff確認・commitができます。

worktreeはGitの自動pruneを避けるためlockされ、ptygridは自動削除しません。
作業を回収・commitした後、不要になったworktreeは通常のGitコマンドで明示的に
片付けてください。`<path>`と`<branch>`はペインのbranch表示とツールチップで確認できます。

```bash
git worktree unlock <path>
git worktree remove <path>   # dirtyならGitが拒否する
git branch -d <branch>
```

setupまたはagent起動に失敗してもworktreeは保持され、エラーにpathが表示されます。
内容を確認せず`--force`で削除しないでください。

## セッション復元

ptygridは最後に開いていたproject、ペイン順、列レイアウト、最大化状態をapp-dataへ
自動保存します。次回起動時に現在の`mterm.yml`を読み直し、設定定義を新しいPTYとして
再起動します。AI CLIに会話再開用commandがある場合は`resume`で指定できます。

```yaml
agents:
  - name: codex
    cmd: codex
    resume: codex resume --last
  - name: claude
    cmd: claude
    resume: claude --continue
```

`resume`を省略した定義は`cmd`を再実行します。通常の起動やペインの再起動には
`resume`ではなく、そのsessionを起動したcommandが使われます。

この機能は終了済みprocessへの再接続ではありません。adhoc shellも新しいdefault shellとして
開き直され、以前のscrollbackは復元されません。worktree sessionは保存pathが同じrepositoryの
有効なlinked worktreeであることを確認して再利用し、setup commandは再実行しません。

保存JSONにcommand、terminal出力、環境変数は含まれません。状態ファイルが壊れている、
project directoryが移動した、または定義が削除された場合は画面に復元エラーを表示します。

## Queen のセットアップ

Queen はアプリ内に常駐する MCP サーバーです(streamable HTTP、bind は 127.0.0.1 のみ)。
各エージェント CLI に MCP サーバーとして登録すると、そのエージェントが
[18個のツール](#queen-ツールリファレンス)を使えるようになります。

### Claude Code

```bash
claude mcp add -s user --transport http queen http://127.0.0.1:39237/mcp
```

> ⚠️ **`-s user` を必ず付けてください。** デフォルトの local スコープは「コマンドを実行した
> ディレクトリ限定」の登録になるため、ペインの作業ディレクトリと違う場所で登録すると
> Claude Code から Queen が見えません(実例あり)。プロジェクト単位で共有したい場合は
> `-s project`(`.mcp.json` がリポジトリに作られる)も使えます。

### Codex CLI

`~/.codex/config.toml` に追記:

```toml
[mcp_servers.queen]
url = "http://127.0.0.1:39237/mcp"
```

### Grok CLI

```bash
grok mcp add -s user -t http queen http://127.0.0.1:39237/mcp
grok mcp doctor    # 接続確認(handshake OK / 18 tools discovered が出れば成功)
```

### ポートについて

39237 が使用中の場合、Queen は自動で +1 を 39246 まで試します。フォールバックした場合は
登録 URL の読み替えが必要です(ツールバーのバッジに実際のポートが表示されます)。
固定したい場合は `mterm.yml` の `queen.port` を指定してください。

## Queen ツールリファレンス

| ツール | 引数 | 説明 |
|---|---|---|
| `list_agents` | なし | 実行中セッションと mterm.yml 定義の一覧(状態・フォアグラウンドプロセス名付き) |
| `read_output` | `agent`, `lines?`(default 100, 1..1000), `raw?`(default false) | 指定ペインの直近出力。デフォルトでペイン寸法に合わせてANSIカーソル移動・画面消去・alternate screenを再構成。`raw: true`で生出力 |
| `send_message` | `agent`, `text`, `submit?`(default true) | 指定ペインの stdin へ書き込み。`submit: true` で末尾に Enter を付与 |
| `spawn_agent` | `name` | **mterm.yml で定義された名前のみ**起動可(許可リスト方式) |
| `notify` | `title`, `message` | アプリ内トースト通知を表示 |
| `set_pin` | `key`, `value`, `expectedRevision?` | project内の短い共有値を作成・安全に更新。既存値の更新には現在のrevisionが必須 |
| `list_pins` | なし | project内のpinとrevisionをkey順で一覧表示 |
| `delete_pin` | `key`, `expectedRevision` | revisionが一致するpinだけ削除 |
| `create_note` | `title`, `body`, `tags?` | project内に永続noteを作成 |
| `list_notes` | `query?`, `limit?`(default 50, max 200) | noteを更新日時の新しい順で検索・一覧表示 |
| `get_note` | `id` | 安定したIDでnoteを1件取得 |
| `update_note` | `id`, `expectedRevision`, `title?`, `body?`, `tags?` | revisionが一致するnoteの指定fieldだけ更新 |
| `delete_note` | `id`, `expectedRevision` | revisionが一致するnoteだけ削除 |
| `send_inbox` | `sender`, `recipient`, `subject`, `body` | stable mailboxへ永続messageを送る。live PTYには入力しない |
| `list_inbox` | `mailbox`, `afterId?`, `includeAcknowledged?`, `limit?` | ID昇順でinboxを読む。defaultは未ackだけ |
| `ack_inbox` | `id`, `recipient` | 宛先が一致するmessageをidempotentにacknowledge |
| `reply_inbox` | `id`, `sender`, `body` | 元宛先からcorrelated replyを送り、元messageをacknowledge |
| `await` | `mailbox`, `afterId?`, `includeAcknowledged?`, `limit?`, `timeoutMs?` | cursor以降のInbox到着をtimeout/cancelまで待つ |

### 宛先(`agent`)の名前解決

定義から起動したペインは`codex #4`、adhoc shellは`shell #5`のようにsession IDを
表示します。shell内でCodexを手動起動してもheaderの名前は`shell`のままですが、
`list_agents`の`foreground`には`codex`が現れます。現在のIDを確認してから、
`read_output` / `send_message`の宛先を次の規則で指定します:

1. **`#<id>`** — session IDの厳密指定(例: `"#4"`)。複数ペイン時はこれを推奨
2. **mterm.yml の定義名 / session名** — 完全一致し、実行中の候補が1つだけの場合
3. **foreground process名** — 完全一致し、候補が1つだけの場合。shell内で手動起動した
   `codex` / `claude` / `grok`も判定できる

同じ名前やforeground processが複数ある場合は、最新ペインを推測して送信せず、
`use one of: #2, #4`のように候補IDを返します。例えばCodexが3面あるなら
`agent: "codex"`ではなく`agent: "#4"`と伝えてください。定義できるペインには
`codex-impl`、`codex-review`、`claude-test`のようなrole名を付けると、人にもagentにも
意図が分かりやすくなります。見つからない場合も、errorに実行中sessionの一覧
(foreground process名付き)が含まれます。

例として、Codexが`#3`と`#5`の2面にある場合、次のように依頼します。

> `#3`に「変更をレビューして」と送って、回答を読んで

MCP toolの引数では`{ "agent": "#3", ... }`です。`agent: "codex"`は曖昧errorに
なるため、意図しないペインへの誤送信は起きません。

Claude Codeへ「`grok #2で作業させて`」「`codex #3にレビューを依頼して`」と頼んだ場合、
Queen接続済みなら既存ペインの指定として扱い、`list_agents`でIDを確認してから
`read_output` / `send_message`を使います。新しいGrok/Codexプロセスを起動する意味ではありません。

### Pins / Notes の同時編集

PinsとNotesは読み込んだ`mterm.yml`のdirectory単位で分離され、app-data内のSQLiteへ
永続化されます。repository内に管理fileは作りません。各recordには単調増加する`revision`があり、
既存recordの更新・削除では、直前に取得した`expectedRevision`が一致した場合だけcommitされます。

複数agentが同じrevisionを同時更新した場合は、先に成立した1件だけが成功します。後続は
`conflict`になり、新しい内容を上書きしたり削除したりしません。`list_pins` / `get_note`で
最新版を読み直し、内容をmergeしてから新しいrevisionでretryしてください。異なるkeyやnote IDは
独立して更新できます。

推奨する更新手順:

1. `list_pins`または`get_note`で値と`revision`を読む
2. 内容を更新し、取得したrevisionを`expectedRevision`へ指定する
3. `conflict`なら最新版を再取得し、自分の変更をmergeしてretryする

例えば担当paneを共有するpinは、初回だけrevisionを省略して作成します。

```json
{ "key": "task/owner", "value": "#3" }
```

`set_pin`の返却値が`revision: 1`なら、変更時は
`{ "key": "task/owner", "value": "#5", "expectedRevision": 1 }`とします。設計判断や
長い経緯はpinではなく`create_note`へ保存し、返された安定IDを共有してください。

### Inbox / Reply

Inboxは`send_message`と用途が異なります。`send_message`は今動いているPTYへ直接入力し、
Inboxは相手が後から読めるproject-scopedな永続messageをSQLiteへ追記します。

Inboxの`sender` / `recipient` / `mailbox`には、`codex-review`や`claude-impl`のような
安定したrole名を使用します。app再起動で変わる`#3`などのsession IDは拒否されます。

```json
{
  "sender": "claude-impl",
  "recipient": "codex-review",
  "subject": "Review request",
  "body": "Commit 71a483bを確認してください"
}
```

受信側は`list_inbox`で未ack messageを取得します。返答する場合はmessage IDを指定します。

```json
{ "id": 12, "sender": "codex-review", "body": "問題ありません" }
```

`reply_inbox`はreplyを元senderへ送り、`inReplyToId`と`rootMessageId`でthreadを維持しながら、
元messageを同じtransactionでacknowledgeします。返答しない場合は`ack_inbox`を使います。
同じackを再実行しても状態は壊れません。既定の`list_inbox`はack済みmessageを除外するため、
履歴が必要な場合だけ`includeAcknowledged: true`を指定してください。

Phase 3.7ではMCP clientをmailboxへ認証bindingしていないため、sender/recipientは明示値です。
Queenはlocalhost専用ですが、mailbox名をaccess-control境界として扱わないでください。
subjectは256 bytes、bodyは64 KiB、projectごとに50,000 messagesが上限です。message本文の
更新・削除は提供しないため、訂正は新しいmessageまたはreplyとして送ります。

### Inboxを待つ(`await`)

`list_inbox`を短い間隔で繰り返す代わりに、`await`で新しいmessageの到着を待てます。

```json
{
  "mailbox": "codex-review",
  "afterId": 12,
  "timeoutMs": 30000
}
```

- ID 12より後に一致messageがすでにあれば即時return
- 到着時は`messages`と最大IDの`nextCursor`、`timedOut: false`を返す
- deadlineでは空の`messages`、入力cursor、`timedOut: true`を正常return
- default timeoutは30秒、指定範囲は1 ms〜5分
- MCP clientがrequestをcancelすると、Inboxを変更せず直ちにcancellation errorで終了

次の呼出しでは、前回返された`nextCursor`を`afterId`として渡します。`await`自体はmessageを
acknowledgeしないため、処理完了後に`ack_inbox`または`reply_inbox`を呼んでください。

## 実践レシピ: エージェント間協調

### 別のエージェントにタスクを依頼する

Claude Code のペインでこう頼むだけです:

> `#3`に「src/session.rsをレビューして」と送って、回答が出たら要約して

Claude Code が Queen の `send_message` → `read_output`(ポーリング)を使って実行します。

### 送信前に相手の状態を確認する(推奨)

対話型 TUI の composer に**未送信テキストが残っている**ことがあります(アップデート確認
ダイアログが Enter を消費するケースなど)。`send_message` の前に `read_output` で状態を確認し、
未送信テキストが残っていたら **`text: ""` + `submit: true` で Enter のみを送出**して押し込めます。

### 回答の完了を判定する

`read_output` は「今の画面」を返すだけなので、長いタスクはポーリングで待ちます。
安定した判定方法: **出力が2回連続(10〜15秒間隔)で変化しなくなったら完了**とみなす。
TUI のスピナーは経過秒数を更新し続けるため、出力が静止した = 応答完了と判断できます。

Grokなど画面全体を頻繁に再描画するTUIに対しても、`read_output`はカーソル移動と消去を
反映するため、過去の再描画を単純連結しません。それでもTUIが表示内容を更新し続ける間は
完了判定が遅れる場合があります。`sent to #<id>`が出ていれば送信自体は成功しています。
長時間待つ場合は成果物の更新も確認してください。詳細は`docs/troubleshooting.md`を参照してください。

### 外部からスクリプトで操作する

ptygrid のペイン外(通常のターミナルや CI)から Queen を叩く場合は、
リポジトリ同梱の `scripts/queen-send.py` が使えます:

```bash
python3 scripts/queen-send.py '#3' "テストを実行して"   # 送信 → 完了待ち → 出力表示
python3 scripts/queen-send.py '#3' --read --lines 50    # 読むだけ
python3 scripts/queen-send.py '#3' --enter              # Enter のみ送出
```

shellで`#`はcomment開始文字なので、引数をsingle quoteで囲んでください。

## 保存データと安全性

ptygridのruntime管理データはTauriのapp-data directoryへ保存し、project repositoryへ
管理fileを追加しません。

| データ | 保存先(app-data配下) | 内容 |
|---|---|---|
| logical session state | `project-state/` | project、layout、pane順、定義名、worktree参照 |
| linked worktree | `worktrees/` | opt-inで作成したworktreeとbranch |
| Queen Pins / Notes / Inbox | `queen/queen.sqlite3` | canonical project directoryごとの共有データ |

terminal出力、展開後の環境変数、`QUEEN_URL`、起動command本文はsession stateへ保存しません。
Queenはlocalhost (`127.0.0.1`) のみにbindし、`spawn_agent`は読み込んだ`mterm.yml`の
定義名だけを許可します。認証はないため、信頼できないlocal processが動く環境ではQueenを
`queen.enabled: false`で無効化してください。

## 困ったときは

- 実際のドッグフーディングで判明した罠(登録スコープ、サンドボックスの localhost 制限、
  TUI 出力の読み方、composer 二重入力など)は [troubleshooting.md](troubleshooting.md) にまとまっています。
- 設計の背景・アーキテクチャは [design.md](design.md) を参照してください。
