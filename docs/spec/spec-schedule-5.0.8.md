# 仕様書: `schedule:` — 時刻で workflow を起こす（Phase 5.0.8）

作成日: 2026-08-05 / ステータス: **ドラフト（未実装）** / 対象: `workflows:` の宣言と
`orchestrator.rs` の driver ループ

> **採番（3 行）**: patch 番号 `5.0.8` は**提案であり、確定はユーザー判断による**。
> [plan.md](../design/plan.md) §1 の脚注※2 が「5.0.6 を orchestrator の計測に充てる」案を
> 未確定のまま置いており、[spec-oneach-reply-5.0.7.md](spec-oneach-reply-5.0.7.md) も同じ扱い
> なので、その次に来る本書も同じ扱いとする——**5.0.7 が確定するまで 5.0.8 も確定しない**。

関連: [spec-phase5-0.md](spec-phase5-0.md) §8（**本書はここの却下を覆す。理由は 1.2**）、
[plan.md](../design/plan.md) §3 P3（escalation。本書の前提条件）、
[autonomous-operation-guide.md](../guide/autonomous-operation-guide.md)、
[CONTRACT.md](../../CONTRACT.md)「Phase 5.0 追加契約」、
[ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md)

実装対象: `src-tauri/src/config.rs`（`WorkflowDef` / `validate_workflows`）、
`src-tauri/src/orchestrator.rs`（`driver_loop` に 1 段、純関数の `next_fire_at`）、
`src-tauri/src/commands.rs`（読み取り用の Tauri command 1 本）。
`queen.rs` / `queen_store.rs` の**スキーマは無変更**、frontend は表示の追加のみ。

---

## 1. 目的と背景

### 1.1 いま設定で書けないこと

workflow を起こす経路は 2 つしかない — GUI の ▶ と Queen MCP の `spawn_workflow` で、
**どちらも人間かエージェントが「いま」呼ぶ**ものである。「毎朝 9 時に流す」は設定として
存在しない。

回避策は OS 側の launchd / cron から叩くことだが、それは 1.2 の理由で採らない。

### 1.2 なぜ spec-phase5-0 §8 の却下を覆すのか

[spec-phase5-0.md](spec-phase5-0.md) §8 は明確に却下している:

> **専用ジョブスケジューラを内蔵する案**（cron 的 trigger を workflow に統合）: 却下。
> ptygrid の責務は「PTY を持って monitor する」ことに閉じる。cron は OS 側の
> launchd / systemd / cron を使ってもらう。

この判断は**責務の線引き**として書かれており、そこは今でも筋が通っている。覆す理由は
2 つあり、どちらも当時の前提が変わったことによる。

**(a) 想定利用者が変わった。** 上の一文は「launchd を操作できる利用者」を暗黙に想定して
いる。実際の想定利用者には、cron の書式を知らない初級エンジニアや、プログラムは書かないが
バイブコーディングで開発を始めた層が含まれる。この層にとって OS cron は選択肢ではなく、
しかも**失敗の仕方が悪い**: crontab / plist の書式、対話シェルと違う PATH（`claude` が
見つからない）、出力がどこにも出ないので失敗に気づかない。「エラーが出にくい構造にする」
という方針の真逆である。

**(b) 調査の資産が ptygrid 側にしか無い。** OS cron で `claude -p` を回して失敗したとき
残るのはログファイル 1 本である。ptygrid の中で起こした run は、ペインのスクロールバック、
step ごとの所要と待ち（`ended_at_ms` / `waited_for_pane_ms`、5.0.6）、SQLite に永続化された
run 履歴、Queen の inbox スレッドを残す。**手で ▶ した run と夜中に自動で走った run が
同じ資産を残す**ことが、ハーネスとしての本体価値である。

**(c) 実装が新しいサブシステムにならない。** 却下の理由は「責務が広がる」ことだったが、
`driver_loop` は既に 200ms で回っている（`orchestrator.rs`、`DRIVER_TICK`）。スケジューラは
新しいスレッドでもストアでもなく、**そのループに「due か？」という述語を 1 段足すだけ**で
足りる（3.7）。当時想定された「ジョブスケジューラを内蔵する」ほどの重さは無い。

