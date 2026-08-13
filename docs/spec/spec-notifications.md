# ptygrid 仕様: アウトオブアプリ通知（OS / チャットWebHook）

作成日: 2026-07-17 / 状態: 実装済み（Phase 4.4.2）/ 対象: セッション終了・エージェント状態変化・workflow の retry 枯渇の外部通知
（2026-08-13、Stage A-4 で 3 つ目のイベント源「workflow escalation」を追加。→ §2.1 / §5.3 /
[../../CONTRACT.md](../../CONTRACT.md) 続報16）

関連: [spec-agent-status.md](spec-agent-status.md)（通知イベントの供給源: blocked/done 検出）/
[design.md](../design/design.md)（アーキテクチャ原則）/ [competitive-landscape.md](../design/competitive-landscape.md)
（「通知リング / 要承認ハイライト」バックログ）/ [plan.md](../design/plan.md)（バージョニング）/
[../CONTRACT.md](../../CONTRACT.md)（IPC/MCP 契約）/
[../ptygrid.example.yml](../../ptygrid.example.yml)（注釈付き設定例）。

実装: [../src-tauri/src/notifications.rs](../../src-tauri/src/notifications.rs)（本体）/
[../src-tauri/src/config.rs](../../src-tauri/src/config.rs)（`notifications:` スキーマ）/
配線元 [../src-tauri/src/agent_status.rs](../../src-tauri/src/agent_status.rs)・
[../src-tauri/src/session.rs](../../src-tauri/src/session.rs)・
[../src-tauri/src/orchestrator.rs](../../src-tauri/src/orchestrator.rs)（retry 枯渇、Stage A-4）。

---

## 1. 目的と背景

ptygrid は複数の AI CLI を PTY ペインで並行実行する。Phase 4.4.0 の意味的状態検出
（[spec-agent-status.md](spec-agent-status.md)）で「動作中／承認待ち／完了」が**画面内で**
色分け表示されるようになったが、これは **ptygrid のウィンドウを見ている間**にしか役に立たない。
長時間タスクを走らせて席を外す・別アプリで作業する・スマホしか手元にない、という状況では、
「エラーで落ちた」「承認待ちで止まっている」を取りこぼす。

本仕様は、その取りこぼしを**アプリの外**（デスクトップ OS 通知、および Slack / Mattermost /
Discord / Telegram のチャット）へ**エッジトリガの通知**として届ける。設計上の要点は次の2つ。

- **全部は要らない。** 「止まったときだけ」「不正終了だけ」といった粒度を、ユーザーが選べること。
  通知過多はオオカミ少年化し、結局みんな通知を切る。
- **エラーは握り潰さない。** 既定は無音ではなく `critical`（エラーのみ）に寄せ、`silent` は明示選択に
  する。

### 意味的状態・プロセス生死との関係

通知は**新しい状態レイヤを足さない**。既存の**エッジ**をそのまま外部へ中継するだけである。

- **プロセス生死**（`SessionState`、[../CONTRACT.md](../../CONTRACT.md) Phase 1）の `exited` 遷移。
- **意味的状態**（`AgentStatus`、[spec-agent-status.md](spec-agent-status.md)）の `blocked` / `done`
  への変化。
- **workflow step の retry 枯渇**（`StepOutcome`、Stage A-4）— 予算を使い切った瞬間。

前の2つは既に「変化した瞬間」だけ発生するイベントである。3つ目だけは供給側の事情が違い、
workflow driver は 200ms の**ポーリング**で、「この step は retry を使い切った」は以後の全 tick で
真であり続ける**レベル**である。**エッジ化は orchestrator 側の責務**とし、`StepOutcome::escalated`
（`#[serde(skip)]`）を立てた tick の分だけを通知レイヤへ渡す（5.3）。したがって
**通知レイヤはポーリングも重複除去も行わない**という 6.1 の前提は 3 源とも維持される。

---

## 2. モデル: イベント × レベル

### 2.1 イベント種類（`NotifyEvent`）

| イベント | 重大度 | 供給源 | 意味 |
|---|---|---|---|
| `error` | 最高 | `session::handle_eof` の終了で exit code が 0 以外 / 不明（シグナル・reap 失敗）／`orchestrator::take_escalations`（workflow step の retry 枯渇、Stage A-4） | 不正終了・クラッシュ／自動復旧の手が尽きた失敗 |
| `needs-attention` | 高 | `agent_status` が `blocked` へ変化 | 承認 / 入力 / 権限プロンプトで停止（人待ち） |
| `complete` | 中 | 終了で exit code が 0、または `agent_status` が `done` へ変化 | 正常完了 |
| `progress` | 低 | （予約。現在どの供給源も発火しない） | 途中経過。`all` のみ受信 |

