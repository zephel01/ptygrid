# 仕様書: `ptygrid init` — 設定ファイルの自動生成（Phase 5.0.2）

作成日: 2026-07-29 / ステータス: **ドラフト（未実装）** / 対象: `ptygrid.yml` の初回生成

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
- 検出結果からの **`ptygrid.yml` 生成**（テンプレート埋め込み方式）と**自己検査**
- **プレビュー UI**（検出結果・生成先・生成物・自己検査結果を見せてから書き込む）
- 既存設定がある場合の**別名（sidecar）生成 + 差分表示**、アトミック書き込み（temp + rename）
- Tauri command 3 本の追加 / CONTRACT.md 先行追記 / userguide・`ptygrid-yml-guide.md` の同時更新

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
| D4 | ローカル LLM ルータの生死 | `127.0.0.1:3456` へ `TcpStream::connect_timeout`（既定ポートは `router.settings.json` / `example/team-preset` に登場） | 不要 | **小** | `local` エージェントの雛形を出さない |
| D5 | 既存の設定ファイル | `resolve_config_path_pure`（`config.rs:1599-1641`）と同じ探索順。**内容は読むが parse しない**（3.4） | 不要 | **小** | 衝突なしとして通常生成 |
| D6 | projects root | `app_settings` の `projectsRoot` / `list_dirs_at` | 不要 | **小** | UI の生成先候補列挙のみ。無ければ現在の work だけ |
| D7 | 既存 MCP 登録 | **5.0.2 では検出しない**（TOML パーサ未依存等、5.0.3 へ） | — | — | — |

**決定: 検出はすべて best-effort。個々の検出が失敗しても init 全体は失敗しない**——代わりに何を
見て何を出したかを生成物の先頭コメントに必ず記録する（4 章の `# 検出:` 行）。D4 の TCP 接続を
含め検出は呼び出しスレッド内で完結させ、同期 I/O を PTY / async ランタイムに載せない
（`notifications.rs:311-319` の作法）。

### 3.3 生成先と trust の関係

**決定: 既定の生成先は `<work>/ptygrid.yml`（`ConfigOrigin::Project`）。** `~/.ptygrid/ptygrid.yml`
（`Global`）は UI で明示的に選んだときだけのオプション。`<launch>`（起動 cwd）には生成しない——
GUI 起動では起動 cwd がユーザーの意図と結びつかない。`~/.ptygrid/` を作るコードは現状どこにも
無いので、Global 選択時は **init が `create_dir_all` する**。

**決定: Project 生成物は trust 確認の対象になる。init はこれを迂回せず、`trust::add_trusted`
（`trust.rs:194`）も呼ばない。** `trust::is_trusted_pure`（`trust.rs:94-99`）は `Global` / `Default` を
無条件 trusted、`Project` / `Launch` は trust ストアに載るまで untrusted とするため、
**`~/.ptygrid` に生成すればプロンプト無しで autostart が走る**——生成先の選択がそのまま security
posture を変える。だからこそ Project を既定にする: 生成物にも autostart は入りうるが（既定 false、
直後に true へ書き換えられる）、**「ptygrid 自身が生成したファイルだから」を理由に S2 gate を
免除しない**——生成物か否かと信頼できるフォルダか否かは独立の問題であり、後者は既存どおり
ユーザーの明示同意で決める（`trust.rs:1-22`）。

### 3.4 既存ファイルがあるとき — 上書きせず sidecar に書く

**決定: 既存の設定ファイルがあるときは、それを一切変更しない。生成物は隣の別名ファイルに書く。**

| 項目 | 決定 |
|---|---|
| sidecar のファイル名 | **`ptygrid.init.yml`**（生成先ディレクトリ直下） |
| 既に sidecar もある場合 | **上書きする**（init の作業用出力であり、ユーザーの資産ではない） |
| 既存ファイルの扱い | **テキストとして読み、行差分の表示にだけ使う。parse も書き戻しもしない** |
| 採用のしかた | ユーザーが手でマージするかリネームする。init は代行しない |
| 差分の見せ方 | **行単位の 2 ペイン表示**（左 = 既存 / 右 = 生成物）。構造マージ表示は 3.1 の問題に戻るため作らない |

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

コメントアウトは**キー行ごと**に行う（`processes:` だけ残すと値が null になり parse に失敗しうる。
この事故は自己検査で必ず捕まる、7 章に回帰テスト）。

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

---

## 4. 生成される `ptygrid.yml`（実物）

書式は `example/` 配下と `ptygrid.example.yml` に実在する書き方に揃える。

### 4.1 最小構成（`claude` だけが見つかった場合）