### 1.3 この機能が守るべき性質

本書の設計判断はすべて次の 1 文から出ている。

> **無人で走る機能は、失敗しないことより「静かに失敗しないこと」のほうが重要である。**

初級者にとって最悪なのは Failed ではなく、**何も起きていないのに何も表示されない**状態で
ある。3 章の決定はほぼ全部これを避けるためにある。

---

## 2. スコープ / 非スコープ

### 2.1 スコープ

- `WorkflowDef` への **`schedule:`** の追加（限定語彙。cron 式ではない、4.1）
- `validate_workflows` への検証規則 S1〜S7（4.2）
- `driver_loop` への 1 段（`tick_schedules`）と、純関数 `next_fire_at`（3.7）
- 発火を**見送る**条件の定義（重なり 3.3 / 枠不足 3.4）と、その理由の記録
- 連続失敗による自動停止（3.5）
- 次回・最終実行を読む Tauri command 1 本と、それを出す frontend（6 章）
- CONTRACT.md 先行追記（5 章）と `example/` サンプル 1 本

### 2.2 非スコープ

- **cron 式（`0 9 * * *`）** — 1.2 (a) のとおり、書式そのものが障壁になる層が対象である。
  書ける人向けの別キーとして足す余地は 8 章に残す
- **アプリが起動していない間の発火と取りこぼしの追跡** — 3.1 のとおり**仕様として追わない**
- **祝日 / 稼働日カレンダー** — 「平日」は月〜金の固定。祝日判定は地域データを抱え込む
- **重なったときに積む（`queue`）** — 3.3 のとおり既定は skip 固定。knob は 8 章
- **1 つの workflow に複数のスケジュール** — 8 章
- **escalation（通知）そのものの実装** — [plan.md](../design/plan.md) §3 P3 の担当。本書は
  **依存を宣言するだけ**で、通知経路は作らない（3.6）

---

## 3. 設計

### 3.0 用語

| 語 | 意味 |
|---|---|
| **スケジュール** | `workflows.<name>.schedule` の宣言 |
| **発火** | スケジュールが `spawn_workflow` を 1 回呼ぶこと |
| **次回時刻**（`next_fire_at`） | 現在時刻より後で、宣言に合致する最も早いローカル時刻 |
| **見送り**（skip） | 次回時刻に達したが、条件が揃わないので発火しないこと。**必ず理由が残る** |

### 3.1 発火の条件（論点 0）

**決定: アプリが起動している間だけ発火する。起動していなかった時間の分は追わない。**

ptygrid はデスクトップアプリであってデーモンではない。ノートを閉じていた夜の分は起きない
——これは実装で埋められる性質ではないので、**仕様として断言する**。取りこぼしを追う機構
（起動時のキャッチアップ、溜まった分の一括発火）は作らない。

これは妥協ではなく、この機能でいちばん事故りやすい形を消す決定である。キャッチアップを
入れると「朝アプリを開いた瞬間に 5 本が同時に走り出し、9 面のグリッドを埋め、課金が積み
上がる」が起きる。1.3 の性質に照らして、**取りこぼすほうが安全**である。

代わりに、起きなかったことが**見える**必要がある（6 章）: workflow 一覧に「次回 09:00
（あと 3 時間）／最終実行 昨日 09:00 成功」を常時出す。最終実行が 3 日前で止まっていれば、
アプリを開いていなかったことがそこで読める。

### 3.2 書き方（論点 1）

**決定: 限定語彙の構造体で書く。cron 式は受け付けない（2.2）。**

```yaml
workflows:
  nightly-review:
    schedule:
      every: day          # day | weekday | hour
      at: "09:00"         # every: day/weekday なら HH:MM、hour なら MM
    pattern: pipeline
    steps: [...]
```

| `every` | 意味 | `at` の形式 |
|---|---|---|
| `day` | 毎日 1 回 | `"HH:MM"` |
| `weekday` | 月〜金の 1 回 | `"HH:MM"` |
| `hour` | 毎時 1 回 | `"MM"` |

