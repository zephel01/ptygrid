# 仕様書: `onEach: reply` — 上流の返信 1 本ごとに下流を 1 つ起こす（Phase 5.0.7）

作成日: 2026-07-31 / ステータス: **ドラフト（未実装）** / 対象: `workflows:` の依存の書き方と
`orchestrator.rs` の tick

> **採番（3 行）**: patch 番号 `5.0.7` は**提案であり、確定はユーザー判断による**。
> [plan.md](../design/plan.md) §1 の脚注※2 が「5.0.6 を orchestrator の計測に充てる」案を
> 未確定のまま置いているため、その次に来る本書も同じ扱いにする——**5.0.6 が確定するまで
> 5.0.7 も確定しない**。番号が決まるまで plan.md 側の行は「（案）」表記のままとする。

関連: [plan.md](../design/plan.md) §3 P5・§4 項目 3/4・§6.11・§6.12、
[spec-phase5-0.md](spec-phase5-0.md)（workflow の原設計）、
[CONTRACT.md](../../CONTRACT.md)「Phase 5.0 追加契約」、
[ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md)、
`example/measure-parallelism` / `example/measure-coldstart`（本書の根拠になった実測サンプル）

実装対象: `src-tauri/src/config.rs`（`WorkflowStep` / `JoinOn` / `validate_workflows`）、
`src-tauri/src/orchestrator.rs`（`advance_run` の tick）。`queen_store.rs` / `queen.rs` /
frontend は**無変更**（5 章・6 章）。

---

## 1. 目的と背景

### 1.1 いま設定で書けないこと

現在の `dependsOn` は **step の完了を待つ関門**である。完了の定義は `joinOn` の 4 値
（`all` / `any` / `n` / `reply`、`config.rs:567-585`）しかなく、いずれも「**その step が終わったか**」
を判定する述語で、`ready_steps`（`orchestrator.rs:1972-1996`）はその述語が真になるまで下流の
`Pending` を spawn しない。

したがって次の形は**設定として存在しない**:

> 上流が「1 ファイル書けた」と報告するたびに、下流のコピーを 1 つ起こす。

`coder` が 10 ファイル書く workflow で `reviewer` が `dependsOn: [coder]` を持つとき、reviewer は
**10 ファイル目が終わるまで 1 秒も動けない**。回避策は「coder を 10 step に手で割る」しかないが、
それは何ファイルになるかを設定を書く時点で知っている場合にしか書けない。

`onEach: reply` は、**上流の返信 1 本ごとに下流のコピーを 1 つ spawn する**依存の形である。

### 1.2 なぜこれを `mode: serve`（常駐ワーカー）より先に作るか — 2026-07-31 の実測

判断の根拠は推測ではなく実測である（macOS 実機、詳細は plan.md §6.11 / §6.12）。

| 何を測ったか | サンプル | 結果 |
|---|---|---|
| orchestration 自体のコスト | `example/measure-parallelism`（`sh -c 'sleep N'` でエージェントを置換し think time を排除） | **1 依存あたり 200ms**（`DRIVER_TICK_MS`、`orchestrator.rs:3349` ちょうど 1 回分）+ **spawn の事務処理 0.1 秒程度**。`measure-2-split` の step 所要は直列版と同一で、**同時 3 枚でも spawn は重くならない** |
| cold start | `example/measure-coldstart`（同じ agent を 3 段並べ、1 段目 fresh / 2・3 段目 再利用） | fresh spawn **7.7 秒** / 再利用 **4.1・4.5 秒** → **cold start は約 3.4 秒**。ばらつき 0.4 秒に対し 8 倍以上あり有意 |

**cold start の 3.4 秒は下限である。** 計測用プロンプトが「考えず、調べず、ファイルも読まず」と
明示的に禁じているため、実タスクなら入るはずの CLAUDE.md / リポジトリ / pins の読み直しが一切
入っていない。測れたのは起動の事務コストだけで、実タスクではこれより大きくなる
（`example/measure-coldstart` 冒頭の但し書き）。

**結論**: `mode: serve`（常駐ワーカー）が節約するのは 1 step あたり 3.4 秒 + 文脈の読み直し分で、
10 step 回しても数十秒にしかならない。対して `onEach: reply` が節約するのは「上流が全部終わるまで
下流が待つ」という**工程まるごとの待ち時間**で、実タスクなら分単位になる。**ドライバは取り分を
ほとんど食っていない**（1 依存 200ms）ので、**削るべきは spawn のコストではなく待ち時間**である。
`mode: serve` を捨てるのではなく順番が後になる（非スコープ、2.2）。

なお同じ実測で、**走り続けているエージェントが 2 通目・3 通目の kickoff を実際に拾えること**が
確認できている（`measure-coldstart` の 3 段がすべて route 3 = 返信で完了した）。本書が前提に
する「上流が生き続けたまま何度も返信する」挙動は、実機で成立することが確認済みである。

---

## 2. スコープ / 非スコープ

### 2.1 スコープ

- `WorkflowStep` への **`onEach: reply`**（下流側）と **`joinOn: stream`**（上流側）の追加
- `validate_workflows` への検証規則追加（4.2）
- `advance_run` の tick に **unit の切り出し / コピーの採番 / コピーの spawn / stream の閉じ**を追加
- pane 上限（9 面）待ち行列との相互作用の定義（3.6）
- retry / timeout / condition / handoffTo / fail-fast との相互作用の定義（3.7・3.8）
- CONTRACT.md 先行追記（5 章）と `example/` サンプル 1 本

### 2.2 非スコープ

- **`mode: serve`（常駐ワーカー）** — 1.2 のとおり取り分が 1 桁小さい。順番が後（plan.md §3 P5）
- **unit ごとの `condition:` 評価**（「`.rs` を含む unit だけ review する」） — 既存 `condition:` は
  「依存が満たされた瞬間に 1 度だけ」評価する設計（`orchestrator.rs:2053-2056`）であり、
  unit ごとの評価はその意味づけを二重にする。MVP では併用を **load 時 reject**（4.2）にし、
  拡張は 8 章に残す
- **上流の協調キャンセル** — 下流のコピーが失敗したとき上流を止めること。in-flight の兄弟を
  協調キャンセルする機構は現状 `cancel_stragglers` にしか無く（`config.rs:561-566` が明記する
  未実装事項）、本書はそこに手を入れない。代わりに「**新しい unit を切り出さない**」で止める（3.8）
- **`onEach` を含む run の resume** — `resume_workflow` は `Running` の step を base id 単位で
  **コピーごと全部捨てて 1 本の `Pending` に畳む**（`orchestrator.rs:1220-1247`）ため、
  完了済みコピーの記録が消える。加えて `kickoff_root_msg_id` は `#[serde(skip)]`
  （`orchestrator.rs:152-157`、CONTRACT 続報7 (7)）なので再起動後に上流のスレッドを相関できない。
  MVP では **`onEach` を含む workflow の resume を明示的に拒否**する（3.10・4.2）
- **`onEach` を複数の `dependsOn` に付けること** — 「A の返信ごと **かつ** B の返信ごと」は
  直積の意味が決まらない。`dependsOn` 1 件に限定する（既存 `condition:` と同じ制約、
  `config.rs:805-811`）
