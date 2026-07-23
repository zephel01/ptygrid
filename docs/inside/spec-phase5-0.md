# ptygrid 仕様: Phase 5.0 — Orchestrated & Remembering（宣言的DAG / 共有メモリ / Local Provider / AI Arena）

作成日: 2026-07-22 / 状態: draft / 対象 Phase: 4.5（Phase 5.5 通信・観測基盤リリース後）

関連: [spec-agent-status.md](../spec-agent-status.md)（意味的状態の下地 / `working|blocked|done|idle` を workflow 状態機械が消費）/
[spec-notifications.md](../spec-notifications.md)（workflow 完了・失敗の外部通知は既存経路にそのまま乗る）/
[spec-team-presets.md](../spec-team-presets.md)（Phase 4.3 の `team_presets` — 本 Phase の workflow はその上位互換）/
[spec-phase5-5.md](spec-phase5-5.md)（MCP RC / OTel / Ring — workflow span と Ring のカラーソースを引き継ぐ）/
[design.md](../design.md)（hot path 分離・config-as-code・推測回避）/
[plan.md](../plan.md)（バージョニング）/ [competitive-landscape.md](../competitive-landscape.md)（worktree系 / 協調系の分岐）/
[../CONTRACT.md](../../CONTRACT.md)（IPC/MCP 契約の追記先）/
[../ptygrid.example.yml](../../ptygrid.example.yml)（注釈付き設定例）。

実装（新規モジュール想定）:
[../src-tauri/src/orchestrator.rs](../../src-tauri/src/orchestrator.rs)（M3 DAG / Supervisor 状態機械）/
[../src-tauri/src/memory.rs](../../src-tauri/src/memory.rs)（M4 semantic memory）/
[../src-tauri/src/memory_embed.rs](../../src-tauri/src/memory_embed.rs)（embedding backend 抽象）/
[../src-tauri/src/provider.rs](../../src-tauri/src/provider.rs)（S1 local provider ヘルスチェック・env 注入）/
[../src-tauri/src/arena.rs](../../src-tauri/src/arena.rs)（S7 Arena backend）/
配線元 [../src-tauri/src/queen.rs](../../src-tauri/src/queen.rs)（新規 MCP tools）/
[../src-tauri/src/queen_store.rs](../../src-tauri/src/queen_store.rs)（memory テーブル追加、sqlite-vec ロード）/
[../src-tauri/src/config.rs](../../src-tauri/src/config.rs)（新スキーマ）/
[../src-tauri/src/session.rs](../../src-tauri/src/session.rs)（env 注入フックの再利用のみ）。
Frontend: [../src/lib/Arena.svelte](../../src/lib/Arena.svelte) /
[../src/lib/WorkflowPanel.svelte](../../src/lib/WorkflowPanel.svelte) /
[../src/lib/MemoryPanel.svelte](../../src/lib/MemoryPanel.svelte) /
[../src/lib/ProviderStatus.svelte](../../src/lib/ProviderStatus.svelte)。

---

## 1. 目的と背景

Phase 4.4 までの ptygrid は「複数の PTY をユーザーの目で監督する道具」だった。手動 spawn、`autostart`、`autorestart`、Phase 4.3 の `team_presets`（一括起動）まではあるが、**「a を終わらせてから b を、b 三並列の結果を c に集約」**のような**依存関係**は表現できず、**「昨日 claude が学んだこと」を今日の codex が知る**手段も無い（既存 pins/notes は短い共有・記録用途、inbox は一過性の messaging）。またクラウド API キー前提の CLI が主で、**フルローカル運用**は各 CLI 側の環境変数を人間が個別に張る必要があった。

本 Phase は「多エージェント CLI をペインで並べて眺める」段階から、**多エージェント CLI に依存関係と長期記憶とローカル基盤を与える段階**へ移行する。四つの機能を一つの Phase として束ねる:

- **M3 宣言的 DAG / Supervisor Orchestration** — `ptygrid.yml` に `workflows:` を書くと、pipeline / fan-out / supervisor / handoff の四パターンが宣言的に動く。既存 `spawn_agent` allowlist を**一切破壊しない**（workflow 内 spawn も allowlist 経由）。
- **M4 共有セマンティックメモリ** — `queen.sqlite3` に sqlite-vec による embedding + FTS5 のハイブリッド検索インデックスを追加し、`memory.remember` / `memory.recall` を 4 スコープ（user / agent / session / project）で提供する。
- **S1 Local-first Provider 統合** — `provider:` キーで `local:ollama:qwen3-coder:30b` のような宣言を書くと、起動時に localhost のヘルスチェックと env 注入が自動化される。M4 の embedding も同じ provider で走る。
- **S7 前半 AI Arena ビュー** — 同一タスクを N エージェントに並列に投げて出力を横並びで見る UI。M3 の fan-out パターンから自動で開く。Diff/Vote まで（Merge は S7 後半 / Phase 6.0 以降）。

**共通の設計原則**:

- **既存 allowlist の非破壊**: workflow 内でも、Arena 起動でも、spawn できるのは `ptygrid.yml` の `agents:` に宣言された名前だけ。
- **既存 spec への薄い上乗せ**: workflow の状態は `agent-status`（Phase 4.4）+ `session-state`（Phase 1）+ `queen-notify` で表現。新しい**下位**イベントは足さない。
- **project 境界**: memory も workflow 実行履歴も、Pins/Notes/Inbox と同じ **canonical config directory** でスコープする。project 未読込時は tool 呼び出しをすべて error にする。
- **hot path 分離**: orchestrator の状態機械・embedding 計算は PTY reader スレッドから物理的に分離する。

---

## 2. モデル

### 2.1 Workflow モデル（M3）

`ptygrid.yml` の宣言に対応する型:

