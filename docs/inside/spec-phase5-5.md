# ptygrid 仕様: Phase 5.5「Observable & Standards-Compliant」— MCP 2026-07-28 RC 追随 / OTel GenAI 計装 / Agent Status Rings

作成日: 2026-07-22 / 状態: ドラフト / 対象: Phase 5.5（未実装・仕様のみ）

関連: [spec-agent-status.md](../spec-agent-status.md)（意味的状態 `AgentStatus` の供給源。本仕様の Rings と OTel の親フレームとして再利用）/
[spec-notifications.md](../spec-notifications.md)（`error` / `needs-attention` エッジ配線と competing しない設計）/
[design.md](../design.md)（hot path 分離・config-as-code・推測回避）/
[competitive-landscape.md](../competitive-landscape.md)（「通知リング / 要承認ハイライト」バックログを本仕様の M5 に格上げ）/
[plan.md](../plan.md)（バージョニング）/
[../CONTRACT.md](../../CONTRACT.md)（IPC / MCP 契約）/
[../ptygrid.example.yml](../../ptygrid.example.yml)（注釈付き設定例）。

実装（新規）: [../src-tauri/src/queen_compat.rs](../../src-tauri/src/queen_compat.rs)（MCP RC 互換ルータ）/
[../src-tauri/src/observability.rs](../../src-tauri/src/observability.rs)（OTel エクスポータ・span 生成）/
[../src-tauri/src/cost.rs](../../src-tauri/src/cost.rs)（pricing table / 通貨変換）/
[../src-tauri/src/status_ring.rs](../../src-tauri/src/status_ring.rs)（Ring 集約と派生モデル）。
配線元は既存 [../src-tauri/src/queen.rs](../../src-tauri/src/queen.rs)・[../src-tauri/src/agent_status.rs](../../src-tauri/src/agent_status.rs)・[../src-tauri/src/session.rs](../../src-tauri/src/session.rs)。DB は既存の SQLite（[../src-tauri/src/db.rs](../../src-tauri/src/db.rs) 相当）に `spans` テーブルを追加。

---

## 1. 目的と背景

ptygrid は Phase 3〜4.4 で「複数 AI CLI を PTY で並行実行し、Queen（18 tools）で協調させる」までを揃えたが、Phase 4.4 の意味的状態と通知（[spec-agent-status.md](../spec-agent-status.md) / [spec-notifications.md](../spec-notifications.md)）を実運用に投入した結果、次の3つの構造的な穴が同時に露呈した。

- **穴 M1 — プロトコル互換**: Queen は rmcp `streamable-http` の 2025 系実装で、`initialize` ハンドシェイクと `Mcp-Session-Id` によるスティッキー経路に依存している。上流の Model Context Protocol は 2026-07-28 RC で **stateless（セッションレス）** へ大きく舵を切り、`Mcp-Method` / `Mcp-Name` ルーティングヘッダを必須化した。Roots / Sampling / Logging は 12 ヶ月 deprecation window で撤去予定。追随しなければ、来年出そろう Tier 1 SDK のクライアント（Claude Code / Codex 側の MCP クライアント）から Queen が徐々に読めなくなる。
- **穴 M2 — オブザーバビリティ**: いま ptygrid は「1 ペイン単位の PTY 生死」「意味的状態バッジ」「resource 監視（CPU/RSS）」を持つが、**どのエージェントが何秒でいくら燃やしたか**を追跡する経路が無い。ローカル LLM 混在チーム（Phase 4.3 `team_presets`）に入ってから、コスト事故（Opus に丸投げしっぱなし）と潜在バグ（tool 呼び出しの階層が見えない）の両方が起きている。
- **穴 M5 — 可視性**: 9 ペイン時、`spec-agent-status.md` の状態バッジ（ヘッダー内 8px の丸）は視認距離が足りない。通知バッジ（Queen inbox）と git dirty、コスト live 表示が別 UI に散らばっているので、目線を1点に集めたい。

本仕様はこの3穴を **一段の Phase 5.5「Observable & Standards-Compliant」** として統合する。M1 で「外向きの契約（MCP）」を新標準へ寄せ、M2 で「内側の観測」を W3C Trace Context で貫き、M5 でその**結果**（状態・通知・コスト）を人間の目線に集約する。M1 の trace propagation が M2 の親スパンを与え、M2 の実測コストが M5 の live indicator を駆動する、という依存の向きで、単発機能の足し算ではなく1つの「オブザーバビリティ回路」として設計する。

### 既存レイヤとの棲み分け（重複させない）

| 供給源 | 事実の粒度 | 表現 | 本仕様が触るか |
|---|---|---|---|
| `SessionState`（Phase 1） | プロセス生死 | ペインヘッダ状態ドット | いじらない |
| `AgentStatus`（Phase 4.4） | 意味的状態（blocked/working/done/idle/unknown） | ヘッダーバッジ + 通知リング | **Ring 恒常化のため参照**（変更なし） |
| `session-resources`（Phase 3.7） | CPU / RSS の低頻度サンプル | サイドバー数値 | 参照のみ |
| `notifications`（Phase 4.4.2） | エラー / 承認待ちエッジ | OS / Slack 等 | 変更なし |
| **本仕様（Phase 5.5）** | **モデル呼び出しの因果 + トークン + コスト** | **トレース + Ring** | 新設 |

Ring は「新しい事実」ではなく、上表の**既存の事実を1本の輪郭に畳んだビュー**である。回路の中で新たにデータを作るのは M2（OTel）だけであり、M5 は集約、M1 は経路の付け替えである。

---

## 2. モデル

### 2.1 トレースモデル（M2）

OTel の GenAI semantic conventions 2026 に準拠する（本仕様バンドル基準は 1.36+）。各 span の代表 attribute は次の通り。プロンプト本体は attribute に置かず **span event** として emit する（既定は非記録）。

