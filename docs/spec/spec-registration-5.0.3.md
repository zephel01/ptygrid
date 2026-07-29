# 仕様書: Queen の MCP 登録の代行（Phase 5.0.3）

作成日: 2026-07-29 / ステータス: **ドラフト（未実装）** / 対象: agent CLI への `queen` MCP 登録

> **採番**: [spec-init-5.0.2.md](spec-init-5.0.2.md) 冒頭の注記のとおり
> **5.0.2 = `ptygrid init` / 5.0.3 = Queen MCP 登録の代行**（本書）で確定済み。

関連: [spec-init-5.0.2.md](spec-init-5.0.2.md)（本書が前提とする規律の出典）、
[plan.md](../design/plan.md) §2・§4、[CONTRACT.md](../../CONTRACT.md)、
[queen-universal-register.md](../design/queen-universal-register.md)、
[troubleshooting.md](../guide/troubleshooting.md)

---

## 1. 目的と背景

**エージェントを 1 体増やすたびに人間が登録コマンドを打つ運用をやめる。**

登録は現在すべて人手である。Queen バッジのパネル（`App.svelte:2062-2106`）は 6 個のコピーボタンを出すだけでバックエンドは
一切関与しない（`invoke` が 1 本も無い）。CLI を 1 つ足すたび、トークンを再生成するたびに、claude なら shell に 2 コマンド
を貼り、codex / grok なら TOML を手で追記する。

実体は検出 + 差分生成 + 承認 + アトミック適用で、新しいプロトコルも信頼境界も作らない。ただし **5.0.2 が書くのは ptygrid
自身の設定だったが、5.0.3 が書くのは他社ツールのホーム配下の設定である。** ptygrid が壊せば ptygrid と無関係な codex の
使用が止まる。この非対称が本書のほぼ全ての決定の根拠になる。

---

## 2. スコープ / 非スコープ

### 2.1 スコープ

- claude / codex / grok の検出と既存 `queen` 登録の読み取り
- 登録の代行（claude は CLI 実行、codex / grok は `toml_edit` の値単位編集）と登録解除（3.7）
- 差分プレビューと明示的な承認、temp + rename、タイムスタンプ付きバックアップ、冪等
- 適用後の検証、「ペインの再起動が必要」表示（3.9）、失敗時にコピペ経路へ落ちる導線
- command 3 本 + イベント 1 本 / CONTRACT.md 先行追記 / docs と実装の食い違いの解消（3.0）

### 2.2 非スコープ

- 上記 3 つ以外の CLI の代行 — 汎用 3 コピー（`App.svelte:336-386`）が担当
- 複数 ptygrid インスタンスの同時登録 — 非対応（3.8）。検出して警告するまで
- トークン再生成 / ポート変更を契機とする自動再登録 — 通知（3.10）まで。自動適用は無確認で他社設定を書く経路になり
  3.1 の決定と背反する
- Claude Code の Bash サンドボックスへの対処 — `troubleshooting.md:117` が「アプリ側では対処不能」と結論。6 章 E-12
  で扱い方だけ決める
- ペインの自動再起動 — 既存 `restart_session`（`commands.rs:266`）への導線は出すが、代行が作業中セッションを殺さない
- 登録名の可変化（`queen-<port>`） — 3.8 の帰結として `queen` 固定を維持

---

## 3. 設計

### 3.0 前提作業 — docs と実装の食い違いを確定させる

**決定: 「docs と実装のどちらが実機で正しいか」の確定を completion gate に含める（7.1）。本書はどちらが正とも断定しない。**

| # | docs | 実装（`App.svelte`） |
|---|---|---|
| 1 | **grok は CLI**: `grok mcp add -s user -t http …` + `grok mcp doctor`（`README.md:133-134`, `userguide.md:392-397`） | **grok は TOML**: `~/.grok/config.toml` に `[mcp_servers.queen]` + `bearer_token_env_var`。codex と**1 バイトも違わない**（`:307-317`） |
| 2 | **codex は URL 埋め込み**: `url = "…/mcp?token=…"`（`README.md:137-140`, `userguide.md:384-390`） | **codex は env 参照**: `url`（token 抜き）+ `bearer_token_env_var = "QUEEN_TOKEN"`（`:294-297`） |

