# 仕様書: `ptygrid init` — 設定ファイルの自動生成（Phase 5.0.2）

作成日: 2026-07-29 / 改訂: 2026-07-30（D4 の意味づけ格下げとローカル LLM プローブの追加 — §3.9 / §4.4）
/ ステータス: **ドラフト（未実装）** / 対象: `ptygrid.yml` の初回生成

> **採番（3 行）**: `5.0.2` / `5.0.3` は [plan.md](../design/plan.md) §1 脚注※のとおり食い違ったまま
> **欠番**だった空き番号。この番号を onboarding 系に充て、**5.0.2 = `ptygrid init`（本書）/
> 5.0.3 = Queen MCP 登録の代行**と確定した（Memory / Provider は **5.0.6 以降**へ付け直し、非スコープ）。

関連: [plan.md](../design/plan.md) §1・§4、[CONTRACT.md](../../CONTRACT.md)、
[ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md)、
[queen-universal-register.md](../design/queen-universal-register.md)（5.0.3 の下敷き）

---

## 1. 目的と背景

**初回導入と日々の増減で `ptygrid.yml` を人間が手書きしなくて済むようにする。** 作業フォルダと
ホスト環境を走査し、意味を持つ `ptygrid.yml` を**コメント付きテキストとして生成**して提示する。
生成物は書き込む前に必ず `parse_config` を通し、通らなければ 1 バイトも書かない。

出発点は、手書き前提の設計が作者本人の環境で維持できなくなったことである。ドッグフーディング中の
個人設定は **790 行**まで膨張し、**21 体中 6 体が無参照**、**workflow 4 本が化石**になっていた
（2026-07-29 に 506 行へ棚卸し）。config-as-code は宣言が正しく保たれている前提で価値を出す設計
であり、その前提が崩れていた。新規ユーザー側も同じ弱点を持つ: 設定が無いとき ptygrid は**シェル
1 枚だけを開き**（起動時の自動ロードは `allowDefault: false` で呼ばれ、`not_found:` エラーに
`App.svelte:1525-1527` がフォールバックする）、そこから先は `example/` を読んで自分で YAML を
書くしかない。**この「シェル 1 枚だけ出ている状態」を init の起動点と定める**（6 章）。

init が塞ぐのは初回導入側の穴で、棚卸しは 2.2 のとおり別 patch に切る。実体は**検出 + テンプレート
文字列 + 既存 `parse_config` による自己検査**であり、新しいパーサも新しい信頼境界も作らない——
「既存の検証経路の外に出ない」ことで投資対効果を成立させる。

---

## 2. スコープ / 非スコープ

### 2.1 スコープ

- ホスト環境と作業フォルダの**検出**（PATH 上の agent CLI / プロジェクト種別 / git / ローカルルータ）
- **ローカル LLM のプローブ**（ユーザーがボタンを押したときだけ走る別命令。`scan` には混ぜない — §3.9）
- 検出結果からの **`ptygrid.yml` 生成**（テンプレート埋め込み方式）と**自己検査**
- **プレビュー UI**（検出結果・生成先・生成物・自己検査結果を見せてから書き込む）
- 既存設定がある場合の**別名（sidecar）生成 + 差分表示**、アトミック書き込み（temp + rename）
- Tauri command 4 本の追加 / CONTRACT.md 先行追記 / userguide・`ptygrid-yml-guide.md` の同時更新

### 2.2 非スコープ

- **既存 `ptygrid.yml` を読んで書き戻すこと** — 禁じ手。根拠は 3.1 の round-trip 実測
  （コメント消失・`env` 出力順不定など）。ユーザーの設定はコメントが本体であり、壊す実装は採らない
- **`init --prune`（未参照定義の削除・棚卸し）** — 既存ファイルの読み書きを伴い設計がまるごと
  変わるため**別 patch に切る**
- **Queen MCP 登録の代行** — **5.0.3** に譲る（8 章）
- **`workflows:` / `team_presets:` の生成** — 化石化した workflow 4 本の負債経緯から、init が
  DAG を書き起こす害のほうが大きい（9 章に未決として残す）
- **CLI サブコマンド `ptygrid init`** — 5.0.2 では実装しない（3.6）
- **`~/.claude.json` 等ホーム設定の書き換え** — init は ptygrid 自身の設定しか書かない

---

## 3. 設計

### 3.1 生成方式 — テンプレート埋め込み（serde シリアライズは使わない）

**決定: 生成物は Rust 側の文字列テンプレートとして組み立てる。`serde_norway::to_string` は使わない。**
根拠は round-trip 実測（`serde_norway 0.9.42`）:

| 実測した挙動 | init にとっての意味 |
|---|---|
| コメントが完全に消える | `example/*/ptygrid.yml` も `ptygrid.example.yml` もコメントが本体。生成物の価値が消える |
| `env`（`HashMap`）の出力順が不定（挿入順でもソート順でもない） | 同じ入力から同じバイト列が出ない = **冪等性と diff 安定性が壊れる** |
| 空 `Config` から `agents: []` / `processes: []` だけが出る | `config.rs:29-32` に `skip_serializing_if` が無い |
| 折り畳みスカラー `>-` が 1 行の引用文字列に潰れる | `instructions` の可読性が死ぬ |

テンプレートは `init.rs` の文字列定数として持つ（`include_str!` は検出分岐に使えず不可）。生成
YAML は既存のキー命名（`AgentDef` は snake_case、`WorkflowDef` 系は camelCase）を踏襲し、init が
生成するのは前者の範囲だけ。

### 3.2 検出するもの

コスト表記は「小 = 数十行 / 中 = 100〜300 行 / 大 = それ以上」。