| span 種別 | `span.kind` | 親 | 代表 attribute | 代表 event |
|---|---|---|---|---|
| `pane.session` | `INTERNAL`（長寿命） | なし | `ptygrid.session.id`, `ptygrid.session.kind`, `ptygrid.agent.name` | — |
| `agent.turn` | `CLIENT` | `pane.session` | `gen_ai.system`（=`anthropic`/`openai`/…）, `gen_ai.request.model` | ユーザー可視のプロンプト境界 |
| `gen_ai.chat` | `CLIENT` | `agent.turn` | `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.response.finish_reasons`, `gen_ai.time_to_first_token_ms`（1.36+）, `ptygrid.cost.usd_micro` | `gen_ai.system_instructions` / `gen_ai.input.messages` / `gen_ai.output.messages`（**opt-in** のみ） |
| `queen.tool` | `SERVER` | `agent.turn`（W3C 継承） | `mcp.tool.name`, `mcp.method`（=`tools/call`）, `ptygrid.queen.tool_family`（`inbox`/`git`/…） | 引数と結果（opt-in） |
| `pty.io.block` | `INTERNAL` | `agent.turn` | `ptygrid.pty.bytes`, `ptygrid.pty.wraps` | — |

**規約**:

- `gen_ai.chat` を親にせず、**`agent.turn` を親**にする（1 ターン内に複数モデル呼び出し・複数ツール呼び出しがあるため）。これが Ring のコスト live 表示（5章）の集計単位でもある。
- ptygrid 独自の属性は `ptygrid.*` 名前空間に閉じる（OTel の予約と衝突しない）。
- `ptygrid.cost.usd_micro` は **μUSD**（`u64` に安全に載る整数、`price_per_token * tokens` を micro に丸め）。通貨は `ptygrid.cost.currency` に生 3 文字コードで別記する。
- `_meta.traceparent` 経由で MCP 側から親コンテキストが与えられたときは、`queen.tool` の親をそちらに差し替える（M1 §3.3）。

### 2.2 コストモデル（M2）

コストは推論本体（`gen_ai.chat`）にだけ載せる。tool 呼び出しはコストゼロ（Queen tool は API 課金対象ではない）。

```rust
pub struct PriceRow {
    pub system: String,          // "anthropic" | "openai" | "xai" | ...
    pub model: String,           // "claude-opus-4-7" | "gpt-5.1" | ...
    pub input_per_mtok: f64,     // USD / 1M input tokens
    pub output_per_mtok: f64,    // USD / 1M output tokens
    pub cache_read_per_mtok: Option<f64>,
    pub cache_write_per_mtok: Option<f64>,
    pub effective_from: Option<chrono::NaiveDate>,
}
```

内蔵デフォルトは `src-tauri/src/cost_defaults.yml` に compile-time 同梱（`include_str!`）し、[`ptygrid.yml`](../../ptygrid.example.yml) の `pricing:` ブロックで**追記・上書き**できる（[spec-agent-status.md](../spec-agent-status.md) §4.4 と同じ merge/replace 方針）。通貨換算は `pricing.fx_rate` に **手動 USD→JPY 等の固定レート** を書ける（外部 FX API を叩かない・オフライン方針）。

### 2.3 Ring モデル（M5）

各 PTY ペインの輪郭（外周 2〜3px）を、**状態色×バッジ**の合成として表現する。状態色は既存 `AgentStatus`、バッジは以下 3 つを右上に重ねる。

| バッジ | 供給源 | 表示条件 |
|---|---|---|
| `inbox N` | Queen `/inbox` の未読件数 | N > 0（既定 9+ 表記に飽和） |
| `git ●` | 既存 `session-resources.git` の dirty フラグ | dirty && running |
| `cost c` | 直近 5 分の μUSD 合計 → 通貨表記 | opt-in（`agent_status.ring.cost: true`） |

Ring 自体は **frontend の派生 view** で backend 追加なし。`inbox` は既存 Queen tool の結果を frontend の store が引く（新 IPC は追加しない、6章）。cost は `agent-cost` イベント（3.4）を購読して直近 5 分の rolling window を持つ。

---

## 3. 検出方式 / メカニズム

### 3.1 MCP 2026-07-28 RC 対応（M1）

現行 Queen は `rmcp` の `streamable-http` で `initialize` 後に `Mcp-Session-Id` を発行し、リクエストをスティッキーに紐付けている。RC ではこれが**廃止**され、各 POST が独立して意味を持つ stateless 前提になる。ptygrid の 127.0.0.1 単一プロセスバインドではロードバランサ不要だが、**クライアント側 SDK（Claude Code / Codex）が RC に追随した時点で旧経路は呼ばれなくなる**ので追随は必須。

移行方針は「**両立ルータ**」。`src-tauri/src/queen_compat.rs` に新規レイヤを設け、単一の Axum ハンドラで **旧 (2025-06) 経路 と RC (2026-07-28) 経路 を feature flag なしに同時受理**する。判別素材は以下:

- `Mcp-Method` / `Mcp-Name` ヘッダの有無 → **有れば RC 経路**、ボディの `method` と一致することを検証（不一致は `400 mismatch`、RC 準拠）。
- `Mcp-Session-Id` 発行の要否 → RC 経路では**発行しない**（返却ヘッダにも載せない）。旧経路互換のため受理はする（無視）。
- `initialize` / `initialized` メソッド → RC 経路では**受理するが no-op**（互換のためエラーにしない。`_meta.protocolVersion` だけ確認して即 200）。
- Roots / Sampling / Logging → RC 経路の deprecation window（12 ヶ月）中は互換維持。`resources/roots` は**現行で未使用**（Queen は roots を公開していない）、`sampling/*` も未使用、`logging/setLevel` のみ受理→内部ログレベルに反映。全て `Deprecation` レスポンスヘッダを付与（3.5）。