- **root fan-out の枝番バグの修正** — `spawn_workflow` の root ループが全コピーに枝番なしの同じ
  `step_id` を付ける既知の問題（`orchestrator.rs:1018-1033`、plan.md §3 バックログ）は本書では
  直さない。**本書の設計はこのバグの修正を前提にしない**（3.3 で衝突しないことを示す）
- **Arena view / WorkflowPanel のグルーピング UI** — 6 章のとおり既存 UI が壊れないことだけを担保し、
  「stream をまとめて畳む」等の表示改善は 5.0.5 Arena（plan.md §3 P6 の順 2）に譲る

---

## 3. 設計

### 3.0 用語

| 語 | 意味 |
|---|---|
| **stream step**（上流） | `joinOn: stream` を宣言した step。生き続けたまま、仕事の単位ごとに inbox 返信を送る |
| **unit** | stream step が送った返信 1 本。番兵（3.4.1）だけは unit ではない |
| **onEach step**（下流） | `onEach: reply` を宣言した step。unit 1 本につきコピーを 1 つ持つ |
| **copy**（コピー） | onEach step の実行実体。`"<id>#<k>"` の step_id を持つ（3.3） |
| **stream が閉じる** | 以後 unit が 1 本も増えないことが確定した状態（3.4） |

### 3.1 なぜ既存の `joinOn: reply` では書けないのか（実装由来の制約）

`detect_reply_completions`（`orchestrator.rs:2415-2564`）は**すでに「返信を step に相関させる」
機構を持っている**。本書はこれを作り直さず、次の 2 点だけを変える。

1. **走査対象は `Running` の step に限られる。** スレッド root は `state == Running` の outcome
   からだけ集められ（`:2440-2445`）、突き合わせループも `Running` 以外を `continue` する
   （`:2482-2485`）。**`joinOn: reply` は最初の返信で `Succeeded` になる**（`:2543-2556`）ので、
   その瞬間に走査対象から外れ、**2 本目以降の返信は誰にも見られない**。
   → `joinOn: reply` の上流に `onEach` を付けても、コピーは最大 1 つしか生えない。
   これは実装の帰結であり、**組み合わせとして load 時に拒否する**（4.2）。
2. **1 tick で拾った複数の返信は 1 本に連結される。** `matched` は `merge_reply_bodies` で
   join され、`reply_body` に 1 本の文字列として入る（`:2507-2529`）。unit として扱うには
   **連結せず 1 本ずつ取り出す**必要がある。

裏を返すと、**この 2 点以外は流用できる**: mailbox は run 専用（`queen:workflow/<name>/<run_id>`、
`orchestrator.rs:777-779`、CONTRACT 続報10）、相関は thread root id、送信者は step の agent 名と
一致すること（`:2489-2516`。人間が手でスレッドに答えても step は完了しない）、消費した返信は
ack して二度拾わないこと——すべてそのまま使う。

### 3.2 設定の書き方（論点 1）

**決定: `dependsOn` の要素型は変えない。`onEach: reply` は**下流 step の**独立したフィールドとして
書き、`dependsOn` をちょうど 1 件持つことを要求する。**

```yaml
workflows:
  review-as-you-go:
    pattern: pipeline
    steps:
      - id: coder
        agent: coder
        joinOn: stream          # ← 上流: 返信を送り続け、番兵で完了する
        kickoff: "..."          # joinOn: stream は kickoff 必須（4.2）
        timeoutMs: 1800000
      - id: reviewer
        agent: reviewer
        dependsOn: [coder]      # ← 既存の書き方のまま
        onEach: reply           # ← 下流: coder の返信 1 本ごとにコピーを 1 つ
        kickoff: "..."
```

**却下した案: `dependsOn` を要素ごとにオブジェクト化する**（`dependsOn: [{step: coder, onEach: reply}]`）。
表現力は上だが、`depends_on: Option<Vec<String>>`（`config.rs:638`）は少なくとも 8 箇所から
**文字列として**読まれている——`detect_cycle`（`config.rs:1122`）、handoffTo の逆辺検証
（`config.rs:883-888`）、`condition` の依存数チェック（`config.rs:805-811`）、pipeline / handoff の
線形性検証（`config.rs:922`/`:1006`）、supervisor の root 依存検証（`config.rs:987-992`）、
`ready_steps`（`orchestrator.rs:1983-1993`）、`transitive_dependents`（`orchestrator.rs:2007-2011`）、
`spawn_workflow` の root 判定（`orchestrator.rs:950`）。untagged enum 化するとこの全部に
アクセサが要る。**onEach は 1 依存にしか付けられない（2.2）ので、いま得られる表現力はゼロ**であり、
churn だけが残る。

**後方互換**: `on_each` は `Option<OnEach>` + `#[serde(default, skip_serializing_if)]` で、
`WorkflowStep` の既存 6 個のオプショナルフィールドと同じ形（`config.rs:637-684`）。
**既存の `ptygrid.yml` は 1 文字も変わらず、同じ意味で動く**。`joinOn: stream` も
`JoinOnName` への enum 追加であり、既存の `all` / `any` / `n` / `reply` の意味は変えない。

### 3.3 採番（論点 2）

**決定: コピーの step_id は `"<id>#<k>"`。`k` は **unit の到着順の 0 始まり連番**で、run 内の
当該 step について単調増加・欠番なし・再利用なし。コピーが 1 つしかなくても必ず `#0` を付ける。**

理由と、既知の枝番問題と衝突しないことの確認:

- **`base_id`（`orchestrator.rs:1375-1380`）を変更しない。** `#` の先頭で切るだけの関数で、
  `dep_satisfied` / `dep_unsatisfiable` / `all_terminal` / `failfast_targets` / `check_timeouts` /
  `arm_retry_backoff` / `cancel_stragglers` / `condition_targets` / `verdict` が全部これ経由で
  step 定義を引く。**同じ規約に乗ることが最大の互換性**であり、`@` 等の別セパレータを導入すると
  この全部に分岐が増える。
- **`fanOut` との併用は load 時 reject**（4.2）。したがって 1 つの step id 空間で `#k` の意味が
  2 つになることはない。
- **root fan-out の既知バグ（plan.md §3 バックログ、`orchestrator.rs:1018-1033`）とは交差しない。**
  あのバグは「`spawn_workflow` の root ループが枝番を付けない」ことであり、**onEach step は
  `dependsOn` を必ず 1 件持つので定義上 root になり得ない**（root 判定は
  `depends_on` が空かどうか、`orchestrator.rs:950`）。onEach のコピーは 100% `spawn_ready` 側の
  経路で作られ、root ループを一度も通らない。**バグが直っても直らなくても本書の設計は変わらない。**
- **`#0` を必ず付ける**のは fan-out（`copies >= 2` のときだけ枝番、`orchestrator.rs:2655-2660`）と
  違う点である。理由は 2 つ: (a) unit が 1 本で終わるか 2 本目が来るかは spawn 時点で分からず、
  後から `reviewer` と `reviewer#1` が混在すると `run.steps` 内で「素の id 行」が
  プレースホルダなのかコピーなのか判別できなくなる（3.4.4 がこの区別に依存する）。
  (b) frontend の `{#each run.steps as step (step.stepId)}`（`src/lib/WorkflowPanel.svelte:212`）は
  `stepId` をキーにした keyed each なので、**キーの一意性が UI の要件**である
  （root fan-out で同一キーが並ぶのが、まさにあのバックログ項目の症状）。
