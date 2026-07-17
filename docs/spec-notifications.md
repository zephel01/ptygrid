# ptygrid 仕様: アウトオブアプリ通知（OS / チャットWebHook）

作成日: 2026-07-17 / 状態: 実装済み（Phase 4.4.2）/ 対象: セッション終了・エージェント状態変化の外部通知

関連: [spec-agent-status.md](spec-agent-status.md)（通知イベントの供給源: blocked/done 検出）/
[design.md](design.md)（アーキテクチャ原則）/ [competitive-landscape.md](competitive-landscape.md)
（「通知リング / 要承認ハイライト」バックログ）/ [plan.md](plan.md)（バージョニング）/
[../CONTRACT.md](../CONTRACT.md)（IPC/MCP 契約）/
[../ptygrid.example.yml](../ptygrid.example.yml)（注釈付き設定例）。

実装: [../src-tauri/src/notifications.rs](../src-tauri/src/notifications.rs)（本体）/
[../src-tauri/src/config.rs](../src-tauri/src/config.rs)（`notifications:` スキーマ）/
配線元 [../src-tauri/src/agent_status.rs](../src-tauri/src/agent_status.rs)・
[../src-tauri/src/session.rs](../src-tauri/src/session.rs)。

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

通知は**新しい状態レイヤを足さない**。既存の2つの**エッジ**をそのまま外部へ中継するだけである。

- **プロセス生死**（`SessionState`、[../CONTRACT.md](../CONTRACT.md) Phase 1）の `exited` 遷移。
- **意味的状態**（`AgentStatus`、[spec-agent-status.md](spec-agent-status.md)）の `blocked` / `done`
  への変化。

どちらも既に「変化した瞬間」だけ発生するイベントなので、通知レイヤはポーリングも重複除去も
行わない（6.1）。

---

## 2. モデル: イベント × レベル

### 2.1 イベント種類（`NotifyEvent`）

| イベント | 重大度 | 供給源 | 意味 |
|---|---|---|---|
| `error` | 最高 | `session::handle_eof` の終了で exit code が 0 以外 / 不明（シグナル・reap 失敗） | 不正終了・クラッシュ |
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

判定は純関数 `should_send(level, event)`（[notifications.rs](../src-tauri/src/notifications.rs)）で、
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
  [config.rs](../src-tauri/src/config.rs) `expand_vars`）。設定ファイルに秘密情報を直書きしなくてよい。
- 前方互換: 未知のキーは無視（他の 4.x ブロックと同様）。`type` と `level` だけは閉じた列挙で、
  綴り間違いは明確な serde エラーになる。

### 4.2 有効化とリロード

`load_config` のたびに `notifications::apply` が現在のブロックを managed state に差し替える
（[commands.rs](../src-tauri/src/commands.rs)）。ファイル監視によるリロードでも即時反映され、
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