**なぜ文字列 1 本（`schedule: "毎日 09:00"`）にしないか**: 自由文字列はタイポを実行時まで
運ぶ。構造体なら `every: dayly` は serde の enum が load 時に落とし、`at: "9時"` は 4.2 の
S2 が落とす。**この 3 つ以外を書けないことが、この設計のいちばんの機能**である。

**なぜ cron 式を一次表現にしないか**: `0 9 * * *` は読めない層が対象である。加えて、cron
式を受けると分・時・日・月・曜日の直積を全部サポートする約束になり、`next_fire_at` の
テスト面が跳ね上がる。語彙を 3 つに絞れば、次回時刻の計算は網羅的にテストできる（7.2）。

**時刻の解釈はローカルタイム**（`chrono::Local`。`chrono` は既に依存に入っている）。UTC に
しない理由は、「毎朝 9 時」と書いた人が期待するのは手元の 9 時だからである。夏時間のある
地域での 2 つの縁は明示的に決める:

- **存在しない時刻**（春の飛び。02:30 が無い日）→ その日は**飛んだ直後の瞬間**に 1 回発火する
- **二重に存在する時刻**（秋の重なり。02:30 が 2 回来る）→ **早いほうの 1 回だけ**発火する

どちらも「1 日に 1 回」を壊さないことを優先した。日本のみを考えるなら不要な規則だが、
書いていないと実装者が各自で違う判断をする種類のものなので決めておく。

### 3.3 重なり（論点 2）

**決定: 同じ workflow の run がまだ終端に達していないときは、発火を見送る（skip）。
理由を記録する。積む選択肢は持たない。**

積む（`queue`）を採らない理由は 9 面上限との掛け算である。1 時間かかる run を毎時起動する
設定は、積む実装なら 3 時間後に 3 本が同時に走ろうとしてグリッドを食い尽くす。初級者が
書きうる設定の中でいちばん事故るのがこれで、しかも**書いた本人には掛け算が見えていない**。
見送りなら、最悪でも「1 本だけ走り続けて、次が飛ばされ続ける」に収まり、6 章の表示で
「見送り（前回の run がまだ走っています）」と読める。

判定は `WorkflowRegistry::active_run_ids()` を name で絞るだけで済む（新しい簿記は不要）。

### 3.4 pane 上限との相互作用（論点 3）

**決定: 発火の直前に root が必要とする枠を数え、足りなければ**発火自体を見送る**。
「発火して 5 分後に Failed」にはしない。**

現状の挙動をそのまま使うと、グリッドが埋まっている時刻に発火した run は root が
`Pending` のまま `WORKFLOW_DEFER_MAX_MS`（5 分）を待って `Failed` になる。ログを読める
人には正しい挙動だが、対象読者には「勝手に始まって勝手に赤くなった」ようにしか見えない。
`pane_budget()` は既にあるので、発火前に 1 回読むだけで避けられる。

**併せて、`schedule` を持つ workflow は `autoClose` の既定を `never` から `success` に
変える。** 無人で走る run が成功したペインを残し続けると、翌日の発火が 3.4 の見送りに
当たる——「昨日成功したせいで今日走らない」は、原因の説明がいちばん難しい形である。
明示的に `autoClose: never` と書いた場合はそれを尊重する（宣言が既定に勝つ）。

### 3.5 暴走の歯止め（論点 4）

**決定: 連続して `maxConsecutiveFailures` 回（既定 3）失敗したスケジュールは、自動的に
停止する。停止は通知を伴い、`ptygrid.yml` は書き換えない。**

毎時走る設定に指示文の誤りがあると、気づかないまま課金が積み上がる。対象読者はまさに
「気づかない」側にいる。歯止めは**あとから足すのでは遅い**種類のもので、既定で入れる。

- 数えるのは**連続**失敗。1 回でも成功したら 0 に戻る
- 停止したスケジュールは 6 章の一覧に「自動停止（3 回連続で失敗）」と出る。再開は
  ユーザーの明示操作（または設定の再読み込み）
