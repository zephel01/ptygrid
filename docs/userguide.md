**日本語** · [English](userguide.en.md)

# ptygrid ユーザーガイド

ptygrid のインストールから、`ptygrid.yml` の書き方、Queen(内蔵 MCP サーバー)を使った
エージェント間協調までを一通り説明します。

## 目次

1. [ptygrid とは](#ptygrid-とは)
2. [インストールと起動](#インストールと起動)
3. [画面の見方](#画面の見方)
4. [ペイン操作](#ペイン操作)
5. [Git status / diff](#git-status--diff)
6. [ptygrid.yml リファレンス](#ptygridyml-リファレンス)
7. [Worktree 分離](#worktree-分離)
8. [セッション復元](#セッション復元)
9. [Queen のセットアップ](#queen-のセットアップ)
10. [Teammates(hooks 受信)](#teammateshooks-受信)
11. [Queen ツールリファレンス](#queen-ツールリファレンス)
12. [チームプリセット(team_presets)](#チームプリセットteam_presets)
13. [実践レシピ: エージェント間協調](#実践レシピ-エージェント間協調)
14. [保存データと安全性](#保存データと安全性)
15. [困ったときは](#困ったときは)

---

## ptygrid とは

複数の AI エージェント CLI(Claude Code / Codex / Grok など)をスプリットペインで
並行実行する統合ターミナルです。ただ並べるだけでなく、内蔵 MCP サーバー **Queen** を通じて
ペイン内のエージェント自身が「他のペインを読む・指示を送る・エージェントを起動する」ことができます。

## インストールと起動

前提ツール:

- Rust(rustup でインストール)
- Node.js 20+
- Git
- macOS: Xcode Command Line Tools
- Linux: WebKitGTK 4.1などのTauri system dependencies

### Linux（Ubuntu / Debian・テスト対応）

Ubuntu 22.04またはDebian 12以降を基準にしています。開発・build用依存を導入します:

> Linux版はPhase 3.9時点でテスト対応（beta）です。build・package生成はCIで検証していますが、
> desktop環境やdistributionごとの安定動作は実機検証を継続しています。

```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

通常の開発起動はmacOSと同じです。Linux packageを作る場合は次を実行します:

```bash
npm install
npm run tauri dev
npm run bundle:linux   # .deb + AppImage
```

成果物は`src-tauri/target/release/bundle/deb/`と`appimage/`へ出力されます。
デスクトップランチャーから起動した場合も、起動時にlogin shell由来の`PATH`を復元するため、
ユーザーが導入したClaude Code / Codex / Grok / GitをPTYから起動できます。

```bash
git clone https://github.com/zephel01/ptygrid.git
cd ptygrid
npm install
npm run tauri dev    # 初回は Rust ビルドで数分かかります
```

ウィンドウが開き、`$SHELL`(zsh 等)が1ペインで動きます。

> ブラウザ単体(`npm run dev`)で開いた場合は PTY が無いため、ローカルエコーのデモ表示になります。

## 画面の見方

- **ツールバー左**: 「+ Shell」ボタン(ペイン追加)、**作業フォルダ**の入力欄＋「読み込み」ボタン(例 `~/works/hoge`。先頭 `~` 可)、読み込み後は設定ファイルの由来バッジ(`設定: プロジェクト内 / 起動フォルダ / ~/.ptygrid / 既定`)と ptygrid.yml で定義したエージェントのチップ(クリックで起動)。読み込み成功時は開いているシェルのペインが作業フォルダへ自動 cd します
- **ツールバー右**: Gitパネルのボタン、全ペインのCPU/メモリ合計、「● Queen :39237」バッジ、ペイン数
  - 🟢 緑 = 稼働中 / 🔴 赤 = 停止 / ⚪ 灰 = 無効(`queen.enabled: false`)
  - クリックで Claude Code 用の登録コマンド(認証トークン込み)をクリップボードにコピー。トークンは保存され再起動後も有効なので、登録は**初回のみ**でOK(トークンを再生成したときだけ再登録)

### 作業フォルダのサジェスト

**作業フォルダ**入力欄はタイプミス防止のため候補（`<datalist>`）を出します。候補は
プロジェクトの**置き場所（projects root）**直下の各フォルダを `<root>/<フォルダ名>` の形で
並べたものです。以前あった独立した「cd…」ボタン／一括cdポップオーバーは廃止され、
「読み込み」が作業フォルダの確定と一括 cd を兼ねます（下記「読み込み = cd」）。

- **ルートの自動記憶**: 「読み込み」に成功すると、読み込んだ作業フォルダの**親ディレクトリ**が
  projects root として自動保存されます（`app-settings.json`。プロジェクトを切り替えても保持）。
  親が `/` やホームディレクトリそのものの場合は、置き場所として広すぎるため保存しません。
  保存は best-effort で、失敗してもトーストや操作の妨げにはなりません。
- **候補の表示**: ルート設定済みなら、アプリ起動時と入力欄をフォーカスしたときにルート直下の
  **非隠しフォルダ**を取得し、`<root>/<フォルダ名>`（先頭 `~` はそのまま）を候補にします
  （名前順・最大200件）。ルート未設定なら候補は出ません（エラーにもなりません）。
- 候補を選ぶか `~/works/hoge` のようにパスを直接入力して「読み込み」を押すと、その作業フォルダを
  読み込みつつ、開いているシェルのペインを同じフォルダへ自動 cd します。
- **各ペイン**: ヘッダーに`<name> #<id>`(adhocは`shell #<id>`)、状態ドット、process tree全体のCPU/メモリ使用量、restart / close / maximizeボタン
- **接続先の表示**: ペインのフォアグラウンドがリモートセッション系コマンドのとき、ペインヘッダーとステータスサイドバーの表示名が**接続先付き**になります。対応: `ssh` / `sftp` / `scp`(リモート側ホスト)/ `mosh`(mosh-client の `-#` 表示引数)/ `telnet`(`host:port`)は `ssh user@host` 形式、`kubectl` / `docker` は exec 系 subcommand のとき `kubectl exec ns/pod` 形式(`-n` は前後どちらでも可)。`.ssh/config` のエイリアス名、`-l user` の畳み込み、`-p` 等の値付きオプションのスキップに対応。コマンドを抜けると次のサンプリング(1秒)で自動的に元へ戻ります。別ホスト・別Podへの打ち間違い防止に
- **トースト通知**: ptygrid.yml の変更検知(Reload)、Queen の `notify` ツール呼び出しなどが右上に表示(5秒で自動消滅)
- **UI言語(⚙ 設定)**: フッター右端の ⚙ ボタンで UI 言語を切替できます — 自動(システム) / English / 日本語。既定は「自動」で、OS が日本語なら日本語、それ以外は英語で表示します。選択は端末ローカル(localStorage)に保存され再起動後も維持。対象はボタン・メニュー・トースト等の UI 文字列のみで、Rust バックエンド発のエラーメッセージや PTY 内のログは翻訳しません(必要ならエージェントに読ませれば言語を問わず解釈できます)

## ペイン操作

| 操作 | 方法 |
|---|---|
| シェルペインを追加 | ツールバーの「+ Shell」 |
| エージェントを起動 | ツールバーのエージェントチップをクリック(または ptygrid.yml で `autostart: true`) |
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

- `ptygrid.yml` 読込済みなら、そのファイルがあるディレクトリのリポジトリを使用します。
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

## ptygrid.yml リファレンス

ツールバーの **「作業フォルダ」欄**に作業対象のフォルダ（例: `~/works/hoge`。先頭 `~` は
ホーム展開）を入れて「読み込み」を押します。設定ファイル `ptygrid.yml` は、その作業フォルダ内に
置く必要はなく、次の順で探索されます:

1. **作業フォルダ内** — `<作業フォルダ>/ptygrid.yml`（無ければ旧名 `<作業フォルダ>/mterm.yml`。
   旧名の互換読み込みは作業フォルダ内のみ）
2. **アプリ起動フォルダ** — ptygrid を起動したフォルダ（`npm run tauri dev` を実行した場所など）の
   `ptygrid.yml`
3. **グローバル設定** — `~/.ptygrid/ptygrid.yml`

最初に見つかったファイルを読み込みます（両方ある場合、作業フォルダ内の `ptygrid.yml` が最優先）。
読み込み後、読み込みボタンの隣に**どこから読んだか**（`設定: プロジェクト内 / 起動フォルダ /
~/.ptygrid / 既定`）を示すバッジが出ます（hover で実際のパスと作業フォルダを表示）。作業フォルダは
cwd 解決・Git パネル・Queen の project scope・セッション復元の基準となる**プロジェクト境界**で、
設定ファイルをどこから読んでも常に作業フォルダが使われます。

**読み込み = cd と同じ動き**: 「読み込み」を押して成功すると、開いているシェルのペインが指定した
作業フォルダへ自動で `cd` します（`cd '<作業フォルダ>'` を送信）。対象は実行中の**シェルのペインだけ**で
（`kind` が pty・状態が running・フォアグラウンドが sh/bash/zsh/fish 等。フォアグラウンド名が取れない
ペインはシェル扱い）、実行中の CLI ペインや transcript(読み取り専用)ペインには送りません。送信後は
「作業フォルダ: … / N ペインに cd を送信」とトーストが出ます。ペインが無い／すべて CLI 実行中でも
エラーにはなりません。

**設定ファイルが無くても開けます**: 3か所いずれにも `ptygrid.yml` が無い場合でも、「読み込み」は
エラーにならず**組み込みの既定設定**（エージェント定義なし・Queen 有効）で成功し、バッジは
`設定: 既定` になります。この状態でも作業フォルダへの cd は行われます。後から
`<作業フォルダ>/ptygrid.yml` を作成すると監視によって検出され、「Reload」トーストから読み込めます
（作成した定義のチップがツールバーに並びます）。

複数プロジェクトで共通の定義を使い回したい場合は `~/.ptygrid/ptygrid.yml` に置き、作業フォルダだけを
切り替えれば同じ設定で別フォルダを対象にできます。

### 信頼確認（未確認フォルダの自動起動ガード）

`ptygrid.yml` は `cmd` / `resume` / `worktree.setup` でコマンドを実行できます。他人のリポジトリ内の
`ptygrid.yml`（作業フォルダ／起動フォルダ由来 = バッジ `プロジェクト内` / `起動フォルダ`）を初めて
読み込んだときは、意図しないコマンドが自動起動しないよう、**`autostart: true` の定義は自動起動を
保留**し、次の確認バナーを出します。

> このフォルダ（&lt;作業フォルダ&gt;）の設定は未確認です。定義されたコマンドを自動起動しますか？

- **「信頼して起動」** を押すとそのフォルダを信頼済みとして記憶し（app-data の
  `trusted-folders.json`）、保留していた `autostart` の定義を起動します。以後、同じフォルダでは
  確認は出ません。
- **「後で」** を押すと何も自動起動しません。設定内容の閲覧やペイン表示は通常どおり行え、
  エージェントチップの ▶ からの**手動起動は確認なしで可能**です（手動操作はゲート対象外）。
- 自分のグローバル設定 `~/.ptygrid/ptygrid.yml`（バッジ `~/.ptygrid`）と、設定ファイルが無いときの
  組み込み既定（バッジ `既定`）は**常に信頼済み**扱いで、確認は出ません。

読み込んだファイルは監視されており（グローバル設定なら `~/.ptygrid`、起動フォルダ設定ならその
フォルダを監視）、変更すると「Reload」トーストから再読込できます。
サンプル: [ptygrid.example.yml](../ptygrid.example.yml)(注釈付き) / [example/](../example/README.md)(用途別)

```yaml
project: my-app

queen:            # 省略可(丸ごと省略でデフォルト動作)
  enabled: true   # デフォルト true。false で Queen を停止
  port: 39237     # デフォルト 39237。使用中なら +1 を 39246 まで試す

agents:           # 対話型 AI CLI
  - name: claude
    cmd: "claude"
    cwd: "."                                   # ptygrid.yml のあるディレクトリ基準の相対パス可
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
| `.cwd` | - | ptygrid.yml の場所 | 作業ディレクトリ。相対パスは ptygrid.yml 基準で解決 |
| `.env` | - | - | 環境変数。値の `${VAR}` はホスト環境から展開(未定義は空文字) |
| `.autostart` | - | `false` | 設定読込時に自動起動 |
| `.autorestart` | - | `never` | `never` / `on-failure` / `always`。連続5回失敗で打ち切り |
| `.resume` | - | `.cmd` | アプリ再起動後のlogical resume時に使うcommand |
| `.worktree.enabled` | - | `false` | 定義の起動ごとにlinked worktreeと専用branchを作る |
| `.worktree.base` | - | `HEAD` | worktree branchの起点となるbranch/tag/commit |
| `.worktree.setup` | - | - | worktree作成後、agent cwdで一度だけ実行するsetup command |

> すべてのセッションには環境変数 `QUEEN_URL`(例: `http://127.0.0.1:39237/mcp?token=<token>`)が
> 注入されます(認証トークン込み)。ペイン内で接続先を確認したいときは `echo $QUEEN_URL` を
> 実行してください。

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
自動保存します。次回起動時に現在の`ptygrid.yml`を読み直し、設定定義を新しいPTYとして
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

## エージェント状態バッジ(agent_status)

ptygrid は、動作中(running)のペインの端末出力から**エージェントの意味的状態**を推定します。
これはプロセスの生死(起動中/実行中/終了)を表す既存の状態ドットとは**別レイヤ**の「推定」で、
生きている PTY の上に重ねて表示します(状態ドットは上書きしません)。

- 🔴 **blocked** — 承認・入力待ちで停止(既知の承認/権限/選択 UI にマッチしたときだけ。誤検出を
  避けるため保守的に判定します)。
- 🟡 **working** — 実行中(`esc to interrupt`、`Thinking` など)。
- 🔵 **done** — 直近の作業が完了した直後(数秒後に自動で idle へ減衰)。
- 🟢 **idle** — 生きているが待機中(どのパターンにもマッチしない)。
- ⚪ **unknown** — 状態を推定するルールセットが無い(バッジ非表示)。

検出は内蔵の既定パターン(`claude` / `codex` / `grok` / `aider`)を出発点に、各ペインの agent 定義名
またはフォアグラウンドプロセス名でルールセットを選びます。手打ちで起動した `claude` / `codex` も
フォアグラウンド名で拾われます。**内蔵パターンは各 CLI の UI 変更で古くなりうる**ため、必要に応じて
`ptygrid.yml` で上書きしてください(変更は config reload で即反映)。

```yaml
agent_status:
  enabled: true          # 既定 true。false で検出を停止
  tail_lines: 24         # 検出に使う末尾行数(4..200)
  debounce_ms: 250       # 評価間隔(100..2000)。バースト出力でも過負荷になりません
  done_linger_ms: 6000   # done を保持してから idle へ減衰(0..60000、0 で done を使わない)
  patterns:
    claude:              # 既定は内蔵ルールへ「追記(merge)」
      blocked:
        - 'Do you want to proceed\?'
      working:
        - 'esc to interrupt'
    codex:
      replace: true      # 内蔵を捨てて完全置換
      blocked:
        - '\[y/N\]'
    "*":                 # 未割当ペインにも当てたい場合のみ定義する generic ルール(opt-in)
      blocked:
        - '\[y/N\]\s*$'
```

パターンは既定で大文字小文字を区別せず、複数行の部分一致で評価します(`(?-i)` などのインライン
フラグで個別に上書き可)。不正な正規表現は**その 1 本だけ**スキップされ、他のパターンは有効なままです。

> 注: バッジ UI 自体は本リリースのヘッダー表示から段階的に拡充します(状態一覧サイドバー・
> 承認待ち通知は後続)。`agent_status` の設定は今のリリースから有効です。

## Queen のセットアップ

Queen はアプリ内に常駐する MCP サーバーです(streamable HTTP、bind は 127.0.0.1 のみ)。
各エージェント CLI に MCP サーバーとして登録すると、そのエージェントが
[18個のツール](#queen-ツールリファレンス)を使えるようになります。

> 🔑 **認証トークンについて（重要）**
> Queen は 127.0.0.1 限定ですが、同一ホストの別プロセスや DNS リバインディングした Web ページ
> からの不正アクセスを防ぐため、`/mcp` は**認証トークン + Host/Origin 検証**で保護されています。
> 登録 URL には `?token=<トークン>` が付きます。
> **このトークンは app-data に保存され、アプリを再起動しても変わりません。登録は初回のみで
> OK です。** トークンを再生成したときだけ再登録が必要です(Teammates パネルの
> 「Queen トークン再生成」で、漏洩時のローテーションができます)。実際の URL は必ず
> ツールバーの「● Queen」バッジをクリックしてコピーしてください(下記コマンドの `<token>` は
> プレースホルダです)。

### Claude Code

```bash
# <token> と <port> はバッジのコピーで実値に置き換わります
claude mcp add -s user --transport http queen "http://127.0.0.1:39237/mcp?token=<token>"
```

> ⚠️ **`-s user` を必ず付けてください。** デフォルトの local スコープは「コマンドを実行した
> ディレクトリ限定」の登録になるため、ペインの作業ディレクトリと違う場所で登録すると
> Claude Code から Queen が見えません(実例あり)。プロジェクト単位で共有したい場合は
> `-s project`(`.mcp.json` がリポジトリに作られる)も使えます。
>
> トークンは再起動後も有効なので、通常は再登録不要です。トークンを再生成した場合だけ
> `claude mcp remove queen` してから登録し直すか、上書き登録します。

### Codex CLI

`~/.codex/config.toml` に追記(URL に token を含める):

```toml
[mcp_servers.queen]
url = "http://127.0.0.1:39237/mcp?token=<token>"
```

### Grok CLI

```bash
grok mcp add -s user -t http queen "http://127.0.0.1:39237/mcp?token=<token>"
grok mcp doctor    # 接続確認(handshake OK / 18 tools discovered が出れば成功)
```

> ℹ️ トークンは URL クエリで渡すため、CLI 側で `--header` などの追加設定は不要です。
> どうしてもヘッダで渡したい場合は `Authorization: Bearer <token>` も受理されます。

### ポートについて

39237 が使用中の場合、Queen は自動で +1 を 39246 まで試します。フォールバックした場合は
登録 URL の読み替えが必要です(ツールバーのバッジに実際のポートが表示されます)。
固定したい場合は `ptygrid.yml` の `queen.port` を指定してください。

## Teammates(hooks 受信)

ツールバー右側の **Teammates バッジ**は、Claude Code 等が発火する teammate ライフサイクル
hook(サブエージェントの起動/停止、アイドル、タスク作成/完了)を受け取るための入口です。
受信 endpoint は Queen と同じ 127.0.0.1 サーバー上の `/hooks/v1/*` で、`Authorization:
Bearer <token>` 必須・ノンブロッキング(常に `200 {"decision":"allow"}`)です。

### 有効化

`ptygrid.yml` にグローバル `teammates:` ブロックを追加します(すべて任意):

```yaml
teammates:
  enabled: true             # default false。true で hook の受信通知を有効化
  hook_notifications: true  # default true。受信時のトースト可否
  global_max_panes: 6       # default 6(1..9)。Phase 4.1 で使用
  hooks_scope: user         # "user" | "project"。default "user"
```

`enabled: false`(デフォルト)の間も token 検証は行いますが、イベント通知は出しません。
バッジは有効なら緑、無効ならグレーで表示されます。

### hooks の登録

バッジをクリックすると設定パネルが開きます:

- **スニペットをコピー**: token を埋め込んだ hooks 定義 JSON をクリップボードへコピーします。
  Claude Code の `settings.json` の `hooks` に貼り付けてください。
- **settings.json へ登録 (user)**: `~/.claude/settings.json` へ hooks 定義を自動マージします
  (既存内容は保持、書込前に `settings.json.ptygrid-backup-<unix秒>` を作成、同一内容なら
  書き込みません)。
- **hook トークン再生成 / Queen トークン再生成**: 漏洩時のローテーション用。対象トークンを
  再生成し、実行中の認証層へ即時反映します(Queen サーバの再起動は不要)。再生成後は
  settings.json / MCP の登録が古いトークンのままになるため、再登録が必要です(パネルが
  通知します)。
- **直近のイベント**: 受信した teammate-lifecycle を最新10件まで表示します。

> ✅ token は app-data(`auth-tokens.json`、Unix では権限 0600)に保存され、アプリを再起動しても
> 変わりません。登録は**初回のみ**でOKです。トークンを再生成したときだけ、スニペットの再コピー
> または settings.json への再登録が必要です。

### observe: read-only transcript ペイン(Phase 4.1)

lead(親エージェント)が subagent を起動したとき、その transcript を **読み取り専用ペイン**として
自動追加できます。有効化は lead の定義に `teams:` ブロックを足すだけです:

```yaml
teammates:
  enabled: true       # グローバルの有効化(上記)も必要
agents:
  - name: claude
    cmd: claude
    cwd: "."
    teams:
      enabled: true         # この lead で transcript ペイン化を行う
      mode: observe         # observe | host(host は Phase 4.2 で実 PTY 化。下記参照)
      max_panes: 3          # この lead が生む transcript ペインの上限(default 3)
      transcript_tail: true # false なら通知だけでペインは作らない(default true)
```

使い方と挙動:

- Claude Code の `SubagentStart` hook を受けると、`~/.claude/` 配下の subagent transcript を
  tail する `claude·sub #<id> ▸<役割> 📖RO` ペインが増えます。親 lead は `↳#<id>` で併記されます。
- ペインは **読み取り専用**です。xterm ではなくスクロールビューで、`role: text` を時系列表示し、
  ツール呼び出しは1行に要約します。入力はできません(Queen の `send_message` も拒否されます)。
- 状態ドットは active(実行中)/ stopped(subagent 終了)。`SubagentStop` を受けると stopped になり、
  ペインは残ります(自分で閉じるまで最終状態を表示)。
- ペインを **閉じても subagent には影響しません**(tail を止めるだけです)。restart はできません。
- 上限(lead ごとの `max_panes`、全体の `teammates.global_max_panes`、グリッド9面)を超えると、
  ペインは作らず日本語バナーで通知します。
- 安全のため、tail するのは `$HOME/.claude/` 配下の絶対パスのみです。それ以外や path 不明の場合は
  ステータス表示のみになります。transcript セッションはセッション復元(resume)の対象外です。
- **起動方法**: ▶ チップからの起動が確実(名前付き lead として `teams:` 設定がそのまま効く)ですが、
  shell ペインで **手打ちした `claude`** でも observe は動きます。`teams.enabled` の名前付き lead が
  1つも無いとき、フォアグラウンドが `claude`(既定。`teammates.teammate_binaries` で変更可)の
  running ペインを **暗黙の observe lead** として拾います(グローバル `teammates.enabled: true` が前提。
  observe 専用で host にはなりません)。名前付き lead があればそちらが優先されます。
- lead に帰属できずペインを作れなかったときは、`teammates.enabled: true` なら
  「サブエージェントを検知したが teams 有効な lead が見つからない」旨をバナー通知します
  (▶ チップからの起動 or `teammates.enabled` の確認を促します)。

### host: 実 PTY teammate ペイン(Phase 4.2・実験機能・既定オフ)

`mode: host` にすると、Claude Code の split-pane teammate(独立した `claude` プロセス)を
ptygrid の **ネイティブな対話 PTY ペイン**としてホストします。read-only の observe と違い、
teammate ペインに直接キー入力でき、resize・スクロールバック・Queen 接続まで通常ペインと
同等に扱えます。**opt-in の実験機能で、既定はオフ**です。

```yaml
teammates:
  enabled: true             # 注: host は per-agent opt-in のためグローバル enabled には依存しません
agents:
  - name: claude
    cmd: claude
    cwd: "."
    teams:
      enabled: true
      mode: host                 # observe | host。host で実 PTY ホスト
      max_panes: 3               # この lead の teammate ペイン上限(1..9)
      teammate_binaries:         # split-window で PTY 起動を許可する argv0 basename(default ["claude"])
        - claude
      fallback_to_observe: true  # host 未使用時に observe へ自動降格(default true)
```

有効化と仕組み:

- 有効化条件は `enabled: true` **かつ** `mode: host` の lead のみです。opt-in が無ければ、env 注入も
  socket サーバ起動もシム配置も **一切行いません**。host は Unix 専用です(Windows では通常セッション
  として起動)。
- lead 起動時、ptygrid が **tmux 互換シムと per-lead の Unix socket サーバを自動配置**し、必要な
  環境変数(`TMUX` / `TMUX_PANE` / `PTYGRID_TEAMS_SOCK` / `PTYGRID_TEAMS_TOKEN` / `PATH` 先頭へ
  シム追加)を lead PTY に自動注入します。**`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` も ptygrid が
  自動注入する**ので、ユーザーが手動で設定する必要はありません。設定は config-as-code が原則で、
  UI からの一時有効化は行いません。
- Claude Code が teammate を split-window で起動すると、`claude·team #<id> ▸<役割>` ヘッダーの
  対話 PTY ペインが増えます。親 lead は `↳#<id>` で併記されます。状態ドットは通常 PTY と同じ
  running / exited(+ exit code)です。⟳再起動・⤢最大化ができます。
- teammate ペインの **閉じるは実プロセスの kill(破壊的操作)**なので、確認(「teammate を停止
  しますか？」)を挟みます。
- **フォールバック**: teammate 検知から 2 秒以内にシム経由の split-window RPC が来ない場合、
  Claude Code が in-process にフォールバックした(シムが使われなかった)と判断します。
  `fallback_to_observe: true` なら自動で observe(read-only transcript ペイン)へ降格し、
  トーストで通知します。この間 Teammates バッジは「host: フォールバック中」を表示します。
- **上限超過**: `teams.max_panes` / `teammates.global_max_panes` / グリッド9面のいずれかに達しても、
  host では teammate セッション自体は生成します(作業を止めない)。ただしグリッドには載せず paneless
  とし、日本語バナーで通知します。Teammates パネルの一覧から「グリッドへ表示」で昇格できます。
- **孤立 teammate**: lead が終了すると、その host teammate PTY は孤立しうります。Teammates パネルは
  「lead 終了済み(孤立 teammate)」として列挙し、「停止」ボタンで掃除できます。
- teammate spawn は Queen の allowlist(`spawn_agent`)を経由せず、(1) config の opt-in、
  (2) socket トークンのハンドシェイク、(3) `teammate_binaries`(既定 `["claude"]`)の argv0 basename
  検証の3段で保護されます。teammate セッションはセッション復元(resume)の対象外です。

## Queen ツールリファレンス

| ツール | 引数 | 説明 |
|---|---|---|
| `list_agents` | なし | 実行中セッションと ptygrid.yml 定義の一覧(状態・フォアグラウンドプロセス名付き) |
| `read_output` | `agent`, `lines?`(default 100, 1..1000), `raw?`(default false) | 指定ペインの直近出力。デフォルトでペイン寸法に合わせてANSIカーソル移動・画面消去・alternate screenを再構成。`raw: true`で生出力 |
| `send_message` | `agent`, `text`, `submit?`(default true) | 指定ペインの stdin へ書き込み。`submit: true` で末尾に Enter を付与 |
| `spawn_agent` | `name` | **ptygrid.yml で定義された名前のみ**起動可(許可リスト方式) |
| `spawn_team` | `preset` | `team_presets:` で宣言したチームを一括起動(詳細は[チームプリセット](#チームプリセットteam_presets))。起動レポートを返す |
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
2. **ptygrid.yml の定義名 / session名** — 完全一致し、実行中の候補が1つだけの場合
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

PinsとNotesは読み込んだ`ptygrid.yml`のdirectory単位で分離され、app-data内のSQLiteへ
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

## チームプリセット(team_presets)

複数エージェントの**名前付きチーム構成**を `ptygrid.yml` に宣言し、1操作で一括起動します
(Phase 4.3)。メンバーは `agents:` 定義の**参照のみ**なので、`spawn_agent` の許可リストで
起動できないものはチームでも起動できません。

```yaml
team_presets:
  daily:                        # プリセット名(ツールバーの 👥 チップに出る)
    lead: local                 # 任意: kickoff の宛先。省略時は最初の非 standby メンバー
    members:
      - agent: local            # agents: の定義名への参照のみ(processes: は不可)
        instructions: >-        # 任意: 起動時に inbox へ配送される役割指示
          一次担当。詰まったら spawn_agent で opus を起動し、inbox で依頼する。
      - agent: opus
        standby: true           # 任意(default false): チーム起動時は立ち上げない待機層
        instructions: "難問のみ担当。"
      - agent: grok
        standby: true
    kickoff: "pins のタスク一覧を読んで着手して。"   # 任意: 起動後 lead へ投函
```

### 起動のしかた

- **ツールバー**: 設定に `team_presets:` があると 👥 チップが並びます。▶ で一括起動し、
  結果(起動 / 既存 / 失敗 / 待機の件数)がトーストで出ます。
- **Queen tool**: エージェント自身も `spawn_team {preset: "daily"}` でチームを組めます。
  どちらも同じ backend 処理で、レポート(JSON)を返します。

### 起動の動き

- 非 standby メンバーを**宣言順に逐次起動**します。既に同名のセッションが生きている
  メンバーは**二重起動せずスキップ**されるので、👥 を何度押しても安全です(冪等)。
- ペイン上限(9面)に達した分は起動されず「失敗(pane limit)」として報告されます
  (部分起動。起動できた分はそのまま使えます)。
- `instructions` と `kickoff` は **Queen の永続 inbox** に配送されます(宛先 mailbox =
  定義名、送信者 = `queen:preset/<プリセット名>`)。standby メンバーの指示も配送される
  ため、後から起動しても `list_inbox` で自分の mailbox を読めば役割が分かります。
  配送が起きるのは**この操作で実際に1体以上起動したときだけ**なので、稼働中のチームに
  もう一度 👥 を押しても指示や kickoff は再送されません。

### 検証エラーになる書き方

`team_presets:` はロード時に検証され、次はエラーになります: `agents:` に無い名前の参照
(`processes:` は不可)、members が空 / 全員 standby、standby メンバーを `lead` に指定、
同一プリセット内での同じ agent の重複宣言。

### 想定パターン: ローカルLLM主体 + クラウド待機(コスト階層型)

「普段はローカルLLM、難しい問題だけ Claude Opus / Grok」を1クリックで再現できます。
ローカル側は Claude Code CLI のまま claude-code-router(coderouter)経由で
llama.cpp / ollama に向け(ルーティングは**プロセス単位の env** で決まるので、
`agents[].env` に `ANTHROPIC_BASE_URL` を書くだけ)、クラウド勢を `standby: true` で
宣言します。エスカレーションは機構ではなく **instructions の規約**です。ただし
「難しいと感じたら」のような**自己判断のトリガーは書かない**でください — ローカルモデルは
難問にも自信を持って普通に答えてしまい、発火しません。**客観条件**で書きます:

> 一次担当への指示例: 「次のどれかに当てはまったら必ずエスカレーションする:
> ①テスト/ビルドが同じ原因で2回連続失敗 ②公開API・保存データ・セキュリティ境界に
> 触る変更（この場合は完了前に opus のレビュー必須） ③人間に『opusに聞いて』と
> 言われた。手順: spawn_agent で "opus" を起動 → inbox で要約と試したことを送る →
> await で回答を待って反映。」

完全なサンプルは [example/team-preset/ptygrid.yml](../example/team-preset/ptygrid.yml) を
参照してください。CLI が同じ `claude` でも ptygrid は定義名でペインを区別し、Queen の
MCP 登録(`-s user`)も1回で全ペインに効きます。

> [!WARNING]
> **settings.json との干渉**: Claude Code は `agents[].env` で渡した環境変数のほかに
> `~/.claude/settings.json`(user) / `.claude/settings.json`(project) の `env` ブロックも
> 読み、**バージョンによっては settings 側がプロセス env に勝ちます**。ローカル向けの
> ルーティングを確実に効かせるには、local エージェントの cmd を
> `claude --settings router.settings.json` のように **per-agent の `--settings`**
> (CLI 引数スコープ = project/user settings より上位)にしてください。プロジェクト直下の
> `.claude/settings.json` に base URL を書く方法は、**同じ作業フォルダで動くクラウド側の
> ペインにも効いてしまう**ため使わないでください。設定ファイル例と着弾確認の手順は
> [verify-team-preset.md](verify-team-preset.md) の A-2b / R1 を参照。

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
| 認証トークン | `auth-tokens.json` | Queen `/mcp` トークンと hook Bearer トークン(version付き、Unix権限0600)。再起動後も有効 |

terminal出力、展開後の環境変数、`QUEEN_URL`、起動command本文はsession stateへ保存しません。
Queenはlocalhost (`127.0.0.1`) のみにbindし、`spawn_agent`は読み込んだ`ptygrid.yml`の
定義名だけを許可します。認証はないため、信頼できないlocal processが動く環境ではQueenを
`queen.enabled: false`で無効化してください。

## 困ったときは

- 実際のドッグフーディングで判明した罠(登録スコープ、サンドボックスの localhost 制限、
  TUI 出力の読み方、composer 二重入力など)は [troubleshooting.md](troubleshooting.md) にまとまっています。
- 設計の背景・アーキテクチャは [design.md](design.md) を参照してください。