`progress` はマトリクスを網羅させるために型としては存在するが、v1 ではイベント源を持たない
（将来 6.3）。

### 2.2 レベルプリセット（`NotifyLevel`）

チャネルが購読する**イベントの束**。設計マトリクスそのまま。

| レベル | error | needs-attention | complete | progress | 想定 |
|---|:---:|:---:|:---:|:---:|---|
| `silent` | — | — | — | — | 通知なし（明示選択） |
| `critical` | ● | — | — | — | 「壊れたときだけ」。**既定** |
| `needs-attention` | ● | ● | — | — | 「止まってたら教えて」 |
| `all` | ● | ● | ● | ● | 全部（短いタスクを回す・監視したい） |

判定は純関数 `should_send(level, event)`（[notifications.rs](../../src-tauri/src/notifications.rs)）で、
上表と1対1に対応する。

---

## 3. チャネル

| `type` | 送信方式 | ペイロード | 必須フィールド |
|---|---|---|---|
| `os` | tauri-plugin-notification（ローカルデスクトップトースト） | title + body | なし |
| `slack` | incoming webhook（HTTP POST） | `{"text": "<title>\n<body>"}` | `webhook` |
| `mattermost` | incoming webhook（Slack 互換） | `{"text": ...}` | `webhook` |
| `discord` | webhook（HTTP POST） | `{"content": ...}` | `webhook` |
| `telegram` | Bot API `sendMessage` | `{"chat_id": ..., "text": ...}` | `bot_token` + `chat_id` |

- Slack と Mattermost は incoming-webhook のペイロード形状が同一なので**同じ送信経路**を共有する。
- `os` は「席にいる間」向け。離席中は無力だが配布・権限以外のコストがなく、在席時は全粒度（`all`）で
  受けたい、という使い分けに向く。
- 必須フィールド欠落・`${VAR}` 展開後の空文字は、その**チャネルだけスキップ**して警告ログを出す。
  設定全体のロードは失敗させない（4.3）。

### 3.1 メッセージ整形

- タイトル: `<絵文字> <who> <動詞>`。`who` は定義名（例 `codex`）、無ければ `#<id>`。
  `project` が読み込まれていれば `[project]` を前置。
  例: `[my-app] ⛔ codex exited abnormally`
- 本文: イベント固有の `detail`（終了なら `exit code 2` 等、blocked なら一致ルール）優先。無ければ
  セッション名を含む定型文。**本文は常に非空**（空文字を拒否する送信先があるため）。
- 絵文字は `error=⛔ / needs-attention=⏳ / complete=✅ / progress=…`（画面内の状態語彙に合わせる）。

**workflow escalation だけは session ではなく step を名乗る**（Stage A-4）。`NotifyContext` に任意
フィールド `origin`（`WorkflowOrigin { workflow, run_id, step_id, attempts }`）が載っているときだけ
整形が分岐する。

- タイトル: `[project] ⛔ <workflow>/<step_id> exhausted its retries`
- 本文: `Workflow '<workflow>' (run <run_id>): step '<step_id>' failed after <n> attempts and has no
  retry budget left. Agent: <agent>. Last error: <error> No further automatic retry will happen.`
  （`detail` は**置き換えではなく末尾に足す**。この 1 通だけで動けるよう、workflow / run / step /
  試行回数を必ず名乗る。）

`origin` を持たない既存 2 源の出力は**バイト単位で不変**である。`session_id: u32` は必須のままで、
枯渇した step は**ペインを持たないことがある**（spawn できずに枯渇した / `check_timeouts` が
kill 済み）ため、`origin` 側は session id を名前に使わない。

---

## 4. 設定（`ptygrid.yml`）

### 4.1 スキーマ

```yaml
notifications:
  enabled: true            # 既定 false（opt-in）。false / 未設定は無送信
  level: critical          # 全チャネル共通の既定プリセット（既定 critical）
  channels:
    - type: os
      level: all            # チャネル個別のレベル上書き（省略時は上の level）
    - type: slack
      webhook: "${SLACK_WEBHOOK_URL}"
    - type: telegram
      bot_token: "${TELEGRAM_BOT_TOKEN}"
      chat_id: "123456789"
      level: needs-attention
      label: mobile         # 装飾ラベル（複数チャネルの区別用、任意）
```

- `enabled` 既定 **false**。opt-in。
- `level` 既定 **critical**。`silent | critical | needs-attention | all`（kebab）。
- チャネルの `level` は**そのチャネルの購読閾値**で、省略時はトップの `level` にフォールバックする。
  これにより「共有 Slack は critical で静かに、手元 Telegram は needs-attention で細かく」が成立する。
- `webhook` / `bot_token` / `chat_id` は**verbatim 保存**し、送信時に `${VAR}` 展開する（`env` 値と同じ扱い、
  [config.rs](../../src-tauri/src/config.rs) `expand_vars`）。設定ファイルに秘密情報を直書きしなくてよい。