```ts
type WorkflowPattern = "pipeline" | "fan-out" | "supervisor" | "handoff";

type WorkflowStep = {
  id: string;                       // step 一意名(workflow 内スコープ)
  agent: string;                    // agents: の定義名(allowlist)
  dependsOn?: string[];             // 先行 step id
  fanOut?: number;                  // 並列数(fan-out パターンでのみ意味を持つ)
  handoffTo?: string;               // 次 step id(handoff は 1 対 1 の引継ぎ)
  joinOn?: "all" | "any" | number;  // fan-out 集約規則(既定 all)
  condition?: string;               // 前 step の inbox 最終メッセージに対する正規表現
  retry?: { max: number; backoffMs?: number };
  timeoutMs?: number;               // step 上限時間、超えたら FAILED
  kickoff?: string;                 // step 起動直後に inbox 投函する初回メッセージ
};

type WorkflowDef = {
  name: string;
  pattern: WorkflowPattern;
  steps: WorkflowStep[];
  onFailure?: "fail-fast" | "continue"; // 既定 fail-fast
};

type WorkflowState = "PENDING" | "RUNNING" | "SUCCEEDED" | "FAILED" | "CANCELLED";
type StepState     = "PENDING" | "RUNNING" | "SUCCEEDED" | "FAILED" | "SKIPPED" | "CANCELLED";

type WorkflowRun = {
  runId: string;                    // ULID、DB に永続化
  name: string;
  state: WorkflowState;
  startedAtMs: number;
  endedAtMs?: number;
  steps: Array<{ stepId: string; agent: string; sessionId?: number; state: StepState;
                 attempts: number; error?: string }>;
};
```

- Workflow は **1 run = 複数 pane**。各 step は既存の `spawn_agent` を通して PTY を生む。
- 状態はイベント駆動で backend が保持し、`workflow-state`（新）で frontend へ配信する。
- **状態の権威**: step の完了判定は既存の意味的状態 `AgentStatus == done`（[spec-agent-status.md](../spec-agent-status.md)）か、PTY 終了 `SessionState == exited && code == 0` のいずれか。**Phase 4.4 の判定を workflow が消費**する構造で、orchestrator 自身は新しい完了検出ロジックを持たない。

### 2.2 Memory モデル（M4）

```sql
CREATE TABLE memory (
  id            INTEGER PRIMARY KEY,
  scope_kind    TEXT NOT NULL,   -- 'user' | 'agent' | 'session' | 'project'
  scope_key     TEXT NOT NULL,   -- e.g. agent 定義名 / '#12' / project canonical path
  kind          TEXT NOT NULL,   -- 'episodic' | 'semantic' | 'procedural'
  content       TEXT NOT NULL,   -- 平文(最大 32 KiB)
  entities      TEXT,            -- JSON: [{name, type}]
  source_agent  TEXT NOT NULL,   -- 誰が remember したか(definition 名 or '#id')
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  ttl_ms        INTEGER,         -- 任意の自動失効
  revision      INTEGER NOT NULL -- pins/notes と同じ楽観並行制御
);
CREATE VIRTUAL TABLE memory_fts USING fts5(content, entities, content='memory', content_rowid='id');
CREATE VIRTUAL TABLE memory_vec USING vec0(id INTEGER PRIMARY KEY, embedding FLOAT[NNN]);
-- NNN は provider が返す次元。起動時 pragma で保存し、次元不一致は「index 再構築が必要」の
-- 明示 error(黙って壊さない)。
```

- **三分類**: `episodic`（過去の会話・出来事）/ `semantic`（事実 "この repo の主 test コマンドは `pnpm test`"）/ `procedural`（手順 "release は `npm run bump && git tag …`"）。
- **四スコープ**: `user`（マシン全体、~/.ptygrid 側に個別 DB を持つ選択もあるが、v1 は project DB 内に `scope_kind='user'` として保持し、export/import で移送可能とする）/ `agent`（定義名で束ねる）/ `session`（`#id`、app 再起動で失効）/ `project`（既存 pins/notes 同様に canonical dir）。
- **`source_agent` を必須列**にする — Pins/Notes は「誰が書いたか」を持たない設計だったが、複数エージェントが同じ memory に書き込むと出所が追えなくなるため、memory では追跡必須。
- **Pins/Notes/Inbox との違い**:

| 種別 | 用途 | 検索 | ライフサイクル |
|---|---|---|---|
| Pins | 短い pinned 値（キー→値、256件上限） | key 完全一致 | 明示更新まで永続 |
| Notes | 構造化ドキュメント（10,000件上限） | FTS5 部分検索 | 明示更新まで永続 |
| Inbox | エージェント間の追記専用 messaging | mailbox で listing | ack で既読化・永続 |
| **Memory** | **長期 retrievable 知識** | **embedding + FTS5 ハイブリッド + entity** | **TTL / forget / cascade** |

### 2.3 Provider モデル（S1）

```ts
type ProviderRef = string;   // "local:ollama:qwen3-coder:30b" | "cloud:anthropic:claude-opus-4" | ...
type ProviderKind = "local" | "cloud";
type ProviderBackend = "ollama" | "lm-studio" | "jan" | "anthropic" | "openai";

type ProviderStatus = {
  ref: ProviderRef;
  kind: ProviderKind;
  backend: ProviderBackend;
  reachable: boolean;
  endpoint?: string;         // "http://127.0.0.1:11434"
  modelReady?: boolean;      // ollama tags API で見つかったか
  detail?: string;           // "connection refused" 等
};
```

宣言形式: `local:<backend>:<model>[:<param>]`。`local:ollama:qwen3-coder:30b` は Ollama バックエンド、モデル名 `qwen3-coder:30b`。cloud は既存の env 変数運用に整合させ、`cloud:anthropic:opus-4` は「解釈用の宣言」のみで env 注入は行わない（Anthropic 側 SDK が自分で拾う）。

### 2.4 Arena モデル（S7 前半）

