# Phase 5.0 / 5.5 / 6.0 staged delivery plan

Last updated: 2026-07-22. Phase 3.0 through 3.9 are complete; Phase 4.0-4.4 の teammate hooks / observe / host / agent-status / team_presets は既存(`spec-agent-status.md` / `spec-notifications.md` / `spec-team-presets.md` / `spec-claude-teams-panes.md`)。本ドキュメントは **Phase 5.0「Orchestrated & Remembering」→ Phase 5.5「Observable & Standards-Compliant」→ Phase 6.0「Secure & Auditable」** の3フェーズを、Phase 3 と同じ「独立に release 可能な小粒 patch の連なり」として展開する。各仕様の詳細は [spec-phase5-0.md](spec-phase5-0.md) / [spec-phase5-5.md](spec-phase5-5.md) / [spec-phase6-0.md](spec-phase6-0.md) を参照。

**実装順の根拠**: Phase 5.0 の Orchestrator を MVO(Minimum Viable Orchestrator = pipeline + fan-out + `spawn_workflow`/`join`/`cancel_workflow` の3 tools + WorkflowPanel)として最初に切り出す。これができた瞬間から Track A/B/C/D を並列に走らせて Phase 5.5 / 6.0 を回す。ptygrid 自体をドッグフーディングして開発体験そのものを検証する。

いずれの release も Phase 0〜4.4 の IPC 契約を保存する。追加契約はその release 自身の `CONTRACT.md` 節でのみ拡張する。Rust と frontend の両 CI を通し、両プラットフォーム(macOS / Linux)で build する。

---

## Phase 5.0 — Orchestrated & Remembering

**先行実装 Phase**。MVO(5.0.0-bootstrap = 5.0.3 の pipeline サブセット + `spawn_workflow`/`join`/`cancel_workflow`)を約 2 週間で切り出し、以降の並列開発の土台にする。

| Release | Scope | Completion gate |
|---|---|---|
| 5.0.0 | **MVO(bootstrap)** — pipeline + fan-out 2 パターン、`spawn_workflow`/`join`/`cancel_workflow` 3 tools、WorkflowPanel(最小)、`workflow_runs` テーブル(user_version v3 骨のみ) | 自分自身の spec 執筆 workflow を pipeline で走らせて動く、既存 team_presets 非回帰 |
| 5.0.1 | Memory 保存経路(embedding 抜き) — `queen.sqlite3` v2→v3 migration 本体、`memory` + FTS5、`memory.remember` / `forget` / `list` | migration 冪等、既存 pins/notes/inbox 非回帰、暗号化オプション skeleton |
| 5.0.2 | Memory embedding + ハイブリッド検索 — `memory_embed.rs` trait、sqlite-vec 起動時 load、`memory_vec`、RRF、`memory.reindex` | fake backend の end-to-end、次元不一致 error、Ollama 手動検証、MemoryPanel(recall UI) |
| 5.0.3 | Provider 統合基盤 — `providers:` スキーマ、`ProviderStatus`、`agents[].provider` + env 注入 | Ollama/LM Studio/Jan ヘルスチェック、mock サーバで env 注入テスト、既存 agent env が provider 由来を上書き |
| 5.0.4 | Orchestrator: supervisor + handoff + retry/timeout — MVO の pipeline/fan-out を完成形へ、`join_on: all/any/n`、handoff pattern | DAG 循環検出、retry / timeout、fail-fast / continue、`any` 時の残 fan-out CANCELLED、inbox generation watch 動作 |
| 5.0.5 | Arena view(S7 前半)— `arena.rs` + `Arena.svelte`、`arena-open`、`arena.vote` / `arena.list_votes`、Diff viewer | fan-out workflow から自動起動、vote が memory(project scope) に記録、Merge は含めない |

**Phase 5.0 完了ゲート**: 4 パターン(pipeline / fan-out / supervisor / handoff)の workflow を実運用で走らせ、`workflow-state` イベントの遷移列が仕様通り。memory を跨いだ recall がプロジェクト境界を越えない。Arena vote が正しく `episodic` として保存される。

**バージョン割当(暫定)**: `v0.5.0`(5.0.0 MVO) → `v0.5.5`(5.0.5 Arena)。

## Phase 5.5 — Observable & Standards-Compliant

Track B(MCP + OTel)+ Track A の一部(Rings + Waterfall)で並列展開。