`App.svelte:307-309` は grok を「実機で確認済み」と書くが docs と食い違っており、どちらが正しいかは未確認
（F-14-2 / F-14-3）。着手前に実機で確定し、負けた方の記述を削除する。

### 3.1 共通フロー — 検出 → 差分プレビュー → 明示的な承認 → 適用

**決定: 無確認では書かない。5.0.2 と同じ規律（`spec-init-5.0.2.md` 3.4 / 3.5 / 6 章）を CLI 実行にもファイル編集にも
適用し、`teams_hooks::write_hook_settings` の確認なし即書き込み（`App.svelte:466-483`。差分表示もバックアップ告知も
無し）は踏襲しない。**

```
mcp_register_scan     検出のみ（CLI の有無 / 設定ファイル / 既存の queen 登録）
  → mcp_register_preview   差分生成。ディスクにもプロセスにも触らない
       ├─ noop → 「既に最新です」。適用ボタンを無効化（3.5）
       └─ 差分あり → before/after（TOML）または argv（CLI）を提示
  → ユーザーの明示的な承認
  → mcp_register_apply     再検査 → バックアップ → temp+rename / Command 実行 → 検証
```

`preview` の `before` は `apply` に `expectedBefore` として送り返し、不一致なら中止する。

### 3.2 CLI ごとの方式表

| CLI | 方式 | 対象 | 依存 | コメント保持 | 冪等の判定方法 |
|---|---|---|---|---|---|
| **claude** | CLI サブコマンド（shell 非経由の `Command` + args） | CLI が管理するファイル（`~/.claude.json` と**推測・未確認**）。ptygrid は直接触らない | 追加なし | **対象外**（ファイルを開かない） | `claude mcp get queen` に現在の URL が出るか |
| **codex** | `toml_edit` の**値単位**編集 | `~/.codex/config.toml` の `mcp_servers.queen` | `toml_edit` を direct dep へ昇格（`Cargo.lock:4446-4452` に既存で新規クレートは実質ゼロ） | **可**（実測 8） | 編集後 `to_string()` と**編集前バイト列**の一致 |
| **grok** | 3.0 の確定に従う（TOML なら codex と / CLI なら claude と同一実装） | `~/.grok/config.toml` または grok CLI | 同上 | 同上 | 同上 |
| その他 | **代行しない**（2.2）。汎用 3 コピーが担当 | — | — | — | — |

### 3.3 claude — CLI 代行（2 プロセスへの分解とタイムアウト）

**決定: `git_service.rs:59-72` の「shell を通さない `Command` + args」方式を流用する。現行のコピー文字列は shell 構文
（`;` / `||` / `2>/dev/null`）を含むため、代行では 2 回の `Command` 実行に分解する。**

| # | argv | 失敗の扱い |
|---|---|---|
| 1 | `claude` `mcp` `remove` `queen` `-s` `user` | **無視**（未登録なら非 0 が正常。`\|\| true` に相当） |
| 2 | `claude` `mcp` `add` `-s` `user` `--transport` `http` `queen` `http://127.0.0.1:39237/mcp?token=<64hex>` | **報告**（非 0 なら適用失敗） |

URL のクォート（`?` の zsh glob 対策）は shell 非経由で不要になる。`-s user` は維持する（local スコープはディレクトリ
限定のため、`App.svelte:260-261`、`userguide.md:368-381`）。

**決定: 外部コマンド実行にタイムアウトを設ける。既定 10 秒、超過したら子プロセスを kill し失敗を返す。** `git_service`
にタイムアウトは無い（grep ヒット 0）が、`claude mcp add` が対話プロンプトに入ると `.output()` は永久にブロックしうる
（**推測・未確認**、F-14-4）ため新規に決める。実装は `spawn()` + 期限付き待機、超過時 `Child::kill()`。CLI 不在は
spawn Err でしか分からず、shell alias / 関数の `claude` は見えない（6 章 E-5）。