- **`ptygrid.yml` は書き換えない。** 「ptygrid はユーザーマシンの状態を勝手に変えない」
  （spec-phase5-0 §8 の Ollama pull 却下と同根）。停止状態は in-memory + Queen の DB に
  持ち、設定ファイルには触らない

`schedule.enabled: false` を書けば宣言側でも止められる（4.1）。

### 3.6 通知（論点 5）

**決定: 本書は escalation を実装しない。ただし `schedule` は escalation を前提とする
機能であり、[plan.md](../design/plan.md) §3 P3 を先に消化することを本書の推奨とする。**

4.4.2 の配送機構（OS 通知 / Slack / Mattermost / Discord / Telegram）は既にあるのに、
workflow の失敗がそこへ流れていない（`ptygrid-yml-guide.md` §1 の実装マトリクスで
`escalation` は ❌ のまま）。無人スケジューラを通知なしで入れると、**失敗を静かに溜める
だけの装置**になり、1.3 の性質に正面から反する。

P3 が入った時点で、`schedule` 由来の run の失敗と 3.5 の自動停止が通知の対象になる。
本書はそのイベントを流す側の口（どの run が schedule 由来か）を用意するに留める。

### 3.7 tick の中での位置と実装の置き場

**決定: `driver_loop` の中、`advance_all` の**前**に 1 段挟む。新しいスレッドは作らない。**

```
driver_loop（200ms）
  → tick_schedules   ← 【新】due なスケジュールを発火 / 見送る
  → advance_all      ← 既存。発火した run はこの直後から普通に進む
```

- **前に置く理由**: 発火した run が同じ tick で `advance_all` に拾われ、手で ▶ した run と
  区別なく進む。後ろに置くと最初の 1 tick（200ms）だけ何もしない run が存在することになり、
  「発火したのに 1 回だけ進まない」という説明の要る状態が生まれる。
- **200ms ごとに時刻計算をしない。** 各スケジュールの `next_fire_at` は算出済みの絶対時刻
  として保持し、tick では比較 1 回で済ませる。発火（または見送り）のたびに次の時刻を
  再計算する。
- 純粋な部分（宣言と現在時刻から次回時刻を出す）は `next_fire_at(schedule, now) -> DateTime`
  という自由関数に切り出す。`mod retry` / `mod condition` / `mod verdict` と同じ posture で、
  PTY もストアも時計も持たない——**この機能でいちばんバグが出るのは時刻計算**なので、
  そこだけは単体で網羅テストできる形にする（7.2）。

### 3.8 ペインの幾何（cols / rows）

**決定: `QUEEN_SPAWN_COLS` / `QUEEN_SPAWN_ROWS` を再利用する。**

`spawn_workflow` は `cols` / `rows` を要求するが、スケジュール発火には UI の呼び出し元が
無い。Queen MCP 経由の `spawn_workflow` が既に同じ問題を持っており、専用の定数で解いて
いる。2 つ目の定数を作る理由が無いので、そのまま借りる。

### 3.9 設定の再読み込みとの相互作用

`ptygrid.yml` は watcher（`notify` クレート）で監視されており、保存すると再読み込みされる。
スケジュール表はそのたびに**作り直す**。

**決定: 作り直した直後の `next_fire_at` は、必ず「現在時刻より後」でなければならない。**

これが無いと、`at: "09:00"` の設定を 09:00 台に保存した瞬間に発火する。設定を書いている
最中に勝手に走り出すのは、対象読者にとって最も怖い挙動である。

### 3.10 却下した代替案

- **cron 式を一次表現にする**: 却下。3.2 のとおり、読めない層が対象であり、テスト面も広がる。
- **OS の launchd / cron から Queen MCP の `spawn_workflow` を叩くラッパを配る**: 却下。
  run 自体は ptygrid の中に生まれるので調査資産は残り、実装変更もゼロで済むという利点は
  実在する。だが plist / crontab を書くこと自体が対象読者の障壁であり、1.2 (a) の失敗の
  仕方（PATH、無言の失敗）を丸ごと引き受けることになる。**launchd を既に使える人向けの
  逃げ道としては有効**なので、8 章に残す。