| Release | Scope | Completion gate |
|---|---|---|
| 5.5.0 | MCP 2026-07-28 RC 両立ルータ(`queen_compat.rs`)、`Mcp-Method` / `Mcp-Name` 受理、`_meta.traceparent` 受け入れ、旧経路 `Mcp-Session-Id` 維持 + Deprecation ヘッダ | 旧 / RC 両クライアントで既存 tools が動く。既存 tool 実装 (`queen.rs`) は無改修。CONTRACT §5.5.1 先行追記 |
| 5.5.1 | `observability.rs` 導入(opentelemetry-rust 1.36+)、SQLite `spans` テーブル、`queen.tool` / `agent.turn` span、hook 由来トークン数 | mock OTLP へのエクスポート、retention GC、`capture_content` バリデーション、CONTRACT §5.5.2 `query_spans` 追記 |
| 5.5.2 | `cost.rs`(内蔵 pricing + `ptygrid.yml` merge/replace)、通貨変換、`gen_ai.chat` 終了で `agent-cost` イベント emit | 純関数コスト計算、unpriced 非表示、fx_rate 反映、CONTRACT §5.5.3 `agent-cost` 追記 |
| 5.5.3 | Agent Status Rings(`status_ring.svelte.ts` + CSS)、状態色 × 形パターン + 3 バッジトレイ、色覚配慮 + reduced-motion 尊重 | axe-core(WCAG AA)通過、9 面 1m 距離視認、userguide "Status Ring" 節 |
| 5.5.4 | Trace Waterfall + Cost Dashboard(サイドパネル)、per-agent breakdown、24h 集計、CSV エクスポート | 1000 span 60fps、`trace-updated` 自動リフレッシュ、userguide "Cost Dashboard" 節 |

**Phase 5.5 完了ゲート**: 旧 MCP クライアント / RC クライアント両方で7日連続稼働、pricing 表が実勢値と 1 週間以内のズレ以内、CONTRACT §5.5 が確定、userguide に 3 節追加。

**バージョン割当(暫定)**: `v0.5.6`(5.5.0) → `v0.5.10`(5.5.4)。Phase 5.0 の v0.5.0-5 に続く連番。

## Phase 6.0 — Secure & Auditable

Track D 主導。sandbox strict(6.0.2)が全 Phase を通じて最大の不確実性項目。

| Release | Scope | Completion gate |
|---|---|---|
| 6.0.0 | Foundation — `sandbox.rs` / `secrets.rs` / `replay.rs` 骨格、SQLite `replays` / `secrets_audit` / `sandbox_events` migration(user_version v4)、Queen tool は空実装 | CI に 3 バイナリ link、既存機能非回帰 |
| 6.0.1 | Sandbox: `filesystem-only` — Linux bwrap / macOS sandbox-exec fallback、プロファイル解決、`sandbox.info` tool | 統合テスト T-504 の一部(step 単位で filesystem-only 隔離) |
| 6.0.2 | Sandbox: `strict` — Firecracker + gVisor(Linux)、Virtualization.framework(macOS)、ウォームプール、vsock queen_relay | 統合テスト T-501, T-504 全通、p95 起動時間 < 100ms(warm hit) |
| 6.0.3 | Secrets: keychain + `short_lived` — `secrets.get` / `secrets.revoke`、`secrets_audit` 記録、OTel span 化(Phase 5.5 と結合) | 統合テスト T-502(fail-close で env fallback しない) |
| 6.0.4 | Secrets: `derived` + HTTP proxy — Infisical / Vault backend、`proxy.rs`(strict 内 MITM)、`service_rules` | redteam T セット通過、prompt injection で実キー漏洩しない |
| 6.0.5 | Replay UI + Export — Svelte 5 timeline、`replay_open` / `replay_export`、cast / mp4 export、Phase 5.0 workflow の step 単位 record 切替 UI | T-503(replay ↔ span 結合)、`m` マーカー ±0ms 精度、`record: false` 区間バナー |

**Phase 6.0 完了ゲート**: 4 プロファイル × 3 OS(Linux / macOS / Windows-limited)の解決テーブルが全通、prompt-injection redteam で実キー流出ゼロ、7 日連続稼働で secrets_audit の欠落なし、replay `.cast` の SHA256 が sandbox_events に記録され改竄検出可能。

**バージョン割当(暫定)**: `v0.6.0`(6.0.0) → `v0.6.5`(6.0.5)。**6.0.5 完了時に v1.0.0 昇格を検討**("Secure & Auditable" 到達がメジャー 1.0 の妥当な基準)。

---

## 固定された設計判断(Fixed design decisions)