- `k` の導出は「その step の既存コピー行数」。コピー行は run から削除されないため単調増加が保証
  される（`spawn_ready` の `retain` が消すのは素の id の `Pending` 行だけ、`orchestrator.rs:2652-2653`）。
  `run.steps` 末尾の並べ替え（`:2676-2682`）は `sort_by_key` = 安定ソートなので、
  同一 step 内のコピーは**到着順のまま**残る。

### 3.4 完了判定 — 「上流がもう返信を送らない」をどう知るか（論点 3）

本書でいちばん難しい論点。**決定: 2 層で閉じる。番兵を第 1 の合図とし、上流 step の終端を
取りこぼしのない backstop とする。**

#### 3.4.1 第 1 層: 番兵メッセージ

**決定: trim 後の本文が `[[end]]` と完全一致する返信を、stream の終了宣言とする。**

| 論点 | 決定 | 理由 |
|---|---|---|
| 判定 | **完全一致**（部分一致・contains は採らない） | unit の本文中で `[[end]]` に言及しただけで stream が閉じるのは事故。「本文全体が番兵」なら誤射しない |
| トークン | 固定文字列 `[[end]]`（定数 `STREAM_END_TOKEN`） | step ごとの正規表現にする案もあるが、フィールドが 1 つ増えるうえ `condition` と同じ「regex が壊れていたら？」問題を持ち込む。固定なら kickoff 文に 1 行書くだけで済む |
| 番兵自身の扱い | **unit にしない**。コピーは生えない | 番兵は仕事の単位ではない |
| 番兵が来たときの step 状態 | `Succeeded`（`mark_terminal` + `StatusView::forget`。ペインは開いたまま） | 既存 `joinOn: reply` の完了処理と同一（`orchestrator.rs:2547-2556`）。ペインが exit しないので `autoClose` は効かない——これは既存の route 3 と同じ既知の挙動 |
| 番兵と unit が同じ本文に混ざったら | **unit として扱う**（完全一致しないため）。stream は閉じず、backstop 待ちになる | 番兵は独立した 1 通で送らせる。kickoff 文にそう書く（3.9） |
| 同一 tick に unit と番兵が並んだら | `list_inbox` の `ORDER BY id ASC`（`queen_store.rs:744`）どおり**古い順に処理**し、番兵より前は unit、番兵より後は破棄して `error` に記録 | 既存 `detect_reply_completions` が「送信順を保つ」ことに依存している点（`:2493-2495`）をそのまま使う |

#### 3.4.2 第 2 層（backstop）: 上流 step の終端

**決定: 上流 step がどの経路であれ終端に達したら、その瞬間に stream は閉じたものとする。**

これは新しい規則ではなく、**実装から導かれる事実**である: `detect_reply_completions` は
`Running` の outcome からしかスレッド root を集めない（`orchestrator.rs:2440-2445`）ので、
**終端に達した step の返信は以後 1 本も相関されない**。つまり「もう unit は増えない」は
証明済みであり、backstop は追加のガードではなく**この事実の宣言**である。

終端に至る経路は既存の 4 つすべてを含む:

| 経路 | 何が起きるか |
|---|---|
| 番兵（3.4.1） | `Succeeded`。正常系 |
| route 1: PTY exit（`classify_session_exit`、`orchestrator.rs:1795-1806`） | エージェントが番兵を送らずに終了した場合。`Succeeded`（exit 0）/ `Failed` |
| route 2: セマンティック `done`（`agent_status`） | 同上。既定の `claude` ルールセットは `done: []` なので通常は発火しない（`example/measure-coldstart` の前提条件 (F)） |
| `timeoutMs` 超過 / `cancel_workflow` | `Failed` / `Cancelled`。**`joinOn: stream` の step には `timeoutMs` を強く推奨**（4.3 のサンプルに入れる）。番兵を送り忘れたエージェントを 5 分でも 1 時間でも待ち続けないための唯一の脱出装置 |

#### 3.4.3 unit が 1 本も来ないまま閉じたとき

**決定: onEach step を `Failed` にする**（理由文字列付き）。`Skipped`（run は green）にはしない。

根拠は既存の `condition:` の判断と同じ形である。`condition_targets` は「依存が返信を残さずに
完了した」場合を `Skipped` ではなく **`Failed`** にしている（`orchestrator.rs:2064-2074`、
`config.rs:660-675`、CONTRACT 続報7 の「決定A」）。理由も同じで、**`Skipped` は
`finalize_state` に対して中立**（`orchestrator.rs:2140-2146`）なので、`Skipped` にすると
「reviewer が 1 度も動かなかった run」が **green で報告される**。operator が
`onEach: reply` と書いた以上、unit が 0 本というのは設定か上流の指示文の不具合であり、
黙って緑にする害のほうが大きい。

**反対意見と、それを採らない理由**: 「上流が正当に『やることが無かった』と判断した」場合まで
red になるのは厳しい。しかし逃げ道はある——その場合エージェントに**「無かった」という unit を
1 本返信させれば**よい（コピーが 1 つ生え、`condition:` 相当の判断は下流の agent が行える）。
設定に knob（`onEachEmpty: fail | skip`）を足す案は 8 章に残し、MVP には入れない。

#### 3.4.4 run 全体の完了判定に必要な変更

既存 `all_terminal`（`orchestrator.rs:2124-2133`）は「宣言された各 step が**最低 1 行の outcome を
持ち**、その全部が `is_terminal`」を要求する。onEach では 2 箇所だけ足りない。

1. **stream が開いている間は完了させない。** コピーが全部終わっても、上流がまだ `Running` なら
   次の unit が来うる。→ `all_terminal` に 1 節を追加: **`on_each` を持つ step は、その依存 step の
   全 outcome が `is_terminal` であることも要求する。**
2. **`dep_satisfied` にも同じ節が要る。** onEach step を `dependsOn` に持つ下流（例: 最後の
   `summary` step）は、`dep_satisfied`（`orchestrator.rs:1813-1837`）が `all` として真になると
   spawn される。**コピー 2 つが成功しただけで stream が開いたまま先へ進んでしまう**ので、
   `dep_satisfied(step)` は `step.on_each.is_some()` のとき **stream が閉じていること**を
   追加要求する。（`dep_unsatisfiable` / `cancel_stragglers` / `verdict` はいずれも
   `dep_satisfied` を経由するので、ここ 1 箇所で揃う。）

素の id の `Pending` プレースホルダ（`spawn_workflow:949-968` が全非 root step に 1 行入れる）は、
**stream トークンとして最初のコピーが生えるまで残し、生えた時点で `retain` で落とす**
（fan-out と同じ形、`orchestrator.rs:2649-2653`。落とす前に `settle_pane_wait` で待ち時間を
コピーへ引き継ぐのも同じ）。unit が 0 本のまま stream が閉じたときだけ、このプレースホルダが
3.4.3 の `Failed` を受け取る行になる。

### 3.5 tick の中での位置

CONTRACT 続報7 (2) が確定させた 10 段の順序を崩さず、**2 段を挿し込む**（`advance_run`、
`orchestrator.rs:3159-3298`）:

```
detect_reply_completions        ← 変更: joinOn: stream の返信を unit として切り出す
  → mint_stream_copies          ← 【新】unit ごとに Pending のコピー行 "<id>#k" を作る
  → detect_completions
  → check_timeouts
  → cancel_stragglers
  → fire_due_retries
  → arm_retry_backoff
  → condition_targets
  → failfast_targets
  → close_stream_targets        ← 【新】上流が終端なら stream を閉じる（3.4.3 の Failed もここ）
  → spawn_ready                 ← 変更: onEach のコピーを 1 スロットずつ spawn する
  → arm_retry_backoff（2 回目）
  → finalize_state
```

置き場所の理由:

- `mint_stream_copies` は `detect_reply_completions` の**直後**。同じ tick で切り出した unit を
  同じ tick で行にする——unit をまたいだ tick に持ち越すと、その間にクラッシュした場合に
  「ack 済みなのに行が無い」unit が生まれる。
- **ack のタイミングを 1 段ずらす。** 現在 `detect_reply_completions` は関数末尾で消費した返信を
  ack する（`:2559-2561`）。stream の unit については、**コピー行を作ってから ack する**
  （ack が先だと、クラッシュ窓で unit が永久に失われる。逆順なら次回起動時に再走査されるだけ）。
  なお `onEach` を含む run は resume しない（2.2）ので、これは「同一プロセス内で `persist_run` の
  前に落ちた場合」の穴を塞ぐための整理であり、完全な保証ではない（8 章に未確認として残す）。
- `close_stream_targets` は `failfast_targets` の**後**、`spawn_ready` の**前**。閉じ判定が
  fail-fast の後なら、fail-fast で潰れた step にさらに閉じの理由文字列を上書きしない
  （`condition` を fail-fast より前に置いた既存の理由付けと同じ、`orchestrator.rs:3231-3237`）。

### 3.6 pane 上限との相互作用（論点 4）

グリッドは 9 面（`WORKFLOW_SESSION_CAP`、`orchestrator.rs:43`）。unit が 20 本来ればコピーは
20 個要る。

**決定: 既存の待ち行列（`Pending` + `deferred_since_ms`）にそのまま乗せる。新しい待ち行列は
作らない。ただし `WORKFLOW_DEFER_MAX_MS`（5 分）の打ち切りは、行列が実際に流れている間は
適用しない。**

| 項目 | 決定 |
|---|---|
| 待ち方 | コピー行は `Pending` のまま `deferred_since_ms` を持つ。`defer_step`（`orchestrator.rs:656-700`）はもともと `base_id(o.step_id) == step.id && Pending` の**全行**を対象にするので、コピー行はそのまま扱える |
| spawn の粒度 | **1 コピー = 1 スロット**。fan-out の all-or-nothing 規則（`orchestrator.rs:2576-2582`）は適用しない——unit は互いに独立で、「一部だけ走らせる」ことに意味があるため。`dep_unsatisfiable` の ceiling 計算が壊れないのは、onEach step の join が `all`/`reply` に限られる（3.7）ため |
| ペイン再利用 | **必ず `reuse_existing = false`。** ここは黙って壊れる箇所なので明記する: `agent_claimed_by_other_step`（`orchestrator.rs:1937-1941`）は `base_id(o.step_id) != step.id` で判定するため、**同じ step の兄弟コピーは「他の step」に数えられない**。既存コードのまま `copies == 1` の経路に載せると、2 つ目のコピーが 1 つ目のペインを黙って引き取り（`spawn_step` の `reusable`、`orchestrator.rs:422-449`）、1 つの session を 2 つの outcome が追跡する。`slots_needed`（`:531-542`）も 0 を返して**上限を踏み越える**。fan-out のコピーが常に fresh spawn なのと同じ扱いにする |
| 打ち切りの上限 | **`WORKFLOW_DEFER_MAX_MS`（5 分）は、その step の兄弟コピーが 1 つも `Running` でないときだけ適用する。** |

最後の 1 行が本節の実質である。`WORKFLOW_DEFER_MAX_MS` の doc comment（`orchestrator.rs:45-55`）は
その存在理由を **「外部（他の run・チーム・operator）がグリッドを占有し続ける wedge から
run を守る」**と明記しており、「a run that owns no pane always has the full budget available」を
前提にしている。ところが onEach では**その run 自身のコピーが 9 面を埋める**。20 unit × 各 5 分の
review なら 10 番目以降の unit は必ず 5 分以上待ち、**行列が正常に流れているだけなのに失敗する**。
これは定数の意図の外側なので、判定に「兄弟が走っているか」を足す:

- 兄弟コピーが `Running` = **行列は流れている** → 打ち切らない（待ち時間は
  `waited_for_pane_ms` に累積し続けるので、事後に「9 面上限がこの run にいくら課金したか」は
  従来どおり読める、`orchestrator.rs:138-151`）
- 兄弟が 1 つも `Running` でない = **グリッドは他人に握られている** → 従来どおり 5 分で失敗
  （`run_is_stalled`（`:637-640`）が付ける「this run holds no pane of its own」の説明もそのまま効く）

**併せて、1 つの stream step が生む unit の総数に上限を置く**: `STREAM_MAX_UNITS = 64`。
超過分は unit にせず、stream step を `Failed`（理由文字列付き）にして stream を閉じる。
根拠は「暴走したエージェントが `run.steps` と `steps_json`（`persist_run`、
`orchestrator.rs:2277-2290`）を無限に膨らませるのを止める」ことで、`INBOX_REPLY_SCAN_LIMIT`
（200、`:2379`）/ `REGISTRY_TERMINAL_CAP`（100、`:246`）/ `MAX_MESSAGES_PER_PROJECT`
（50,000、`queen_store.rs:21`）と同じ posture である。**64 という値そのものは根拠の弱い初期値**
（9 面 × 約 7 波）で、8 章に未確定として残す。

### 3.7 retry / timeout / condition / handoffTo との相互作用（論点 5）

既存機能の多くは「1 step = 1 コピー」ではなく **1 outcome 単位**で書かれているため、実は
そのまま動くものが多い。動かないものだけを reject する。