### 3.4 codex / grok — `toml_edit` による値単位編集

**決定: TOML は必ず `toml_edit` でパースしてから値単位で更新する。テーブルごとの差し替えは禁止する。素朴なテキスト追記も
禁止する。** 根拠はすべて実測（`toml_edit 0.25.13`、`registration-constraints.md` B-4）。

| 落とし穴 | 実測 | 仕様上の対処 |
|---|---|---|
| 同名テーブルのテキスト追記 | **`duplicate key` で parse エラー**（1） | 追記は使わない。必ず parse してから編集 |
| テーブルごとの差し替え | **ユーザーのコメントが消える**（9） | `url` / `bearer_token_env_var` を**キー単位で代入**（8 でコメントは残る） |
| 親 `mcp_servers` が不在 | `Item::Table` 代入が**インラインテーブル**を出し `[mcp_servers.queen]` 形にならない（6） | `Table::new()` + **親に `set_implicit(true)`**（7 で正しいヘッダ形。再適用もバイト同一） |
| CRLF ファイル | **編集しなくても LF に正規化される**（13） | **仕様として明記**（下記） |
| インライン形 / ドットキー形 | 11 / 12 | 検出は**必ず `doc["mcp_servers"]["queen"]` の参照で行う**。テキスト検索は取りこぼす |
| 壊れた TOML | parse が `Err`（14） | **書く前に検出できる**。Err なら 1 バイトも書かず UI にエラーを出す |

`serde` の `toml` クレートは `serde_norway` と同じく**コメントを全消しする**（実測）ため使わない。既定（env 参照）の
書き込みでは既存の行は 1 バイトも変わらない（実測 3 / 8、5.0.2 が禁じた「読んで書き戻す」（同 2.2）の例外として許容）。

```toml
[mcp_servers.queen]
url = "http://127.0.0.1:39237/mcp"
bearer_token_env_var = "QUEEN_TOKEN"
```

**決定: CRLF が LF に正規化される副作用を仕様として明記し、隠さない。** 検出時に改行コードを判定し、CRLF なら
「改行が LF に変換されます（全行が差分になります）」をプレビューに必ず表示する。

### 3.5 冪等性・自己検査・書き込み

**決定: 既に同値なら 1 バイトも書かない。再適用してバイト同一になることを受け入れ条件にする。** TOML は編集後
`to_string()` と編集前バイト列を比較し、一致すれば `written: false` を返しバックアップも作らない（`teams_hooks.rs:830-835`
のバイト列版、実測 4 / 7）。CLI は `claude mcp get queen` の URL が一致していれば実行せず、違えば remove → add
（`add` は "already exists" で上書きしないため、`App.svelte:263-266`。remove 先行の冪等化は既に frontend が解決済み）。

**決定: 書き込み前に自己検査を通し、通らなければ 1 バイトも書かない。** TOML は「編集後の文字列を再 parse して Ok を
確認する」ことを事後条件とする（`spec-init-5.0.2.md` 3.5 の `parse_config` に相当）。

**決定: 書き込みは temp + rename。素の `fs::write` は採らない。**（`spec-init-5.0.2.md` 3.5 と同じ方式、前例
`trust.rs:101-116` 他）`teams_hooks.rs:870` は踏襲しない——`~/.codex/config.toml` は codex 本体が同時に読むため
部分書き込みを読まれるリスクが `settings.json` より高い。

**決定: バックアップは `teams_hooks` 流を流用する。** `<file>.ptygrid-backup-<unix_millis>` + 衝突時の連番
（`teams_hooks.rs:843-865`。ミリ秒精度は事故 M7 への対処）。世代の上限と掃除は決めない（8 章）。