- 前方互換: 未知のキーは無視（他の 4.x ブロックと同様）。`type` と `level` だけは閉じた列挙で、
  綴り間違いは明確な serde エラーになる。

### 4.2 有効化とリロード

`load_config` のたびに `notifications::apply` が現在のブロックを managed state に差し替える
（[commands.rs](../../src-tauri/src/commands.rs)）。ファイル監視によるリロードでも即時反映され、
`enabled: false` / ブロック削除で state はクリアされる（再度有効化するまで無送信）。

### 4.3 バリデーション方針

チャネルのフィールド検証は**パース時ではなく送信時**。半端に埋まったチャネル（例: `webhook` 欠落の
`slack`）があっても config ロードは通り、そのチャネルは送信時にスキップされる。1つの設定ミスで
通知全体が死なないための方針。

---

## 5. イベント源と配線

### 5.1 セッション終了 → error / complete

`session::handle_eof` の `EofOutcome::Exited(info, code)` 分岐で、既存の `pty-exit` /
`session-state` emit の直後に `notifications::dispatch` を呼ぶ。`event_for_exit(code)` が
`Some(0) => complete`、それ以外 → `error`。

- **autorestart 中の途中クラッシュ（`EofOutcome::Restarting`）は通知しない。** 最終的に打ち切られた
  終了（`Exited`）だけが通知される。再起動ループで通知が連発するのを防ぐため。5回打ち切り後の
  最後の `error` が「意味のある1通」になる。

### 5.2 agent-status 変化 → needs-attention / complete

`agent_status::evaluate_tick` で状態が**変化**したとき（`Tracker::observe` が `Some` を返すとき）、
`emit` の直前に `notify_event_for(status)` で `blocked => needs-attention` / `done => complete` を判定し、
発火があれば `dispatch` する。`working` / `idle` / `unknown` は通知しない。

- `blocked` / `done` はいずれも `evaluate_tick`（実出力の分類）経路で発生する。linger 減衰
  （`done`→`idle`）経路は `idle` しか emit しないので通知しない。
- 通知の名前・detail は分類時の `snapshot.name` と一致ルール（`matched`）を使う。

### 5.3 workflow step の retry 枯渇 → error（Stage A-4、2026-08-13）

`advance_run` の末尾（2 本の `arm_retry_backoff` パスと `cancel_stragglers` の**後**）で
`take_escalations`（純関数）が枯渇 step を集め、`registry.put` / `persist_run` /
`emit_workflow_state` の**後**に `notify_escalation` が `dispatch_ctx` を呼ぶ。I/O は最後に置く。

- **判定条件（3 つとも満たすときだけ）**: `Failed` であり、`next_retry_at_ms` が `None`（backoff 待ち
  ではない）であり、`attempts > 0` であり、宣言された `retry:` に対して
  `retry::allows_another(attempts, policy)` が偽。`retry:` を宣言していない step、
  `attempts == 0`（一度も spawn されていない＝評価不能な `condition:` の `Failed`）は**対象外**。
  これは `arm_retry_backoff` が再 spawn を拒否する条件と同じ 3 つである。
- **重複抑止**: `StepOutcome::escalated`（`#[serde(skip)]`）を立てた tick でだけ返す。
  200ms tick 上のレベルをエッジに変えるのはこの層の責務（§1「意味的状態・プロセス生死との関係」）。
  永続化しないので、**resume した run では同じ枯渇がもう 1 通出る**（1 通目を受け取れなかった
  人にこそ必要、という判断）。
- **イベントは `error`**。`needs-attention` にすると、既定の `level: critical` のままの利用者には
  1 通も届かない（§2.2 のマトリクス）。escalation が最も要るのは「誰も画面を見ていない」設定で
  あり、それは既定の設定でもある。
- **config には何も足さない**。宛先と音量は既存の `notifications:` ブロックが決める
  （ptygrid-yml-guide.md §1 の escalation 行は「(config には書かない)」のまま）。
- **ペイン exit 由来の通知は別に出る（抑止しない）**。最後の試行がペインを持っていれば
  5.1 の `error` も出る。前者は「プロセスが落ちた」、後者は「この run のこの step はもう自動では
  戻らない」で、workflow / run / step を名乗るのは後者だけ。抑止するには通知層が session と
  workflow の対応を横断で知る必要があり、6.1 の「重複除去をしない」前提を壊す。
- **run 全体の失敗は通知しない。** 入口は step の retry 枯渇 1 つだけである。

---

## 6. 送信の実装

### 6.1 ホットパスを塞がない