**18 tools のスコープ確認**: `list_agents` / `read_output` / `send_message` / `spawn_agent` / `notify` / `pins/*` / `notes/*` / `inbox/*` / `await` / `git/*` / `worktree/*` / `spawn_team` の全 18（19 本目 `spawn_team` は Phase 4.3 で追加済み）は RC でも **`tools/*` メソッド系のまま**動く。ツール定義（input schema）自体は 2025 系と互換で、変更は「呼ばれ方（ヘッダとセッション）」だけ。したがってツール実装コードには**触らない**（`queen.rs` の各 handler は無改修）。

### 3.2 Trace Context の受け入れと注入（M1 × M2 の結合点）

RC は `_meta.traceparent` / `_meta.tracestate` / `_meta.baggage` の位置を確定させた。ptygrid は次のように扱う。

- **受け取り**: `queen_compat.rs` は JSON-RPC ボディの `params._meta.traceparent` を最優先で読む。無ければ HTTP `traceparent` ヘッダ、それも無ければ新規に親を作る（`observability::start_root("queen.tool")`）。
- **子への継承**: `queen.tool` span を **受け取った親の子として**開く。tool 実装（`inbox_send` など）が内部で `agent.turn` を生む場合（例: `spawn_team` の instructions 配信）は、そこも同じトレース木に載る。
- **応答への注入**: RC 応答の `_meta` に **サーバ側の traceparent を返す**（クライアントのトレース木にサーバー span を接続するため）。既存 tools 呼び出し側にトレース意識は要らない。

### 3.3 OTel パイプライン（M2）

依存は `opentelemetry`（1.36+）と `opentelemetry-otlp` の gzip+http/protobuf。**gRPC は使わない**（tonic を Cargo に持ち込むと build ツリーが 30% 肥大するため。ptygrid は lean な依存構成を優先、[spec-notifications.md](../spec-notifications.md) §6.3 と同方針で reqwest すら避けて `ureq` を採用してきた原則を維持）。

- エクスポータ既定は **`stdout`（無効）**。opt-in で OTLP HTTP エンドポイント（`observability.otlp.endpoint`）または **ローカル SQLite シンク**（`observability.sink: sqlite`）へ流す。
- SQLite シンクは backend 単一プロセス内の同期 batch writer。1 秒間隔 or 512 span でフラッシュ。既存の SQLite（WAL、project-scoped）にテーブルを追加する（3.4）。
- サンプリングは `ParentBased(TraceIdRatio(rate))`、既定 `rate: 1.0`（ローカルなので落とす理由がない、外部 OTLP を使う場合のみ 0.1 等を推奨）。

### 3.4 SQLite スキーマと `agent-cost` イベント

```sql
CREATE TABLE IF NOT EXISTS spans (
  trace_id     BLOB NOT NULL,       -- 16 bytes
  span_id      BLOB NOT NULL,       -- 8 bytes
  parent_id    BLOB,                -- 8 bytes, NULL for root
  name         TEXT NOT NULL,       -- "gen_ai.chat" etc
  kind         INTEGER NOT NULL,    -- OTel SpanKind enum
  start_ns     INTEGER NOT NULL,
  end_ns       INTEGER NOT NULL,
  session_id   INTEGER,             -- ptygrid session id (null for orphan)
  agent        TEXT,                -- "claude" | "codex" | ...
  model        TEXT,
  input_tok    INTEGER,
  output_tok   INTEGER,
  cost_umicro  INTEGER,             -- μUSD
  attrs_json   TEXT,                -- 残余 attributes
  events_json  TEXT,                -- opt-in prompt/response events
  PRIMARY KEY (trace_id, span_id)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS spans_by_session_start ON spans(session_id, start_ns DESC);
CREATE INDEX IF NOT EXISTS spans_by_trace ON spans(trace_id, start_ns);
```

`gen_ai.chat` span 終了時、backend は `agent-cost` Tauri イベントを emit する（5章の Ring / cost live 用）。

```ts
export type AgentCostPayload = {
  id: number;                    // session id
  model: string;                 // "claude-opus-4-7"
  costUmicro: number;            // this call
  inputTok: number;
  outputTok: number;
  ttftMs?: number;
};
```

デバウンスは要らない（`gen_ai.chat` は自然に低頻度）。**ホットパス（PTY reader / regex 評価タスク）から `span.end` を呼ばない**（[spec-agent-status.md](../spec-agent-status.md) §7.1 hot path 分離原則）。span は Queen tool 経路と OTel の受動的な `agent.turn` 抽出（3.5）でのみ生まれる。

### 3.5 `agent.turn` の抽出（PTY 側からモデル呼び出しを見る問題）

ptygrid は PTY 越しに CLI を回すので、モデル呼び出し自体を直接インストルメントできない。取れる素材は3つ:

1. **hook 経由**（Phase 4.0 の teammate hooks 受信基盤の `/hooks/v1/*`）— Claude Code は `PreToolUse` / `PostToolUse` / `Stop` を投げる。ここに `input_tokens`/`output_tokens` を載せてもらう（Claude Code 側の hook 拡張は本仕様の範囲外だが、**設定例と契約だけ用意する**）。
2. **Queen 経由**（M1 の `_meta.traceparent`）— MCP クライアントが `queen.tool` を呼ぶ際に自身の親コンテキストを渡してくれる場合、そこから間接的に「呼び出し中の turn がある」と推定できる。
3. **推定なし**（unknown）— どちらも取れないときは、`agent.turn` は emit しない。既存 `AgentStatus`（`working`）だけが唯一の観測になる。

方針は「**素材が無いなら黙る**」（[spec-agent-status.md](../spec-agent-status.md) §2.3 保守主義に一致）。cost が取れないターンは Ring cost バッジも出さない。**推測でコスト表示は絶対にしない**。ローカル LLM（coderouter 経由）で API 課金が発生しないケースでも「0 円」表示にせず、`unpriced` を返して非表示にする（誤解を招くため）。

### 3.6 Deprecation ヘッダの出力