```ts
type ArenaLaunch = {
  workflowRunId: string;      // fan-out workflow run と 1:1 対応
  taskId: string;             // ULID
  prompt: string;             // 共通プロンプト
  contenders: Array<{ agent: string; sessionId: number; state: StepState }>;
};

type ArenaVote = {
  taskId: string;
  contenderAgent: string;
  voter: "user" | string;     // 将来は他 agent の投票も許す(v1 は user のみ)
  score: 1 | -1;
  reasonText?: string;
};
```

- Arena は独立した状態層を持たず、**M3 の fan-out workflow run を"横並び"に描画する薄いビュー**。fan-out 完了後、各 contender の最新 `read_output` を並置し、diff・投票を行う。

---

## 3. メカニズム

### 3.1 M3 Orchestrator 状態機械（`orchestrator.rs`）

責務:

- **DAG の topological sort と実行**。`depends_on` を辺として build → 循環検出 → step ごとに `PENDING → RUNNING → (SUCCEEDED|FAILED|SKIPPED|CANCELLED)` を進める。
- 実 spawn は**既存の `ConfigManager::resolve_def` + `PtyManager::spawn_agent`**（Phase 4.3 の team preset と同一経路）。**新しい spawn 経路は作らない**。したがって workflow から起動できるのは allowlist（`ptygrid.yml agents:`）にある名前だけ。
- **step 完了判定は集約的**: 次のいずれかで完了と見なす（設計上 3 経路を並置）。
  1. PTY 終了 `SessionState == exited && code == 0` → SUCCEEDED、`!= 0` → FAILED。
  2. **意味的状態 `AgentStatus == done` が `done_linger_ms` 経過**（Phase 4.4 の減衰後）→ SUCCEEDED。CLI が interactive で自然終了しないケース（Claude Code 等）を拾う。
  3. `kickoff` に対する reply を `inbox` に受けた（`reply_inbox` の対象になった）→ SUCCEEDED。**「返信をもって完了」**の明示宣言のみに使う（step に `join_on: "reply"` を書いたときだけ有効化）。
- **失敗時**: `on_failure: "fail-fast"`（既定）は「次の step を PENDING → CANCELLED にし、走行中 step は kill_pty」、`"continue"` は「独立したブランチは走らせ続ける」。
- **retry**: step 単位、`max` 回まで backoff。retry では同じ session id を再利用する既存 `restart_session` を使い、pane 追加を発生させない。
- **timeout**: `timeoutMs` を超えたら `kill_pty` して FAILED（retry があれば消費）。

**fan-out**:

- `fanOut: 3` は同じ agent を 3 並列 spawn。**同じ `agents:` 定義名の複数 session は既に許容**されている（Queen `spawn_agent` が同名 session を複数生む Phase 2.1 以降の実装）。orchestrator は各並列 step に `<agent>#run-<runId>-<i>` の**論理タグ**を付けて追跡する。
- `join_on: all`（既定）= 全 SUCCEEDED で親を進める / `any` = 最初の1つで進める / `n`（数値）= n 個で進める。**join 後、残る fan-out step は `CANCELLED`** にして即 kill する（`any`/数値時の資源解放）。

**supervisor**:

- pattern `supervisor` は「親 step + 複数子 step」の1階層構造。親は `handoff_to` を持たず、子はすべて親を `depends_on` する。子群の完了規則 = `join_on`。失敗規則 = `onFailure`（fail-fast / all-succeed 相当）。**LangGraph の Supervisor パターン**を薄く倣うが、実体は「fan-out + join」でしかない（新しい概念は増やさない）。

**handoff**:

- pattern `handoff` は「a の inbox に届いた最終メッセージを b の kickoff にする」動線。orchestrator は `reply_inbox` を polling せず、`inbox` の generation `watch`（Phase 3.8 `await` と同じ購読）を再利用する。

**Queen MCP tools 追加**:

| tool | args | returns | 説明 |
|---|---|---|---|
| `spawn_workflow` | `{ name: string, args?: Record<string,string> }` | `{ runId, steps: [{stepId, sessionId?}] }` | ロード済み config の workflow を起動、以後 `workflow-state` イベントで進行 |
| `join` | `{ runId: string, timeoutMs?: number }` | `WorkflowRun` | Phase 3.8 `await` と同じ cursor / cancellation 規律。SUCCEEDED/FAILED/CANCELLED になるまでブロック |
| `cancel_workflow` | `{ runId: string, reason?: string }` | `WorkflowRun` | 走行中 step を kill、以後の step を CANCELLED |

### 3.2 M4 Memory 実装（`memory.rs` + `memory_embed.rs`）

**保存経路**:

1. `remember({scope, kind, content, entities?, ttlMs?, sourceAgent?})` を受ける。
2. `entities` 省略時は簡易固有表現抽出（大文字始まり2連続 / `#\d+` issue風など）を軽くやる（optional）。
3. embedding backend（3.3）で `content` を vector 化。次元不一致は「index reset が必要」を明示 error。
4. **1つの `BEGIN IMMEDIATE` transaction 内で**、`memory`（本体）+ `memory_fts`（FTS5）+ `memory_vec`（sqlite-vec）を三重書き込みし、`revision` を採番。既存 Pins/Notes と同じ楽観並行制御規律を継承する。

**検索経路**（`recall`）:

- 入力: `{scope, query, k?=8, kinds?=[...], sourceAgent?}`。
- ハイブリッドスコア = **Reciprocal Rank Fusion**: `rrf(rank_vec, rank_fts) = 1/(60+rank_vec) + 1/(60+rank_fts)`（各サブスコアで独立に上位 k' 件を取ってランクを付け、RRF で合体）。60 は定数（sqlite-vec の Alex Garcia 例で使われる無難な値）。
- 結果に `hitBy: 'vec' | 'fts' | 'both'` を付ける。`explain` で開発者が調整可能。
- 4 スコープの検索順は**優先順に読む**: `session > agent > project > user`。同一 content は id で dedupe。

**忘却経路**（`forget`）:

- `forget({id, expectedRevision})` — 単一削除。
- `forget({scope, cascade: true})` — スコープ内全 memory を削除。**プライバシー要件**として、対応する `memory_fts` / `memory_vec` の該当行も同 transaction で削除する（vec 側のトリガに頼らない、明示 join delete）。
- **export/import**: `memory.export({scope}) -> JSON`（embedding は base64、平文と分離）と `memory.import(payload)` を用意し、別マシンや別 project へ移送できる。
- **暗号化オプション**: `queen.memory.encrypted: true`（`ptygrid.yml`）で content 列を OS keychain 由来の対称鍵で AES-GCM 暗号化して保存する。**FTS5 は暗号化と両立しないため**、暗号化を有効化するとハイブリッド検索は vec のみに縮退する（明示バナー表示 + userguide 注記）。

**embedding backend 抽象**（`memory_embed.rs`）:

```rust
pub trait EmbeddingBackend: Send + Sync {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String>;
    fn dimension(&self) -> usize;
    fn ident(&self) -> &str; // "ollama:nomic-embed-text" 等、DB pragma に保存する
}
```

- **Ollama**: `POST http://127.0.0.1:11434/api/embed` `{"model": "nomic-embed-text", "input": [...]}` → `{"embeddings": [[...]]}`。
- **LM Studio**: OpenAI 互換、`POST http://127.0.0.1:1234/v1/embeddings`。
- **Claude / OpenAI**: cloud キー経由。ptygrid の関与は「backend 呼び出しの実装だけ」に留め、キー管理は既存の env 展開規約を使う。
- 起動時に **`memory.embed.provider` の ident** を DB pragma として保存し、次回起動時に一致しなければ「index の再構築（`memory.reindex`）が必要」を error として明示する。**backend 切替でサイレントに壊れないための不変条件**。

**Queen MCP tools 追加**:

| tool | args | returns |
|---|---|---|
| `memory.remember` | `{scope, kind, content, entities?, ttlMs?}` | `{ memory }` |
| `memory.recall` | `{scope, query, k?, kinds?, sourceAgent?, explain?}` | `{ hits: MemoryHit[] }` |
| `memory.forget` | `{id?, expectedRevision?, scope?, cascade?}` | `{ deleted: number }` |
| `memory.list` | `{scope, kind?, limit?, cursor?}` | `{ items, nextCursor? }` |
| `memory.reindex` | `{scope?}` | `{ reindexed: number }` |

- **scope 引数**は `{ kind: "user"|"agent"|"session"|"project", key?: string }` の構造化オブジェクト。省略時のデフォルトは呼び出し元 agent（`caller_id` に相当する情報が Queen で分かる場合）+ 現在の project。
- **project 未読込時は全 tool を error**（Pins/Notes と同流儀）。

### 3.3 S1 Provider ヘルスチェックと env 注入（`provider.rs`）

**起動時**:

1. `ptygrid.yml` の `providers:` セクション（4.3 参照）を読む。
2. `local:*` の宣言だけを対象に、backend ごとの既知エンドポイント（Ollama `:11434/api/tags`、LM Studio `:1234/v1/models`、Jan `:1337/v1/models`）を **短い timeout (500ms)** で `GET` する。
3. 結果を `ProviderStatus` に保持し、`ProviderStatus` イベントで frontend に emit する（トースト誘発はしない、バナー表示のみ）。

**PTY spawn 時**:

- agents に `provider: "local:ollama:qwen3-coder:30b"` が宣言されていれば、対応する env を注入する:

| CLI 想定 | 注入 env |
|---|---|
| Claude Code + claude-code-router → llama.cpp/ollama | `ANTHROPIC_BASE_URL=http://127.0.0.1:11434`, モデル名は router 側の task で解決 |
| Codex / 汎用 OpenAI 互換 CLI | `OPENAI_BASE_URL=http://127.0.0.1:1234/v1`, `OPENAI_API_KEY=lm-studio`（多くの互換 CLI がキー空を拒否するため dummy） |
| Ollama 直接叩く CLI | `OLLAMA_HOST=http://127.0.0.1:11434` |

- **既存の agent 定義 `env:` は上書きしない**。ユーザーの明示 env が優先。provider 由来の env はデフォルト補完のみ。
- **provider 到達不能**でも spawn 自体は止めない（ユーザーの手動起動ワークフローを潰さない）。バナーで警告し、ログに残す。

### 3.4 S7 Arena（`arena.rs` + `Arena.svelte`）

**起動経路**:

- workflow 定義が `pattern: fan-out` かつ `arena: true`（4.3 参照）で宣言されているとき、`spawn_workflow` の完了時（もしくは step が RUNNING に到達次第）、frontend が **Arena panel** を開くイベント `arena-open` を受ける。手動起動は toolbar から `spawn_workflow` を打つのと同じ。
- Arena は既存の pane を**壊さず、上に重ねる overlay**（modal ではなく、右側 drawer）。個々の contender は既存 pane にそのまま存在し続ける。

**表示要素**:

- N 列の contender カード。各カードは対応 session の最新 `read_output` の tail（末尾 60 行）を表示（再構成テキスト、既存 `read_output` API を再利用）。
- 各カードに 👍/👎 ボタン → `arena.vote` MCP tool を呼ぶ（memory の一種として `kind='episodic'`, `scope=project` に投票理由を残す）。
- **Diff 表示**: 任意 2 カードを選択すると差分（テキストの LCS ベース）を並置。実装は既存 `Git diff` の diff viewer コンポーネントを再利用可能。
- **Merge は本 Phase では未実装**（S7 後半 = Phase 6.0 以降）。**必ず「選ばれた出力を後続 step に手動で回せる」導線に留める**（Vote 結果は memory に残るので、次の step で `memory.recall` から拾える）。

**Arena と Fan-out の関係**:

- fan-out step 群と Arena は 1:1。`join_on: any` で最初の1つが選ばれると、残りの contender は自動的に CANCELLED（3.1）→ Arena では「棄却」ラベルで残す（消さない、ユーザーが比較を続けたいかもしれない）。

