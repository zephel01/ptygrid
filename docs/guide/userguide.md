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
7. [設定ファイルの自動生成(ptygrid init)](#設定ファイルの自動生成ptygrid-init)
8. [Worktree 分離](#worktree-分離)
9. [セッション復元](#セッション復元)
10. [エージェント状態バッジ(agent_status)](#エージェント状態バッジagent_status)
11. [接続コンテキスト表示(pane_context)](#接続コンテキスト表示pane_context)
12. [Queen のセットアップ](#queen-のセットアップ)
13. [Teammates(hooks 受信)](#teammateshooks-受信)
14. [Queen ツールリファレンス](#queen-ツールリファレンス)
15. [チームプリセット(team_presets)](#チームプリセットteam_presets)
16. [ワークフロー(workflows)](#ワークフローworkflows)
17. [スケジュール実行(schedule)と外部通知(notifications)](#スケジュール実行scheduleと外部通知notifications)
18. [実践レシピ: エージェント間協調](#実践レシピ-エージェント間協調)
19. [保存データと安全性](#保存データと安全性)
20. [困ったときは](#困ったときは)

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

### コピー & ペースト

ペイン内のテキストはドラッグで範囲選択し、キーボードまたは右クリックメニューで
コピー & ペーストできます。

| 操作 | macOS | Linux / Windows |
|---|---|---|
| コピー | Cmd+C(選択があるときだけ) | Ctrl+Shift+C |
| 貼り付け | Cmd+V | Ctrl+Shift+V |

- 選択が無いときの Ctrl+C はこれまでどおり PTY へ届きます(SIGINT を奪いません)。
- 右クリックで「コピー / 貼り付け」メニューが出ます。選択が無いときはコピーが
  無効表示になり、ツールチップが選択のしかたを案内します。
- 貼り付けは bracketed paste 経由なので、対応シェルでは複数行を貼っても
  **Enter を押すまで実行されません**(貼った瞬間に走り出しません)。
- vim や対話型エージェント CLI などの TUI がマウスを握っている間は、
  **macOS では Option+ドラッグ、Linux / Windows では Shift+ドラッグ**で選択できます
  (この差は端末エミュレータ(xterm.js)側の仕様で、設定では揃えられません)。

> 実機確認は macOS のみ(ペインをまたいだコピー & 貼り付けと右クリックメニュー)。
> Linux / Windows のキー割り当ては実装済みですが実機未検証です。

### スクロール(作業ログの読み返し)

| 操作 | macOS | Linux / Windows |
|---|---|---|
| スクロール | ホイール / トラックパッド | ホイール |
| 1 ページ上 / 下 | Cmd+↑ / Cmd+↓(Shift+PageUp / PageDown も可) | Ctrl+Shift+↑ / ↓(Shift+PageUp / PageDown も可) |
| 先頭 / 最新 | Cmd+Home / Cmd+End(fn+Cmd+← / →) | Ctrl+Shift+Home / End |

- 上に遡っている間はペイン右下に「↓ 最新へ」ボタンが出ます。キー入力しても最新に戻ります。
- 遡れるのはペインごとに最大 5000 行です。
- **スクロールバック保護**: `clear` や Claude Code が送る「スクロールバック消去」
  (`ESC[3J`)は既定で無視し、過去ログを残します。画面そのもののクリアは通常どおり動きます。
  元の動作(消去を受け付ける)に戻すには、開発者ツールのコンソールで
  `localStorage.setItem("ptygrid.preserveScrollback", "0")` を実行してから再読み込みします。
- **代替画面では xterm のスクロールバックが使えません**: tmux / vim / 全画面型の TUI は代替画面に
  描画するため、ペイン側には履歴が溜まりません。この状態でホイールを回すと、端末の仕様により
  ↑/↓ キーとして届きます(シェルの履歴が次々出る)。tmux の場合は `set -g mouse on`
  (`~/.tmux.conf`)でホイールが tmux の copy-mode に入るようになります。ssh の `.ssh` 定義の
  ペインでは ptygrid が自動で設定します([ssh 接続の永続化と再接続](#ssh-接続の永続化と再接続))。
- **残った画面モードの自動復帰**: tmux / vim / Claude Code などが強制終了したり、手打ちの
  `ssh` の先で動いていた tmux が回線断で切れたりすると、ペインが代替画面のまま残り、シェルに
  戻ってもホイールで履歴が回るだけになります(手動なら `tput rmcup` か `reset` で直ります)。
  ptygrid は次の時点で、ペインの表示側だけを元に戻します(実行中のプログラムには何も送りません)。
  - プロセス終了時・再起動時・`.ssh` 定義の再接続時: 代替画面・マウス報告・入力モードを解除
  - フォアグラウンドがプログラムからシェル(zsh / bash / fish など)に戻ったとき: 代替画面か
    マウス報告が残っている場合だけ解除し、ペインに「画面モードを元に戻しました」と表示します。
    このときプロンプトが消えて見えることがあるので Enter で再表示してください。

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
サンプル: [ptygrid.example.yml](../../ptygrid.example.yml)(注釈付き) / [example/](../../example/README.md)(用途別)

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
| `.ssh.persist` | - | `tmux` | `.cmd` が `ssh …` の定義で、接続先のプロセスを `tmux` / `screen` セッション内に置く(`none` = keepalive のみ)。[ssh 接続の永続化と再接続](#ssh-接続の永続化と再接続) |
| `.ssh.session` | - | `ptygrid-<name>`(2 個目以降 `-2`, `-3`…) | 接続先の tmux / screen セッション名(`[A-Za-z0-9_-]` と `{n}`)。`{n}` は同じ定義の何個目かに置換。`{n}` 無しの固定名は全ペインが同じセッションに入る(鏡写し) |
| `.ssh.remote_cmd` | - | ログインシェル | セッション内で実行するコマンド(例 `claude --continue`)。`.cmd` の宛先の後ろに書いたコマンドでも可(両方は不可) |
| `.ssh.reconnect` | - | `true` | 接続断(ssh exit 255)で自動再接続 |
| `.ssh.keepalive` | - | `15` | `ServerAliveInterval` 秒(`CountMax` は 3 固定 → 約 45 秒で切断検知) |
| `.ssh.max_reconnects` | - | `0` | 連続再接続の上限。`0` = 無制限。安定して接続していた後の切断はカウントをリセット |
| `.ssh.mouse` | - | `true` | tmux のみ。そのセッションに `set-option mouse on` を掛け、ホイールで tmux の履歴をスクロールできるようにする(`false` で付けない) |

> すべてのセッションには環境変数 `QUEEN_URL`(例: `http://127.0.0.1:39237/mcp?token=<token>`)が
> 注入されます(認証トークン込み)。ペイン内で接続先を確認したいときは `echo $QUEEN_URL` を
> 実行してください。

## 設定ファイルの自動生成(ptygrid init)

`ptygrid.yml` を手書きしなくても、作業フォルダと環境を走査して**意味のある設定を
コメント付きで生成**できます(Phase 5.0.2)。入口は2つあります:

1. **ツールバーの「設定を作る」ボタン** — 設定ファイルが見つからない(バッジが
   `設定: 既定`)ときだけ表示されます。設定の無いフォルダを読み込んだ直後がこの状態です。
2. **⚙ 設定メニューの「設定ファイル: 設定を作る」** — フッター右端の ⚙ から。
   こちらは設定を読み込んだ後でも使えます。

パネルを開くと作業フォルダを走査し、検出結果(PATH 上のエージェント CLI、プロジェクト
種別(cargo / npm / python / go)、git リポジトリかどうか、既存設定の有無)と生成される
YAML のプレビューが表示されます。プレビューは書き込む前に自由に編集できます。

- **書き込まれる内容は必ず設定パーサの検証を通っています。** プレビューを編集した場合も
  書き込み直前にもう一度検証され、通らなければ 1 バイトも書きません。
- **既存の `ptygrid.yml` は上書きしません。** 既にある場合は別名 `ptygrid.init.yml` に
  書き(この名前は設定探索の対象外なので、読み込みには影響しません)、内容を見比べてから
  自分で取り込みます。旧名 `mterm.yml` だけが残っているフォルダへの書き込みは拒否されます
  (新しい `ptygrid.yml` が旧設定を黙って無効化しないため)。
- 生成される定義はすべて `autostart: false` なので、書き込んだ直後に何かが勝手に起動する
  ことはなく、[信頼確認](#信頼確認未確認フォルダの自動起動ガード)も出ません。プレビューを
  `autostart: true` に編集して書き込んだ場合だけ、次の読み込みで信頼確認が出ます。

### ローカル LLM プローブ

パネルの「**探す**」ボタンで、ローカル LLM サーバが動いているかを問い合わせられます。

- 対象は `127.0.0.1` の既定 3 ポート(11434 = Ollama / 1234 = LM Studio /
  3456 = claude-code-router)+ 手入力の追加ポート最大 4 つ。レンジスキャンはしません。
  ディスクには何も書きません。
- 応答したサーバのうち、**Anthropic Messages API 互換の確証が取れたもの**(現状は
  version 0.14.0 以上の Ollama)は「Anthropic API 確証あり」と表示され、生成 YAML に
  `agents:` の有効な定義(`ANTHROPIC_BASE_URL` などの `env` 付き、検出モデルの選択可)
  として載ります。確証が取れないもの(OpenAI 互換の応答があっただけのサーバ等)は
  コメントアウトされた形で載り、注記が付きます。

> 生成フロー一式(検出・生成・sidecar・信頼確認との連動)と Ollama の検出は macOS 実機で
> 確認済みです。プローブが生成した定義で実際に CLI が起動するところまでは未確認です。

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

## 接続コンテキスト表示(pane_context)

各ペインの**向き先**を、ヘッダーと画面下部のステータスバーに常時表示します。既定で有効で、
設定は不要です。

```
 ● ▪ claude #2   ⇄ deploy@web-01   ☁ prod-admin · ap-northeast-1   ◍ qwen3-coder   ⑂ main   ▸ …/project/ptygrid
```

| チップ | 内容 |
|---|---|
| `⇄` | `ssh` / `kubectl` などの接続先 |
| `☁` | AWS のプロファイルとリージョン |
| `◍` | エージェント CLI が話している LLM のモデル / エンドポイント |
| `⑂` | git ブランチ（detached HEAD は `@a1b2c3d`） |
| `▸` | 作業ディレクトリ |

並び順は「実行結果を変える度合いの高い順」で、ペインが狭いときは右から欠けます。
ステータスバーには**最後にフォーカスしたペイン**の内容が省略なしで出ます。

止めたい・追従を速くしたいときだけ `ptygrid.yml` に書きます。

```yaml
pane_context:
  enabled: true       # 既定 true。false で表示とサンプリングを停止
  interval_ms: 5000   # 既定 5000、1000〜60000 にクランプ
```

読み取る環境変数は固定の許可リスト（`AWS_PROFILE` / `AWS_REGION` / `ANTHROPIC_BASE_URL` /
`ANTHROPIC_MODEL` ほか計 10 個）だけで、名前が `*_KEY` / `*_TOKEN` / `*_SECRET` などの
資格情報らしいキーは許可リストに載っていても読みません。

> 注意: OS はプロセスの環境変数を**起動時の値**しか公開しないため、direnv などで
> 後から export した値は、そのペインで何かコマンドを実行した時点から表示されます。
> 作業ディレクトリとブランチにこの遅延はありません。

詳細・レシピ・切り分け手順は [pane-context.md](pane-context.md) を参照してください。

## Queen のセットアップ

Queen はアプリ内に常駐する MCP サーバーです(streamable HTTP、bind は 127.0.0.1 のみ)。
各エージェント CLI に MCP サーバーとして登録すると、そのエージェントが
[22個のツール](#queen-ツールリファレンス)を使えるようになります。

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
grok mcp doctor    # 接続確認(handshake OK / 22 tools discovered が出れば成功)
```

> ℹ️ トークンは URL クエリで渡すため、CLI 側で `--header` などの追加設定は不要です。
> どうしてもヘッダで渡したい場合は `Authorization: Bearer <token>` も受理されます。

### ポートについて

39237 が使用中の場合、Queen は自動で +1 を 39246 まで試します。フォールバックした場合は
登録 URL の読み替えが必要です(ツールバーのバッジに実際のポートが表示されます)。
固定したい場合は `ptygrid.yml` の `queen.port` を指定してください。

### MCP 対応バージョン

Queen は MCP プロトコルの複数バージョンを段階的にサポートします。patch 5.5.0(現行リリース)
より、以下の `mcp:` ブロックは実際のリクエスト処理に接続済みです。

```yaml
mcp:
  rc_2026_07_28: true      # default true。2026-07-28 RC 経路(Mcp-Method/Mcp-Name ヘッダ)の受理
  legacy_2025_06: true     # default true。2025-06 旧経路の受理(廃止予定期間中)
  max_body_bytes: 1048576  # default 1 MiB。compat router のボディ上限(超過は 413)
```

> ℹ️ RC 経路では `Mcp-Method`(body の `method` と完全一致)、`tools/*` 呼び出し時は
> `Mcp-Name`(body の `params.name` と完全一致)も検証され、不一致は 400 を返します。
> `initialize` は RC 経路では `protocolVersion: "2026-07-28"` の場合のみ no-op 200
> (不一致は 400)。`legacy_2025_06` 経路の応答には Deprecation / Sunset / Link ヘッダが
> 付与されます。既存の各 MCP ツールの入出力(リクエスト/レスポンス形式)は無変更です。

`mcp:` ブロックには上記3項目に加えて、廃止予定 capability(`sampling/*` / `resources/roots` /
`logging/setLevel`)ごとに no-op 応答を続けるか無効化するかを切り替える `legacy_capabilities`
サブブロックも用意されています。

```yaml
mcp:
  legacy_capabilities:
    sampling: false   # default false。廃止予定 sampling/* は無効(-32601 method_not_found)
    roots: false      # default false。廃止予定 resources/roots は無効(-32601 method_not_found)
    logging: true     # default true。廃止予定 logging/setLevel は 200 no-op のまま維持
```

> ℹ️ こちらも実際の応答分岐に接続済みです。既定値のままであれば挙動は従来通りです。

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
| `spawn_workflow` | `name` | `workflows:` で宣言したワークフローを DAG run として起動(詳細は[ワークフロー](#ワークフローworkflows))。root step の spawn 直後のスナップショットを返す |
| `join_workflow` | `runId`, `timeoutMs?`(default 600000, clamp 1000..3600000) | run が終端(succeeded / failed / cancelled)に達するまで待つ。`{timedOut, run}` を返し、timeout はエラーではない |
| `cancel_workflow` | `runId` | 実行中 run の全ペインを止めて Cancelled にする。終端済み run には冪等(現状スナップショットを返すだけ) |
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

完全なサンプルは [example/team-preset/ptygrid.yml](../../example/team-preset/ptygrid.yml) を
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

## ワークフロー(workflows)

`workflows:` ブロックに step の依存グラフ(DAG)を宣言すると、ptygrid が起動・完了判定・
次段への進行を自動で進めます(Phase 5.0)。[チームプリセット](#チームプリセットteam_presets)が
「全員を一括起動して自由に協調させる」道具なのに対し、workflows は「A が終わってから B、
B が3並列で終わったら C」という**順序と集約を機械に守らせる**道具です。

本章は日常操作の手順と考え方を扱います。フィールドの網羅表・バリデーション規則・
「書けるが動かないフィールド」の線引きは
[ptygrid-yml-guide.md](ptygrid-yml-guide.md)(特に §1 の実装マトリクス)が正です。

### 起動のしかたと観察のしかた

- **ツールバー**: 設定に `workflows:` があると 🔀 チップが並びます。▶ で起動します。
- **左ドックの Workflows タブ**: 宣言済みワークフローの一覧(▶ Run)と、run の履歴
  (state バッジ、step ごとの進行、⏹ Cancel)が見られます。step 行には所要時間と、
  ペイン枠待ちがあれば「待ち」が別々に表示されます(`5.0s(待ち 8.0s)` のように出ます。
  **2つは別物なので足さないでください** — 所要は spawn されてから終わるまで、待ちは
  9面上限が空くのを待った累計です)。
- **Queen tool**: エージェント自身も `spawn_workflow {name: "<ワークフロー名>"}` で起動
  できます。完了待ちは `join_workflow`、停止は `cancel_workflow`
  ([ツールリファレンス](#queen-ツールリファレンス)参照)。
- step が起動できるのは **`agents:` に定義された名前だけ**です(`processes:` は不可。
  `spawn_agent` と同じ許可リスト方式)。

### 動く前提: Queen 登録と「待ち受けループ」

つまずきどころの大半はここです。**step の `kickoff:` は Queen の永続 inbox に配送される
だけで、ペインには何も打ち込まれません。** したがって:

1. **使う CLI に Queen MCP が登録済みであること**([Queen のセットアップ](#queen-のセットアップ))。
   未登録だと 1 段目のペインは開くのに指示を受け取れず、`timeoutMs` まで Running のまま
   止まります。
2. **エージェントの `cmd` に「起動直後に inbox を待ち受けるループ」を埋めること。**
   素の `claude` を起動しても自分から inbox を見にいきません。定型は
   「Queen の `await` を `mailbox=$PTYGRID_MAILBOX` で呼ぶ → 届いた指示に従う →
   `reply_inbox`(`sender=$PTYGRID_MAILBOX`)で結果を返信」です。環境変数
   `PTYGRID_MAILBOX` には、通常の step では agent 定義名、後述の `onEach` のコピーでは
   コピー専用の mailbox 名が入るので、**どの役割でも同じ書き方が使えます**。
   ただし**注入されるのは workflow の step として新規に spawn されたセッションだけ**です
   (2026-08-14 訂正、v0.5.9)。**既存ペインを再利用した step、`autostart` / `spawn_agent` /
   `spawn_team` が立てたペインには入りません**(そこで `$PTYGRID_MAILBOX` を書くと
   シェルが空文字に展開し、`await` は空の mailbox 名を受け取れないのでエラーになります)。
   実務上の落とし穴はこれです: 同名の agent のペインが既に立っている状態で、その agent を
   使う singular な step(`fanOut` の無い step)を走らせると、**新規 spawn ではなく既存ペインの
   再利用になる**ので、そのペインは自分の mailbox 名を知らないまま — つまり 1 つ上の
   「指示を受け取れない」と同じ形で詰まります。**workflow で使う agent は
   `autostart: true` にしない**(既定は `false`)、走らせる前にそのエージェントのペインを
   閉じておく、のどちらかで避けられます。

この2点まで全部埋まった雛形が [example/review-starter/](../../example/review-starter/README.md)
です(実装 → レビュー → ジャッジの3段。`kickoff` の TODO 3 か所を埋めるだけで動きます)。
初めての1本はこれをコピーするのが確実です。

### pattern と dependsOn

`pattern` は pipeline / fan-out / supervisor / handoff の4つです。実行エンジンは実質
pattern を見ておらず(fan-out の並列数展開だけが例外)、**pattern は「この形の DAG しか
書けない」というロード時の検証**として効きます。

| pattern | 形の制約 | 使いどころ |
|---|---|---|
| `pipeline` | 各 step の `dependsOn` は最大1件(線形の鎖。独立した鎖が複数並ぶのは可) | 設計→実装→レビューの直列 |
| `fan-out` | `fanOut: N`(N≥2)の step を1つ以上含む | 同じ定義を N 並列で走らせ `joinOn` で集約 |
| `supervisor` | root がちょうど1つ、**他の全 step が root を `dependsOn` に含む** | 1つの実装を別モデル2体が並行レビューする形など |
| `handoff` | 1本鎖 + `handoffTo` で本文を運ぶ | 返信本文をそのまま次段に渡すリレー |

- `fanOut` は**同じ agent 定義**を複製します。**別モデルを並べたいなら fanOut ではなく
  supervisor の兄弟 step** にします(完全な実例:
  [example/cross-model-review/](../../example/cross-model-review/ptygrid.yml)。supervisor の
  「合流 step の `dependsOn` に root も書く」という直感に反する必須制約もそこで説明されています)。
- 依存の循環・未知 step 参照・`agents:` に無い名前などは**読み込みの時点で**エラーになり、
  設定全体が読み込めなくなります(エラーは該当箇所を名指しします)。

### 完了をどう判定するか(joinOn)

step の完了判定は3経路あります: **(1) PTY が exit code 0 で終了**、**(2) 状態バッジが
done になる**([エージェント状態バッジ](#エージェント状態バッジagent_status)の検出)、
**(3) 自分の kickoff スレッドへの inbox 返信**(`joinOn: reply`)。

対話型 CLI はタスクが終わっても自然終了しないため、**確実なのは (3) の `joinOn: reply`**
です。使うときの前提と注意:

- **同じ step に非空の `kickoff:` が必須**です(無いと読み込みエラー。返信すべきスレッドが
  存在しないため)。
- **最初の返信 = 回答**として扱われます(reply-once)。「了解しました」を先に返すと
  その時点で step が完了し、次の段が始まってしまいます。kickoff とエージェントへの指示に
  「返信は1回だけ・結果を返すこと」を明記してください。
- (2) だけに頼ると、kickoff の無い step が起動直後に「inbox は空です」と応答して
  **何もしていないのに完了扱いになる**事故(空振り done)が起きます。すべての step に
  具体的な kickoff を書くのが原則です。

fan-out の集約は `joinOn: all`(既定。全コピー成功) / `any`(最初の1本) / 数値 `N`
(N 本成功)です。`any` / `N` で勝敗が決まると、**残りのコピーは自動でキャンセル**されます
(走行中なら kill。負けたコピーが失敗していても run は赤くなりません。`joinOn: N` で
N 本に届かなかった本当の失敗は従来どおり赤くなります)。

### step の間で文脈を渡す(kickoff / handoffTo / condition)

- **`kickoff:`** — step の spawn 直後に inbox へ配送される初回指示。何を読み・何をし・
  どこに結果を書き・完了条件は何か、まで書きます(「よしなに」は書きかけ停止のもと)。
- **`handoffTo: <step id>`** — この step の**返信本文**を、宛先 step の kickoff の前に
  連結して渡します。複数の step が同じ宛先を指した場合は**宣言順に全部**連結されます
  (上限 48 KiB は source 間で分配)。宛先 step は「この step だけを `dependsOn` する」
  形でなければなりません(読み込み時に検証)。長文の受け渡しはファイル経由
  (共有ディレクトリに書いてパスを渡す)のほうが確実です — cross-model-review が
  その形の実例です。
- **`condition: "<正規表現>"`** — 依存先の**返信本文**にマッチしたときだけこの step を
  実行するゲートです。挙動は3分岐: マッチ→通常どおり実行 / 非マッチ→Skipped
  (宣言どおり降りたブランチとして run は green のまま) / **依存先が返信を残さずに
  完了していた→Failed**(評価不能。依存先に `kickoff:` + `joinOn: reply` を持たせて
  ください)。**`condition:` は `dependsOn` 1件の step にしか書けません**(本リリース
  v0.5.8 時点)。したがって「レビュアー2体とも ACCEPT なら進む」のような複数依存の判定は
  まだ書けません。

### 失敗にどう備えるか(timeoutMs / retry / onFailure / 通知)

- **`timeoutMs`**(100ms〜24時間) — 超過した step はプロセスごと止められ Failed に
  なります。**ペイン枠の空き待ち時間は含まれません**(カウントは実行開始から)。
  長時間走る step、無人で走らせる step には必ず書いてください。詰まったときの唯一の
  自動の脱出装置です。
- **`retry: { max: 1..10, backoffMs: 0..60000 }`** — Failed になった step を backoff 後に
  同じ step として再起動します。kickoff は再配送されるので、再試行の回も指示を持って
  起動します。
- **`onFailure: fail-fast | continue`**(workflow 直下、既定 fail-fast) — fail-fast は
  失敗 step の下流を Skipped にして run を畳みます。continue は他のブランチを走らせ
  続けます。
- **retry を使い切った step は、外部通知(次章の `notifications:`)へ `error` として
  1通だけ出ます**(escalation)。既定の `level: critical` のままで届きます。注意点が2つ:
  **step 単位のこの通知が対象にするのは「retry を使い切った失敗」だけ**です
  (`retry:` を書いていない step には使い切る予算がありません。ペインの異常終了そのものの
  通知は別経路で従来どおり出ます)。また最後の試行がペインを持っていた場合は
  「プロセスが落ちた」通知と「この step はもう自動では戻らない」通知の**2通が届きます**
  (仕様どおりで、バグではありません)。
- **run 全体が `Failed` で終端したときも、`error` として1通出ます**(2026-08-14 追記、
  v0.5.9)。**`retry:` を1行も書いていなくても出ます** — step 単位の escalation とは別に
  ある run 単位の通知です。ただし**同じ run で step の枯渇 escalation が既に1通でも出て
  いれば、run 単位のほうは出しません**(二重にしないため)。したがって1つの run から出る
  「run が赤い」系のメッセージは多くても1通です。自分で止めた run(`Cancelled`)は対象外
  です。

> escalation 通知の経路は自動テストで裏づけられていますが、実機で OS トースト /
> チャットに届くところまでは未確認です(2026-08-14 時点)。

### 落ちたあとの再開(resume)

アプリのクラッシュ・再起動をまたいでも、実行中だった run は検出されます。設定の読み込みに
成功すると「前回のワークフロー run『<名前>』が途中で中断されています。再開しますか?」の
バナーが出て、**再開**(実行中だった step だけを最初からやり直し、完了済み step は保持)か
**破棄**を選べます。

ただし、**再開が拒否される run が3種類あります**(エラーバナーに
`cannot be resumed` を含む理由が出ます):

1. **`onEach:` を持つ step を含む run**(常に)。
2. **`condition:` を持つ step がまだ走っておらず、その依存先が中断前に完了済みだった run**。
3. **`handoffTo:` の宛先がまだ走っておらず、渡す側が中断前に完了済みだった run**。

2 と 3 の原因は共通で、**step 間で運ばれる返信本文が再起動をまたいで保存されない**ためです。
中途半端に再開すると「約束された文脈を持たない下流」が黙って走ってしまうので、拒否する側に
倒してあります。**同じ定義でも、クラッシュがどこで起きたかによって再開できる run と
できない run があります**。拒否されたら「破棄」を選び、run を最初から流し直してください。
なお実行中の run がある間に該当ワークフローの定義を `ptygrid.yml` から消すと、resume は
「定義なし」で失敗します。定義の編集は run が無いときに。

### 上流の返信1本ごとに下流を起こす(onEach: reply / joinOn: stream)

通常の `dependsOn` は「上流が**全部**終わるまで下流を動かさない」関門です。
「実装が1ファイル書き終えるたびにレビュアーを1体ずつ起こしたい」形は、
上流に `joinOn: stream`、下流に `onEach: reply` を書きます(Phase 5.0.7)。

- 上流は生き続けたまま、仕事の単位ごとに自分の kickoff スレッドへ**何度も返信**します。
  返信1本 = 1 unit で、unit ごとに下流のコピー(`reviewer#0` / `reviewer#1` …。
  1本しか来なくても `#0` が付きます)が1つ spawn されます。
- 終わり方は2層です。第1層は**番兵**: trim 後の本文が `[[end]]` と**完全一致**する返信
  1通(部分一致では閉じません。番兵はコピーを生みません)。第2層は上流 step 自体の終端
  (PTY exit / done / `timeoutMs` / cancel)。**`joinOn: stream` の step には `timeoutMs` を
  必ず書いてください** — 番兵を送り忘れたエージェントを待ち続けない唯一の脱出装置です。
- コピーはそれぞれ**専用の mailbox**(`wf/<run id>/<step id>#<k>` 系)で kickoff を受け取り、
  自分の mailbox 名は `$PTYGRID_MAILBOX` で知ります。`await` と `reply_inbox` の `sender` に
  必ずこの値を使わせてください(でないと返信の相関が取れません)。
- unit の総数は 1 run あたり **64 が上限**(超えると上流が Failed になって閉じます)。
  9面を超えるコピーは待ち行列に入りますが、兄弟コピーが1つでも走っている間は
  5分の打ち切りは適用されません。
- **この step を含む run は resume できません**(前節)。また番兵で完了した上流のペインは
  exit しないので、`autoClose` では閉じません(手で閉じます)。

書き方の全体は [example/review-as-you-go/](../../example/review-as-you-go/ptygrid.yml) を
そのまま使ってください(前提条件・書けない組み合わせ・返信の作法のテンプレートまで
コメントで揃っています)。macOS 実機で 5 unit → 番兵 → まとめ役1回、という一連の完走を
確認済みですが、「1件ごとに返信を刻む」動きそのものはエージェントのモデル次第で、
本数どおりに刻まれない回もあります。

### ssh 接続の永続化と再接続

`cmd: ssh …` の定義に `ssh:` ブロックを付けると、ptygrid は接続先で **tmux(または screen)の
名前付きセッションにアタッチ**する形で ssh を起動します。回線が切れてもリモート側のプロセスは
tmux の中で生き続け、ペインは自動的に再接続して同じセッションへ戻ります。tmux を自前で
再実装するのではなく、「起動コマンドの書き換え・切断検知・再接続・状態表示」だけを ptygrid が
担当します。

```yaml
agents:
  - name: gpu-claude
    cmd: ssh -p 2222 me@gpu-box        # 従来どおりの ssh コマンド(オプションはそのまま残る)
    ssh:
      persist: tmux                    # tmux | screen | none(既定 tmux)
      session: ptygrid-gpu             # 省略時 ptygrid-<name>
      remote_cmd: claude --continue    # 省略時はリモートのログインシェル
      reconnect: true                  # 既定 true
      keepalive: 15                    # ServerAliveInterval 秒(既定 15)
      max_reconnects: 0                # 0 = 無制限(既定)
      mouse: true                      # tmux のホイールスクロール(既定 true)
```

実際に実行されるコマンド(ペインの `cmd` 表示にもこの形で出ます):

```
ssh -p 2222 -t -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -- me@gpu-box \
    'tmux new-session -A -s ptygrid-gpu '\''claude --continue'\'' \; set-option mouse on'
```

- **書き換えの規則**: 自分で書いたオプションは元の位置・元の表記のまま残ります(`~` や `${VAR}` は
  今までどおり展開)。`-o ServerAliveInterval=…` を自分で書いていればそちらが優先されます
  (ssh は最初の値を採用)。`persist: none` は keepalive だけを足し、tmux も `-t` も付けません。
- **再接続の条件**: ssh の終了コードが **255(接続エラー)** のときだけ再接続します。`exit` や
  tmux からの detach(終了コード 0)、接続先に tmux が無い(`127`)などは通常の終了として扱い、
  再接続しません。`autorestart` とは独立で、こちらの「連続 5 回」上限は適用されません。
- **バックオフ**: 1 秒 → 2 秒 → 4 秒 … 最大 30 秒。10 秒以上つながっていた後の切断は 1 秒から
  やり直します(`max_reconnects` のカウントもリセット)。
- **同じ定義を複数ペイン**: チップを 2 回押す・`spawn_agent` を 2 回呼ぶなど、同じ定義を同時に複数起動すると、
  2 個目以降のセッション名は自動で `ptygrid-<name>-2`, `-3`… になり、それぞれ独立したリモートシェルと
  独立した再接続になります(番号は「その定義で今開いているペインの空き最小番号」。終了したペインも
  ✕ で閉じるまで番号を保持するので、⟳ で同じセッションに戻れます)。ペインの表示名(定義名)は
  変わらず、ヘッダーの `⇄ tmux:…` バッジで区別します。`session: "work-{n}"` と書けば番号の位置を
  指定でき、`session: shared` のように固定名を書くと全ペインが同じセッションに入って**鏡写し**になります
  (別ペインで同じ画面を見たいとき用)。
- **表示**: 接続中はヘッダーに `⇄ tmux:ptygrid-gpu` バッジ(hover で接続先)、切断後は黄色の
  `⇄ 再接続中 (n回目)` バッジ。⟳ ボタンで今すぐ再接続、✕ で止められます。
- **手打ちの ssh**: シェルペインで `ssh host` と打って接続していた場合、その ssh が終了して
  シェルに戻ると、ヘッダーに赤い `⇄ host に再接続` ボタンが出ます(ペイン内にも区切り線を
  表示)。押すと同じコマンドラインをそのペインに再入力します。シェルの子プロセスの終了コードは
  外から見えないため、こちらは**提案止まり**で自動では再接続しません。`exit` で自分で抜けた
  ときも出るので、不要なら無視してください(次に ssh すると消えます)。手打ちの場合、接続先の
  プロセスを残したいなら `ssh -t host tmux new -A -s work` のように自分で tmux を付けてください。
- **スクロール**: tmux は代替画面に描画するため、ペイン(xterm)側にはスクロールバックが
  溜まりません。tmux 既定の `mouse off` のままだとホイールが ↑/↓ キーに変換され、シェルの
  履歴が送られるだけになります。そこで `mouse on` をそのセッションにだけ設定します(新規作成時も
  再アタッチ時も適用。グローバル設定は変更しません)。ホイールで tmux の copy-mode に入り、
  `q` で抜けます。遡れる行数は tmux の `history-limit`(既定 2000)に従います。mouse on の間の
  テキスト選択は Option+ドラッグ(macOS)/ Shift+ドラッグ(その他)です。tmux 2.1 未満は
  非対応(`mouse` オプションが無い)。screen にはこの設定はありません(`Ctrl-a [` で copy-mode)。
- **制限**: 接続先に tmux / screen が入っている必要があります(無いと `127` で終了し、その旨が
  ペインに出ます)。認証失敗・ホストダウンも ssh は 255 を返すので、`max_reconnects: 0` の
  ままだと 30 秒間隔で再試行し続けます(✕ で止める、または上限を設定)。`cmd` に `$(…)` や
  パイプなどシェル構文を含める書き方は非対応です。mosh は対象外です。

### ペインの後始末(autoClose / close_on_exit)

終了したセッションのペインは、既定では残ります(exit code と最終出力を見落とさないため)。
自動で閉じたい場合は2つのフィールドがあります(**階層も綴りも違うので注意**):

- `workflows.<名前>.autoClose: never | success | always`(既定 never) — その run の
  step 由来のペイン。workflow 由来のセッションでは**こちらだけ**が評価されます。
- `agents[].close_on_exit: never | success | always`(既定 never) — workflow に属さない
  通常起動のペイン。

`success` は exit code 0 のときだけ閉じ、failed / cancelled のペインは**絶対に閉じません**
(デバッグ用)。クローズは判定から3秒後で、最大化中のペインは閉じません。
`joinOn: reply` / `stream` で完了した step はペインが生きたまま残るので、閉じたいなら
エージェントへの指示に「返信したら終了してよい」を入れます(そうすれば exit 0 →
`autoClose: success` が効きます)。詳細は
[ptygrid-yml-guide.md](ptygrid-yml-guide.md) §4。

### 9面の上限とどう付き合うか

グリッドは最大9面で、workflow もこの枠を使います。運用上の要点:

- **終了済み(Exited)のペインも枠を数えます。** 大きめの run を流す前はグリッドを
  空にしてください。前の run の残骸が原因の枠不足がいちばん多い詰まり方です。
- 空きが無い step は失敗せず **Pending のまま待ちます**。step 行に
  `waiting for a free pane slot (N/9 occupied)` と理由が出て、空きが出ると自動で
  spawn されます。待ちは最大5分で、それでも空かなければ Failed です(`retry:` があれば
  再試行対象)。`Pending` のまま動かない step を見たら、まずこの理由表示で
  「依存待ち」か「枠待ち」かを見分けてください。
- fan-out は **all-or-nothing** です(空きがコピー数に足りなければ1本も spawn しません)。
- **`fanOut` を持つ step は root に置かず、手前に軽い step を1段挟んでください。**
  root に置いても動きますが、全コピーが同じ step_id になってパネル上で区別できなく
  なります。仕事をしない `gate` step(sleep 0 のシェル)を root にして fan-out step に
  `dependsOn: [gate]` させるのが定石です — 実例と理由の詳細は
  [example/measure-parallelism/](../../example/measure-parallelism/ptygrid.yml)
  (並列化の効きと orchestration のコストを測る計測サンプル。spawn し直しのコストは
  [example/measure-coldstart/](../../example/measure-coldstart/ptygrid.yml)で測れます)。

## スケジュール実行(schedule)と外部通知(notifications)

### 時刻で自分から始める(schedule:)

workflow の起動方法は「人が ▶ を押す」「エージェントが `spawn_workflow` を呼ぶ」に加えて、
**時刻が来たら自分で始まる** `schedule:` があります(Phase 5.0.8)。workflow 直下に書きます:

```yaml
agents:
  - name: reviewer
    cmd: "claude"       # 実物は起動直後に await する指示を cmd に埋める(example を参照)

workflows:
  scheduled-review:
    schedule:
      every: day        # day | weekday | hour の3語彙だけ。cron 式は書けない
      at: "09:00"       # day / weekday は "HH:MM"、hour は "MM"(毎時 MM 分)
      # enabled: false            # 消さずに止めたいとき
      # maxConsecutiveFailures: 3 # 既定 3(1〜10)
    pattern: pipeline
    steps:
      - id: review
        agent: reviewer
        joinOn: reply
        timeoutMs: 1800000     # 無人で走るので上限は必ず書く
        kickoff: >-
          昨日からの変更を確認し、気になった点を返信してください。
```

- **cron 式は書けません。** 書けるのは `every: day / weekday / hour` + `at:` の3語彙だけで、
  書けないことが機能です(タイプミスが実行時まで運ばれない)。`every: dayly` も
  `at: "9時"` も保存した時点で読み込みエラーになります。さらに `schedule:` の下は
  **キー名も閉じています**(`enable: false` のような1文字違いは黙って無視されず、
  読み込みで落ちます。「未知フィールドは無視」というこの設定ファイルの原則の唯一の例外)。
  時刻はローカルタイムです。
- **アプリが起動している間だけ発火し、起動していなかった分は追いません**(これは仕様です。
  朝アプリを開いた瞬間に溜まった数本が同時に走り出すほうが事故のため)。代わりに
  Workflows パネルの 🕒 行に「次回」と「最終実行」が常に出ます — 最終実行が数日前で
  止まっていたら、それは「アプリを開いていなかった」の表示です。
- **見送りの条件**は3つで、どれも発火の前に判定され、理由がパネルに出ます:
  (1) 同じ workflow の前の run がまだ終わっていない(積みません)、
  (2) グリッドに root ぶんの空きが無い(始めてから5分待って赤くする形にはしません)、
  (3) 予定時刻を大きく過ぎていた(猶予は `hour` が5分、`day` / `weekday` が15分。
  スリープ復帰で「毎朝9時のレビュー」が夕方に始まるのを防ぎます。数分の遅れなら
  普通に走ります)。見送った回は次の予定時刻まで再試行しません。
- **`autoClose` の既定が変わります。** `schedule:` を持つ workflow は `autoClose` 未宣言の
  とき `success` として扱われます(昨日のペインが残って今日の発火が枠不足で飛ぶのを
  避けるため)。明示宣言があればそちらが優先です。
- **連続で失敗すると自動停止します**(既定3回、`maxConsecutiveFailures` で 1〜10。
  cancel は失敗に数えません)。止まる前も「連続失敗 1/3(あと2回で自動停止)」が
  パネルに出ます。停止状態はアプリが覚えているだけで **`ptygrid.yml` は書き換えません**
  (再起動でカウンタは0に戻ります)。**解除は、その workflow(またはそれが使う agent)の
  宣言を編集して保存すること**で起きます。kickoff の書き間違いを直して保存すれば、
  また動き出します。無関係な workflow を編集しても解除されません。再開ボタンはありません。
- 細かい規則: 同じ時刻に複数のスケジュールが due のときは名前の昇順で処理します。
  時計の巻き戻しや OS のタイムゾーン変更を検知すると次回時刻を計算し直します。
  パネルの表示は60秒間隔の更新なので、設定を保存してから 🕒 行が変わるまで最大1分
  かかります(再読み込み自体は即座です)。

動く実例とコメントは [example/scheduled-review/](../../example/scheduled-review/ptygrid.yml)
にあります。試すときは `at` を「いまから2分後」にして保存してください
(**保存した瞬間には発火しません**。設定を書いている最中に走り出さないためです)。

> `schedule:` は実装と自動テストは入っていますが、**実機で時刻発火を待った検証はまだ
> 行われていません**(2026-08-14 時点)。無人運用に載せる前に、上の「2分後」の手順で
> 発火・見送り・自動停止を一度自分の環境で確かめることをおすすめします。
> また無人の失敗を外へ知らせたい場合は、次節の `notifications:` を有効にしてください。
> **run が `Failed` で終端すれば、`retry:` の有無に関わらず1通出ます**(2026-08-14、
> v0.5.9。schedule のために `retry:` を全 step に足す必要はありません。前章
> 「失敗にどう備えるか」参照)。

### アプリの外へ通知する(notifications:)

セッションの異常終了・承認待ち・完了を、デスクトップ通知やチャット
(Slack / Mattermost / Discord / Telegram)へ届けられます(Phase 4.4.2)。
**opt-in** で、ブロックを書かなければ何も送りません。

```yaml
notifications:
  enabled: true            # 既定 false
  level: critical          # 全チャネル共通の既定。silent | critical | needs-attention | all
  channels:
    - type: os             # デスクトップトースト
      level: all           #   チャネル個別に上書き可(省略時は上の level)
    - type: slack
      webhook: "${SLACK_WEBHOOK_URL}"   # ${VAR} は送信時にホスト環境変数から展開
    - type: telegram
      bot_token: "${TELEGRAM_BOT_TOKEN}"
      chat_id: "123456789"
      level: needs-attention
```

イベントは自動で判定されます: `error`(exit code 非0などの異常終了、workflow の
retry 枯渇、および **workflow の run 全体が `Failed` で終端したとき**(2026-08-14 追記、
v0.5.9。step の枯渇 escalation が既に出た run では出ません)) /
`needs-attention`(状態バッジが blocked = 承認・入力待ち) /
`complete`(exit 0 または done)。`level` はチャネルが購読する束です:

| level | 届くもの | 想定 |
|---|---|---|
| `silent` | なし | 明示的に無音にしたいとき |
| `critical`(既定) | error のみ | 「壊れたときだけ」 |
| `needs-attention` | error + needs-attention | 「止まってたら教えて」 |
| `all` | すべて(完了含む) | 短いタスクの監視 |

- 既定が `silent` ではなく `critical` なのは、「全部切ったつもりが異常終了まで
  握り潰していた」を避けるためです。
- チャネルごとに `level` を上書きできるので、「共有 Slack は critical で静かに、
  手元の Telegram は needs-attention で細かく」ができます。
- `webhook` / `bot_token` の欠落や `${VAR}` 展開後の空文字は、**そのチャネルだけ**
  送信時にスキップされます(設定全体の読み込みは失敗しません)。
- `autorestart` による再起動ループの途中のクラッシュは通知されず、打ち切り後の最後の
  終了だけが1通になります。
- macOS の OS 通知は、バンドル済み・署名済みアプリ + OS の通知許可が前提です。
  `npm run tauri dev` の素の起動では表示されないことがあります(webhook 側は影響なし)。

全チャネルの注釈付き設定例は [ptygrid.example.yml](../../ptygrid.example.yml) にあります。

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
長時間待つ場合は成果物の更新も確認してください。詳細は`docs/guide/troubleshooting.md`を参照してください。

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
- 設計の背景・アーキテクチャは [design.md](../design/design.md) を参照してください。