RC の 12 ヶ月 window に従い、旧仕様のリクエストを受理したときは応答ヘッダに次を付与する:

```
Deprecation: Sat, 28 Jul 2027 00:00:00 GMT
Sunset: Wed, 28 Jul 2027 00:00:00 GMT
Link: <https://modelcontextprotocol.io/spec/2026-07-28>; rel="deprecation"
```

`Mcp-Session-Id` を発行する系（旧経路）でのみ付与。RC 経路では付与しない。backend ログには **1 リクエストあたり最大 1 行**の `deprecated_route` 警告を per-day dedupe で出す（ログ爆発を避ける、[spec-notifications.md](../spec-notifications.md) §6.2 と同じ精神）。

### 3.7 Status Ring の合成（M5）

CSS で `.pane { position: relative; }` の外周に、疑似要素 `::before`（状態リング）と `::after`（バッジトレイ）を積む。Svelte 5 runes で `$derived` を使い、以下の派生:

```ts
// src/lib/status_ring.svelte.ts (新規)
const ringColor = $derived.by(() => {
  const s = ui.agentStatus[id];
  if (s === "blocked") return "var(--ring-blocked)";  // WCAG AA on light+dark
  if (s === "working") return "var(--ring-working)";
  if (s === "done")    return "var(--ring-done)";
  if (s === "idle")    return "var(--ring-idle)";
  return "transparent";
});
```

CSS カスタムプロパティは既存の `.dot.state-*` と別 namespace（`--ring-*`）。**色覚配慮**として:

- 赤 / 青 の識別に頼らず**明度差**を 3:1 以上確保（WCAG AA）。
- blocked は静的リングを基本にし、`prefers-reduced-motion: no-preference` のときだけ 1Hz 以下の淡いパルスを許可。
- 色だけでなく **形**でも区別: blocked は実線、working は破線、done は二重線、idle は細線。色覚単色ユーザでもパターンで判別できる。

バッジトレイは 3 バッジ横並び。溢れは `+N` 表記に丸める。9 面時に**視認距離 1m** で判別できる最小サイズ（実測 8mm 相当）を CSS で確保する。

---

## 4. 設定（`ptygrid.yml`）

### 4.1 スキーマ

```yaml
# --- Phase 5.5 追加ブロック（すべて任意・opt-in） ---

mcp:
  # RC 追随の feature flag。既定 true（新規デプロイは即 RC 経路も受理）
  rc_2026_07_28: true
  # 旧経路も残す（deprecation window 内・既定 true・window 終了で false 推奨に変更）
  legacy_2025_06: true
  # sampling / roots / logging の受理(RC で deprecated)
  legacy_capabilities:
    sampling: false          # 既定 false(未使用のため即オフ)
    roots: false
    logging: true            # ログレベル制御のみ残す

observability:
  enabled: false             # 既定 false(opt-in、[spec-notifications.md] と同じ opt-in 方針)
  sink: sqlite               # sqlite | otlp | none
  otlp:
    endpoint: "${OTEL_EXPORTER_OTLP_ENDPOINT}"
    headers:
      Authorization: "Bearer ${OTEL_TOKEN}"
  sample_rate: 1.0           # 0.0..=1.0
  capture_content: false     # プロンプト本体を span event に載せるか(既定 false)
  capture_encrypt_key: ""    # 非空なら AES-GCM で events_json を暗号化(32B hex)
  retention_days: 30         # SQLite spans テーブルの保持日数(0=無期限)

pricing:
  currency: USD              # 表示通貨(USD | JPY | EUR | ...)
  fx_rate:                   # USD からの手動レート(外部 API に問い合わせない)
    JPY: 158.0
    EUR: 0.92
  # 内蔵デフォルトに追記(既定 merge、replace: true で置換)
  models:
    - system: anthropic
      model: claude-opus-4-7
      input_per_mtok: 15.0
      output_per_mtok: 75.0
    - system: anthropic
      model: claude-sonnet-5
      input_per_mtok: 2.0
      output_per_mtok: 10.0
    - system: openai
      model: gpt-5.1
      input_per_mtok: 5.0
      output_per_mtok: 20.0

ui:
  ring:
    enabled: true            # 既定 true(agent_status が有効なら Ring も既定 on)
    cost: false              # 既定 false(コスト live バッジは明示 opt-in)
    inbox: true              # 既定 true(Queen inbox 未読集約)
    git_dirty: true          # 既定 true
    cost_window_s: 300       # 直近何秒のコストを集約するか(60..=3600)
    show_at_idle: false      # idle 状態のペインでも Ring を出すか(既定 false=控えめ)
```

### 4.2 マージ／置換セマンティクス

`pricing.models` は [spec-agent-status.md](../spec-agent-status.md) §4.2 と同じ規則で内蔵デフォルトへ **既定マージ（追記）**。同一 `(system, model)` が重複した場合は**ユーザー定義が優先**（順序ではなくキー一致で上書き）。`replace: true` を model 単位で付ければ内蔵の同キーを廃棄。

`mcp.*` / `observability.*` はスカラのみで merge 概念なし。未知キーは前方互換のため無視（他の 4.x ブロック同様）。

### 4.3 バリデーション方針

- `observability.enabled: true` かつ `otlp.endpoint` 空 → SQLite シンクにフォールバック + backend 警告 1 行。
- `capture_content: true` かつ `capture_encrypt_key` 空 → **設定エラーで拒否**（プロンプトが平文でディスクに書かれる事故を仕様レベルで防ぐ）。他のフィールドと違い、これは load を失敗させる。
- `pricing.fx_rate.<CCY>` が数値でない → その通貨だけ無効化（USD 表示にフォールバック）。設定全体は通す。
- `ui.ring.cost_window_s` は clamp `60..=3600`（範囲外は端に張り付ける、警告のみ）。

---

## 5. Contract 追加（CONTRACT.md への追記断面）