| # | 検出対象 | 取り方 | 外部プロセス | コスト | 検出できなかったとき |
|---|---|---|---|---|---|
| D1 | PATH 上の agent CLI | `PATH` を `split_paths` → `KNOWN_AGENTS`（14 個、`pty.rs:89-104`）を join → `is_file`（Unix は実行ビット / Windows は `PATHEXT` 展開） | 不要 | **小** | `agents:` を生成せず、コメントアウトの雛形だけ出す（4.1） |
| D2 | プロジェクト種別 | work 直下の `Cargo.toml` / `package.json` / `pyproject.toml` / `go.mod` の存在確認 | 不要 | **小** | `processes:` 案内ブロックを出さない |
| D3 | git リポジトリか | `git_service::repository_root` | **要**（`git`） | **小** | worktree の案内を出さない。**`git` 未インストールでも init は成功する** |
| D4 | 既定ルータポートに何か居るか（**弱い手がかり**） | `127.0.0.1:3456` へ `TcpStream::connect_timeout`（200ms・1 回。既定ポートは `router.settings.json` / `example/team-preset` に登場） | 不要 | **小** | `local` エージェントの雛形を出さない |
| D5 | 既存の設定ファイル | `resolve_config_path_pure`（`config.rs:1599-1641`）と同じ探索順。**内容は読むが parse しない**（3.4） | 不要 | **小** | 衝突なしとして通常生成 |
| D6 | projects root | `app_settings` の `projectsRoot` / `list_dirs_at` | 不要 | **小** | UI の生成先候補列挙のみ。無ければ現在の work だけ |
| D7 | 既存 MCP 登録 | **5.0.2 では検出しない**（TOML パーサ未依存等、5.0.3 へ） | — | — | — |

**決定: 検出はすべて best-effort。個々の検出が失敗しても init 全体は失敗しない**——代わりに何を
見て何を出したかを生成物の先頭コメントに必ず記録する（4 章の `# 検出:` 行）。D4 の TCP 接続を
含め検出は呼び出しスレッド内で完結させ、同期 I/O を PTY / async ランタイムに載せない
（`notifications.rs:311-319` の作法）。

**D4 は「何が居るか」を答えない。** TCP connect が通ったという事実は、そのポートで LISTEN して
いるプロセスがあることしか意味しない——HTTP を話すかも、`/v1/messages` を持つかも、そもそも
LLM かも分からない。D4 の意味づけは §3.9 で**「既定ルータポートに何かが居るという弱い手がかり」
に格下げ**され、実在確認は別命令のプローブ（§3.9）が担う。D4 自体を削らないのは
`InitScanReport.routerPort` の破壊的変更を避けるためで、`scan` の速度契約（best-effort・
非ブロッキング）も無変更のまま維持する。

### 3.3 生成先と trust の関係

**決定: 既定の生成先は `<work>/ptygrid.yml`（`ConfigOrigin::Project`）。** `~/.ptygrid/ptygrid.yml`
（`Global`）は UI で明示的に選んだときだけのオプション。`<launch>`（起動 cwd）には生成しない——
GUI 起動では起動 cwd がユーザーの意図と結びつかない。`~/.ptygrid/` を作るコードは現状どこにも
無いので、Global 選択時は **init が `create_dir_all` する**。

**決定: 「`<launch>` には生成しない」を backend 側でも担保する。** `dir` が省略され、かつ
読み込み済み config も無い場合、`init_scan` / `init_preview` / `init_write` は
`std::env::current_dir()`（= 起動 cwd）へフォールバックせず、**`no_target_dir:` エラー**を返す
（`commands.rs::init_dir`）。他の command（`git_status` 等）が使う `project_dir()` は
現在ロード済み config → `current_dir()` の順にフォールバックするが、init 系だけはこの
フォールバックを持たない——起動 cwd への暗黙の書き込みを構造的に防ぐための意図的な違いである。

**決定: Project 生成物は trust 確認の対象になる。init はこれを迂回せず、`trust::add_trusted`
（`trust.rs:194`）も呼ばない。** `trust::is_trusted_pure`（`trust.rs:94-99`）は `Global` / `Default` を
無条件 trusted、`Project` / `Launch` は trust ストアに載るまで untrusted とするため、
**`~/.ptygrid` に生成すればプロンプト無しで autostart が走る**——生成先の選択がそのまま security
posture を変える。だからこそ Project を既定にする: 生成物にも autostart は入りうるが（既定 false、
直後に true へ書き換えられる）、**「ptygrid 自身が生成したファイルだから」を理由に S2 gate を
免除しない**——生成物か否かと信頼できるフォルダか否かは独立の問題であり、後者は既存どおり
ユーザーの明示同意で決める（`trust.rs:1-22`）。

### 3.4 既存ファイルがあるとき — 上書きせず sidecar に書く

**決定: 生成先ディレクトリに `ptygrid.yml` 本体が既に存在するときは、それを一切変更せず、
生成物は隣の別名ファイルに書く。** sidecar になるかどうかは「`resolve_config_path_pure` が
何らかの既存設定を見つけたか」ではなく、**`<dir>/ptygrid.yml` そのものが存在するか**だけで
決まる（`init.rs::sidecar_needed`）。

| 項目 | 決定 |
|---|---|
| sidecar 判定条件 | **`<dir>/ptygrid.yml` が存在するときだけ**。単独の `mterm.yml`（旧名）は sidecar を発生させない——生成先は素直に `<dir>/ptygrid.yml` のままになり、その書き込みだけを `init_write` が `legacy_config:` で拒否する（後述）。`ptygrid.yml` と `mterm.yml` が両方存在するときは前者を検出した時点で sidecar になり、旧名は既に探索順で無効化されているため書き込みは通る |
| sidecar のファイル名 | **`ptygrid.init.yml`**（生成先ディレクトリ直下） |
| 既に sidecar もある場合 | **上書きする**（init の作業用出力であり、ユーザーの資産ではない） |
| 既存ファイルの扱い | **テキストとして読み、行差分の表示にだけ使う。parse も書き戻しもしない** |
| 採用のしかた | ユーザーが手でマージするかリネームする。init は代行しない |
| 差分の見せ方 | **行単位の 2 ペイン表示**（左 = 既存 / 右 = 生成物）。構造マージ表示は 3.1 の問題に戻るため作らない |

frontend は書き込みが拒否されるケースを `init_preview` の結果だけから予測できる:
**`scan.existing?.legacy === true && preview.sidecar === false`** が真のとき、`init_write` は
必ず `legacy_config:` で失敗する（`<dir>/ptygrid.yml` が存在しないのに legacy が存在する
= 単独の `mterm.yml` のケース）。

`ptygrid.init.yml` は探索対象（`ptygrid.yml` / `mterm.yml`）に含まれず、watcher のファイル名
フィルタにも掛からないため**`config-changed` を誤射しない**。2.2 が禁じるのは書き戻し経路であり、
差分表示用の読み取り専用アクセスは安全である。

### 3.5 自己検査と書き込みフロー

**決定: 生成した文字列が `parse_config` を通ることを、書き込み関数の事後条件として仕様に含める。**

