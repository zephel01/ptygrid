# 実装設計: pane 上限の待ち行列化 / driver tick 軽量化 / inbox mailbox 分離

baseline = HEAD `52de433`。行番号は baseline 基準。wire schema は原則不変。

## 事前確認済みの事実

- F1: `session.rs:386` `remove: manual_kill` → 自然終了セッションは `Exited` のままマップに残る。
- F2: orchestrator 側に pane 上限のテストは存在しない（`team_presets.rs:347` のみ）。
- F3: `WorkflowRegistry` は `get`/`list`/`put` のみ。evict は commands.rs/lib.rs/queen.rs にも無い。doc の「last 100 runs」は虚偽。
- F4: mailbox 名は MCP schema に露出していない。`InboxMessage.sender` として payload に出るのみ。文書記載は `CONTRACT.md:1819` / `queen.rs:864` / `docs/guide/ptygrid-yml-guide.md:62`。
- F5: `queen_store.rs:21 MAX_MAILBOX_BYTES = 128`。
- F6: `advance_all` / `spawn_ready` はテストから直接呼ばれない。`advance_run` は 5 本のテストが直接呼ぶ。
- F7: `list_running_workflow_runs` は `WHERE state='running'` 固定。

---

## B-1 / B-2: 軽量セッション API（最初に入れる）

`session.rs`（`list_sessions` と `resource_roots` の間）に追加:

```rust
#[derive(Debug, Clone)]
pub(crate) struct SessionStateInfo {
    pub id: u32,
    pub name: Option<String>,
    pub state: SessionState,
    pub code: Option<i32>,
}

pub(crate) fn session_states(&self) -> Vec<SessionStateInfo>   // id 昇順、ロック 1 回、ps fork なし
pub(crate) fn live_session_count(&self) -> usize               // state != Exited の数、Vec を作らない
```

- `list_sessions_with` とは**共通化しない**（cmd/worktree/teammate/pid の clone を避けるのが目的）。代わりに一致をテストで固定する。
- `name` は含める（`live_session_id` が名前で引くため）。`cmd`/`worktree`/`teammate`/`kind`/`foreground` は含めない。

置換対象（grep で全ヒットを確認すること）:

| 箇所 | 置換 |
|---|---|
| `orchestrator.rs:233 live_session_id` | `session_states()` |
| `orchestrator.rs:286` 上限判定 | A-2 で削除 |
| `orchestrator.rs:1759 detect_completions` | `session_states()`（本命） |
| `team_presets.rs:78 live_session_id` | `session_states()` |
| `team_presets.rs:139` 上限判定 | `live_session_count()` |
| `queen.rs:723 list_agents` | `session_states()` |
| `commands.rs:276` | **変更しない**（frontend が `foreground` を消費） |

新規テスト: `session_states_agrees_with_list_sessions`（id 集合 / state / code / name の一致を実際に突き合わせる）、`live_session_count_excludes_exited_slots`。

イベント駆動化は今回スコープ外。ただし `advance_all` を sleep タイミングに依存させない（冪等のまま保つ）。

---

## B-4: WorkflowRegistry の evict と tick の shallow 化

```rust
pub fn active_run_ids(&self) -> Vec<WorkflowRunId>   // 非終端 run の id だけ。String clone のみ
const REGISTRY_TERMINAL_CAP: usize = 100;
fn evict_terminal(map: &mut HashMap<..>)             // put() 内・同一ロックで呼ぶ
```

- `advance_all` は `registry.list()` をやめて `active_run_ids()` を回す（終端 run の deep clone を全廃）。
- evict は `put` に置く（registry を変更する唯一の choke point）。**evict 内で emit / persist を呼んではならない**（ロック規律）。
- evict の順序キーは `ended_at_ms.unwrap_or(started_at_ms)` 降順。`spawn_workflow` は全 root 失敗時に `ended_at_ms: None` の終端 run を作るため、`unwrap()` や `unwrap_or(0)` は不可。
- 非終端 run は決して evict しない。
- doc comment（`orchestrator.rs:162`）を実装に合わせて訂正。

既存 API 利用者の影響: `list_workflow_runs` は「終端 100 + 全 live」に上限化（frontend はイベント累積なので実害はリロード後の seed のみ）。`join_workflow` / `cancel_workflow` は evict 済み run に対して `not found`（200ms 以内に 100 本終端しない限り起きない）。`resume_workflow` / `abandon_workflow` は影響なし。

新規テスト: `active_run_ids_skips_terminal_runs`、`registry_evicts_the_oldest_terminal_runs_beyond_the_cap`、`registry_never_evicts_a_non_terminal_run`、`registry_evicts_by_ended_at_and_falls_back_to_started_at`。