### 5.1 Queen（MCP）契約

- **HTTP ヘッダ受理**: `Mcp-Method`（値 = リクエストボディの `method` と一致必須）、`Mcp-Name`（tool 呼び出しなら `params.name`、resource 系なら URI）を追加受理。**不一致は 400。** `Mcp-Session-Id` は**受理はする（互換）／発行しない（RC 経路）**。
- **JSON-RPC メタ**: `params._meta.traceparent` / `.tracestate` / `.baggage` を受理し、応答の `result._meta` に自サーバ側 traceparent を返す。
- **`initialize` 系メソッド**: RC 経路は no-op 200。旧経路は従来通り `Mcp-Session-Id` を発行して 200。
- **Deprecation ヘッダ**: 旧経路応答に付与（3.6）。
- **18（+1）tools の I/O 定義は無変更**。ツール実装コード（`queen.rs`）に手を入れず、`queen_compat.rs` がヘッダとメタだけ翻訳する。

### 5.2 Tauri Command（新設）

| command | args | returns | 説明 |
|---|---|---|---|
| `query_spans` | `{ sessionId?: number, traceId?: string, sinceNs?: number, limit?: number }` | `Span[]` | Waterfall / cost breakdown 用の SQLite クエリ。frontend の open-code SQL を排し、read only の投影に限定 |
| `set_capture_content` | `{ enabled: boolean }` | `void` | 実行時に prompt 本体キャプチャを toggle。config の `capture_content` を上書き（永続化はしない、次回起動で config 値へ戻る） |

read-only。既存 `list_sessions` / `read_output` / `spawn_agent` などは**一切変更しない**（[spec-agent-status.md](../spec-agent-status.md) §8.3 の非回帰と同じ扱い）。

### 5.3 Tauri Event（新設）

| event | payload | 説明 |
|---|---|---|
| `agent-cost` | `{ id, model, costUmicro, inputTok, outputTok, ttftMs? }` | `gen_ai.chat` span 終了時に 1 回 emit（3.4） |
| `trace-updated` | `{ traceId: string, sessionId?: number }` | 新しい root span が SQLite に書かれたときの軽い通知。frontend の waterfall が「更新あり」を検出する用 |

**新しい event を emit するのはこの2つだけ**。既存 `pty-output` / `pty-exit` / `session-state` / `agent-status` / `config-changed` / `session-resources` / `queen-notify` は**すべて不変**。

### 5.4 型（TS 相当）

```ts
export type Span = {
  traceId: string;                // 32 hex
  spanId: string;                 // 16 hex
  parentId?: string;
  name: string;
  kind: "server" | "client" | "internal" | "producer" | "consumer";
  startNs: number;
  endNs: number;
  sessionId?: number;
  agent?: string;
  model?: string;
  inputTok?: number;
  outputTok?: number;
  costUmicro?: number;
  attrs?: Record<string, unknown>;
  events?: { name: string; timeNs: number; attrs: Record<string, unknown> }[];
};
```

### 5.5 破壊的でないこと

- Phase 3.x〜4.4 の全 CONTRACT 断面は**追加のみ**で維持。
- 旧 MCP クライアントは `mcp.legacy_2025_06: true` の間は無改修で動く。
- `observability.enabled: false`（既定）では新規テーブルの作成もイベントの emit も**一切起きない**（[spec-notifications.md](../spec-notifications.md) の opt-in と同じ姿勢）。

---

## 6. エッジケース / 相互作用

### 6.1 `agent-status` との重複回避

Ring 色は `AgentStatus` を**そのまま**引く。新しい状態モデルを作らず、`spec-agent-status.md` §2 の 5 値のみを表す。**Ring は事実の再表示であって新たな判定はしない**。したがって `agent_status.enabled: false` のとき Ring も自動で色を落として `unknown`（透明）扱い。

### 6.2 通知（[spec-notifications.md](../spec-notifications.md)）との共存

- コスト超過通知（例「10 分で $5 を超えた」）は **本仕様で新設しない**。通知は既存の 4 イベント（`error`/`needs-attention`/`complete`/`progress`）に閉じる。将来 `progress` の供給源として cost threshold を足す余地があるが（[spec-notifications.md](../spec-notifications.md) §9）、Phase 5.5 では出さない。
- 「握り潰さない」原則との整合: OTel エクスポート失敗（OTLP endpoint に届かない）は backend ログのみ、UI に出さない。ただし **SQLite シンクの書き込み失敗**は `queen-notify` 経路で1回だけトーストする（データ欠損は誤解の温床なので）。

### 6.3 `session-resources` との重複回避

CPU/RSS/git dirty は既存 `session-resources` で 2 秒に 1 度サンプルされている。Ring はこの既存値を**再利用**し、新しい polling を足さない（Phase 3.7 の resource 監視がすでに解決している問題を再解決しない）。

### 6.4 host teammate / observe transcript

- host teammate（Phase 4.2）の実 PTY もペインなので Ring を出す。**lead と teammate は独立**にリング色を持つ（lead の `AgentStatus` を子に持ち込まない、[spec-agent-status.md](../spec-agent-status.md) §11 host teammate 節参照）。
- observe transcript（Phase 4.1）は PTY を持たないので **Ring 色は `unknown`（薄グレー枠）**、バッジは `inbox` のみ表示（cost / git dirty は無意味）。

### 6.5 config reload

- `observability.enabled` を false → true にしたら、次のトレース木から採取が始まる（過去は取れない、原理上）。
- `pricing.models` の差分は**次回の `gen_ai.chat` span 終了から反映**。過去 span の `cost_umicro` は再計算しない（履歴の書き換えを避ける）。
- `mcp.rc_2026_07_28` を false → true に切り替えても既存の TCP コネクションはそのまま動く（判定は request 単位）。

### 6.6 プロンプトのプライバシー