```yaml
# ptygrid.yml — ptygrid init が生成しました (2026-07-29)
# 検出: claude (PATH) / Cargo.toml / git リポジトリ
# 中身はすべて手で編集できます。全ブロックの注釈つき例は ptygrid.example.yml、
# 用途別の見本は example/ を参照してください。

project: my-app          # 作業フォルダ名から。ヘッダーに出る表示名

agents:
  - name: claude
    cmd: "claude"
    cwd: "."
    autostart: false     # 読み込みと同時に起動するなら true（初回は手動 ▶ 起動）
```

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
# 次のブロックのコメントを外してください（agents と同じフィールドを持ちます）。
# processes:
#   - name: web
#     cmd: "npm run dev"
#     cwd: "."
#     autostart: false
#     autorestart: on-failure   # 異常終了時のみ再起動
```

### 4.3 生成しないもの

- **`team_presets:` / `workflows:`** — 2.2 のとおり非スコープ。コメントで
  「チーム一括起動は `example/team-preset`、DAG は `example/adaptive-orchestration` を参照」と案内するに留める
- **`teammates:` / `agents[].teams` / `worktree:`** — オプトイン機能。検出から推測しない
  （D3 が git repo を見つけても worktree は生成せず、案内コメントだけ出す）
- **`notifications:` / `mcp:`** — 秘密情報や互換フラグを機械が推測しない

---

## 5. CONTRACT.md 追記項目

既存設計原則との整合は 3 章の各決定に書いた（project 境界 = 3.3、推測拒否 = 3.2 / 3.7、信頼確認 =
3.3、既存の検証経路の外に出ないこと = 3.5）ので繰り返さない。本章は CONTRACT 追記項目と wire の
形だけを定める。

### 5.1 CONTRACT.md 追記項目（実装前に先行追記）

1. `InitTarget` / `InitScanReport` / `InitPreview` / `InitWriteResult` の確定形（フィールド名は camelCase）
2. Tauri command 3 本のシグネチャ（5.2）
3. **生成物の不変条件**: 「`init_write` が書き込む内容は必ず `parse_config` を通っている」
4. **sidecar 規約**: `ptygrid.init.yml` / 設定探索の対象外 / watcher のファイル名フィルタに掛からない
5. **新規イベントを追加しない**ことの明示（生成後の通知は既存 `config-changed` で足りる）
6. **非回帰宣言**: `load_config` / `ConfigInfo`（全 field）/ `config-changed` /
   `trust_working_folder` / `is_working_folder_trusted` は**不変**。本節はすべて additive

### 5.2 wire（Tauri command）の形

```ts
type InitTarget = "project" | "global";  // project = <work>/ptygrid.yml, global = ~/.ptygrid/ptygrid.yml

interface InitScanReport {
  dir: string;                     // 走査した作業フォルダ（絶対パス）
  agents: string[];                // PATH で見つかった名前（KNOWN_AGENTS の宣言順）
  projectKinds: string[];          // "cargo" | "npm" | "python" | "go"（複数可・空可）
  gitRepo: boolean;
  routerPort: number | null;       // 応答したローカルルータのポート。無ければ null
  existing: ExistingConfig | null; // 探索順で最初に当たった既存設定
}
interface ExistingConfig { path: string; origin: "project"|"launch"|"global"; legacy: boolean }
                                   // legacy: true = mterm.yml（旧名）
interface InitPreview {
  content: string;                 // 生成された YAML 全文
  path: string;                    // 書き込み予定の絶対パス（sidecar のときは sidecar 側）
  target: InitTarget;
  sidecar: boolean;                // 既存があるため別名に書く場合 true
  valid: boolean;                  // 自己検査（3.5）の結果
  error?: string;                  // valid=false のときの parse / validate エラー
  existingContent?: string;        // sidecar のとき、差分表示用に読んだ既存の生テキスト
  scan: InitScanReport;
}
interface InitWriteResult {
  path: string; bytes: number; sidecar: boolean;
  trustPromptExpected: boolean;    // target=project かつ autostart 付き定義があれば true
}
```

| command | args | returns | 説明 |
|---|---|---|---|
| `init_scan` | `{ dir?: string }` | `InitScanReport` | 検出のみ。ディスクに**書かない** |
| `init_preview` | `{ dir?: string, target?: InitTarget }` | `InitPreview` | 生成 + 自己検査。ディスクに**書かない** |
| `init_write` | `{ dir?: string, target?: InitTarget, content: string }` | `InitWriteResult` | `content` を再検査してから temp + rename で書く |

配線先は `commands.rs` の `#[tauri::command]` 群 + `lib.rs:88-118` の `generate_handler!`。

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
- **コメントアウト事故の回帰**: `processes:` のキー行だけを残した YAML が `parse_config` で
  Err になること（3.5 の罠を固定する）
- 生成物内の `agents[].name` の一意性 / sidecar 名の決定 / 既存検出（`mterm.yml` → `legacy: true`）

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