---

## A: pane 上限を「失敗」から「待ち」へ

### A-0 表現方法（決定済み）

新 `StepState` は作らない。`WorkflowState::Pending` も使わない（F7 により resume 提示から消えるため）。
**`StepOutcome` に `#[serde(skip)] deferred_since_ms: Option<u64>` を足し、step は `Pending` のまま据え置く**（`next_retry_at_ms` と同型の既存パターン）。wire 不変。`ready_steps` が毎 tick 冪等に再導出するので「次 tick で再試行」は追加機構ゼロで成立し、`all_terminal` が `Pending` を非終端として扱うので run は `Running` のまま＝fail-fast カスケードが止まる。

`StepOutcome` のリテラルは 6 箇所（`:273, :287(削除), :304, :316, :505, :720`）＋テストヘルパ `:2677 mk_outcome`。

### A-2 `spawn_step` から上限判定を剥がす

`orchestrator.rs:286-297` の上限分岐を削除し、容量判断を呼び出し側（スケジューラ）に移す。
**`spawn_step` の呼び出し元 4 箇所すべてにゲートを入れること**: `spawn_workflow` root ループ（`:538`）/ `spawn_ready`（`:2037`）/ `respawn_fresh`（`:2119`）/ `fire_due_retries` の 2 分岐。1 箇所でも漏れると上限が無効化される。

### A-3 `spawn_ready` の容量予算

```
tick 冒頭で session_states() を 1 回取得 → live = state != Exited の数
budget = WORKFLOW_SESSION_CAP - live
step ごとに:
  copies  = copies_for(pattern, step)
  reusing = copies == 1 && reuse_existing && live_session_id(..).is_some()
  needed  = if reusing { 0 } else { copies }
  if needed > budget { deferral; continue; }   // ★ 部分 spawn は絶対にしない
  budget -= needed;
```

- **fan-out の all-or-nothing が不変条件**。空き < copies なら 0 copy。`cancel_stragglers`（`:2337` doc）と `dep_unsatisfiable`（`:1355` doc）の両方がこれを前提にしている。
- `run.steps.retain(...)`（`:2029`）を**容量判定より後ろに移す**。先に消すと deferral 時に Pending プレースホルダが消滅し、run が永久 Running になる。
- `changed` は deferral のみの tick では false（無駄な emit/persist を避ける）。ただし `deferred_since_ms` を初めて立てた tick と error 文字列が変わった tick は true。
- `ready_steps` / `condition_targets` / `failfast_targets` / `all_terminal` / `finalize_state` / `cancel_workflow` は**変更不要**。

### A-4 root と retry のゲート

- root ループ: 入らない root は `spawn_step` を呼ばず、非 root と同形の Pending プレースホルダ（fan-out でも **bare-id 1 本**）を push し `deferred_since_ms = Some(now)`。`ready_steps` は依存なし step を必ず返すので次 tick で拾われる。
- `fire_due_retries`: 容量不足なら `next_retry_at_ms = now + DRIVER_TICK_MS` にして `continue`。**`attempts` を増やさないこと**（増やすと待ち時間で `retry.max` を食い潰し恒久 Failed になる）。in-place `restart_session` 分岐（`:2240`）はゲート不要（純増ゼロ）。

### A-5 有界な待ち（デッドロック安全弁）

```rust
pub const WORKFLOW_DEFER_MAX_MS: u64 = 300_000; // 5 分
```
`deferred_since_ms` からの経過がこれを超えたら従来どおり `Failed`（error は待機時間入り、`attempts = 1` にして `arm_retry_backoff` の `attempts==0` ガードを通す）。
「全 step Pending かつ live 0」というデッドロック述語は採らない（live 0 なら budget 9 で必ず spawn できるので成立しない）。実際の wedge は run 外のペインが枠を占める外部要因なので、**有界待ちで必ず終端する保証**に置き換える。
診断用に `run_is_stalled(run) = どの step もペインを持たず、deferred な step がある` を error 文字列に添える（状態遷移はしない）。

### A-6 待機理由の可視化

deferral 時に `outcome.error = Some(format!("waiting for a free pane slot ({live}/{CAP} occupied)"))`、spawn 成功時に `None` へクリア。`error` は既存の wire フィールドなので schema 変更なし。

### A-7 live カウントの統一 / Exited の扱い