| 機能 | onEach step での挙動 | 根拠 |
|---|---|---|
| **`timeoutMs`** | **コピーごとに独立して効く**。無変更 | `check_timeouts` は `run.steps` の各 outcome を回し、step 定義を `base_id` で引く（`orchestrator.rs:2708-2718`）。コピー行はそのまま対象になる。待ち行列にいる間は `Pending` かつ `started_at_ms == 0` なので計時されない（既存の意図どおり、`:2692-2699`） |
| **`retry`** | **コピーごとに独立して効く**。ただし 1 点だけ実装追加が要る | `arm_retry_backoff`（`:2835`）/ `respawn_fresh`（`:2789`）とも `base_id` 経由・`step_id` 保持で書かれている。追加が要るのは **unit 本文の再配送**: `respawn_fresh` は `carried` を引数で受けて `deliver_kickoff` し直す（`:2802`）ので、**そのコピーの unit 本文を outcome に持たせておく**必要がある（`#[serde(skip)] stream_body: Option<String>`。`reply_body` は「この step の agent が言ったこと」で意味が違うので流用しない） |
| **`condition`** | **同一 step での併用を reject。stream step を依存に持つ step でも reject** | `condition_targets` は依存の `reply_body` を 1 度だけ評価する（`:2097-2101`）。stream step の `reply_body` は「直近の返信」でしかなく、どの unit を指すのか決まらない。既存も同じ理由で「`condition` の依存が fan-out step であること」を拒否している（`config.rs:818-825`）ので、その規則の自然な拡張である |
| **`handoffTo`** | **onEach step / stream step のどちらに書いても reject。onEach step を `handoffTo` の宛先にするのも reject** | `handoff_bodies`（`:1906-1926`）は「どのコピーの返信を運ぶか」を宣言順の先勝ちで決める。既存も同じ理由で `fanOut` + `handoffTo` を拒否している（`config.rs:859-871`、「which copy's reply would be carried is undefined」）。宛先側の拒否は、onEach のコピーが**自分の unit 本文を carried 半分として使う**ため、handoff の carried と衝突するからである |
| **`fanOut`** | **同一 step での併用を reject** | 3.3 のとおり `#k` の意味が二重になる |
| **onEach step 自身の `joinOn`** | **`all`（既定）と `reply` を許可。`any` / `n` / `stream` は reject** | `n` は load 時に「pattern の実効コピー数」に対して範囲検証される（`config.rs:779-797`）が、onEach のコピー数は**実行時まで決まらない**ので検証できない。`any` は `cancel_stragglers`（`:3080-3149`）が最初の成功時点で残りのコピーを kill するが、コピーは互いに独立した仕事の単位なので kill は明確に誤り。`reply` は**コピーごとに独立して効く**（各コピーが自分の kickoff スレッドに返信して完了する）ので許可する |
| **上流 stream step 側の他フィールド** | `kickoff` **必須**（`joinOn: reply` と同じ理由、`config.rs:827-845`）。`fanOut` / `handoffTo` / `condition` は reject | 返信すべきスレッドが無ければ unit を送りようがない |
| **`pattern`** | `pipeline` / `fan-out` / `supervisor` で使える。**`handoff` では reject** | handoff は「全 step が root からの単一鎖に過不足なく乗る」ことを要求し（`config.rs:1088-1094`）、鎖は `handoffTo` で繋がる。onEach は `handoffTo` を持てないので鎖に乗れない |
| **`autoClose` / `close_on_exit`** | 変更なし。ただし **stream step のペインは番兵で完了しても exit しない**ので `autoClose` は効かない | route 3 の既知の性質（`example/measure-coldstart` の autoClose 節）。straggler kill 時に workflow 所属を見失う既知の問題（plan.md §3 バックログ）も本書では触らない |

### 3.8 失敗と部分成功（論点 6）

**決定: 新しい失敗ポリシーは作らない。既存の `onFailure`（`fail-fast` 既定 / `continue`）の
意味をそのまま適用する。ただし fail-fast のとき、`onEach` step は新しい unit を切り出さない。**

| 事象 | `fail-fast`（既定） | `continue` |
|---|---|---|
| コピー 1 つが `Failed`（retry 予算も尽きた） | `finalize_state` が run を `Failed` にする（`orchestrator.rs:2157-2159`。`Failed` は無条件で red）。**加えて `mint_stream_copies` はこの step の新しい unit を切り出すのをやめる**——止めないと、run が既に doomed と決まったあとに 10 枚のペインを開き続ける | 他のコピーと他のブランチは走り続ける。unit の切り出しも続く |
| 上流 stream step が `Failed`（timeout / 番兵前の異常終了） | stream は閉じる（3.4.2）。既に生えたコピーは**最後まで走る**（in-flight の協調キャンセルは未実装、`config.rs:561-566`）。`failfast_targets` が onEach step の**下流**に `Skipped` を伝播する | 同左（伝播なし） |
| unit 0 本で閉じた | 3.4.3 のとおり `Failed` | 同じ（`Failed` の意味は policy に依らない） |

**「新しい unit を切り出さない」を上流のキャンセルにしない理由**: 上流を kill する機構は
`cancel_stragglers` にしかなく、それは「join がもう満たされた」ケース専用に書かれている。
fail-fast のために新しいキャンセル経路を作るのは本書のスコープを超える（2.2）。切り出しを
止めるだけなら**純粋な述語の追加**で済み、既存の失敗経路の意味も変わらない。

### 3.9 上流エージェントへの指示の書き方（論点 7）

**決定: 新しい語彙は要らない。既存の `kickoff:` + Queen の `reply_inbox` / `await` で表現できる。
ただし「返信 1 本 = 1 unit」「番兵で終わる」を kickoff 文に書くのは operator の責任とし、
サンプル（4.3）で定型文を提供する。**

実装から確認できる事実（推測ではない）:

1. **kickoff の宛先は agent 名の mailbox。** `deliver_kickoff` は
   `send_inbox(sender = queen:workflow/<name>/<run_id>, recipient = step.agent, ...)`
   を呼ぶ（`orchestrator.rs:801-809`）。エージェントは `await` を **mailbox = 自分の定義名**で
   呼んで待つ（`example/measure-coldstart` と同じ形）。
2. **返信は必ず kickoff メッセージの id に対して行う。** `reply_inbox` は
   `sender` が**元メッセージの recipient と完全一致**することを要求し（`queen_store.rs:815-820`）、
   返信の宛先を**元メッセージの sender**（= run mailbox）にし、`root_message_id` を継承する
   （`:837-841`）。つまりエージェントは自分の返信に返信できない（自分は自分の返信の recipient では
   ないため）。**毎回 kickoff の id を使って返信する**のが唯一の正しい書き方であり、その結果
   全 unit が同じ thread root を共有して `detect_reply_completions` の相関が成立する。
3. **同じメッセージへの複数回の返信は許される。** `reply_inbox` は元メッセージの ack を
   `COALESCE` で行う（`:848-854`）ので 2 回目以降も失敗しない。
4. **`await` のタイムアウトは 1..300000ms、既定 30000ms**（`queen.rs:1183-1189`）。
   `example/measure-coldstart` が採った 55000ms の理由（クライアント側のツールタイムアウトの
   内側かつ既定より長い）はそのまま流用できる。

kickoff の定型（4.3 のサンプルで配る文言の骨子）:

> 仕事を **1 単位ずつ**進めてください。1 単位が終わるたびに、queen の `reply_inbox` を
> `id=<この kickoff の id>`, `sender=<あなたの定義名>`, `body=<その 1 単位の説明>` で呼んで
> **すぐ次の単位に進んで**ください。まとめて報告しないこと。すべて終わったら、**本文が
> `[[end]]` だけの返信を 1 通**送ってから待機をやめてください。`[[end]]` は他の文と混ぜないこと。

### 3.10 却下した代替案

- **上流に何も書かせず、`joinOn: reply` の意味を「返信ごと」に変える**: 却下。既存の
  `joinOn: reply` は「最初の返信を答えとみなす」ことが仕様として明記され
  （`orchestrator.rs:2398-2407` の PROTOCOL 注記）、`example/measure-coldstart` がその上に
  乗っている。破壊的変更になる。
- **番兵を使わず、上流の終端だけで閉じる**: 却下。route 1（PTY exit）は対話型 CLI では起きず、
  route 2 は `claude` の既定ルールセットが `done: []` なので発火しない
  （`example/measure-coldstart` 前提条件 (F)）。**残るのは `timeoutMs` だけ**になり、
  正常系ですら「タイムアウトするまで下流の完了が確定しない」workflow になる。
- **番兵を使わず、無返信が N tick 続いたら閉じる（quiet timeout）**: 却下。
  `detect_reply_completions` の PROTOCOL 注記（`:2404-2407`）が既に同じ罠を記録している——
  「静かな tick を待っても打ち切りが 200ms 後ろにずれるだけ」。エージェントが考え込んでいる時間と
  仕事が終わった時間は区別できない。