```
検出（3.2）
  → テンプレート組み立て（3.1）        ← ここまで純関数・I/O ゼロ
  → parse_config(&content)              ← validate_team_presets / validate_workflows を内包
       ├─ Err → 書かない。エラーを UI に返す（ptygrid のバグとして扱う）
       └─ Ok  → プレビュー表示（ユーザーが編集可）
  → init_write(content)
       → parse_config(&content) を再実行 ← 編集後の内容に対してもう一度
       ├─ Err → 書かない
       └─ Ok  → temp へ書く → rename（アトミック）
```

`parse_config` は `pub`（`config.rs:1323-1328`）で `validate_team_presets` / `validate_workflows`
を必ず内部で呼び、I/O 不要で unit テストに組める。プレビューは編集可能で編集後にも同じ検査を
掛けるため、「書き込まれた設定は必ず parse を通る」が経路によらず成立する。書き込みは
**temp + rename**（前例: `trust.rs:101-116` / `app_settings.rs:71-86`）——素の `fs::write` は
途中まで書かれた YAML をロードが読むと parse エラーになるため採らない（`config.rs:1494`）。

**自己検査で担保されないもの**: agents 内の名前重複（`ConfigManager::resolve_def` は `.find()` で
先勝ちする）は**init が生成器側で名前一意性を保証する**（D1 は `KNOWN_AGENTS` 由来で元から一意。
既存ファイルとの重複は sidecar 方式なので実害なし）。`cmd` の実行可能性 / `cwd` の存在 / `${VAR}`
の解決も同様に担保されない。

コメントアウトは**キー行ごと**に行う。ただしその理由は当初の想定（「値が null になり parse に
失敗する」）とは異なる。`serde_norway 0.9` で実測した挙動は次のとおり
（`init.rs` の `partially_commented_out_block_is_caught_by_the_self_check` 参照）:

| 残し方 | `parse_config` | 理由 |
|---|---|---|
| `processes:` の後に値ノードを一切残さない（キー行のみ） | **Ok** | フィールドが missing 扱いになり `#[serde(default)]` が空 `Vec` を供給する |
| `processes: null` / `processes: ~` | **Err** | 明示的な unit 値になり、`Vec` に対する型エラーになる |
| 半端に残ったリスト要素（例: `- name: a` で `cmd` が欠落） | **Err** | 必須フィールド不足で構造体のデシリアライズが失敗する |

つまり「キー行だけ残す」こと自体は parse を落とさない。それでもキー行ごとコメントアウトする
方針を維持するのは**正しさのためではなく可読性のため**——中途半端に値だけが残った雛形は
「どこまでコメントを外せば有効になるか」が分かりにくく、キー行から丸ごと揃えたほうが
ユーザーが編集しやすい。実際に自己検査で必ず捕まるのは、明示的な `null`/`~` と、半端な
リスト要素を残した場合である（7 章に回帰テスト）。

### 3.6 呼び出し口 — GUI を主とし、CLI サブコマンドは 5.0.2 では作らない

**決定: 5.0.2 の入口は GUI のみとする。** CLI 化自体は構造上無理がない（`__tmux-compat` の argv
ディスパッチ前例、`lib.rs:38-47`）が、**Windows のリリースビルドにはコンソールが無く**
（`main.rs:2`）、「何を検出し何を書こうとしているか見せて確認を取る」init の本質が成立しない。
将来足すとしても副の入口に留める（9 章）。

### 3.7 生成物の `autostart` は既定 false

**決定: init が生成する `agents:` / `processes:` の `autostart` はすべて `false` にする。**
初回に勝手にプロセスが立ち上がらないことを優先する（`example/basic/ptygrid.yml` の
`autostart: true` は人間が意図した設定であり、機械が推測した定義とは前提が違う）。副作用として
**init 直後は trust プロンプトが出ない**（`App.svelte:913-917`）。3.3 と整合する意図した挙動で、
ユーザーが true に変えて再読み込みしたとき初めて信頼確認が走る。

### 3.8 生成後の反映

**決定: 書き込み command とロードは分離する。`init_write` は自分で `ConfigManager::load` を
呼ばない。** `load` は `inner` を `Mutex` で保持する（`config.rs:1413`, `:1454`）ため、同じロックを
取りながら書くとデッドロックし得るからである。`<work>/ptygrid.yml` へ書くと watcher が約 300ms 後
に `config-changed` を emit するが frontend は自動 reload しない（`stores.svelte.ts:310-312`）。
反映方法（自動ロードかトーストか）は**未決**（9 章）とし、UI 仕様は当面 (a) を前提に書く。sidecar
への書き込みではファイル名フィルタにより反映の問題自体が発生しない。

### 3.9 ローカル LLM プローブ — 実在確認をボタン起動の別命令に切る

**3.9.0 前提の訂正（旧記述が置いていた仮定）**

本書の初版は「ローカル LLM を Claude Code から使うには **coderouter のような translation 層を
挟む**」を暗黙の前提にしていた（D4 の名前が「ローカル LLM ルータの生死」であること、§4.2 の
生成コメントが `claude --settings router.settings.json` + `ANTHROPIC_BASE_URL:
"${CODEROUTER_URL}"` という router 経由の形しか示さないこと）。**この前提は現状に合わない。**

**Ollama は v0.14.0 以降、LM Studio も、Anthropic Messages API 互換の `/v1/messages` を持つ。**
したがって `ANTHROPIC_BASE_URL` をローカルのエンドポイントへ直接向ければ Claude Code は繋がり、
translation 層は要らない。coderouter は「唯一の道」ではなく**選択肢のひとつ**に格下げする
（複数バックエンドへのルーティングなど、router 固有の価値は残る。§4.2 のコメントブロック自体は
§4.4 の抑止条件に当たらないかぎり従来どおり出す）。

**3.9.1 決定**

**決定: `scan` の D4（`127.0.0.1:3456` への 200ms TCP connect 1 回）はそのまま残す。**
削ると `InitScanReport.routerPort` が破壊的変更になるためで、意味づけだけを
「既定ルータポートに何かが居るという**弱い手がかり**」へ格下げする。**プローブ結果があるときは、
プレビュー生成でプローブ側を優先する**（§4.4）。

**決定: 実在確認は `GET /v1/models` に統一し、ベンダー分岐を持たない。** Ollama も LM Studio も
coderouter も `/v1/models` に答える。「Ollama ならこう、LM Studio ならこう」という製品名での
分岐を書かずに 1 本の手順で識別できる——ベンダー分岐は増えるたびに壊れる保守負債であり、
init が抱えるものではない。