上限判定を全面的に `state != Exited` に統一する。**Exited ペインは自動 reap しない**（`EofAction::Exit { remove: manual_kill }` は意図的設計で、frontend の終了コード表示と `restart_session` の復帰を支えている。`handle_eof` の 3 フェーズとレースするため外部からの割り込みも危険）。カウント側を `live_session_id` に合わせることで 2 箇所の不一致が解消する。

**2026-07-30 追記: 実機検証で上記の判断を反転した。** 8 面埋まった状態で `smoke` の step `a`(t1) が 9 枚目を占有し、`close_on_exit` 未指定のため自然終了後も `Exited` のままセルを占有し続けた → 次 step の判定は「`state != Exited` の数」で live=8 と見て空きありと誤認し spawn → frontend は `ui.panes.length`（グリッドの全セル数）でしか描画できず「ペイン上限のため表示できません」を出し、セッションは headless のまま走った(詳細は plan.md §6.6)。**占有判定を `occupied_pane_count()`（全 state、`Exited` 含む＝グリッドの全セル数）へ変更し、`live_session_count()` は削除した**。本節冒頭の「Exited を自動 reap しない」判断は維持している。reuse 判定 `live_session_id`（`state != Exited`）も無変更。CONTRACT.md 続報10 に訂正+追記あり。

### A-8 `timeout_ms` の意味論（決定）

**待機中は進まない**。`check_timeouts` は `Running` かつ `started_at_ms != 0` のみを見る現行のままで既に正しい（変更ゼロ）。理由: `timeout_ms` はエージェントの実行時間であり、キュー待ちを含めると grid 占有状況で成否が変わり再現性を失う。待ちの有界性は `WORKFLOW_DEFER_MAX_MS` が別に持つ。doc comment に明記する。

### A-9 `team_presets.rs:139`（決定）

**意味論は変更しない**（`spec-team-presets.md §4.3` の部分起動は意図的設計、`start_team` は同期 1 ショットで再試行する driver がいない、既存テスト `pane_cap_yields_partial_launch_with_explicit_failures` が仕様を固定）。ただし live カウント是正（`live_session_count()` へ差し替え）は行う＝1 行。既存テストは `/bin/cat` 8 本＝全 Running なので通る。

### A 新規テスト

`spawn_step_no_longer_fails_on_capacity` / `spawn_ready_defers_a_step_that_does_not_fit_and_keeps_it_pending` / `spawn_ready_never_partially_spawns_a_fanout_step` / `spawn_ready_spawns_the_deferred_step_once_a_slot_frees` / `advance_run_keeps_a_run_running_while_a_step_waits_for_a_pane` / `spawn_workflow_leaves_a_root_pending_when_the_grid_is_full` / `fire_due_retries_postpones_without_burning_the_retry_budget_when_the_grid_is_full` / `spawn_ready_fails_a_step_that_has_waited_past_the_defer_budget` / `check_timeouts_never_fires_on_a_step_that_is_waiting_for_a_pane` / `a_deferred_step_reports_why_it_is_waiting`。

---

## C-1: mailbox 単位の watch チャネル

```rust
type MailboxKey = (String, String);   // (project_id, mailbox)

pub struct QueenStore {
    connection: Mutex<Connection>,
    inbox_generation: watch::Sender<u64>,                        // 全体。残す（既存テスト 3 箇所が読む）
    mailbox_waiters: Mutex<HashMap<MailboxKey, Arc<watch::Sender<u64>>>>,
}
```

- `notify_inbox(&self, project, mailbox)`: 全体チャネルを回し、**存在するエントリだけ**に通知する（エントリを作らない）。`send_inbox` は `recipient`、`reply_inbox` は `reply.recipient` を渡す。
- `await_inbox`: **最初の `list_inbox` より前に購読する**（唯一の lost-wakeup 防止機構。ループ内に移動させない）。以降のループは 1 行も変えない。
- GC: `MailboxSubscription` の `Drop` で (a) 自分の Receiver を先に drop → (b) `mailbox_waiters` をロック → (c) `Arc::ptr_eq` で同一性確認 かつ `receiver_count()==0` なら remove。3 条件すべて必要。
- **ロック順序の規約**: `connection` → `mailbox_waiters` の一方向のみ。`send_inbox`/`reply_inbox` は connection guard 保持中に `notify_inbox` を呼ぶため、逆順が 1 箇所でもあればデッドロックする。`subscribe` も `Drop` も connection を触らない。
- `ack_inbox` は現行どおり notify しない。

新規テスト: `await_inbox_is_not_woken_by_traffic_on_another_mailbox`（本体）/ `mailbox_watch_entry_is_dropped_when_the_last_waiter_leaves`（`#[cfg(test)]` の `mailbox_waiter_count()` を追加）/ `two_waiters_on_one_mailbox_are_both_woken`。