**決定（不変条件）: 代行は `mcp_servers.queen` 以外の値を一切変更しない。** claude の非 user スコープ、
`auth-tokens.json`、`~/.claude/settings.json`、`ptygrid.yml` も触らない。値単位の編集（3.4）と編集後バイト列の比較で
機械的に担保し、CONTRACT.md に明記する（4.1）。

### 3.6 認証方式 — 既定は env 参照、URL 埋め込みは警告つきの選択肢

**決定: 既定の認証方式は `bearer_token_env_var`（env 参照）とする。URL 埋め込み（`?token=`）は選択肢として残すが、
選ぶときは警告を添える。** 根拠はトークンの露出——URL 埋め込みは token が子プロセスの argv に平文で載る
（`/proc/<pid>/cmdline` が他プロセスから読めることは `pty.rs:672-678` で確認済み。`auth-tokens.json` は 0600 で守る
`token_store.rs:155-162` と非対称）。env 参照はトークン再生成にも強い（`queen.rs:346-350`, `session.rs:344-349`）。

**決定: env 参照方式の副作用を UI とドキュメントで必ず伝える。** `QUEEN_URL` / `QUEEN_TOKEN` は ptygrid が spawn する
セッションにだけ注入される（`session.rs:312-355`, `:344-349`）のに対し登録先の `~/.codex/config.toml` はユーザー全体
の設定であるため:

> **ptygrid の外で codex を起動すると `QUEEN_TOKEN` が無く、queen への接続は 401 になる。**

この 1 行をプレビューと userguide の両方に必ず出す（5 章 / 7.1 gate 4）。claude で env 参照相当が使えるかは未確認
（`queen-universal-register.md:27-31`）。確認が取れるまで claude は URL 埋め込みでのみ代行し、適用前に argv 露出の
警告を必須表示する（8 章。脅威モデルは `token_store.rs:13-14` と同じ同一ホスト・同一ユーザーの範囲で許容）。

### 3.7 登録解除（アンインストール導線）

**決定: 登録解除を 5.0.3 の同じ patch に含める。残骸を残さない。** 登録を消すコードは現在どこにも無く、放置すると
codex / claude は起動のたびに繋がらない MCP サーバーへ接続を試みる。claude は `claude mcp remove queen -s user`
（3.3 の 1 本目と同一 argv。今度は失敗を報告する）、TOML は `mcp_servers.queen` を remove し親が空になれば親も remove。
claude の非 user スコープ（`troubleshooting.md:17-25` の事故）は勝手に消さず、検出して警告だけ出す。

**決定: `toml_edit` の remove が直前のユーザーコメントを巻き込む問題は既知として明記し、「消える行をプレビューで先に
見せてから承認を取る」ことで扱う。** 実測 10 のとおり remove は直前のコメントも消す（`# top` + `[mcp_servers.queen]`
だけのファイルが空文字列になった）。分離 API は未確認のため消失を防ぐのではなく可視化する——`lostComments` に消える
行を入れ UI が列挙し、バックアップは解除時も必ず取る。

### 3.8 複数 ptygrid インスタンス — 5.0.3 では非対応

**決定: 複数インスタンスの同時登録は 5.0.3 では非対応と明示する。検出したら警告を出すところまでを実装する。**
`bind_with_fallback`（`queen.rs:249-259`）で 2 つ目は 39238 で listen するがトークンは app-data 共有で両インスタンス
同一（`token_store.rs:173-192`）、登録名も `queen` 1 つしかないため、後発インスタンスが登録を上書きすると先のペインが
後発に繋がり 401 にもならず黙って別のグリッドを操作する（`Host` のポート検証 `queen.rs:397-401` では検出できない）。
検出条件は既存 `url` のポートが自分の bind 済みポートと異なることとし、`foreignPort` を UI に警告表示する（5 章）。
上書きの可否はユーザーの選択に残す。登録名のポート込み化は `QUEEN_URL` と tool 名の対応が崩れるため採らない（2.2）。