**決定: Anthropic Messages API 互換の「確証」は、`GET /api/version` が取れた場合に限る。**
`/v1/models` が答えることは `/v1/messages` があることを**証明しない**——OpenAI 互換のみのサーバも、
Ollama v0.14.0 未満も、まったく同じ応答を返す。したがって確証は 3 値で持つ:

| `anthropic` | 意味 | ラベル例 |
|---|---|---|
| `Some(true)` | 確証あり（現状は `/api/version` が `0.14.0` 以上の Ollama のみ） | `Ollama 0.14.3` |
| `Some(false)` | 確証をもって非対応（`/api/version` は取れたがバージョンが下回る） | `Ollama 0.13.1` |
| `None` | 不明（OpenAI 互換の応答があっただけ） | `127.0.0.1:1234 (OpenAI 互換の応答)` |

**決定: 対象ポートは既定 3 本 + 手入力欄。** 既定は `11434`（Ollama）/ `1234`（LM Studio）/
`3456`（coderouter）。UI に追加ポートの入力欄を 1 つ置き、**最大 4 本**まで足せる
（`MAX_EXTRA_PORTS`）。既定 3 本と合わせて重複除去・昇順にしてから当たる。
**127.0.0.1 以外には接続しない。**

**決定: プローブは `scan` に混ぜず、ユーザーがボタンを押したときだけ走る別命令
（`init_probe_llm`）にする。** 理由は速度契約: `scan` は「設定を作る」を押した直後に走り、
検出はすべて数十 ms 級（§3.2 のコスト表は全項目「小」）で終わる前提で UI が組まれている。
HTTP を 3〜7 本投げるプローブは秒級になりうるため、`scan` に載せると**現行の速度契約が壊れる**。
別命令にすれば「待たされてよい代わりに、`scan` は無変更」という交換が成立する。

**3.9.2 時間とバイト数の上限**

プローブは**待たされてよい**が、**無限に待つことは許さない**。上限は定数で持つ:

| 定数 | 値 | 何を止めるか |
|---|---|---|
| `PROBE_PORT_TIMEOUT` | 1 秒 | 1 ポートあたりの接続 + 読み取り |
| `PROBE_TOTAL_BUDGET` | 3 秒 | 全体。使い切ったら `timedOut: true` で打ち切り、**間に合った分は返す** |
| `PROBE_MAX_BYTES` | 64 KiB | `/v1/models` 応答本文の読み取り上限（巨大応答での OOM を防ぐ） |
| `PROBE_MAX_MODELS` | 20 件 | `models` の件数上限 |
| `MAX_EXTRA_PORTS` | 4 本 | 手入力で足せるポート数 |

ポートごとに短命スレッドを立てて並行に走らせ、全体予算で回収を打ち切る。**応答が無いことは
エラーではない**（`endpoints` が空になるだけ）。エラー接頭辞は `bad_port:` の 1 つだけで、
`0` を含むか追加分が `MAX_EXTRA_PORTS` を超えた場合に返す。`init_scan` / `init_preview` と同じく
**ディスクには一切触らない**。

**3.9.3 誤検出への態度**

**決定: 確証が無いものを「ある」と書かない。** `1234` は LM Studio 専用ではない汎用ポートであり、
**TCP connect が通っただけで「LM Studio あり」と生成物に書くのは誤検出**である。プローブは
2 段構えでこれを避ける: (1) TCP ではなく `GET /v1/models` の 200 応答と `data[].id` の抽出まで
通ったものだけを「応答あり」とする、(2) それでも `/v1/messages` の存在は未確認なので、
`anthropic == Some(true)` 以外は**有効行にしない**（§4.4）。

見つからなかったことより、**居ないものを居ると書くことのほうが害が大きい**——前者はユーザーが
手入力欄でポートを足せば回復するが、後者は「生成された設定が動かない」という形で init 全体の
信頼を落とす。ラベルも同じ方針で、**名乗り（`/api/version`）が取れたときだけ製品名を入れ**、
取れなければ `127.0.0.1:<port> (OpenAI 互換の応答)` のように事実だけを書く。

**3.9.4 代替案と却下理由**

- **ポートのレンジスキャン**（例: 1000-65535 や「よくある LLM ポート 50 本」を舐める）: 却下。
  無関係なローカルサーバに GET を投げることになる。ptygrid が勝手にユーザーのマシンを
  ポートスキャンする挙動は、init が持ってよい権限を明らかに超える。当たるのは**既定 3 本 +
  手入力のみ**に固定する。
- **プローブを `scan` に混ぜる**: 却下。§3.9.1 のとおり `scan` の速度契約（数十 ms 級・
  非ブロッキング）を壊す。ユーザーが「設定を作る」を押すたびに秒単位で待たされる。
- **frontend が生成テキストを後から書き換える**（プローブ結果を JS 側で YAML に差し込む）: 却下。
  **生成は backend の責務**（§3.1・§3.5）。frontend が生成物を組み立てると、`parse_config` に
  よる自己検査を通す前の文字列が 2 系統でき、「書き込まれた設定は必ず parse を通る」という
  不変条件（§3.5）の保証点が 1 箇所でなくなる。プローブ結果は `init_preview` に**引数として
  渡す**（`llm`）。
- **`/v1/messages` に実際に POST して確かめる**: 却下。確証としては最も強いが、**推論が走る**——
  ユーザーのマシンで意図しないモデルロードと GPU 消費を起こし、ローカルモデルによっては数十秒
  ブロックする。検出のために副作用のある呼び出しを投げない（§3.7 の「勝手にプロセスを立ち上げ
  ない」と同根）。確証は副作用のない `GET /api/version` までに留める。
- **`/api/version` が無いものを Ollama 以外と断定して `Some(false)` にする**: 却下。
  `/api/version` を持たない Anthropic 互換サーバは現に存在しうる。「不明」を `None` として
  そのまま持ち、コメント行として出して**ユーザーに判断を返す**（§4.4）。

---

## 4. 生成される `ptygrid.yml`（実物）

書式は `example/` 配下と `ptygrid.example.yml` に実在する書き方に揃える。

### 4.1 最小構成（`claude` だけが見つかった場合）