`capture_content: true` かつ `capture_encrypt_key` 設定時のみ span events を保存する。暗号化は AES-256-GCM、鍵は `~/.ptygrid/observability.key`（0600）or config の `${VAR}` 展開で env 注入。**平文でディスクに書く選択肢は仕様上存在しない**（4.3 のバリデーションで拒否）。

### 6.7 MCP session-less 化と await

Queen `await` tool は「呼び出し中スレッドが長時間 blocking する」性格の tool で、旧 stateful 経路では 1 コネクション占有だった。RC 経路でも仕様上は独立 POST だが、**streamable HTTP のロングポーリング**で挙動は同等（RC は「stateless」でも「short-lived」ではない、byteiota 記事参照）。したがって `await` は無改修で動くが、コネクション上限は要監視項目としてリリースノートに明記する。

---

## 7. テスト計画

### 7.1 純関数・ロジックテスト（`cargo test`）

- **ヘッダ / メソッド一致**: `Mcp-Method` と body `method` の不一致を 400 で拒否。一致なら受理。`Mcp-Name` の欠落は `tools/*` メソッドのみ 400、`ping` は 200。
- **旧 / RC 両立ルータ**: 同一エンドポイントに旧クライアント（`initialize` → `Mcp-Session-Id` 発行期待）と RC クライアント（`_meta.traceparent` + `Mcp-Method`）を交互に投げて双方 200 になること。混線しないこと。
- **Deprecation 発火条件**: RC 経路応答に `Deprecation` 無し、旧経路にのみ有り。ログの per-day dedupe が効くこと。
- **traceparent 伝搬**: 受け取った `traceparent` の `trace_id` が `queen.tool` span の trace_id に一致し、応答 `_meta.traceparent` の `parent_id` が `queen.tool` span_id と一致すること。
- **コスト計算**: `(system, model, in_tok, out_tok)` から μUSD 整数値への写像がテーブル通り。キャッシュヒット時のディスカウント適用。unpriced（テーブル外）は None を返し、非表示になること。
- **通貨変換**: `pricing.currency: JPY` かつ `fx_rate.JPY: 158.0` で表示文字列が期待通り。fx_rate 欠落時 USD へフォールバック。
- **SQLite 保持**: `retention_days: 30` で古い span が起動時と 24h ごとに DELETE される（GC バッチのユニットテスト）。
- **`capture_content` バリデーション**: 暗号鍵欠落で config load が **Err** を返す（他のフィールドと違い load を通さない、4.3）。

### 7.2 統合テスト（backend）

- モック OTLP endpoint に対して 100 span を投げ、gzip payload と semconv attribute のキー名を検証。
- rmcp との共存: 既存 18 tools を旧 / RC 両経路から呼び、レスポンス形状が同一であること。

### 7.3 Frontend（`svelte-check` + 単体可能なら）

- `agent-cost` 受信で Ring cost バッジが `pricing.currency` の表記になる。5 分ウィンドウを跨いだ古いイベントは集計から落ちる。
- Ring の状態パターン（実線 / 破線 / 二重線 / 細線）が `AgentStatus` に対応。`prefers-reduced-motion` でパルスが止まる。
- `query_spans` の waterfall レンダリングが 1000 span でも 60fps を割らない（虚数値でスクロールテスト）。

### 7.4 アクセシビリティ検証

- WCAG AA コントラスト比: Ring 色 4 種 × light/dark テーマ で自動アサート（axe-core 相当を CI 化）。
- 色覚シミュレーション（deuteranopia / protanopia）で 4 状態が判別可能なことをスクリーンショット diff で確認。

### 7.5 実機手動検証（macOS 必須 / Linux ベストエフォート）

1. Claude Code から Queen tool を呼び、`traceparent` が SQLite の spans に載る。
2. `spawn_team` で 3 エージェント起動 → 各ペインの Ring が起動〜working〜idle と遷移。
3. OTLP endpoint（例 Jaeger）へ 10 分流し、trace tree が `agent.turn → gen_ai.chat + queen.tool` の形で見える。
4. `capture_content: false` の間、spans の events_json が空であること（プロンプトが漏れない）。
5. 旧仕様の MCP クライアントで tools が動く（互換確認）。RC 対応クライアントでも動く（両立確認）。
6. `pricing.currency: JPY` で Ring cost バッジが「¥…」表記になる。
7. blocked ペインで Ring が実線赤 + inbox バッジ（未読ある場合）+ git dirty 円が同時に出て、判別可能（1m 距離）。

---

## 8. 設計判断（採用しない案とその理由）

- **OpenTelemetry Rust の代わりに独自軽量計装**: 却下。gen_ai semconv は流動的で、追随を独自コードでやると人手が持たない。opentelemetry-rust 依存は build 時間 +15 秒程度で受容可能。
- **gRPC OTLP**: 却下。tonic + hyper + h2 が依存ツリーを著しく肥大させる。HTTP/protobuf で機能・性能とも同等。
- **Prometheus / metrics 系のみで cost を扱う**: 却下。**因果**（どの turn がどのモデルを呼んだか）が失われる。metrics は集計指標として二次的に足す余地はある（将来）。
- **rmcp の旧経路を即撤去**: 却下。RC は 12 ヶ月 window を明示している。ユーザーの Claude Code / Codex が追随するまでの期間、片側だけを動かすと fragmentation が起きる。両立ルータで一段吸収する。
- **`initialize` の負担を残すため RC でも `Mcp-Session-Id` を出す**: 却下。RC 準拠クライアントは Session-Id を無視するし、こちらから発行するとロードバランサ経路で誤ってスティッキーが復活しうる。仕様通り「発行しない」。
- **プロンプト本体を attribute に載せる**（OTel の一部 SDK 慣習）: 却下。GenAI semconv は **span event として opt-in** を推奨。attribute はサンプリング / エクスポートで欠落しにくく高価値だが、機密の温床。仕様通り event 側に閉じる。
- **cost 超過通知を本仕様で足す**: 却下。通知の粒度設計は [spec-notifications.md](../spec-notifications.md) の 4 イベントで完結させたい。「別レイヤの雑音を通知に混ぜない」原則を維持。将来 progress イベント源として cost を足す余地は残す。
- **Ring を CSS ではなく Canvas で描く**: 却下。CSS の box-shadow + border で十分。Canvas は accessibility ツリーから落ちる。
- **色だけで状態を区別**: 却下。色覚多様性配慮でパターン（実線 / 破線 / 二重線 / 細線）を併用（3.7）。
- **リモート pricing 更新（自動）**: 却下。オフライン方針を維持。[spec-agent-status.md](../spec-agent-status.md) §11「パターン陳腐化」と同じで、内蔵デフォルト + config 上書きで運用する。fx レートも手動固定。
- **OAuth 2.1 / OIDC を Phase 5.5 で実装**: 却下。ptygrid は 127.0.0.1 バインド + token（Phase 4.3 で永続化済み）で完結しており、OAuth の恩恵が薄い。RC の OAuth 硬化は将来リモート実行モードへ拡張したときの土台として位置付ける（本仕様は「対応可能性を明記」まで）。
- **Roots / Sampling / Logging を即撤去**: 却下。12 ヶ月 window に従い受理は残す。ptygrid は Roots も Sampling も未使用のため実害はないが、旧 SDK クライアントの互換のために No-op で受ける。