### 3.9 適用しても既存の CLI セッションには反映されない

**決定: 適用後に「そのペインの再起動が必要」を UI に必ず出す。** `troubleshooting.md:33-35` の「MCP tool 一覧は起動時に
読み込まれ、登録前から動いている session には即時反映されない」を踏まえ、`restartRequired`（常に `true`）を返し、
パネル内に残る注記として出す。既存 `restart_session`（`commands.rs:266`）への導線を添えるが自動では再起動しない（2.2）。

### 3.10 ポート / トークンの変更を frontend へ push する

**決定: 新規イベント `queen-changed` を追加する。additive とし、既存 wire は一切変えない。** 5.0.2 は「新規イベントを
追加しない」と決めた（`spec-init-5.0.2.md` 5.1-5）が、現状 push 経路が存在しない（`emit("queen…")` の grep ヒット 0、
`refreshQueenStatus` `stores.svelte.ts:171-179` は起動時・`loadConfig` 後・トークン再生成後の 3 箇所のみでポーリング
無し）ため追加する。穴は 2 つ: (1) `run_server` が `tauri::async_runtime::spawn`（`queen.rs:261`）なため起動直後は
bind 前の `running:false` を拾いうる（現行 UI は無言で何もしない、`App.svelte:258`。レース幅は**推測・未実測**、
F-14-6）、(2) `apply`（`queen.rs:212-246`）は desired_port 変更時にポートが変わるが既存登録は古いポートのまま
frontend は知り得ない。`queen-changed` は bind 完了時・再 bind 時・トークン再生成時に emit する（payload は
`QueenStatusInfo` と同形）。自動再登録はしない（8 章）。

---

## 4. CONTRACT.md 追記項目と wire

### 4.1 CONTRACT.md 追記項目（実装前に先行追記）

1. `McpTarget` / `McpAuthMode` / `McpAction` の列挙値と `McpRegisterScan` / `McpRegisterPreview` / `McpApplyResult` の
   確定形（フィールド名は camelCase）
2. command 3 本のシグネチャ（4.2）と新規イベント `queen-changed`（payload = `QueenStatusInfo`）
3. 不変条件（3.5）: 「`mcp_servers.queen` 以外の値を変更しない」「適用される TOML は必ず再 parse を通っている」
   「同値なら 1 バイトも書かない」
4. バックアップ規約: `<file>.ptygrid-backup-<unix_millis>`（`teams_hooks` と同一命名。世代管理なし）
5. 非対応の明示: 複数インスタンスの同時登録（3.8）、claude の非 user スコープ（3.7）
6. 非回帰宣言: `queen_status` / `QueenStatusInfo`（全 field）/ `regenerate_auth_tokens` / `register_teammate_hooks` /
   `restart_session` は不変。本節はすべて additive。Queen バッジの既存 6 ボタン（`App.svelte:256-386`）も撤去しない

### 4.2 wire（Tauri command / イベント）の形

```ts
type McpTarget   = "claude" | "codex" | "grok";
type McpMethod   = "cli" | "toml";
type McpAuthMode = "env" | "url";
type McpAction   = "register" | "unregister";

interface McpExistingEntry { url: string | null; authMode: McpAuthMode | null; ours: boolean; foreignPort: number | null; otherScopes: string[]; }
interface McpTargetScan { target: McpTarget; method: McpMethod; available: boolean; configPath: string | null; parseError: string | null; crlf: boolean; existing: McpExistingEntry | null; }
interface McpRegisterScan { queenRunning: boolean; port: number | null; targets: McpTargetScan[]; warnings: string[]; }
interface McpRegisterPreview { target: McpTarget; action: McpAction; authMode: McpAuthMode; method: McpMethod; path: string | null; before: string | null; after: string | null; commands: string[][]; noop: boolean; lostComments: string[]; tokenInArgv: boolean; tokenInFile: boolean; valid: boolean; error?: string; warnings: string[]; }
interface McpApplyResult { target: McpTarget; action: McpAction; written: boolean; path: string | null; backupPath: string | null; verified: boolean | null; verifyNote?: string; restartRequired: boolean; }
```