```yaml
# ptygrid.yml — ptygrid init が生成しました (2026-07-29)
# 検出: claude (PATH) / Cargo.toml / git リポジトリ
# 未検出: ローカル LLM ルータ (127.0.0.1:3456)
# 中身はすべて手で編集できます。全ブロックの注釈つき例は ptygrid.example.yml、
# 用途別の見本は example/ を参照してください。

project: my-app          # 作業フォルダ名から。ヘッダーに出る表示名

agents:
  - name: claude
    cmd: "claude"
    cwd: "."
    autostart: false     # 読み込みと同時に起動するなら true（初回は手動 ▶ 起動）

# Cargo.toml を検出しました。dev サーバーやテスト watch を常駐させるなら
# 次のブロックの各行の先頭 # を外してください（agents と同じフィールドを持ちます）。
# processes:
#   - name: dev
#     cmd: "<常駐させたいコマンド>"
#     cwd: "."
#     autostart: false
#     autorestart: on-failure   # 異常終了時のみ再起動

# git リポジトリを検出しました。ペインごとに linked worktree を切るなら
# example/worktree を参照してください（init は worktree: を生成しません）。

# チーム一括起動 (team_presets:) は example/team-preset、
# DAG オーケストレーション (workflows:) は example/adaptive-orchestration を参照。
```

先頭コメントは「見つけたもの（`# 検出:`）」だけでなく「探したが見つからなかったもの
（`# 未検出:`）」も記録する——不在ブロックの理由をその場で説明するためで、上の例では
ローカル LLM ルータが応答しなかったことがそれに当たる（`init.rs::missing_items`）。

D1 が何も見つけなかった場合は `agents:` ブロックごと出さず、`# agents:` から始まる
**キー行ごとコメントアウトした雛形**（3.5）と「CLI を入れたらコメントを外す」案内だけを出す。

### 4.2 複数の CLI が見つかった場合

```yaml
# ptygrid.yml — ptygrid init が生成しました (2026-07-29)
# 検出: claude / codex / grok (PATH) / package.json / git リポジトリ /
#       ローカル LLM ルータ 127.0.0.1:3456 (応答あり)

project: my-app

# queen: ペイン間の読み書き・メッセージ・spawn を仲介する内蔵 MCP サーバー。
# 各 CLI への登録コマンドはツールバーの Queen バッジからコピーできます。
queen:
  enabled: true
  port: 39237

agents:
  - name: claude
    cmd: "claude"
    cwd: "."
    autostart: false

  - name: codex
    cmd: "codex"
    cwd: "."
    autostart: false

  - name: grok
    cmd: "grok"
    cwd: "."
    autostart: false

  # ローカル LLM ルータ (127.0.0.1:3456) が応答しました。使うならコメントを外し、
  # router.settings.json を用意してください（env だけに頼らず --settings を渡すのが
  # 確実な理由は example/team-preset/ptygrid.yml を参照）。
  # - name: local
  #   cmd: "claude --settings router.settings.json"
  #   cwd: "."
  #   env:
  #     ANTHROPIC_BASE_URL: "${CODEROUTER_URL}"
  #   autostart: false

# package.json を検出しました。dev サーバーやテスト watch を常駐させるなら
# 次のブロックの各行の先頭 # を外してください（agents と同じフィールドを持ちます）。
# processes:
#   - name: dev
#     cmd: "npm run dev"
#     cwd: "."
#     autostart: false
#     autorestart: on-failure   # 異常終了時のみ再起動

# git リポジトリを検出しました。ペインごとに linked worktree を切るなら
# example/worktree を参照してください（init は worktree: を生成しません）。

# チーム一括起動 (team_presets:) は example/team-preset、
# DAG オーケストレーション (workflows:) は example/adaptive-orchestration を参照。
```

上の `# ローカル LLM ルータ (127.0.0.1:3456)` ブロックは **D4（弱い手がかり）由来**であり、
「そのポートに何かが LISTEN していた」以上のことは主張していない。**「ローカル LLM を使うには
router を挟むしかない」という読みは §3.9.0 のとおり誤り**で、Ollama v0.14.0 以降 / LM Studio は
`ANTHROPIC_BASE_URL` を直接向ければ繋がる。実在を確かめたうえで直接続きの定義を出すのは
§4.4 のプローブ経路の役目であり、このブロックは**プローブ結果に同じポートが含まれない場合だけ**
従来どおり出す（§4.4 の抑止規則）。

`processes:` の雛形の `name:` は常に `dev` で固定（npm 前提の `web` ではない）。`cmd:` は
npm プロジェクトのときだけ `"npm run dev"` を出し、それ以外の種別（cargo / python / go）では
`"<常駐させたいコマンド>"` というプレースホルダになる（`init.rs::render_config`、
`marker_for` と組み合わせた分岐）。

### 4.3 生成しないもの

- **`team_presets:` / `workflows:`** — 2.2 のとおり非スコープ。コメントで
  「チーム一括起動は `example/team-preset`、DAG は `example/adaptive-orchestration` を参照」と案内するに留める
- **`teammates:` / `agents[].teams` / `worktree:`** — オプトイン機能。検出から推測しない
  （D3 が git repo を見つけても worktree は生成せず、案内コメントだけ出す）
- **`notifications:` / `mcp:`** — 秘密情報や互換フラグを機械が推測しない

### 4.4 プローブ結果があるときの `agents:` 生成

**決定: 生成規則は「確証ありは有効行、それ以外はコメント行」の 1 本だけ。** プローブ結果
（`InitProbeReport.endpoints`）は `init_preview` の `llm` 引数として backend に渡り、
`init.rs::render_config` が生成する（§3.9.4 のとおり frontend は生成テキストに触らない）。

| 条件 | 出し方 |
|---|---|
| `anthropic == Some(true)` | **有効行**。1 エンドポイントにつき 1 定義 |
| `anthropic == Some(false)` / `None` | **コメント行**で同じ形。直前に「`/v1/messages` が応答するかは未確認」の趣旨を 1 行入れる |

名前は **`local-<port>`**（衝突回避のため常にポート付き）。`cmd` は `claude --model <models[0]>`、
`env` に `ANTHROPIC_BASE_URL: "http://127.0.0.1:<port>"` を置く。Ollama の場合は
`ANTHROPIC_AUTH_TOKEN: "ollama"` も添える（ドキュメント上「必須だが無視される」ため、値に意味は
なく秘密でもない——§4.3 の「秘密情報を推測しない」に抵触しない）。**`autostart` は §3.7 のとおり
常に `false`**（プローブ経路でも例外を作らない。`trust::add_trusted` も呼ばない）。他に検出した
モデルがあれば `# 他に: a / b / c` を 1 行コメントで添える。