- **アプリ起動時に取りこぼし分を流す**: 却下。3.1 のとおり、朝に 5 本が同時に走り出す。
- **重なったら前の run をキャンセルして始め直す**: 却下。1 時間かかる run を毎時起動する
  設定で、**永久に完走しない run** が生まれる。しかも赤くならないので気づけない。
- **`schedule` を workflow の外（トップレベルの `schedules:` ブロック）に置く**: 却下。
  「いつ走るか」は workflow の性質であり、宣言から離すと 2 箇所を突き合わせないと読めない。
  config-as-code の唯一のファイルを保つ原則（spec-phase5-0 §8）とも整合する。
- **秒単位・分単位の間隔（`every: 5m`）**: 却下。実エージェントの run は分単位で終わらない
  （cold start だけで 3.4 秒、実タスクなら数分）ので、5 分間隔は 3.3 の見送りを量産する
  だけになる。最短を「毎時」に固定するのは、書けてしまう事故を減らす選択である。

---

## 4. `ptygrid.yml` スキーマ

### 4.1 追加フィールド

```rust
// config.rs — WorkflowDef に 1 フィールド追加
#[serde(default, skip_serializing_if = "Option::is_none")]
pub schedule: Option<Schedule>,          // YAML: schedule

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub every: Every,                     // day | weekday | hour
    pub at: String,                       // "HH:MM" または "MM"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,            // 既定 true
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_consecutive_failures: Option<u32>,  // 既定 3
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Every { Day, Weekday, Hour }
```

`WorkflowDef` は `rename_all = "camelCase"` なので YAML キーは `schedule` /
`maxConsecutiveFailures`。**既存の `ptygrid.yml` は 1 文字も変わらず、同じ意味で動く**。

### 4.2 検証ルール（`validate_workflows` への追加）

すべて **load 時 reject**。既存のエラー文言と同じく workflow 名を必ず含める。

| # | 規則 | なぜ |
|---|---|---|
| S1 | `every` は `day` / `weekday` / `hour` のみ（serde の enum が担保） | 3.2。タイポを実行時まで運ばない |
| S2 | `every: day` / `weekday` の `at` は `HH:MM`（00:00〜23:59）。`every: hour` の `at` は `MM`（00〜59） | 3.2。`"9時"` も `"25:00"` も load で落とす |
| S3 | `maxConsecutiveFailures` は 1..=10 | `retry.max` と同じレンジ・同じ理由 |
| S4 | `schedule` を持つ workflow の `steps` が空でないこと（既存規則が担保） | 発火して何もしない設定を作らない |
| S5 | `schedule` を持つ workflow に `onEach` を含む step があってもよい（拒否しない） | 無人で最大 64 unit が開く形にはなるが、`STREAM_MAX_UNITS` と 3.4 の見送りで有界。禁止するほどの根拠が無い |
| S6 | 同じ workflow に `schedule` は 1 つだけ（構造上そうなる） | 2.2。複数持たせるなら 8 章 |
| S7 | `schedule` を持つ workflow が `pattern: handoff` でも構わない | 制約する理由が無いことを明示する（V8 と同じ「規則を置かない」宣言） |

### 4.3 実例

```yaml
project: nightly

queen:
  enabled: true
  port: 39237

agents:
  - name: reviewer
    cmd: >-
      claude "…（起動直後に await を呼ぶ定型。example/review-as-you-go 参照）…"
    cwd: "."
    autostart: false

workflows:
  nightly-review:
    # 毎朝 9 時に 1 回。アプリが起動している間だけ動く（3.1）。
    schedule:
      every: day
      at: "09:00"
    pattern: pipeline
    # autoClose は書かなくてよい。schedule を持つ workflow の既定は
    # success になる（3.4）。
    steps:
      - id: review
        agent: reviewer
        joinOn: reply
        timeoutMs: 1800000
        kickoff: >-
          昨日の変更を見て、気になった点を 3 つまで挙げてください。
          終わったら queen の reply_inbox で返信してください。
```