---

## 9. リリース段階分割案

[plan.md](../plan.md) の流儀（y=Phase 番号 = `0.5.z`、CONTRACT 先行追記、両プラットフォーム CI、1 リリース = 1 patch）に従い、**Phase 5.5** を 5 段に分ける。

### Phase 5.5.0 — MCP RC 両立ルータ（backend のみ、UI 変化なし）

- `queen_compat.rs`: `Mcp-Method` / `Mcp-Name` 受理・整合検証・旧経路の `Mcp-Session-Id` 維持・Deprecation ヘッダ付与。
- `_meta.traceparent` の受け取りと応答注入（この時点では OTel エクスポートはまだ無効、span は落とすだけ）。
- ptygrid.yml に `mcp:` ブロック追加、既定は「両立 ON」。
- **completion gate**: `cargo test`（新設ルータ単体）・実機で Claude Code 旧クライアント / RC クライアント両方が動く・CONTRACT に §5.1 を先行追記・既存 18 tools 非回帰。

### Phase 5.5.1 — OTel 計装と SQLite シンク（データ流通の開通）

- `observability.rs`: opentelemetry-rust 導入、SDK 初期化、SQLite `spans` テーブル DDL、batch writer。
- `queen.tool` span の生成（親 = `_meta.traceparent`）、`agent.turn` の hook 経路（Phase 4.0 の teammate hooks 側から `input_tokens` を受ける契約）。
- ptygrid.yml `observability:` 追加。既定 `enabled: false`。
- **completion gate**: `capture_content` バリデーションテスト、モック OTLP へのエクスポート、SQLite の retention GC、CONTRACT §5.2 の `query_spans` 先行追記、既存 event 非回帰。

### Phase 5.5.2 — Cost 計算と `agent-cost` イベント

- `cost.rs`: 内蔵 pricing、`ptygrid.yml` merge/replace、通貨変換、μUSD 型。
- `gen_ai.chat` span 終了時の cost 属性付与 + `agent-cost` emit。
- frontend: session cost の生値 store（Ring はまだ出さない、この段では数値をヘッダのデバッグ領域に出すだけ）。
- **completion gate**: cost 計算純関数テスト、unpriced が非表示になること、fx_rate 反映、通貨表記、CONTRACT §5.3 `agent-cost` 先行追記。

### Phase 5.5.3 — Agent Status Rings（frontend 中心）

- `status_ring.svelte.ts` + CSS: 状態リング（色 × 形パターン）+ 3 バッジトレイ + アクセシビリティ配慮。
- Ring は `agent_status.enabled` + `ui.ring.enabled` 両方 true のときのみ描画。
- 色覚シミュレーションとコントラスト自動アサートを CI に組み込む。
- **completion gate**: `svelte-check`/build、axe-core（WCAG AA）通過、reduced-motion 尊重、9 面時の視認性実機確認（1m 距離）、userguide に「Status Ring」節。

### Phase 5.5.4 — Trace Waterfall + Cost Dashboard UI

- 新規サイドパネル（既存の左サイドバーとは別 tab or 別画面）: waterfall（`Span[]` を `query_spans` で引く）、per-agent cost breakdown、24h cost 集計。
- backend 変更なし（既存の SQLite と `query_spans` を使うだけ）。
- **completion gate**: 1000 span でスクロールがガタつかない（60fps 目標）、`trace-updated` イベントで自動リフレッシュ、CSV エクスポート（コスト精算用途）、userguide に「Cost Dashboard」節。

### バージョン割り当て

| バージョン | 内容 |
|---|---|
| `v0.5.6` | Phase 5.5.0（MCP RC 両立ルータ） |
| `v0.5.7` | Phase 5.5.1（OTel + SQLite） |
| `v0.5.8` | Phase 5.5.2（Cost 計算 + agent-cost） |
| `v0.5.9` | Phase 5.5.3（Status Rings） |
| `v0.5.10` | Phase 5.5.4（Waterfall + Dashboard） |
| `v0.5.11〜` | 残 Defer 消化・OAuth 2.1 拡張の実験・pricing 表の 3 ヶ月ごとの更新(Phase 6 系突入で v0.6.0 へ minor bump) |

> Phase 5.0(Orchestrated) の `v0.5.0〜v0.5.5` に続く連番。Phase 5 系(=v0.5.z)の間は連番で埋め、Phase 6.0(Secure) 突入時に `v0.6.0` へ minor bump する。

### Phase 5.5 完了の条件（macro completion gate）