```yaml
  # 127.0.0.1:11434 は Anthropic Messages API 互換です (Ollama 0.14.3)。
  - name: local-11434
    cmd: "claude --model qwen3-coder:30b"
    cwd: "."
    env:
      ANTHROPIC_BASE_URL: "http://127.0.0.1:11434"
      ANTHROPIC_AUTH_TOKEN: "ollama"   # 必須だが無視される
    autostart: false
    # 他に: gpt-oss:20b / llama3.1:8b

  # 127.0.0.1:1234 は /v1/models に応答しましたが、/v1/messages が応答するかは未確認です。
  # 動くかどうかは実際に起動して確かめてください。
  # - name: local-1234
  #   cmd: "claude --model openai/gpt-oss-20b"
  #   cwd: "."
  #   env:
  #     ANTHROPIC_BASE_URL: "http://127.0.0.1:1234"
  #   autostart: false
```

**抑止規則**: プローブ結果が 1 件でもあるとき、§4.2 の 3456 用コメントブロック
（`router.settings.json` の案内）は**同じポートが結果に含まれる場合だけ**抑止する。含まれない
なら従来どおり出す——D4 が拾ったポートについてプローブが何も言えていない状況では、弱い手がかり
でも残すほうが情報量が多い。

**決定性は維持する**（§3.1 の `env` 順序不定問題と同じ要求）。同じ入力（`scan` + `llm` + 当日日付）
から**同じバイト列**が出ること: `models` は受け取った順を保持し（ソートし直さない）、
エンドポイントは**ポート昇順**で出す。

**`llm` が `None` または空のときの出力は現行とバイト単位で同一**であること。これはプローブが
既存の生成経路に対して純粋に additive であることの担保であり、専用テストを 1 本置く（§7.2）。

---

## 5. CONTRACT.md 追記項目

既存設計原則との整合は 3 章の各決定に書いた（project 境界 = 3.3、推測拒否 = 3.2 / 3.7、信頼確認 =
3.3、既存の検証経路の外に出ないこと = 3.5）ので繰り返さない。本章は CONTRACT 追記項目と wire の
形だけを定める。

### 5.1 CONTRACT.md 追記項目（実装前に先行追記）

1. `InitTarget` / `InitScanReport` / `InitPreview` / `InitWriteResult` の確定形（フィールド名は camelCase）
2. Tauri command 4 本のシグネチャ（5.2。`init_probe_llm` を含む）
3. **生成物の不変条件**: 「`init_write` が書き込む内容は必ず `parse_config` を通っている」
4. **sidecar 規約**: `ptygrid.init.yml` / 設定探索の対象外 / watcher のファイル名フィルタに掛からない
5. **新規イベントを追加しない**ことの明示（生成後の通知は既存 `config-changed` で足りる）
6. **非回帰宣言**: `load_config` / `ConfigInfo`（全 field）/ `config-changed` /
   `trust_working_folder` / `is_working_folder_trusted` は**不変**。本節はすべて additive
7. `LocalLlmEndpoint` / `InitProbeReport` の確定形（camelCase）と `init_probe_llm` のシグネチャ、
   `bad_port:` エラー接頭辞（§3.9）
8. **`init_preview` の `llm` 引数追加が additive であること**の宣言:
   `llm` が `None` / 空のとき出力は**現行とバイト単位で同一**。`init_scan` /
   `InitScanReport`（`routerPort` を含む全 field）は**不変**

### 5.2 wire（Tauri command）の形

```ts
type InitTarget = "project" | "global";  // project = <work>/ptygrid.yml, global = ~/.ptygrid/ptygrid.yml

interface InitScanReport {
  dir: string;                     // 走査した作業フォルダ（絶対パス。`.`/`..` を字句的に畳んだ値。
                                    // symlink 解決＝canonicalize はしない — `absolute_dir`）
  agents: string[];                // PATH で見つかった名前（KNOWN_AGENTS の宣言順）
  projectKinds: string[];          // "cargo" | "npm" | "python" | "go"（複数可・空可）
  gitRepo: boolean;
  routerPort: number | null;       // 応答したローカルルータのポート。無ければ null
  existing: ExistingConfig | null; // 探索順で最初に当たった既存設定
}
interface ExistingConfig { path: string; origin: "project"|"launch"|"global"|"default"; legacy: boolean }
                                   // legacy: true = mterm.yml（旧名）。origin は既存 ConfigOrigin
                                   // をそのまま再利用するため型としては 4 値だが、scan() の経路
                                   // （resolve_config_path_pure 由来）が実際に返すのは
                                   // "project" | "launch" | "global" の 3 値のみ。"default" は
                                   // 理論上の型としてのみ存在し、init からは出ない
interface InitPreview {
  content: string;                 // 生成された YAML 全文
  path: string;                    // 書き込み予定の絶対パス（sidecar のときは sidecar 側。
                                    // dir と同じく字句正規化済み・canonicalize はしない）
  target: InitTarget;
  sidecar: boolean;                // <dir>/ptygrid.yml が既に存在するため別名に書く場合 true
                                    // （mterm.yml 単独の存在は sidecar にしない — 3.4）
  valid: boolean;                  // 自己検査（3.5）の結果
  error?: string;                  // valid=false のときの parse / validate エラー
  existingContent?: string;        // sidecar のとき、差分表示用に読んだ既存の生テキスト
  scan: InitScanReport;
}
interface InitWriteResult {
  path: string; bytes: number; sidecar: boolean;
  trustPromptExpected: boolean;    // target=project かつ autostart 付き定義があれば true
}

// --- ローカル LLM プローブ（§3.9。additive） ---
interface LocalLlmEndpoint {
  port: number;                    // 応答したポート
  models: string[];                // GET /v1/models の data[].id。PROBE_MAX_MODELS で打ち切る
  anthropic: boolean | null;       // true = 確証あり / false = 確証をもって非対応 /
                                    // null = 不明（OpenAI 互換の応答があっただけ）
  label: string;                   // 表示用。名乗りが取れたときだけ製品名を入れる
                                    // 例: "Ollama 0.14.3" / "127.0.0.1:1234 (OpenAI 互換の応答)"
}
interface InitProbeReport {
  probedPorts: number[];           // 実際に当たったポート（重複除去・昇順）
  endpoints: LocalLlmEndpoint[];   // 応答があったものだけ。ポート昇順
  timedOut: boolean;               // 全体予算（3 秒）を使い切って打ち切ったか
}
```