各フィールドの意味は 3 章の対応する決定（3.4 〜 3.9）と 6 章（E-4 / E-12）で述べたとおり。

| command | args | returns | 説明 |
|---|---|---|---|
| `mcp_register_scan` | `{}` | `McpRegisterScan` | 検出のみ。ディスクにもプロセスにも**書かない** |
| `mcp_register_preview` | `{ target, action, authMode? }` | `McpRegisterPreview` | 差分生成 + 自己検査。**書かない** |
| `mcp_register_apply` | `{ target, action, authMode?, expectedBefore? }` | `McpApplyResult` | `expectedBefore` と現ファイルが不一致なら中止（3.1）。一致すれば再検査 → バックアップ → temp+rename / `Command` 実行 → 検証 |

| event | payload | 説明 |
|---|---|---|
| `queen-changed` | `QueenStatusInfo` | bind 完了 / 再 bind / トークン再生成時（3.10）。additive |

配線先は `commands.rs` の `#[tauri::command]` 群 + `lib.rs:88-118` の `generate_handler!`。

---

## 5. UI 仕様

**入口は Queen バッジのパネル**（`App.svelte:2062-2106`）。現行 6 ボタンの上に代行セクションを足し、既存ボタンは
1 つも撤去しない（`spec-init-5.0.2.md` 8 章の二段構え）。Queen 未 bind のときは代行ボタンを無効化して理由を明記する
（現行の無言 return をやめる。6 章 E-4）。CLI 不在なら「見つかりません」+ 対応するコピーボタンへの導線（3.3）。

「登録する」で開くプレビューモーダルが見せるもの（文言は実装時に決める。代表例のみ下記）:

| # | 要素 | 内容 |
|---|---|---|
| 1 | 対象と方式 | 例:「`~/.codex/config.toml` を編集します」 |
| 2 | 差分 | TOML は行単位の 2 ペイン（sidecar は使えないため唯一の「見せる」手段、`spec-init-5.0.2.md` 3.4）。CLI は
argv 2 行（トークンはマスクしつつ実行時は完全な値が `/proc/<pid>/cmdline` に載ることを併記） |
| 3 | 認証方式 | 既定 `env 参照`。`URL 埋め込み` を選ぶと 4 の警告が入れ替わる |
| 4 | 警告ブロック | 該当分のみ・常時表示・折りたたまない（下記） |
| 5 | バックアップ先 | 適用前にバックアップを作る旨を表示 |
| 6 | `noop` のとき | 変更が無い旨を表示し `[適用]` を無効化 |

警告ブロックは 6 種（env 参照・URL 埋め込み・CRLF・他インスタンス・他スコープ・解除時。3.4 / 3.6 / 3.7 / 3.8 で
対応済み）。代表として env 参照（既定）の警告は必ずこの 1 行を含める:

> ptygrid のペインの外で codex を起動すると `QUEEN_TOKEN` が無く 401 になります。

ボタンは `[適用]` / `[代わりにコピーする]` / `[キャンセル]`。中央は適用せず現行のコピペ経路へ逃げる導線として必ず置く。
適用後はトーストではなくパネル内に残る注記を出す:

> ✅ codex に登録しました（バックアップ: `config.toml.ptygrid-backup-1753…`）
> ⚠ この登録は、いま動いているペインには反映されません。 `[このペインを再起動]`

---

## 6. 危険と、その潰し方

`registration-constraints.md` E 章の 12 項目を漏らさず処理する。詳細は各節、「非スコープ」には理由を添える。