---

## 5. wire 契約の差分と CONTRACT.md 追記項目

### 5.1 wire の差分

| wire | 差分 |
|---|---|
| `StepOutcome` / `WorkflowRun` | **無変更**。schedule 由来の run も手で ▶ した run と同じ形 |
| `workflow-state` イベント | **無変更** |
| Queen MCP tools | **無変更**（`spawn_workflow` の引数も意味も変えない） |
| Tauri commands | **1 本追加**（`list_schedules`。読み取り専用、6 章） |
| `ptygrid.yml` | `schedule`（新規キー）1 つだけ。既存設定はバイト単位で同じ意味 |

**結論: 追加のみ。** run の形が変わらないので、schedule 由来かどうかを実行時に区別する
必要は無い（区別が要るのは 3.6 の通知だけで、そこは run ではなくスケジュール側が持つ）。

### 5.2 CONTRACT.md 追記項目（実装前に先行追記）

1. `schedule:` の**意味の定義**（3.2 の語彙とローカルタイム解釈、DST の 2 つの縁）
2. **アプリが起動している間だけ発火し、取りこぼしは追わないこと**（3.1）——これが
   契約であり、実装の限界ではないこと
3. **見送りの 2 条件**（重なり 3.3 / 枠不足 3.4）と、見送りには必ず理由が残ること
4. `schedule` を持つ workflow の `autoClose` 既定が `success` に変わること（3.4）
5. 連続失敗による自動停止（3.5）と、**`ptygrid.yml` を書き換えないこと**
6. load 時検証 S1〜S7
7. **非回帰宣言**: `StepOutcome` / `WorkflowRun` / `workflow-state` / Queen MCP tools は
   すべて不変。`schedule` を書かない設定の挙動は 1 バイトも変わらない
8. **既知の限界**: (a) 起動していない間は発火しない、(b) cron 式は書けない、
   (c) 祝日カレンダーは無い、(d) 通知は P3 の実装を待つ

---

## 6. UI（WorkflowPanel）への影響

**決定: 表示を足す。`workflow-state` は変えない。**

3.1 で「取りこぼしを追わない」と決めた以上、**起きなかったことが見える**必要がある。
workflow 一覧の各行に 2 つ足す:

```
nightly-review   毎日 09:00   次回 09:00（あと 3 時間）   最終 昨日 09:00 成功
hourly-check     毎時 :05     見送り（前回の run がまだ走っています）
weekly-audit     平日 18:00   自動停止（3 回連続で失敗）
```

- データ源は新しい読み取り専用 Tauri command `list_schedules` 1 本。ポーリングは
  frontend 側（1 分間隔で足りる。次回時刻は分単位でしか動かない）
- **「最終実行」が数日前で止まっていること自体が、アプリを開いていなかったことの表示に
  なる**（3.1）。これが無いと、この機能は「たまに動かない」ように見える

---

## 7. 段階的な作り方とテスト

### 7.1 MVP の線引き

| 入るもの | 入らないもの（後続） |
|---|---|
| `every: day / weekday / hour` + `at`、S1〜S7 | cron 式（8 章） |
| `next_fire_at` と `tick_schedules`、見送り 2 条件 | 積む（`queue`）、複数スケジュール |
| 連続失敗の自動停止、`enabled: false` | 祝日カレンダー |
| `list_schedules` と一覧表示 | 通知（P3 の担当、3.6） |
| `example/` サンプル 1 本 + CONTRACT 追記 | 実行履歴の GC |

### 7.2 テスト

**この機能でいちばんバグが出るのは時刻計算**なので、そこに厚みを置く。`next_fire_at` は
純関数なので、実時計を使わずに網羅できる。

- **`next_fire_at`**: (a) `day` で当日の時刻をまだ過ぎていない / 過ぎている、(b) `weekday`
  で金曜の夜 → 次は月曜、土日に発火しない、(c) `hour` で `:05` が当該時をまたぐ、
  (d) 月末・年末をまたぐ、(e) 存在しない時刻 / 二重に存在する時刻（3.2 の 2 規則）