| command | args | returns | 説明 |
|---|---|---|---|
| `init_scan` | `{ dir?: string }` | `InitScanReport` | 検出のみ。ディスクに**書かない** |
| `init_preview` | `{ dir?: string, target?: InitTarget, llm?: LocalLlmEndpoint[] }` | `InitPreview` | 生成 + 自己検査。ディスクに**書かない**。`llm` 省略時は現行と同一出力（§4.4） |
| `init_write` | `{ dir?: string, target?: InitTarget, content: string }` | `InitWriteResult` | `content` を再検査してから temp + rename で書く |
| `init_probe_llm` | `{ ports?: number[] }` | `InitProbeReport` | ローカル LLM のプローブ（§3.9）。`ports` は**追加ポートのみ**（既定 3 本と合わせて重複除去・昇順）。ディスクに**書かない** |

`init_probe_llm` のエラー接頭辞は **`bad_port:`** の 1 つだけ（`0` を含む、または追加分が
`MAX_EXTRA_PORTS` = 4 本を超えた場合）。**応答が無いことはエラーではない**——`endpoints` が空の
`InitProbeReport` が返るだけである。`dir` を取らないため `no_target_dir:` の対象外。

`dir` 省略時は、現在ロード済み config の working dir を使う。それも無ければ 3.3 のとおり
`current_dir()`（起動 cwd）へは落とさず、**`no_target_dir:`** エラーを返す（`commands.rs::init_dir`）。
そのほかのエラー接頭辞（`no_home:` / `invalid_config:` / `legacy_config:`）を含む一覧は
CONTRACT.md「Phase 5.0.2 追加契約」のエラー接頭辞規約を正とする。

配線先は `commands.rs` の `#[tauri::command]` 群 + `lib.rs:88-118` の `generate_handler!`
（`init_probe_llm` の登録で init 系は 4 本になる）。

---

## 6. UI 仕様

| 状況 | 入口 | 生成モード |
|---|---|---|
| 設定なし（シェル 1 枚のフォールバック状態、1 章） | **主入口**。ツールバー / 空状態に「設定を作る」ボタン | 通常生成 |
| 設定を読み込み済み | ⚙ 設定メニューから（副） | sidecar 生成（3.4） |

プレビューモーダルで見せるもの:

1. **検出結果の一覧** — 見つかったものと**見つからなかったもの**の両方を出す
   （「`git` が見つかりません」＝「worktree 案内が出なかった理由」）
2. **生成先の絶対パス**と `Project` / `Global` の選択。Global 選択時は
   「**このフォルダの設定は信頼確認なしに autostart が走ります**」を併記（3.3）
3. **生成物のプレビュー（編集可能）** + 自己検査バッジ（✅ / ❌ + エラー文）と、
   **autostart がすべて false であること**（3.7）の明示、を常時表示
4. **sidecar のとき**: 書き込み先が `ptygrid.init.yml` であること、既存ファイルは変更されないこと、
   行単位の 2 ペイン差分
5. **ローカル LLM プローブ（§3.9）**: `[ローカル LLM を探す]` ボタン + 追加ポートの入力欄
   （1 つ。最大 4 本）。押したときだけ `init_probe_llm` が走り、最大 3 秒待たされうることを
   ボタン近傍に明記する（`scan` と違い即答しないため）。結果は
   **確証あり / 未確認 / 打ち切り（`timedOut`）**を区別して見せ、`anthropic !== true` のものには
   「`/v1/messages` が応答するかは未確認」と併記する（§3.9.3 の態度を UI でも崩さない）。
   結果は `init_preview` の `llm` 引数として渡し直し、**生成テキストは frontend で組み立てない**
   （§3.9.4）。プローブ未実行のままでも書き込みまで到達できる（プローブは必須手順ではない）

ボタンは `[書き込む]` / `[クリップボードにコピー]` / `[キャンセル]`。コピーは「ディスクに書かずに
済ませる逃げ道」として必ず用意し、自己検査が ❌ のときは `[書き込む]` を無効化する。

**trust 確認との順序**: `init_write` →（既存の `loadConfig`）→ `maybeAutostart`
（`App.svelte:908-918`）→ 必要なら trust プロンプト（3.7・3.8 の帰結どおり、init 自身は trust の
状態を一切変更しない）。Global の警告は 2 に留め、「Global に置けばプロンプトを回避できる」という
抜け道を UI が積極的に勧めない。

---

## 7. リリース計画とテスト

### 7.1 Phase 5.0.2 — `ptygrid init`

**入るもの**: 2.1 の実装一式（`init.rs` に検出・テンプレート・自己検査・atomic write）、
CONTRACT.md「Phase 5.0.2 追加契約」、userguide / `ptygrid-yml-guide.md` の init 節。
**入らないもの**: 2.2 の非スコープに同じ。`example/` も変更しない。

`completion gate:` unit + integration が通り、`svelte-check` / `npm run build` が 0 errors、
CI（macOS / Ubuntu）green、CONTRACT.md への先行追記完了、userguide 更新、実機手動検証の項目が
plan.md §2 に U 番号として登録されていること。

> バージョン割当: plan.md §4「次タグの前提」の (a) / (b) が未確定のため、**本書はタグ番号を
> 確定させない**。patch 番号 5.0.2 とタグ `vX.Y.Z` の対応はリリース時に plan.md §4 で決める。

### 7.2 テスト

**unit（`cargo test`）** — 検出/テンプレートは純関数化し `is_file` を注入可能にする
（`resolve_config_path_pure`（`config.rs:1603`）と同じ手法）。

- PATH 探索（複数ディレクトリ / 実行ビット無し / 同名複数ヒットは先頭）とプロジェクト種別 4 種を assert
- **テンプレート生成の決定性**: 同じ `InitScanReport` から**同じバイト列**が出ること
  （3.1 の `env` 順序不定問題を踏まないことの回帰）
- **自己検査**: 全テンプレート分岐（agents 0 / 1 / 3 体 × ルータ有無 × project 種別）の出力が
  `parse_config` を通ること
- **コメントアウト事故の回帰**: 明示的な `processes: null` / `agents: ~` は `parse_config` で
  **Err** になり、値ノードを持たないキー行のみ（例: `processes:` の後に何も続かない）は
  **Ok**（`#[serde(default)]` が空 `Vec` を供給する）になること。両方を固定する（3.5 の実測表）