---

## 4. 設定（`ptygrid.yml` スキーマ拡張）

### 4.1 全体像

```yaml
# 既存 project / queen / teammates / notifications / agent_status / agents / processes /
# team_presets ブロックはすべて不変。以下は本 Phase の追加ブロック(すべて任意)。

providers:                        # M3/M4/S1 共通の provider 宣言
  - ref: local:ollama:qwen3-coder:30b
    endpoint: http://127.0.0.1:11434   # 省略時は backend 既知値
  - ref: local:lm-studio:qwen3:4b
  - ref: cloud:anthropic:claude-opus-4
  embedding:                      # 任意。ここで指定した provider を memory.embed に使う
    provider: local:ollama:nomic-embed-text
    dimension: 768                # 明示指定推奨(起動時の DB pragma と一致必須)

agents:
  - name: claude-local
    cmd: "claude"
    provider: local:ollama:qwen3-coder:30b   # ← 新: env 自動注入
    env:
      ANTHROPIC_API_KEY: dummy               # ユーザー env は provider 由来より優先

queen:
  memory:
    enabled: true                 # 既定 true。false で memory tools を disable
    encrypted: false              # 既定 false。true で content を暗号化(FTS 併用不可)
    max_bytes: 32768              # content 上限、既定 32 KiB
    ttl_default_ms: null          # 全 memory の既定 TTL、null で無期限

workflows:                        # M3 の宣言
  review-and-fix:
    pattern: pipeline
    steps:
      - id: review
        agent: claude-local
        kickoff: "src/ を読み、危険箇所を pin してくれ。終わったら reply して。"
        joinOn: reply             # inbox reply で完了と見なす
        timeoutMs: 600000
      - id: fix
        agent: codex
        dependsOn: [review]
        kickoff: "先行の review pin を読み、パッチを作成してほしい。"

  triple-review:
    pattern: fan-out
    arena: true                   # ← Arena UI を自動で開く
    steps:
      - id: candidate
        agent: claude-local
        fanOut: 3
        joinOn: all
        kickoff: "この feature の設計案を1つ書いてくれ。"

  supervisor-example:
    pattern: supervisor
    onFailure: fail-fast
    steps:
      - id: plan
        agent: claude-local
        kickoff: "計画を立てて子タスクへ handoff。"
      - id: worker-a
        agent: codex
        dependsOn: [plan]
      - id: worker-b
        agent: grok
        dependsOn: [plan]
        joinOn: all               # supervisor は plan + workers で all
```

### 4.2 バリデーション

- **workflow 名は非空・一意**。
- **steps は 1 件以上**、`id` は workflow 内で一意。
- **`agent` は `agents:` 定義への参照のみ**（`processes:` は不可、Phase 4.3 team_presets と同流儀）。
- **DAG 検証**: `depends_on` の循環はロード失敗（error）。unknown step id 参照もロード失敗。
- **pattern と field の整合**:
  - `pipeline`: 各 step は最大1つの `dependsOn` を持ち、全体で線形 DAG。
  - `fan-out`: `fanOut >= 2` の step が最低1件。
  - `supervisor`: 1 個の root step（`dependsOn` 空）+ 全ての子が root に `dependsOn`。
  - `handoff`: 全 step が `handoffTo` を持つか、末端でだけ持たない。
- **未知フィールドは無視**（forward compat）。ただし `pattern` / `joinOn` / kind enum は閉じたセット。

### 4.3 内蔵既定と env 注入テーブル

`provider.rs` に backend ごとの env 注入既定を**バイナリコンパイル時**にハードコードする（`include_str!` の必要すら無い定数テーブル）。ユーザーが `agents[].env` を書いた場合は明示エントリが provider 由来を上書きする（3.3）。

---

## 5. Contract 追加（`CONTRACT.md` への追加断面）

`CONTRACT.md` に **`# Phase 5.0 追加契約`** として以下を additive に追記する。

### 5.1 ptygrid.yml スキーマ

- 4.1 の `providers` / `queen.memory` / `agents[].provider` / `workflows`。
- **既存 `agents[]` の `provider` は任意フィールド**。未指定なら従来通り env 注入は行わない。
- **workflow の検証エラーは `parse_config` を error で失敗させる**（team_presets と同流儀）。壊れた宣言を黙って読むと allowlist の意味が崩れるため。

### 5.2 新イベント

| event | payload | 説明 |
|---|---|---|
| `workflow-state` | `WorkflowRun` | run の state 変化ごとに emit（step 個別遷移も同 payload を送る、frontend は差分表示） |
| `provider-status` | `ProviderStatus[]` | 起動時 + config reload 時 + 手動 refresh 時。ヘルスチェック結果 |
| `arena-open` | `{ workflowRunId: string, taskId: string, contenders: {agent: string, sessionId: number}[] }` | fan-out workflow で `arena: true` のとき frontend に Arena drawer を開かせる |
| `memory-changed` | `{ scope: {kind, key}, deltaKind: "insert"\|"update"\|"forget", ids: number[] }` | frontend の MemoryPanel が listing を差分更新するのに使う |

### 5.3 新 Queen MCP tools（21 → 26 本目）

- `spawn_workflow` / `join` / `cancel_workflow`（M3、3.1）
- `memory.remember` / `memory.recall` / `memory.forget` / `memory.list` / `memory.reindex`（M4、3.2）
- `arena.vote` / `arena.list_votes`（S7 前半、3.4）
- `provider.status`（S1、`GET` のみ・副作用なし、cloud キーは返さない）

計 **11 tools 追加**。既存 18〜19 tools はすべて不変。全 tool は camelCase 引数を保つ（既存規約）。

### 5.4 新 Tauri commands