---

## C-2: workflow_mailbox に run_id を含める

```rust
fn workflow_mailbox(workflow_name: &str, run_id: &str) -> String {
    format!("queen:workflow/{workflow_name}/{run_id}")
}
```

配線: `deliver_kickoff`(`:351`) に `run_id` 追加 / `detect_reply_completions`(`:1855`) は **`workflow_name` 引数を削除**して冒頭で `let mailbox = workflow_mailbox(&run.name, &run.run_id);` を作る / `spawn_ready`・`respawn_fresh`・`fire_due_retries` に `run_id: &str` 追加（既に `#[allow(clippy::too_many_arguments)]` あり）/ `advance_run` で `run.run_id.clone()` を hoist。

**新規制約**: mailbox 長 = 44 + len(name) ≤ 128 → workflow 名は 84 バイト以下。`config.rs:696 validate_workflows` に load 時検査を追加し、実行時の不可解な失敗を前倒しする。テスト `workflow_name_longer_than_the_mailbox_budget_is_rejected_at_load`。

**旧名フォールバックは実装しない**: 旧 mailbox の thread が残る唯一の経路は跨アップグレードの in-flight run だが、resume すると `Running` step は全部 `Pending` に戻り新 mailbox へ再 kickoff される（`kickoff_root_msg_id` は `#[serde(skip)]` で永続化もされない）。旧 mailbox の残メッセージは別 recipient なので新スキャンに影響しない（`MAX_MESSAGES_PER_PROJECT` 枠を食うだけ）。

**必ず書き換える文言**: `queen.rs:864` の "concurrent runs of the SAME workflow name share one inbox mailbox..." は事実と反するので撤回する。

`INBOX_REPLY_SCAN_LIMIT`(200) と per-thread アンカリングは**変更しない**（run 内の未 ack バックログ対策として引き続き必要）。doc comment のみ更新。

修正が必要な既存テスト（5 本）: `deliver_kickoff_returns_the_thread_root_and_noops_without_a_body`(`:4202`) / `detect_reply_completions_completes_a_reply_joined_step`(`:4229`) / `..._ignores_a_reply_from_anyone_but_the_step_agent`(`:4281`) / `..._records_the_body_without_completing_a_non_reply_join`(`:4308`) / `..._joins_every_reply_on_the_thread`(`:4338`)。
**`deliver_kickoff` に渡す run_id と `mk_run`(`:4082`) の `run_id: "r1"` を必ず一致させること**（不一致だと mailbox が食い違い、テストは panic せず静かに誤った assert を通す）。`const TEST_RUN_ID: &str = "r1";` を置いて両方から参照する。

---

## スコープ外（理由つき）

- コネクションプール化 / `spawn_blocking` 化 / write coalescing: C-1 で waiter 数が落ちた後に再測定すべき。`QueenStore` は `State<'_, QueenStore>` として借用で配られており `'static` 化が全モジュールに波及する。
- driver のイベント駆動化。
- ANSI 増分パース / リングバッファ（提案 2）。
- run 終端時の mailbox GC（C-2 により構造的に可能になったが別 PR）。

---

## 実装順序

1. B-1/B-2（土台。単体で性能効果あり）
2. B-4（独立）
3. A（1 に依存。最も慎重に）
4. C-1（独立）
5. C-2/C-3（4 の後）
6. CONTRACT.md 続報10 + docs 更新

各段階で `cargo test` と `cargo clippy --all-targets --all-features -- -D warnings` を green に保つこと。

## レビュー重点

1. 容量ゲートの網羅性（`grep -n "spawn_step(" orchestrator.rs` の全ヒット）
2. fan-out の all-or-nothing（空き < copies で 0 copy）
3. `retain` が容量判定の後ろにあるか
4. `fire_due_retries` で `attempts` が増えていないか
5. `deferred_since_ms` が spawn 成功 / Skipped / Cancelled でクリアされるか
6. `session_states` と `list_sessions` の乖離をテストが実際に突き合わせているか
7. `put` 内 evict が emit / persist を呼んでいないか
8. evict キーの `unwrap_or(started_at_ms)` フォールバック
9. `await_inbox` の購読が最初の query より前か
10. watch GC の 3 条件（Receiver 先 drop / ロック下判定 / `Arc::ptr_eq`）
11. ロック順序（`mailbox_waiters` 保持中に connection を取らない）
12. テストの run_id 一致（`TEST_RUN_ID`）
13. `queen.rs:864` の description 書き換え
14. clippy `-D warnings`