- 生成物内の `agents[].name` の一意性 / sidecar 名の決定 / 既存検出（`mterm.yml` → `legacy: true`）
- **プローブの非退行**: `init_preview` の `llm` が `None` / 空のとき、出力が現行と**バイト単位で
  同一**であること（§4.4。既存の生成テストが落ちないことでも担保されるが、明示のテストを 1 本置く）
- **プローブ結果からの生成**: `anthropic == Some(true)` は有効行・それ以外はコメント行になること、
  名前が `local-<port>` になること、`autostart` が常に `false` であること、エンドポイントが
  **ポート昇順**・`models` が**受け取った順**で決定的に出ること、3456 用コメントブロックの抑止が
  **同じポートが結果に含まれるときだけ**起きること
- **`init_probe_llm` の引数検証**: `0` を含む / 追加ポートが 4 本を超える場合に `bad_port:` を返し、
  応答が無いだけのケースは**エラーにならず** `endpoints` が空になること（HTTP は注入可能にして
  ネットワーク非依存にする。D1 の `is_file` 注入と同じ手法）

**integration（`cargo test`）** — 一時ディレクトリは
`std::env::temp_dir().join(format!("ptygrid-init-{}", std::process::id()))` + 末尾 `remove_dir_all`
方式（`tempfile` は dev-deps に無いため。`config.rs:2231-2235` 他と同じ）。

- `init_preview` → `init_write` → `resolve_config_source_pure` が生成物を `Project` として拾い、
  `ConfigManager::load` が成功すること
- sidecar 経路で**既存ファイルが 1 バイトも変わらない**こと
- 自己検査を通らない `content` を `init_write` に渡すと**ファイルが作られない**こと
- Global 生成で `~/.ptygrid/` が `create_dir_all` されること / rename 前の temp が残らないこと

**frontend** — `svelte-check` / `npm run build` の 0 errors のみを要求する（描画確認は含めない）。

**実機手動検証**（macOS 必須 / Linux ベストエフォート）— **本書は手順だけを書き、実施状況は
書かない**。実施状況は plan.md §2 に U 番号として登録し、§1 の実機検証列から参照する。

1. 設定の無いフォルダで起動 → シェル 1 枚 → 「設定を作る」→ 検出結果が実際の PATH と一致すること
2. 書き込み → 読み込み → agents チップが出ること / **trust プロンプトが出ないこと**
3. 生成物の `autostart` を手で `true` にして再読み込み → **trust プロンプトが出ること**
4. 既存 `ptygrid.yml` があるフォルダで init → `ptygrid.init.yml` が生成され、既存ファイルの
   mtime と内容が変わらないこと / `config-changed` トーストが出ないこと
5. Ollama v0.14.0 以降を起動した状態で `[ローカル LLM を探す]` → `Ollama <version>` として
   確証つきで出ること、生成された `local-11434` を起動して**実際に応答が返ること**
   （`ANTHROPIC_BASE_URL` 直結で translation 層なしに繋がることの実機確認 — §3.9.0）。
   何も起動していない状態では 3 秒以内に「見つかりませんでした」で終わること

Windows は `PATHEXT` 展開と npm シム（`.cmd`）の扱いが未確認のため、plan.md の U8 の範囲で扱う（9 章）。

---

## 8. 次の patch — 5.0.3 Queen MCP 登録の代行（概要のみ）

現状、Queen の MCP 登録はユーザーが手でコマンドを実行する（Queen バッジがコピー、`App.svelte:256-360`。
codex / grok には TOML スニペット）。5.0.3 は**この実行を ptygrid が代行する** patch とし、
`git_service.rs:59-96` 型（shell を介さない `Command` + `args()`）を流用する（冪等化は frontend が
既に解決済み）。論点は「ホーム設定書き換えの可否」「CLI 不在 / npm シム失敗時のフォールバック」
「token 再生成での URL 埋め込み型登録の失効」の 3 つで、**失敗時は現行のクリップボード方式に
落とす二段構え**が基本線。詳細は
[queen-universal-register.md](../design/queen-universal-register.md) に譲る。

---

## 9. 未解決事項

3 章で決定として潰していない論点を残す。各項目は「論点 + (a)/(b) + 倒し方の帰結」の 2 行に絞る。

- **生成直後の反映**: (a) `init_write` 成功後に frontend が自動で `loadConfig()` を呼ぶ、
  (b) 既存の `config-changed` トーストを押させる。当面は (a)（3.8）— watcher イベントの前後関係に
  余地が残るが、**推測であり未実測**。
- **テンプレートの段数**: (a) 「最小 + 複数 CLI」の 2 系統に留める（現方針、4 章）、(b) `example/`
  の 7 サンプルに寄せ `web-dev` / `worktree` / `teammates` 相当まで増やす。(b) は `workflows:` 生成
  の是非も絡み、化石化した workflow 4 本と同じ棚卸し負債が増える。
- **`mterm.yml`（旧名）が存在するプロジェクト**: (a) 生成を中断しリネームを促す（現方針）、
  (b) 警告つきで生成する。どちらでも旧名は探索順で黙って無効化されうるため、`existing.legacy`
  を返して黙って進めないことだけは共通。
- **「同値なら書かない」の実現**: `teams_hooks` の JSON マージ流 no-op 判定は YAML では作れない。
  (a) sidecar の既存内容とバイト列比較してスキップ、(b) 毎回書く。当面は (a)（sidecar のみ適用）。
- **CLI サブコマンド**: 5.0.2 では作らない（3.6）。足すなら (a) `ptygrid init --print`（出力のみ）、
  (b) 対話 CLI。Windows のコンソール不在を踏むのは (b) だけなので、足すなら (a) から。
- **init 済みの記録**: 「このフォルダは init 済み」を app-data に覚えるか（`project_state.rs:82-92`
  の FNV ハッシュ方式が流用可）。当面は覚えない — 先頭コメント（4 章）がマーカーとして働くため、
  必要性が実証されるまで作らない（YAGNI）。
- **未確認（Windows の PATH 探索）**: `PATHEXT` 展開の要否、npm 由来 CLI が `.cmd` シムであるため
  `Command::new("claude")` が直接 exec できないことは**いずれも推測であり実機未確認**。D1 の
  Windows 実装は U8（plan.md §2）で確認後に確定する。
- **未確認（シンボリックリンク経由の work フォルダ）**: trust ストアのパスは `canonicalize` 済み
  （`trust.rs:77-85`）なので、シンボリックリンク経由の work だと trust 判定と生成先が食い違う
  余地がある。**推測であり未確認**。実機再現を確認してから対処を決める。