- `spawn_workflow` / `cancel_workflow` / `list_workflow_runs`
- `memory_status`（DB 内 memory 件数、embedding provider ident、暗号化状態を返す）
- `provider_status`（`ProviderStatus[]` を pull 型でも返す）
- `arena_open`（frontend が toolbar から手動で開く用、workflow run id を要求）

### 5.5 非回帰

- 既存 `session-state` / `agent-status` / `queen-notify` / `session-resources` は不変。
- 既存 `spawn_agent` / `list_agents` / `send_message` / `read_output` / `notify` / Pins / Notes / Inbox / `await` / `spawn_team` は**引数・返り値ともに不変**。
- 既存 `queen.sqlite3` schema `PRAGMA user_version` は 2 → **3** へ移行。migration は transactional。v2 のままの起動時に `memory*` テーブル + provider ident pragma を追加する。**未知の新 version は黙って開かない**（Phase 3.6 の規律を継承）。

---

## 6. エッジケース / 相互作用

### 6.1 workflow と既存 team_presets の関係

`team_presets`（Phase 4.3）は「複数エージェントを一括起動する薄いラッパー」であり、依存関係を持たない。workflow はその上位互換に位置するが、**team_presets は撤去しない**（既存 users の互換維持）。両方ロードされていても互いに独立。将来的に `workflow: single-shot` を用意すれば team_presets の完全上位となるが、v1 では並置する。

### 6.2 workflow と agent-status の相互作用

- 意味的状態 `blocked` は **step 完了と見なさない**。「人間の入力待ち」で workflow が固まるのは想定内の挙動で、`timeoutMs` があれば timeout FAIL、無ければ人間の介入を待って進む。
- workflow 中の pane が `blocked` になったら、既存の Phase 4.4 通知経路が発火する（何もしなくて良い）。
- Arena view で「ペインを見なくても vote できる」のは意味的にまずいので、Arena は `AgentStatus == done` になった contender からのみ vote 可能にする。

### 6.3 memory と scope の切替

- `session` scope の memory は **app 再起動で参照できなくなる**（session id は再起動で連番リセット）。DB 上は残存するが listing/recall では現在 session と一致しない limit を超えて出さない。**定期 GC** は行わず、ユーザーの `forget` に委ねる（データ喪失の意外性を最小化）。
- `agent` scope は「定義名」で束ねる。定義名を rename すると memory が孤立する。**rename ヘルパー** `memory.rename_agent(old, new)` は v1 では実装しないが、export/import で移送可能。

### 6.4 provider 到達不能時の workflow

- fan-out workflow を実行し、途中で `provider_status` が unreachable になっても、既に spawn 済みの pane は動き続ける。**新しい step の spawn 時**にのみ provider の到達性を再確認し、不能なら該当 step を FAILED にする（それが retry を消費する）。orchestrator は provider を積極的に heal しない。

### 6.5 embedding backend の切替

- ident が変わった場合、`memory.reindex` を明示的に呼ばない限り以後の `remember` は error で拒否する。**サイレントに index を壊さない** — これは spec の中心的な安全性条件。

### 6.6 workflow cancel と kill_pty の相互作用

- ユーザーが個別に `kill_pty` した pane が workflow step に属している場合、その step は自動的に FAILED になり、`onFailure` の規約に従う。逆に workflow を `cancel_workflow` すると、走行中の step 全ての pane に `kill_pty` を発行する。**「workflow が生成した pane はユーザーの明示閉じで殺されうる」ことを設計として認める**（ptygrid の pane 中心哲学は維持）。

### 6.7 OpenTelemetry GenAI との連携（Phase 5.5 で導入済み）

Phase 5.5 で仕込まれた OpenTelemetry GenAI トレースに、workflow run 全体を span で括る（`workflow.name`, `workflow.pattern`, `workflow.step.id` を属性）。**新しい telemetry backend は導入しない**、既存 exporter に流すだけ。

### 6.8 プライバシー

- `memory.export` はデフォルトで **`source_agent` と `entities` を含む**が、`--redact` オプションで agent 名を hash 化できる。cloud 提供者にダンプを送るケースを想定。
- `memory.forget({scope: {kind:"user"}, cascade: true})` は「全 memory 削除」の危険操作なので、Queen tool 側で `confirm: "ERASE ALL"` の文字列必須にする（Pins/Notes の delete が revision 必須なのと同じ「うっかり防止」）。

---

## 7. テスト計画

### 7.1 純関数・ロジックテスト（Rust, `cargo test`）

- **DAG 構築**: `depends_on` からの topological sort、循環検出（cycle → error）、unknown id 参照 → error。
- **step 完了規則**: 3 経路（exit 0 / agent-status done linger 経過 / inbox reply）が独立に SUCCEEDED を返すこと。混在時の優先順位。
- **join_on**: `all` / `any` / `n` それぞれで正しく親を進めること。`any` 時に残り fan-out を CANCELLED にすること。
- **retry**: `max` 消費後の FAILED 遷移、backoff の待機。
- **timeoutMs**: 超過で FAILED、kill_pty 呼び出しの記録。
- **fail-fast vs continue**: 失敗時の他 step への波及。
- **memory embedding ident 不一致**: pragma 保存 → 起動時再チェック → 不一致で error、`reindex` 後に成功。
- **RRF ハイブリッドスコア**: vec のみヒット / fts のみヒット / both で `hitBy` が正しい。
- **forget cascade**: memory / memory_fts / memory_vec の三重削除が同 transaction で成立。
- **provider ヘルスチェック**: mock http サーバに対し reachable / unreachable / タイムアウトを判定。
- **env 注入**: ユーザー明示 env が provider 由来を上書きすること、provider 未指定で従来通り env が付かないこと。

### 7.2 integration テスト

- `queen.sqlite3` v2 → v3 migration が既存 pins/notes/inbox データを壊さないこと。
- fake embedding backend（次元 4 の固定 vector を返すダミー）で `remember` → `recall` の end-to-end。
- workflow を pipeline / fan-out / supervisor / handoff の各パターンで 1 本ずつ実行し、`workflow-state` イベントの遷移列を assert。