- **`onEach` を新しい `pattern:` にする**（`pattern: stream`）: 却下。`pattern` は
  **形状バリデーション**と `copies_for` の 1 箇所にしか効かない設計で、ドライバは意図的に
  pattern 非依存に保たれている（`config.rs:518-532`、CONTRACT 続報7 (1)）。onEach は step 単位の
  性質であり、workflow 全体の形状ではない。
- **コピーの kickoff を `handoffTo` の carried 機構に相乗りさせる**: 却下。`handoff_bodies` は
  **step 単位の map**（`HashMap<target_step_id, String>`、`orchestrator.rs:1906`）で、
  コピーごとに違う本文を運べない。unit 本文はコピー行に持たせる（3.7 の `stream_body`）。
- **`onEach` を含む run も resume する**: 却下。`resume_workflow` は `Running` のコピーがあると
  その base id の **outcome を全部捨てて 1 本の `Pending` に畳む**（`orchestrator.rs:1220-1247`）ので、
  完了済みコピーの記録ごと消える。加えて `kickoff_root_msg_id` が `#[serde(skip)]` なので
  上流スレッドを相関し直せない。**中途半端に resume するより、明示的に拒否して
  `abandon_workflow` に落とすほうが安全**である（4.2 で load 時ではなく resume 時のエラーとする）。

---

## 4. `ptygrid.yml` スキーマ

### 4.1 追加フィールド

```rust
// config.rs — JoinOnName に 1 値追加（既存 4 値の意味は不変）
pub enum JoinOnName { All, Any, Reply, Stream }

// config.rs — WorkflowStep に 1 フィールド追加
#[serde(default, skip_serializing_if = "Option::is_none")]
pub on_each: Option<OnEach>,          // YAML: onEach

#[derive(...)]
#[serde(rename_all = "lowercase")]
pub enum OnEach { Reply }             // YAML: onEach: reply
```

`WorkflowStep` は `#[serde(rename_all = "camelCase")]`（`config.rs:630`）なので YAML キーは
`onEach`。`onEach` を 1 値だけの enum にするのは、将来 `onEach: line` 等を足す余地を残しつつ
今日は 1 つしか意味を持たせないため（`JoinOn` が untagged enum で `all` と `3` を両方受ける
のと同じ、`config.rs:567-572`）。

### 4.2 検証ルール（`validate_workflows` への追加）

すべて **load 時 reject**。既存のエラー文言と同じく workflow 名と step 名を必ず含める
（`config.rs:699-702` の方針）。

| # | 規則 | なぜ |
|---|---|---|
| V1 | `onEach` を持つ step は `dependsOn` を**ちょうど 1 件**持つ | 2.2。既存 `condition` と同形（`config.rs:805-811`） |
| V2 | `onEach` step の唯一の依存は **`joinOn: stream`** を宣言していなければならない | 3.1 の (1)。`reply` は最初の返信で走査対象から外れ、`all`/`any`/`n` は返信で完了しないので閉じる合図が `timeoutMs` しか無くなる |
| V3 | `joinOn: stream` の step は**非空の `kickoff:`** を持つ | `joinOn: reply` と同じ理由（`config.rs:827-845`）。返信すべきスレッドが無い |
| V4 | `onEach` と `fanOut` の併用を拒否 | 3.3。`#k` の意味が二重になる |
| V5 | `onEach` と `condition` の併用を拒否／`condition` の依存が `joinOn: stream` の step であることを拒否 | 3.7。既存の「`condition` の依存が fan-out」拒否（`config.rs:818-825`）の拡張 |
| V6 | `onEach` step / `joinOn: stream` step が `handoffTo` を持つことを拒否。`handoffTo` の**宛先**が `onEach` step であることも拒否 | 3.7。既存の `fanOut` + `handoffTo` 拒否（`config.rs:859-871`）と同じ論法 |
| V7 | `onEach` step の `joinOn` は `all`（未宣言含む）か `reply` のみ。`any` / `n` / `stream` を拒否 | 3.7。`n` は実行時までコピー数が決まらず範囲検証（`config.rs:779-797`）が成立しない |
| V8 | `joinOn: stream` の step を `dependsOn` に持つ step は、**`onEach` を持つか、さもなければ普通の依存として扱われる**（拒否はしない） | 「stream の全 unit が終わってから 1 度だけ動く summary step」は正当な書き方。3.4.4 の `dep_satisfied` 拡張がこれを支える |
| V9 | `pattern: handoff` の workflow に `onEach` / `joinOn: stream` が現れることを拒否 | 3.7。鎖に乗れない |
| V10 | `joinOn: stream` の step が `onEach` の依存として**一度も使われていない**場合を拒否 | 番兵と unit の protocol を要求しておいて誰も読まない設定は、ほぼ確実に書き間違い。`handoffTo` の逆辺要求（`config.rs:883-896`）と同じ「書いたのに効かない」を潰す規則 |

**resume 時の拒否（load 時ではない）**: `resume_workflow` は、対象 run の workflow 定義に
`onEach` を持つ step があれば `Err("workflow '<name>' contains an onEach step and cannot be resumed; ...")`
を返す（3.10）。frontend の resume バナーは既存の失敗表示に落ちる。

### 4.3 実例

```yaml
# 「coder が 1 ファイル書き終えるたびに reviewer を 1 人起こす」
project: review-as-you-go

queen:
  enabled: true          # 必須。unit の配送も返信も durable inbox 経由
  port: 39237

agents:
  - name: coder
    cmd: "claude"
    cwd: "."
    autostart: false
  - name: reviewer
    cmd: "claude"
    cwd: "."
    autostart: false

workflows:
  review-as-you-go:
    pattern: pipeline
    onFailure: continue        # レビュー 1 件の失敗で run 全体を落とさない（3.8）
    steps:
      - id: coder
        agent: coder
        joinOn: stream         # ← 返信を送り続け、[[end]] で完了する
        # 番兵を送り忘れたときの唯一の脱出装置（3.4.2）。必ず書くこと。
        timeoutMs: 1800000     # 30 分
        kickoff: >-
          src/ 以下のモジュールを 1 つずつ実装してください。
          1 モジュール書き終えるたびに、queen の reply_inbox を
          id=<このメッセージの id>, sender=coder, body=<書いたファイルのパスと要点>
          で呼び、すぐ次のモジュールに進んでください。まとめて報告しないこと。
          全部終わったら、本文が [[end]] だけの返信を 1 通送ってから待機をやめてください。
          [[end]] は他の文と混ぜないこと。

      - id: reviewer
        agent: reviewer
        dependsOn: [coder]
        onEach: reply          # ← coder の返信 1 本ごとに reviewer#k が 1 つ生える
        joinOn: reply          # ← コピーごとに独立して効く（3.7）
        timeoutMs: 600000
        retry:
          max: 1
          backoffMs: 5000
        kickoff: >-
          直前に示されたファイルだけをレビューしてください。他のファイルは見ないこと。
          終わったら queen の reply_inbox で結果を返信してください。

      - id: summary
        agent: coder
        dependsOn: [reviewer]  # ← stream が閉じ、全コピーが終わってから 1 度だけ動く（3.4.4）
        joinOn: reply
        kickoff: "レビュー結果を 1 つにまとめてください。"
```