`dispatch` は managed state を**短時間ロックしてスナップショットをクローン**し、ロックを離してから
送信する。したがって PTY リーダースレッドや agent-status 非同期タスクの上で**ロックを保持したまま
I/O しない**。

- OS 通知はインラインで `show()`（ローカル呼び出し、ブロックしない）。
- チャットの webhook は**デタッチした `std::thread` 上で** ureq（同期・全体10秒タイムアウト）で送信する。
  ブロッキング I/O が非同期ランタイムにも PTY リーダーにも波及しない。fire-and-forget。

### 6.2 失敗はログのみ

OS 通知の `show()` 失敗、webhook POST の失敗は `eprintln!` で記録するだけで、呼び出し側へ伝播しない。
通知はベストエフォートであり、送れなかったからといってセッションのライフサイクルを乱さない。

### 6.3 依存

- `tauri-plugin-notification`（v2）— macOS/Linux/Windows のデスクトップトースト。バックエンドから
  `app.notification().builder()…show()` で呼ぶ。
- `ureq`（v2, features `json`, `tls`）— 軽量な同期 HTTP クライアント。lean な依存構成を保つため
  reqwest ではなくこれを採用。デタッチスレッドで使うので同期でも問題ない。

---

## 7. 既定と事故防止

- **opt-in**: `enabled` 既定 false。何も設定しなければ一切送らない。
- **エラー優先**: 既定 `level` は `silent` ではなく `critical`。「全部 OFF にしたつもりが不正終了も
  握り潰していた」を避ける。無音にしたいときは明示的に `silent` を選ぶ。
- **macOS の注意**: OS 通知は**バンドル済み・署名済みアプリ + OS の通知許可**が前提。`tauri dev` の
  素の起動では表示されないことがある。webhook 側はこの影響を受けない。

---

## 8. 契約への影響

- `Config` に任意フィールド `notifications: Option<NotificationsConfig>` が増える。`load_config` が返す
  `ConfigInfo.config` に additive に載る（省略可能・後方互換）。
- **新規 IPC コマンドは無い。** 通知はバックエンド内部の外向き送信であり、フロントエンドの新イベント
  も追加しない。既存の `session-state` / `agent-status` イベントはそのまま。
- Stage A-4 でも wire は変わらない。`StepOutcome::escalated` は `#[serde(skip)]` なので
  `workflow-state` イベントにも `workflow_runs.steps_json` にも出ない（→ CONTRACT.md 続報16）。
- capabilities への追記は不要（プラグインをバックエンドから呼ぶため。webview→プラグインの権限は
  使わない）。

---

## 9. 今後（v2 候補）

- **スロットリング / ダイジェスト**: 同一イベントの連発をまとめる（ループで100回エラー時に100通知を
  防ぐ）。v1 では見送り。config の未知キー無視により、`throttle_ms` 等を後方互換で足せる。
- **`progress` イベント源**: フェーズ移行などを `all` 向けに供給する。
- **再通知抑制**: 同一 blocked が続くときの再送間隔（`renotify_ms`）。

---

## 10. テスト

純ロジックはユニットテストで担保（`cargo test`）。

- ルーティング: `should_send` がマトリクスと一致（silent/critical/needs-attention/all × 4イベント）。
- チャネル選択: 個別 `level` 上書きとグローバル既定のフォールバック、`silent` チャネルの無受信、
  config 順の保持。
- 終了コード写像: `event_for_exit` / `exit_detail`（0=complete、非0/None=error）。
- メッセージ整形: 名前→`#id` フォールバック、`project` 前置、`detail` 優先と非空保証。
- WebHook ペイロード: Slack `text` / Discord `content` / Telegram URL・body の形状。
- config スキーマ: kebab レベル・`type` 列挙のパース、未知キー無視、個別レベル上書き。
- workflow escalation（Stage A-4、5 本）: 枯渇で**ちょうど 1 回**返り以後の tick では返らないこと
  （`an_exhausted_retry_budget_escalates_once_and_never_again`）、予算が残っている / backoff 待ち /
  `attempts == 0` / まだ `Running` では返らないこと
  （`a_retry_that_still_has_budget_left_does_not_escalate`）、`retry:` 未宣言の step は対象外
  （`a_step_with_no_retry_policy_never_escalates_however_hard_it_failed`）、本文が workflow / run /
  step / 試行回数 / agent / 最後のエラーを名乗ること
  （`escalation_names_the_workflow_run_step_and_attempt_count`）、既定の `critical` チャネルに
  届くこと（`escalation_reaches_a_channel_left_at_the_default_critical_level`）。
  **判定と整形はいずれも純関数側で固定しており、`dispatch` は経由しない。**

**実機検証は未実施**（Stage A-4）。OS トースト / Slack に実際に届いたところは見ていない
（→ [plan.md](../design/plan.md) §2 の U18）。
