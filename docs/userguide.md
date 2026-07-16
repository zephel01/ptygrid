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
12. [困ったときは](#困ったときは)

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
- **ツールバー右**: 読み取り専用Gitパネルのボタンと「● Queen :39237」バッジ
  - 🟢 緑 = 稼働中 / 🔴 赤 = 停止 / ⚪ 灰 = 無効(`queen.enabled: false`)
  - クリックで Claude Code 用の登録コマンドをクリップボードにコピー
- **各ペイン**: ヘッダーに名前、状態ドット、process tree全体のCPU/メモリ使用量、restart / close / maximize ボタン
- **トースト通知**: mterm.yml の変更検知(Reload)、Queen の `notify` ツール呼び出しなどが右上に表示(5秒で自動消滅)

## ペイン操作

| 操作 | 方法 |
|---|---|
| シェルペインを追加 | ツールバーの「+ Shell」 |
| エージェントを起動 | ツールバーのエージェントチップをクリック(または mterm.yml で `autostart: true`) |
| 再起動 | ペインヘッダーの restart。**ペインとセッション ID を保ったまま**同一設定で再起動 |
| 閉じる | ペインヘッダーの close |
| 最大化/復帰 | ペインヘッダーの maximize |

- ペインは**最大9面**。Queen の `spawn_agent` で起動されたセッションも自動でペインが追加されます(上限到達時はバナーで通知され、セッション自体は動き続けます)。
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
[13個のツール](#queen-ツールリファレンス)を使えるようになります。

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
grok mcp doctor    # 接続確認(handshake OK / 13 tools discovered が出れば成功)
```

### ポートについて

39237 が使用中の場合、Queen は自動で +1 を 39246 まで試します。フォールバックした場合は
登録 URL の読み替えが必要です(ツールバーのバッジに実際のポートが表示されます)。
固定したい場合は `mterm.yml` の `queen.port` を指定してください。

## Queen ツールリファレンス

| ツール | 引数 | 説明 |
|---|---|---|
| `list_agents` | なし | 実行中セッションと mterm.yml 定義の一覧(状態・フォアグラウンドプロセス名付き) |
| `read_output` | `agent`, `lines?`(default 100, 1..1000), `raw?`(default false) | 指定ペインの直近出力。デフォルトで ANSI 除去 + `\r` 上書き畳み込み済み。`raw: true` で生出力 |
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

### 宛先(`agent`)の名前解決

各ペインのheaderは `codex #4`、`claude #5` のように名前とsession IDを表示します。
`list_agents`でも現在のIDを確認できます。`read_output` / `send_message` の宛先は次の順で
解決されます:

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

### Pins / Notes の同時編集

PinsとNotesは読み込んだ`mterm.yml`のdirectory単位で分離され、app-data内のSQLiteへ
永続化されます。repository内に管理fileは作りません。各recordには単調増加する`revision`があり、
既存recordの更新・削除では、直前に取得した`expectedRevision`が一致した場合だけcommitされます。

複数agentが同じrevisionを同時更新した場合は、先に成立した1件だけが成功します。後続は
`conflict`になり、新しい内容を上書きしたり削除したりしません。`list_pins` / `get_note`で
最新版を読み直し、内容をmergeしてから新しいrevisionでretryしてください。異なるkeyやnote IDは
独立して更新できます。

## 実践レシピ: エージェント間協調

### 別のエージェントにタスクを依頼する

Claude Code のペインでこう頼むだけです:

> codex に「src/session.rs をレビューして」と送って、回答が出たら要約して

Claude Code が Queen の `send_message` → `read_output`(ポーリング)を使って実行します。

### 送信前に相手の状態を確認する(推奨)

対話型 TUI の composer に**未送信テキストが残っている**ことがあります(アップデート確認
ダイアログが Enter を消費するケースなど)。`send_message` の前に `read_output` で状態を確認し、
未送信テキストが残っていたら **`text: ""` + `submit: true` で Enter のみを送出**して押し込めます。

### 回答の完了を判定する

`read_output` は「今の画面」を返すだけなので、長いタスクはポーリングで待ちます。
安定した判定方法: **出力が2回連続(10〜15秒間隔)で変化しなくなったら完了**とみなす。
TUI のスピナーは経過秒数を更新し続けるため、出力が静止した = 応答完了と判断できます。

### 外部からスクリプトで操作する

ptygrid のペイン外(通常のターミナルや CI)から Queen を叩く場合は、
リポジトリ同梱の `scripts/queen-send.py` が使えます:

```bash
python3 scripts/queen-send.py codex "テストを実行して"   # 送信 → 完了待ち → 出力表示
python3 scripts/queen-send.py codex --read --lines 50    # 読むだけ
python3 scripts/queen-send.py codex --enter              # Enter のみ送出
```

## 困ったときは

- 実際のドッグフーディングで判明した罠(登録スコープ、サンドボックスの localhost 制限、
  TUI 出力の読み方、composer 二重入力など)は [troubleshooting.md](troubleshooting.md) にまとまっています。
- 設計の背景・アーキテクチャは [design.md](design.md) を参照してください。