- **旧 MCP クライアント / RC クライアント両方**での実運用（Claude Code / Codex）検証が終わる
- `capture_content: true` を含むフルセットで **7 日間**の連続稼働を1回通す（SQLite の膨張・GC・OTLP 通信のリークを確認）
- pricing 表が **リリース時点の実勢値**と 1 週間以内のズレ以内
- CONTRACT.md の Phase 5.5 追加断面（§5.1〜§5.4）が確定
- userguide に「Observability」「Cost Dashboard」「Status Ring」の 3 節が入る

---

## 10. リスクと未解決事項

- **RC 仕様の最終確定タイミング**: 2026-07-28 RC は 10 週間の validation window 中に細部が動きうる。Phase 5.5.0 の実装凍結は **仕様最終化を待って**行う（早出しはしない）。
- **rmcp 側の RC 対応の実状**: rmcp が RC を公式サポートする前に本仕様が動く場合、`queen_compat.rs` で薄く自前パースする覚悟が要る。純粋な axum ハンドラで完結するよう設計しておく（3.1 の意図）。
- **OTel Rust の semconv バージョン追随**: 1.36+ が `gen_ai.time_to_first_token_ms` を持つが、attribute 名は 1.37 で改名されうる。エクスポート時にキー名を集中管理する `sem::GEN_AI_*` const module を作り、追随コストを局所化する。
- **pricing の陳腐化**: [spec-agent-status.md](../spec-agent-status.md) §11 と同じ課題。内蔵デフォルトはリリースごとに更新 + ユーザー上書き前提。将来「pricing 更新チャンネル」（署名付き・手動取得）を足す余地を残す。
- **多エージェント PTY で `agent.turn` の帰属が取れないケース**: hook を投げない CLI（旧 codex の一部モード等）ではモデル呼び出し数・トークン数が観測できない。3.5 で「黙る」方針だが、Ring cost が長期に「無表示」だとユーザーは「壊れている」と感じる。userguide に「未対応 CLI では cost バッジは出ません」を明記する。
- **1m 距離視認**: 9 面時のフォントサイズと Ring 太さの実測は macOS 27" と 14" MBA で並行検証する必要がある。CI では代替できない。
- **OAuth 2.1 硬化の実装余地**: RC は `iss` 検証などを要求する。本仕様は 127.0.0.1 バインドで恩恵薄だが、将来のリモート実行モード（design.md §1 の「対象外」欄にある将来課題）に備えて `queen_compat.rs` の auth 層を差し替え可能な trait にしておく。

---

## 11. 参考

- Model Context Protocol Blog, [The 2026-07-28 MCP Specification Release Candidate](https://blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate/) — stateless 化、`Mcp-Method` / `Mcp-Name` 必須化、`_meta` の trace context、12 ヶ月 deprecation window。本仕様 M1 の一次資料。
- [MCP 2026-07-28 spec: what changed, what breaks (Stacktree)](https://stacktr.ee/blog/mcp-2026-spec-changes) — 破壊的変更の網羅、既存 SDK からの移行ケース。
- byteiota, [MCP Goes Stateless: The 2026 Release Candidate Explained](https://byteiota.com/mcp-goes-stateless-2026-release-candidate/) — long-polling / streamable HTTP の挙動整理、`await` 系 tool の影響。
- 4sysops, [2026-07-28 MCP: stateless, multi-round-trip, routable headers, authorization hardening](https://4sysops.com/archives/2026-07-28-model-context-protocol-mcp-stateless-multi-round-trip-routable-headers-authorization-hardening/) — 認可強化と `iss` 検証。
- OpenTelemetry Blog, [Inside the LLM Call: GenAI Observability with OpenTelemetry](https://opentelemetry.io/blog/2026/genai-observability/) — `gen_ai.usage.input_tokens` / `output_tokens`、`gen_ai.system_instructions` / `input.messages` / `output.messages` の opt-in、`gen_ai.client.token.usage` / `client.operation.duration`。本仕様 M2 の一次資料。
- Greptime, [How OpenTelemetry Traces LLM Calls, Agent Reasoning, and MCP Tools](https://greptime.com/blogs/2026-05-09-opentelemetry-genai-semantic-conventions) — MCP tool を子 span にする現行の慣例。
- Uptrace, [OpenTelemetry for AI Systems: LLM and Agent Observability (2026)](https://uptrace.dev/blog/opentelemetry-ai-systems) — 実装レイヤ選定（HTTP/protobuf vs gRPC）の判断材料。
- Finout, [Anthropic API Pricing in 2026](https://www.finout.io/blog/anthropic-api-pricing) / [Claude Opus 4.7 Pricing 2026](https://www.finout.io/blog/claude-opus-4.7-pricing-the-real-cost-story-behind-the-unchanged-price-tag) — 内蔵 pricing table の一次値。
- TLDL, [Anthropic API Pricing (July 2026)](https://www.tldl.io/resources/anthropic-api-pricing) — Opus 4.8 / Sonnet 5 の per-1M 単価スナップショット。
- Shuttle, [How to Build a Streamable HTTP MCP Server in Rust](https://www.shuttle.dev/blog/2025/10/29/stream-http-mcp) — rmcp `streamable-http` の実装パターン（`queen_compat.rs` のベース）。
- [docs/spec-agent-status.md](../spec-agent-status.md) — `AgentStatus` 5 値、hot path 分離、config-as-code の merge/replace 規約、blocked 保守主義。Ring 色の供給源。
- [docs/spec-notifications.md](../spec-notifications.md) — opt-in / エラー握り潰さない原則、ureq デタッチ送信の設計。cost 通知を本仕様に混ぜない根拠。
- [docs/design.md](../design.md) — hot path 分離・config-as-code・推測回避。
- [docs/plan.md](../plan.md) — バージョニング規約（1 リリース = 1 patch）。
- [CONTRACT.md](../../CONTRACT.md) — 既存 IPC / MCP 契約。本仕様の追記先。