### 7.3 frontend（`svelte-check`）

- `workflow-state` 受信で `ui.workflowRuns` の増分更新、`SUCCEEDED/FAILED/CANCELLED` で終端表示。
- Arena drawer が `arena-open` で開く、vote ボタンで `arena.vote` を invoke。
- ProviderStatus バナーが unreachable のときに警告色で表示、reachable で控えめ表示。

### 7.4 実機手動検証（macOS 必須 / Linux ベストエフォート）

1. `local:ollama:*` を宣言した agent が Ollama 起動時に env `ANTHROPIC_BASE_URL` 付きで走る。Ollama 停止時にはバナー警告のみ、pane は起動する。
2. `workflows.review-and-fix` を `spawn_workflow` で走らせ、review pane が inbox reply を返すと fix pane が起動する。
3. `workflows.triple-review` で 3 pane が並列生成、Arena drawer が自動で開き、いずれかを vote すると `memory` に記録される。
4. `memory.remember` → 別 session で `memory.recall` が同一 project scope で取れる。
5. `queen.memory.encrypted: true` にして再起動、DB 内 content が平文で見えないこと、recall は vec-only であること、バナーで縮退表示が出ること。
6. `cancel_workflow` で残り step が CANCELLED になり pane が閉じる。
7. workflow 途中で手動 `kill_pty` した pane の step が FAILED になり、fail-fast なら他 pane も止まる。

---

## 8. 設計判断（採用しない案とその理由）

- **専用ジョブスケジューラを内蔵する案**（cron 的 trigger を workflow に統合）: 却下。ptygrid の責務は「PTY を持って monitor する」ことに閉じる。cron は OS 側の launchd / systemd / cron を使ってもらう。
- **workflow DSL の外部化**（`.workflow.yml` / Airflow-DAG 風の Python）: 却下。**config-as-code の唯一のファイルは `ptygrid.yml`** を保つ（Phase 1 からの原則）。マルチファイル分割は将来。
- **memory の中央化（S3 / cloud DB 同期）**: 却下。ptygrid は「オフラインで完結する」を強い原則にしている（Phase 4.4 の内蔵既定パターン方針と同根）。同期は export/import + 外部ツールに委ねる。
- **embedding を workflow trigger にも使う**（"あるパターンに近い出力が出たら別 step を起動する"）: 却下。工作物としては魅力的だが、workflow の DAG 実行と embedding 検索を同じ hot loop で回すと診断困難な系になる。将来 `condition` field を正規表現以外に拡張する余地は残す（未実装）。
- **Arena の Merge 操作**を Phase 5.0 に含める: 却下。Merge は本質的に「別のエージェント run を挟む」動作で、S7 後半 = Phase 6.0 以降に分離する。v1 の Arena は Vote までに留め、選ばれた出力を後続 step に手動で回す形にする。
- **provider を agent 定義外で切り替え可能に**（runtime overriding）: 却下。宣言と実行の対応を崩さない（allowlist 哲学と同根）。runtime に provider を差し替えたいなら別 agent 定義として書く。
- **workflow 実行履歴を全て DB に**: 現状は保持するが、GC は v1 では実装せず、10,000 run で古い順に削るハードキャップのみ設定する（後日の telemetry export と兼ねる）。
- **Ollama の埋め込みモデル自動 pull**: 却下。ptygrid はユーザーマシンの状態を勝手に変えない（Git worktree の dirty 削除禁止と同哲学）。バナーで「`ollama pull nomic-embed-text` を実行してください」と案内するに留める。

---

## 9. 段階分割案（Phase 5.0.0 〜 4.5.5）

[plan.md](../plan.md) の y=Phase、z=Phase 内連番規約に従い、4.5 を **6 段階**に分ける。1リリース = 1 patch。

### Phase 5.0.0 — Provider 統合の基盤

- `providers:` スキーマ、`ProviderStatus`、`provider_status` イベント、`agents[].provider` + env 注入。
- **workflow / memory / arena は未着手**。
- completion gate: `cargo test` / `svelte-check` / build / 両プラットフォーム CI、CONTRACT の該当節を先行追記、userguide に「ローカル provider」節。

### Phase 5.0.1 — Memory: 保存経路（embedding 抜き）

- `queen.sqlite3` v2→v3 migration、`memory` テーブル + FTS5 のみ（vec テーブルはスキーマだけ用意し空）。
- `memory.remember` / `memory.forget` / `memory.list`（FTS5 検索のみ）。`memory.recall` は FTS5 のランクで返す。
- completion gate: migration 冪等性テスト、既存 pins/notes/inbox 非回帰、暗号化オプションのスケルトン。

### Phase 5.0.2 — Memory: embedding + ハイブリッド検索

- `memory_embed.rs` の trait、Ollama / LM Studio / OpenAI / Anthropic backend、pragma ident。
- `sqlite-vec` の起動時 load、`memory_vec` テーブル、RRF ハイブリッドスコア、`memory.reindex`。
- Arena に先行して MemoryPanel（recall UI + entity 抽出プレビュー）。
- completion gate: fake backend integration、次元不一致 error、reindex 動作、実 Ollama での手動検証。

### Phase 5.0.3 — Orchestrator: pipeline + supervisor

- `orchestrator.rs` の DAG 実装（pipeline / supervisor）、`spawn_workflow` / `join` / `cancel_workflow` の 3 tools。
- `WorkflowPanel.svelte`（run listing + state 表示）。
- **fan-out は未実装**（次段で足す）。
- completion gate: DAG 循環検出、retry / timeout、fail-fast/continue、既存 team_presets 非回帰。

### Phase 5.0.4 — Orchestrator: fan-out + handoff

- fan-out step の並列 spawn、`join_on: all|any|n`、handoff pattern。
- completion gate: `any` 時の残 fan-out CANCELLED、handoff の inbox generation watch 動作。