- **再読み込み**: 設定を保存した瞬間の時刻が `at` と一致していても発火しないこと（3.9）
- **見送り**: (a) 同名 run が非終端なら発火しない、(b) `pane_budget` が root の必要数に
  満たなければ発火しない、(c) いずれも理由が読めること
- **自動停止**: 3 回連続失敗で停止し、間に 1 回成功が挟まればカウンタが 0 に戻ること
- **`autoClose` の既定**: `schedule` を持ち `autoClose` 未宣言の workflow が `success`
  として扱われ、明示宣言があればそれが勝つこと（3.4）
- **非回帰**: `schedule` を書かない既存 workflow に対して `driver_loop` の挙動が現行と
  同一であること（明示のテストを 1 本置く）

**実機手動検証**（macOS 必須）— **本書は手順だけを書き、実施状況は plan.md §2 に U 番号で
登録する**。

1. `at` を 2 分後に設定して保存し、**保存直後に発火しないこと**と、2 分後に発火することを
   確認する（3.9 と 3.1 の両方を 1 回で見る）
2. 1 の run を走らせたまま次の発火時刻を迎えさせ、見送りと理由の表示を確認する（3.3）
3. 8 面を手で埋めた状態で発火時刻を迎えさせ、発火せず理由が出ることを確認する（3.4）
4. 失敗する設定で 3 回発火させ、自動停止することを確認する（3.5）

`completion gate:` unit + 統合が通り、`svelte-check` / `npm run build` が 0 errors、
CI（macOS / Ubuntu）green、CONTRACT.md への先行追記完了、`example/` サンプル 1 本追加、
実機手動検証 1〜4 が plan.md §2 に U 番号として登録されていること。

---

## 8. 未解決事項 / 未確認

### 8.1 未決（倒し方の帰結つき）

- **cron 式の別キー**: (a) 足さない（現方針）、(b) `schedule.cron:` として書ける人向けに
  足す。(b) はパーサ依存（`cron` / `saffron` クレート）と DST の扱いを持ち込むが、
  「毎週火曜と木曜」のような表現は語彙 3 つでは書けない。**要望が出てから**倒す。
- **launchd ラッパの同梱**: 3.10 で却下したが、既に launchd を使える利用者には有効な逃げ道
  である。`example/` に置くか、置くと「こっちが正道」に見えて対象読者が迷うか。現方針は
  置かない。
- **`maxConsecutiveFailures = 3`**: 数字の根拠は弱い。`retry.max` の 1..=10 に合わせただけ
  で、実運用の頻度（毎日か毎時か）によって適正が違う可能性がある。
- **自動停止の永続化先**: in-memory だけだと再起動で解除される（毎朝失敗する設定が毎朝
  3 回試される）。Queen の DB に持つのが素直だが、`user_version` を上げるかどうかは
  5.5.1 の 3 テーブル導入（spec-phase5-5）と衝突しうるので、着手時に整理する。
- **見送りの記録場所**: run が生まれないので `workflow_runs` には書けない。in-memory の
  「最終見送りとその理由」で足りるか、履歴として残すべきか。

### 8.2 未確認（推測として明示する）

- **200ms tick に 1 段足すコストは無視できる**はず（比較 1 回）だが、**未測定**。
  スケジュール数が実用範囲（数本）である前提に乗っている。
- **`chrono::Local` がアプリのライフタイム中に OS のタイムゾーン変更を拾うか**は未確認。
  ノート PC を持って移動した場合の挙動が変わりうる。
- **3.4 の「発火前に枠を数える」で本当に取りこぼしが防げるか**は未検証。root が
  `fanOut` を持つ場合の必要数の数え方（`copies_for`）まで含めて実機で見る必要がある。
- **対象読者が実際にこの語彙で書けるか**は未確認。3.2 の 3 語彙は本書の推測であり、
  ユーザーテストに相当するものは行っていない。