`reviewer` のコピーが受け取る kickoff は、**unit 本文 + 宣言された `kickoff:`** を
`compose_kickoff`（`orchestrator.rs:1574-1584`）で合成したもの——`handoffTo` の合成規則と
まったく同じ形（carried が先、宣言テキストが後、48KiB / 64KiB のクリップも同じ）である。
新しい合成規則は作らない。

---

## 5. wire 契約の差分と CONTRACT.md 追記項目（論点 8）

### 5.1 wire の差分 — **追加フィールドはゼロ**

| wire | 差分 |
|---|---|
| `StepOutcome`（`workflow-state` / `spawn_workflow` / `join_workflow` / `list_workflow_runs` の返り値） | **フィールドの増減なし。** コピーの識別は既存の `stepId` に載る `"<id>#<k>"` だけで足り（3.3）、unit 本文は `#[serde(skip)] stream_body` として内部簿記に留める（`reply_body` / `kickoff_root_msg_id` / `next_retry_at_ms` と同じ扱い、CONTRACT 続報7 (7)） |
| `WorkflowRun` | 形は不変。**`steps[]` が run の途中で増える頻度が上がる**（従来は spawn 時にだけ増えた）。件数は `STREAM_MAX_UNITS`（3.6）で有界 |
| `workflow-state` イベント | **新規イベントなし。** 既存イベントが unit ごとに 1 回多く飛ぶ。`advance_run` の `if changed` ガード（`:3293-3297`）は従来どおり効くので、無変更 tick では飛ばない |
| Tauri commands / Queen MCP tools | **無変更。** `spawn_workflow` / `cancel_workflow` / `list_workflow_runs` / `join_workflow` のシグネチャも意味も変えない |
| `ptygrid.yml` スキーマ | `onEach`（新規キー）と `joinOn: stream`（既存キーの新しい値）の 2 つだけ。**既存設定はバイト単位で同じ意味** |
| `resume_workflow` | 新しいエラー文字列 1 本（4.2）。既存の workflow に対する挙動は不変 |

**結論: 完全に additive。** 破壊的変更は無く、`StepOutcome` の JSON 形すら変わらないので、
5.0.6 以前に永続化された `steps_json` の deserialize も影響を受けない。

### 5.2 CONTRACT.md 追記項目（実装前に先行追記）

1. `onEach: reply` と `joinOn: stream` の**意味の定義**（3.2・3.4）と、**番兵 `[[end]]` が
   wire protocol の一部であること**（trim 後の完全一致）
2. `"<id>#<k>"` の採番規約が **onEach にも適用**され、`k` は unit 到着順で単調増加、
   **コピーが 1 つでも `#0` を付ける**こと（3.3）
3. **stream が閉じる条件**（番兵 / 上流 step の終端の 2 層）と、`dep_satisfied` / `all_terminal` が
   onEach step に対して「stream が閉じていること」を追加要求すること（3.4.4）
4. `advance_run` の tick 順序を **10 段 → 12 段**に更新（3.5。CONTRACT 続報7 (2) の差し替え）
5. **`WORKFLOW_DEFER_MAX_MS` の適用条件の変更**（兄弟コピーが `Running` の間は打ち切らない、3.6）
   と、新定数 `STREAM_MAX_UNITS`
6. load 時検証 V1〜V10（4.2）と、**resume の拒否**
7. **非回帰宣言**: `StepOutcome` / `WorkflowRun` / `workflow-state` / Tauri commands /
   Queen MCP tools は**すべて不変**。既存の `joinOn: all|any|n|reply` の意味も不変。
   本節はすべて additive
8. **既知の限界の明記**: (a) `onEach` を含む run は resume できない、(b) unit の ack と
   コピー行の永続化の間にクラッシュ窓が残る（3.5）、(c) 番兵で完了した stream step の
   ペインは exit しないので `autoClose` が効かない

---

## 6. UI（WorkflowPanel）への影響

**決定: frontend は無変更で成立する。本 patch では触らない。**

- `{#each run.steps as step (step.stepId)}`（`src/lib/WorkflowPanel.svelte:212`）は
  `stepId` をキーにした keyed each。3.3 の「常に `#k` を付ける」規約により**キーは一意**で、
  root fan-out のバックログ項目（同一キーが並ぶ）と同じ症状は起きない。
- 行が run の途中で増えるのは fan-out で既に起きている（`spawn_ready` が copies 個 push する）。
  増える**回数**が変わるだけで、描画経路は同じ。
- `StepOutcome` の型（`src/lib/types.ts:123-140`）は無変更なので `svelte-check` に影響しない。
- **将来やること（本 patch 外）**: コピーが 20 行並ぶとパネルが縦に伸びる。畳み表示や
  「stream: 12/20 完了」のような集約は 5.0.5 Arena（plan.md §3 P6 の順 2）で扱う。

---

## 7. 段階的な作り方（論点 9）とテスト

### 7.1 MVP の線引き

**MVP（最小で価値が出る切り口）= 「1 本の stream から 1 種類の下流を起こす」まで。**

| 入るもの | 入らないもの（後続） |
|---|---|
| `onEach: reply` / `joinOn: stream` / 番兵 / 上流終端 backstop | unit ごとの `condition:`（2.2） |
| `#k` 採番、`dep_satisfied` / `all_terminal` の stream 節 | `onEach` を含む run の resume（2.2） |
| pane 待ち行列への相乗りと 5 分打ち切りの条件付き化、`STREAM_MAX_UNITS` | `maxInFlight`（同時実行数の明示上限。8 章） |
| コピー単位の `timeoutMs` / `retry`（`stream_body` の再配送を含む） | 上流の協調キャンセル（2.2） |
| V1〜V10 の load 時検証、resume 拒否 | `mode: serve`（2.2） |
| `pattern: pipeline` / `fan-out` / `supervisor` での利用 | Arena / パネルの集約表示（6 章） |
| `example/` サンプル 1 本 + CONTRACT 追記 | `onEachEmpty` 等の knob（8 章） |

この線を引く理由: **「上流が終わるまで下流が待つ」時間を削る**という 1.2 の目的は、上の左列
だけで達成される。右列はどれも「削れる時間」ではなく「使い勝手」か「別の目的」であり、
左列が実機で成立することを確認する前に足すと、失敗したときの切り分けが増える。

### 7.2 テスト

**unit（`cargo test`）** — 既存の `orchestrator.rs` / `config.rs` のテストと同じく、PTY も store も
使わない純関数に寄せる。`advance_run` を直接呼ぶ既存の統合スタイル（ドライバスレッドを
起こさない）も踏襲する。

- **検証**（`config.rs`）: V1〜V10 のそれぞれについて拒否 1 本 + 受理 1 本。特に **V2**
  （`joinOn: reply` の上流に `onEach` を付けると reject）は 3.1 の実装事実を固定する回帰
- **採番**: 3 unit で `#0`/`#1`/`#2` が付き、コピーが 1 つでも `#0` になること。
  `base_id` が全部 `reviewer` を返すこと
- **stream の閉じ**: (a) 番兵で `Succeeded` になり、番兵の本文が unit にならないこと、
  (b) 番兵より後の同一 tick の返信が破棄されること、(c) 上流が `timeoutMs` で `Failed` に
  なっても stream が閉じ、run が終端に到達すること（wedge しないこと）