### Phase 5.0.5 — Arena view（S7 前半）

- `arena.rs` + `Arena.svelte`、`arena-open` イベント、`arena.vote` / `arena.list_votes`。
- fan-out workflow から自動 open、Diff viewer、Vote → memory 記録。**Merge は含めない**。
- completion gate: fan-out 済 workflow から Arena が自動起動、vote が `memory` に project scope で記録される、既存グリッド操作の非回帰。

> バージョン割当（暫定）: Phase 5.0 は **`v0.5.0〜v0.5.5`**(MVO 5.0.0 → Arena 5.0.5)。1 patch = 1 stage。以降 Phase 5.5(Observable) が `v0.5.6〜v0.5.10`、Phase 6.0(Secure) が `v0.6.x` を消化する。Phase 5 系(=v0.5.z)の間は連番で埋め、Phase 6 系突入で `v0.6.0` へ minor bump。

---

## 10. リスクと未解決事項

- **sqlite-vec の配布**: 各 platform 用の loadable extension を bundle するか、`sqlite-vec` crate の in-process 統合を使うか未確定。後者を採用し、追加のバイナリ配布を避けたい。macOS/Linux/Windows のビルドマトリクスで実測後決定。
- **embedding backend の次元差**: Ollama `nomic-embed-text` は 768 次元、`mxbai-embed-large` は 1024 次元、OpenAI `text-embedding-3-small` は 1536 次元。混在は許さない（pragma で 1 project 1 次元）。
- **workflow 中の Claude Code teammate**: Phase 4.2 の host teammate PTY が workflow の子 step になった場合の閉じ挙動は実機確認が必要。初期は「host teammate は workflow で spawn しない」を実装的な制約とし、明示 error を返す。
- **arena vote 集約とバイアス**: v1 は user 投票のみ。将来「他エージェントの投票」を許すと、supervisor 型で「上位 agent が下位 agent を評価」が可能になるが、悪用（自演の高評価）を排除する仕組みが必要。設計は Phase 5.x 以降。
- **memory と GDPR-like 要件**: user scope memory の export/forget を **1 tool で完結**させることは既に本 spec に入っているが、`~/.ptygrid` の全 project にまたがる横断削除は v1 スコープ外。
- **workflow の可視化**: 大きな DAG（step 10 個超）を frontend で描画する UI は Phase 5.0.3 の Panel では「flat list」に留め、graph 描画は将来。

---

## 11. 参考

- **LangGraph Supervisor パターン** — <https://langchain-ai.github.io/langgraph/tutorials/multi_agent/agent_supervisor/>（fan-out + supervisor の状態機械モデル）
- **sqlite-vec（Alex Garcia）** — <https://github.com/asg017/sqlite-vec>（`vec0` virtual table、`MATCH` KNN）
- **Ollama Embeddings API** — <https://github.com/ollama/ollama/blob/main/docs/api.md#generate-embeddings>（`/api/embed`、`model` + `input[]`）
- **LM Studio OpenAI 互換 API** — <https://lmstudio.ai/docs/api/openai-api>（`:1234/v1/embeddings`）
- **Jan** — <https://jan.ai/docs/api-server>（`:1337/v1/*` の OpenAI 互換）
- **Parallel Code AI Arena** — <https://parallelcode.app/>（同時 N モデル、side-by-side、diff/vote）
- **Anthropic Claude Code / Squads / Subagents** — Phase 5.5-4.2 で既に統合済み。workflow の handoff / supervisor は Claude Code の subagent とほぼ同型（[spec-claude-teams-panes.md](../spec-claude-teams-panes.md) 参照）。
- **Reciprocal Rank Fusion** — Cormack, Clarke, Büttcher (2009), "Reciprocal rank fusion outperforms Condorcet and individual rank learning methods."
- **OpenTelemetry GenAI semantic conventions** — <https://opentelemetry.io/docs/specs/semconv/gen-ai/>（Phase 5.5 で導入、workflow span 属性の受け皿）

---

## 追補: 5.0.1 Workflow Resume(落ちたときの途中再開 + Y/N 確認)

作成: 2026-07-23 / 状態: draft(次実装対象)

### 目的
アプリ落ち・再起動で in-memory registry が消え、実行中 workflow が失われる。SQLite 永続化(5.0.1 で予約済みの `workflow_runs`、user_version 2→3)を前倒しし、再起動時に中断 run を検出して**ユーザーに Y/N で再開を確認**する。

### 設計
1. **永続化**: `queen.sqlite3` に `workflow_runs(run_id PK, name, state, started_at_ms, ended_at_ms, steps_json, project_dir)`。`WorkflowRegistry::put` から write-through(driver の状態遷移ごとに UPDATE)。
2. **検出**: `load_config` 成功時、その project の state='running' の run を SELECT → `workflow-resume-pending` イベントで frontend へ(複数可)。
3. **Y/N UI**: バナー「前回の run '<name>' が途中で中断されています。再開しますか?」+ [再開] / [破棄] ボタン(既存 trustPrompt バナーと同パターン)。
4. **再開(Y)** `resume_workflow(runId)`: succeeded/failed/skipped step は保持。**running だった step は pending に戻す**(PTY は死んでいるため再 spawn。エージェントは inbox/pins/memory から文脈を回収できる)。同じ run_id のまま registry に載せ、既存 driver が続きを進める。
5. **破棄(N)** `abandon_workflow(runId)`: DB 上で state='cancelled' + error="abandoned after restart" に更新(再プロンプト防止)。
6. **契約追加**: Tauri commands `resume_workflow` / `abandon_workflow`、イベント `workflow-resume-pending`。Queen tool は不要(人間の意思決定なので UI のみ)。
7. **エッジ**: 同名 agent の live セッションが既にいる場合は pipeline の冪等 reuse 規則をそのまま適用。fan-out の running コピーは全て pending へ。resume 前に config が変わり workflow 定義が消えていたら「定義なし」エラーでバナー表示のみ。