| # | 危険 | 処理 | 要点 |
|---|---|---|---|
| **E-1** | ホーム配下の設定を他ツールと共有 | **潰す** | → 3.5 |
| **E-2** | 冪等性が無いと壊れる（実測 1） | **潰す** | → 3.4 / 3.5 |
| **E-3** | トークン / ポート変更後の再登録 | **一部潰す + 一部非スコープ** | → 3.6 / 3.10（自動再登録は非スコープ） |
| **E-4** | Queen 未 bind なのに登録される | **潰す** | → 4.2 / 5 章 |
| **E-5** | CLI が無い / 仕様が変わった | **潰す** | → 3.3 / 3.5（検証失敗 ≠ 登録失敗は E-12） |
| **E-6** | 複数 ptygrid インスタンス | **非スコープ（検出と警告は潰す）** | → 3.8 |
| **E-7** | 手で登録済みの場合の衝突 | **潰す** | → 3.1 / 3.7 |
| **E-8** | アンインストール導線が無く残骸になる | **潰す** | → 3.7 |
| **E-9** | token が argv とファイルに平文で載る | **潰す（軽減）** | → 3.6 / 8 章 |
| **E-10** | env 方式は ptygrid のペイン内でしか効かない | **潰す（周知で）** | → 3.6 / 5 章 / 7.1 gate 4 |
| **E-11** | 既存 CLI セッションに反映されない | **潰す** | → 3.9 |
| **E-12** | サンドボックスが 127.0.0.1 を塞ぐ | **非スコープ（切り分けだけ潰す）** | `troubleshooting.md:117` の「アプリ側では対処不能」を受け、書き込み成功なら登録成功とし、到達性の検証失敗は `verified: null` に留めて登録失敗とは扱わない |

---

## 7. リリース計画とテスト

### 7.1 Phase 5.0.3 — 登録代行

**入るもの**: 3.0 の食い違い解消、`toml_edit` の direct dep 昇格、TOML 編集 / CLI 実行 / 検出 / 差分生成、command 3 本 +
イベント 1 本、登録解除、UI（5 章）、CONTRACT.md「Phase 5.0.3 追加契約」、README / userguide の書き換え。
**入らないもの**: 2.2 の非スコープに同じ。加えて Queen バッジの既存 6 ボタンは 1 つも変更しない。

`completion gate:`

1. **3.0 の食い違いが確定していること** — grok の方式（CLI か TOML か）と codex の認証方式が実機で確認され、
   **README / userguide / `App.svelte` / 本 spec の 4 者が一致し、負けた方の記述が削除されている**こと
2. unit + integration が通り、`svelte-check` / `npm run build` が 0 errors、CI（macOS / Ubuntu）green
3. CONTRACT.md への先行追記完了（4.1 の 6 項目）
4. **userguide に E-10 の副作用（ptygrid 外の codex は 401）が明記されている**こと
5. 実機手動検証の項目が plan.md §2 に U 番号として登録されていること（7.2）

> バージョン割当: `spec-init-5.0.2.md` 7.1 と同じく本書はタグ番号を確定させない。

### 7.2 テスト

**unit（`cargo test`）** — TOML 編集は「文字列 in → 文字列 out」の純関数に切り I/O ゼロでテストする
（`spec-init-5.0.2.md` 7.2 の F12 と同じ手法）。括弧内は B-4 の実測番号。

- 冪等（4 / 7）・no-op で `written: false` かつバックアップ無し・コメント保持（8）・親テーブル不在でインライン化
  しない（6 → 7）
- 検出: ヘッダ形 / インライン形（11）/ ドットキー形（12）/ 壊れた TOML（14）/ CRLF（13）
- 解除: `queen` と空親が消え `lostComments` に巻き添え行が入る（10）
- claude の argv 2 本分解（3.3）/ `expectedBefore` 不一致で apply 中止（3.1）

**integration（`cargo test`）** — 一時ディレクトリは `std::env::temp_dir().join(format!("ptygrid-mcpreg-{}",
std::process::id()))` + 末尾 `remove_dir_all`（`tempfile` は dev-deps に無い。`config.rs:2231-2235` 他と同じ）。