- **unit 0 本**: 上流が番兵だけ送った run で onEach step が `Failed` になり、
  run が **green にならない**こと（3.4.3。`Skipped` にした場合に green になってしまうことの回帰）
- **完了判定**: コピーが全部 `Succeeded` でも上流が `Running` の間は `all_terminal` が false、
  かつ onEach step を `dependsOn` に持つ下流が spawn されないこと（3.4.4 の 2 箇所）
- **ペイン再利用の禁止**: 2 つ目のコピーが 1 つ目のペインを adopt しないこと
  （`slots_needed` が 1 を返すこと）。3.6 の「黙って壊れる」経路の回帰
- **待ち行列**: 9 面を埋めた状態で 12 unit を投げ、(a) 兄弟が走っている間は 5 分を超えても
  失敗しないこと、(b) 兄弟が 1 つも走っていなければ従来どおり 5 分で失敗すること
- **上限**: `STREAM_MAX_UNITS` 超過で stream step が `Failed` になり、それ以上コピーが増えないこと
- **retry**: コピーが失敗して再 spawn されたとき、**その unit の本文が再配送される**こと
  （空の kickoff にならないこと）
- **fail-fast**: コピー 1 つが terminal `Failed` になった後、新しい unit が切り出されないこと
  （`onFailure: continue` では切り出され続けること）
- **非回帰**: `onEach` / `joinOn: stream` を含まない既存の workflow 定義に対して、
  `advance_run` の挙動が現行と同一であること（既存 56 本が落ちないことに加え、
  明示のテストを 1 本置く）

**実機手動検証**（macOS 必須 / Linux ベストエフォート）— **本書は手順だけを書き、実施状況は
書かない**。実施状況は plan.md §2 に U 番号として登録する。

1. 4.3 のサンプルを実機で流し、**上流が 3 本目の返信を送る前に 1 人目の reviewer が
   動き出していること**（= 直列でないこと）をスクリーンショットで確認する
2. 同じ課題を `onEach` 無し（直列）と `onEach` 有りで**各 2 回ずつ**流し、run の壁時計を比べる
   （エージェントの所要はばらつくので 1 回では差が判定できない。plan.md §3 P5 の方針）
3. 番兵を送り忘れる指示文にわざと差し替え、`timeoutMs` で run が終端に到達すること
4. unit を 12 本出させ、9 面上限で待ち行列ができ、**5 分を超えても待ちが失敗にならない**こと
   （3.6 の条件付き打ち切り）

`completion gate:` unit + 統合が通り、`svelte-check` / `npm run build` が 0 errors、
CI（macOS / Ubuntu）green、CONTRACT.md への先行追記完了、`example/` サンプル 1 本追加、
実機手動検証 1〜4 が plan.md §2 に U 番号として登録されていること。

> バージョン割当: plan.md §4「次タグの前提」に従い、**本書はタグ番号を確定させない**。
> patch 番号 5.0.7 とタグ `vX.Y.Z` の対応はリリース時に plan.md §4 で決める（冒頭の採番注記）。

---

## 8. 未解決事項 / 未確認

3 章で決定として潰していない論点と、**確認できていないこと**を分けて残す。

### 8.1 未決（(a)/(b) と、倒し方の帰結）

- **番兵トークンの形**: (a) 固定文字列 `[[end]]`（現方針）、(b) step ごとの正規表現
  （`streamEnd:`）。(b) はフィールドが 1 つ増え、`condition` と同じ「壊れた regex をどう扱うか」を
  持ち込む。実機でモデルが `[[end]]` を正確に再現できないことが分かったら (b) へ倒す。
- **`STREAM_MAX_UNITS = 64`**: 値そのものに根拠は薄い（9 面 × 約 7 波）。上限を置くこと自体は
  `INBOX_REPLY_SCAN_LIMIT` / `REGISTRY_TERMINAL_CAP` と同じ posture で決定済みだが、
  **数字は実タスク測定（plan.md §3 P5）の後に見直す**。
- **同時実行数の明示上限（`maxInFlight`）**: いまは 9 面上限が事実上の絞りになっている。
  (a) 足さない（現方針）、(b) step ごとに `maxInFlight: 3` を書けるようにする。(b) は
  「他の run にペインを譲る」用途で欲しくなるが、pane 待ちと二重の待ち行列になるので保留。
- **unit 0 本のときの扱い**: (a) `Failed`（現方針、3.4.3）、(b) `onEachEmpty: fail | skip` の knob。
  (b) は「正当に仕事が無かった」ケースを green にできるが、knob 1 つで
  `condition:` の決定A（`Failed`）と食い違う 2 つ目の規則が生まれる。
- **unit ごとの `condition:`**: いまは併用を reject（V5）。将来 unit 本文に対して評価する形にすると
  「`.rs` を含む unit だけ review」が書けるが、既存 `condition` の「1 度だけ評価」の意味づけを
  二重にする。別フィールド（`onEachIf:`）にするかどうかも含めて未決。
- **`onEach` を含む run の resume**: いまは拒否（4.2）。直すには `kickoff_root_msg_id` の
  永続化（CONTRACT 続報7 (7) の resume ギャップ）と `resume_workflow` のコピー畳み込み
  （`orchestrator.rs:1220-1247`）の両方を変える必要があり、**それ自体が独立した patch** になる。

### 8.2 未確認（実装/実測で確かめていないこと。推測として明示する）

- **実タスクでの取り分**: 1.2 の「実タスクなら分単位」は**合成 workflow と cold start の実測からの
  外挿であり、実タスクでは未測定**。plan.md §3 P5 のベースライン測定（同じ課題を 2 回ずつ）で
  確かめる。本書の投資判断はこの外挿に乗っている。
- **エージェントが「1 単位ごとに返信する」指示に従うか**: `example/measure-coldstart` で
  確認できているのは「**走り続けているエージェントが 2 通目・3 通目の kickoff を拾える**」ことまで
  （plan.md §6.12）。**「1 単位ごとに自発的に返信を刻む」挙動は未確認**であり、モデル依存の
  可能性が高い。ローカル LLM では指示追従がさらに弱い（同サンプルの注記）。
- **ack とコピー行の永続化の間のクラッシュ窓**（3.5）: 順序を入れ替えて窓を狭めるが、
  `persist_run` は tick の末尾（`orchestrator.rs:3293-3297`）なので**完全には塞がらない**。
  resume を拒否している以上（4.2）実害は「そのプロセスの run が壊れる」だけだが、
  **実測していない**。
- **1 tick に大量の unit が来たときの挙動**: `INBOX_REPLY_SCAN_LIMIT`（200、`:2379`）と
  `list_inbox` の clamp（1..200、`queen_store.rs:735`）により 1 tick / 1 スレッドあたり 200 件で
  頭打ちになり、残りは次 tick に回る**はず**だが、**未検証**。
- **`STREAM_MAX_UNITS` 級のコピーを持つ run の `steps_json` サイズ**: `stream_body` を
  `#[serde(skip)]` にしたので 1 行あたりは小さい**はず**だが、64 行 × 毎 tick の
  `upsert_workflow_run` のコストは**未測定**。
- **Windows**: 本 patch は PTY にも PATH にも触らないので影響は無い**はず**だが、
  他機能と同じく U8（plan.md §2）の範囲で確認するまで**未確認**とする。