- **既存 `spawn_agent` 許可リストは全 Phase を通じて非破壊**。Phase 5.0 の workflow / Phase 6.0 の sandbox でも spawn できるのは `agents:` に宣言された名前だけ。
- **project 境界の統一**: pins / notes / inbox / memory / workflow 実行履歴 / replay / secrets_audit は同じ canonical config directory でスコープする。
- **opt-in が既定**: Phase 5.5 `observability.enabled: false` / Phase 5.0 `queen.memory.enabled: true` but empty by default / Phase 6.0 `sandbox.default_profile: filesystem-only`(off ではなく)。既存挙動を勝手に変えない。
- **hot path 分離**: PTY reader スレッドから、orchestrator の状態機械・embedding 計算・OTel export・redaction stream すべて物理的に分離する。
- **オフライン方針**: pricing 表・FX レート・memory embedding・sandbox kernel いずれも外部 API に自動問合せしない。ユーザーの手動更新 + 内蔵デフォルトで運用する。
- **CONTRACT.md は additive のみ**: どの patch も既存契約を破壊せず、`# Phase 5.0 追加契約` / `# Phase 5.5 追加契約` / `# Phase 6.0 追加契約` 節で拡張する。前 Phase と競合したら後 Phase を優先。
- **UI と backend の分離**: WorkflowPanel / MemoryPanel / Ring / ReplayViewer / SandboxBadge / Arena の frontend 純増 patch(5.0.0/5.0.2/5.0.5 / 5.5.3/5.5.4 / 6.0.5)は backend 変更を含まない。
- **SQLite `PRAGMA user_version` の bump**: 2 → 3(Phase 5.0.0 workflow_runs + 5.0.1 memory)→ 4(Phase 6.0.0 replay/secrets/sandbox)。未知の新 version は黙って開かない(Phase 3.6 の規律を継承)。

---

## Release discipline

Phase 3 の [phase3.md](phase3.md) と同じ流儀を継続する。各 release で:

1. Add the precise backend/frontend contract to `CONTRACT.md`(該当 Phase 節に追記)。
2. Keep new service logic outside `lib.rs` and the session hot path(`sandbox.rs` / `secrets.rs` / `orchestrator.rs` / `memory.rs` / `observability.rs` / `provider.rs` などのモジュール分割)。
3. Add pure unit tests for parsing / state transitions and focused integration tests for external process behavior(vault backend / OTLP exporter / sandbox VMM 起動 は mock)。
4. Run `cargo test`, `cargo check`, `cargo clippy --all-targets --all-features`, `npm run check`, `npm run build`。
5. Update the user guide only for behavior included in that release。
6. `docs/CHANGELOG.md` に entry を追加(Phase 6.0.0 以降は必須化)。

## 並列開発トラック(MVO 完成後)

MVO(5.0.0)が動いた瞬間から `ptygrid.yml` の `workflows:` セクションで 4 Track を並列に走らせる:

| Track | 内容 | 対応 patch |
|---|---|---|
| **A: UI** | Svelte 5 中心の frontend 純増 | 5.5.3 Rings / 5.5.4 Waterfall / 5.0.5 Arena / 6.0.5 Replay UI |
| **B: MCP + 観測** | MCP RC 追随 + OTel + Cost | 5.5.0 / 5.5.1 / 5.5.2 |
| **C: Memory + Provider + Orchestrator 完成** | Track B と独立、Phase 5.0 の完成 | 5.0.1 / 5.0.2 / 5.0.3 / 5.0.4 |
| **D: Security** | sandbox + secrets + replay backend | 6.0.0 / 6.0.1 / 6.0.2 / 6.0.3 / 6.0.4 |

各 Track 内は `pipeline` workflow(design → implement → verify → docs)、Track D は `verify → redteam → docs` の5段。詳細は `ptygrid.yml` の `workflows:` セクション。

## 参照仕様

- [spec-phase5-0.md](spec-phase5-0.md) — 宣言的 DAG / Supervisor、共有セマンティックメモリ、Local-first Provider、AI Arena(前半)
- [spec-phase5-5.md](spec-phase5-5.md) — MCP 2026-07-28 RC 追随、OpenTelemetry GenAI、Agent Status Rings
- [spec-phase6-0.md](spec-phase6-0.md) — Sandboxed Execution Pane、Credential Proxy、Session Replay
- [phase3.md](phase3.md) — 前段(Phase 3.0〜3.9 完了)の流儀
- [competitive-landscape.md](../competitive-landscape.md) / [research/competitive-2026-07-22.md](../research/competitive-2026-07-22.md) — 本ロードマップの背景となる 2026 年競合調査