- `preview` → `apply` → 再 `preview` が `noop: true` / 再 parse を通る / temp が残らない
- バックアップは `ptygrid-backup-<ms>` 命名、同一ミリ秒衝突で `-1` が付く
- `mcp_servers.queen` 以外のキーが 1 バイトも変わらない（4.1-3 の回帰）
- CLI は実在しない実行ファイル名で spawn Err 経路、すぐ終わらないプロセスでタイムアウト経路（3.3）を突く

**frontend** — `svelte-check` / `npm run build` の 0 errors のみを要求する。

**実機手動検証**（macOS 必須 / Linux ベストエフォート）— 本書は手順だけを書き、実施状況は書かない。実施状況は
plan.md §2 に登録する。現行の最大は U10 なので、以下は U11 以降の採番候補として列挙する。

| 候補 | 内容 | 由来 |
|---|---|---|
| a | **grok の実際の登録方式**（`grok mcp add -s user -t http` が存在するか / TOML が効くか）。**gate 1 の必須項目** | F-14-2 |
| b | **codex の `bearer_token_env_var` サポート**と URL 埋め込みとの優劣。**gate 1 の必須項目** | F-14-3 |
| c | claude の user スコープ登録が実際にどのファイルに入るか（`~/.claude.json` と**推測**） | F-14-1 |
| d | `claude mcp add` が対話プロンプトを出す条件（タイムアウト 10 秒の妥当性） | F-14-4 |
| e | `claude mcp list` / `mcp get` の出力形式（検証に parse できるか） | F-14-7 |
| f | Windows の npm シム（`.cmd`）で `Command::new("claude")` が exec できるか（U8 の範囲） | F-14-5 |
| g | bind 完了前に `refreshQueenStatus` が走るレースの幅と、`queen-changed` で解消されること | F-14-6 |
| h | 登録 → 既存ペインでは見えない → 再起動すると見える、の一連（3.9 / E-11 の価値検証） | `troubleshooting.md:33-35` |
| i | **ptygrid の外で** codex を起動すると queen が 401 になること（E-10 の確認） | 3.6 |
| j | 解除 → 残骸が無く、他の設定が無傷であること（E-8） | 3.7 |

---

## 8. 未解決事項

3 章で決定として潰していない論点を残す。各項目は「論点 + (a)/(b) + 倒し方の帰結」に絞る。

- **grok / codex の正方式**: (a) 実装（TOML + `bearer_token_env_var`）が正、(b) docs（`grok mcp add` / URL 埋め込み）が正。
  本書は断定せず 3.0 の gate で実機確定する（(b) なら codex は E-9 の露出が増える）。
- **claude で env 参照相当が使えるか**: (a) 使える → 既定を env 参照へ、(b) 使えない → URL 埋め込み + 必須警告のまま
  （現方針、3.6）。未確認。
- **トークン再生成 / ポート変更後の自動再登録**: (a) 登録先を app-data に記録して自動再適用、(b) `queen-changed` で
  通知するだけ（現方針、3.10）。(a) は無確認書き込み（3.1 と背反）に踏み込む。
- **ptygrid が書いた登録であることの印**: MCP 登録はキー名 `queen` しか手掛かりが無く手書きと区別できない。
  (a) TOML にマーカーコメントを書く、(b) 印を置かず `url` のポート一致で近似（現方針、`ours`）。
- **バックアップの世代管理**: (a) 無制限（`teams_hooks` 踏襲、現方針）、(b) N 世代で打ち切る。
- **解除時のコメント巻き添え**: (a) 可視化して承認を取る（現方針、3.7）、(b) `toml_edit` の下層 API で `decor` を退避
  してから remove（可能かは未確認）。
- **登録の検証方法**: (a) `claude mcp get queen` で設定を読むだけ（現方針）、(b) `/mcp` へ到達性確認まで行う
  （E-12 で失敗しうる）。
- **未確認（Windows）**: npm シムと PATH 探索は `spec-init-5.0.2.md` 9 章と同じ未確認事項で、U8 の範囲で扱う。
