# ptygrid 作業計画 (plan.md)

更新日: 2026-08-04 / 実装基準: `main`（PR #10 のあと PR #11 / #12 = `17860e0` / `b8300a4` が
マージされ、`.gitignore` 追加の `89411b9` が直接コミットされている）。**2026-08-04 時点の訂正**:
`feat/terminal-copy-paste` は PR #13 として `main` にマージ済み（`main` = `5c43019`）、
`feat/step-timing-5.0.6` は origin へ push 済みで `main` より 9 コミット先行（PR は未マージ）。
本文中に残る「push 未・PR 未作成」の記述はこの日より前のもので、§4 項目 1 / 項目 7 と
§6.10 / §6.11 に該当箇所がある。作業中のブランチは `feat/step-timing-5.0.6`。
**2026-08-13 時点の訂正**: 最新タグは `v0.5.8`（2026-08-13 作成 → §4・§6.23）。
`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` の 3 ファイルを
`0.5.8` に揃えたコミットに打った。**タグ `v0.5.7` が指すコミットは 3 ファイルとも `0.5.6` の
ままである**（→ §4 の項目 6）ため、v0.5.7 断面の version 表示だけは実態とずれている。

**この文書の読み方**: 「結局どこまで終わって何が残っているか」は §1 の通し進捗表 1 本で足りる。
状態を書くのは §1 だけ、実機検証の詳細は §2、タグとバージョンは §4、日付つきの経緯は §6 にしか
書かない。同じ事実を複数箇所に持つとどれが最新か分からなくなるため、この分担を規律とする。

**状態記号（3 つだけ）**: **✅ 完了**（ソースがあり自動テストが通り、実機側の積み残しが明記されて
いない）/ **🚧 一部完了**（ソースはあり自動テストは通るが実機検証などが未消化 → §2 の項番）/
**⬜ 未着手**（対応ソースが存在しない。spec だけがある、または欠番）。

関連文書: Phase 3.x の詳細実績は `docs/inside/phase3.md`（git 管理外）、wire 契約は
[CONTRACT.md](../../CONTRACT.md)、config フィールド単位の実装状況は
[ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md) §1、teams 設計は
[spec-claude-teams-panes.md](../spec/spec-claude-teams-panes.md)、方向性の背景は
[competitive-landscape.md](competitive-landscape.md)。

---

## 1. 通し進捗表

Phase 0 から 6.0 までを 1 本の表にした（時系列かつ patch 番号順）。**この文書で状態を宣言するのは
この表だけ**。「リリース」列はタグ名のみを持ち内容は §4、「実機検証」列の `(U*)` は §2 の項番。

| 番号 | 内容 | 状態 | リリース | 実機検証 |
|---|---|---|---|---|
| 0 | 単一 PTY ペイン（`pty.rs`） | ✅ | — | 記録なし |
| 1 | マルチペイン + config-as-code（現 `ptygrid.yml`）、autostart / restart | ✅ | — | 記録なし |
| 2〜2.1 | Queen（内蔵 MCP サーバー、基本 5 tools）+ ドッグフーディング反映 | ✅ | — | 記録なし |
| 3.0〜3.8 | Git status/diff/stage/commit、opt-in worktree 分離、logical resume、リソース監視、Queen pins/notes/inbox/reply/await（18 tools）、SQLite `user_version` 1→2 | ✅ | — | 記録なし |
| 3.9 | Linux テスト対応（PATH 復元、Ubuntu CI、`.deb` / AppImage） | 🚧 | — | CI 済 / 実機常用は未（U7） |
| 4.0 | teammate hooks 受信基盤（`/hooks/v1/*`、token 認可、toast、Teammates バッジ、`teammates:` ブロック、settings.json 半自動登録） | ✅ | v0.4.2 | 記録なし |
| 4.1 | observe: `transcript` ペイン種別（PTY なし論理セッション）、SubagentStart で read-only tail 自動生成、`agents[].teams` | ✅ | v0.4.2 | 記録なし |
| 4.2 | host: tmux 互換シム + per-lead Unix socket RPC 配線、env/PATH 注入、実 PTY teammate ペイン、フォールバック検知→observe 降格、frontend 一式 | 🚧 | v0.4.2 | 未（U6） |
| 4.3 | Queen team preset（`team_presets:` + Queen tool `spawn_team`（19 本目）+ 👥 一括起動 UI + example/team-preset） | ✅ | v0.4.6 | 記録なし |
| 4.4.0〜4.4.1 | エージェント意味的状態の検出（working / blocked / done / idle）+ 左ステータスサイドバー | ✅ | v0.4.4 | 記録なし |
| 4.4.2 | アプリ外通知（セッション終了・blocked/done エッジを OS 通知 / Slack / Mattermost / Discord / Telegram へ中継、`notifications:`） | ✅ | v0.4.6 | 記録なし |
| 4.4.3 | ssh 接続先表示（`session-resources.foreground.detail?` を additive 追加）+ フォアグラウンド名解決の汎用化 | ✅ | v0.4.8 / v0.4.9 | 済（macOS） |
| （UI 横断） | UI 多言語化 en/ja（型付き辞書 `i18n.svelte.ts` + ⚙ 設定メニュー、既定は OS 言語追従） | ✅ | v0.4.7 | 記録なし |
| （UI 横断） | ターミナルのコピー & ペースト（macOS 限定のアプリメニュー App / Edit / Window + `tauri-plugin-clipboard-manager`、コピーは macOS が Cmd+C・それ以外が Ctrl+Shift+C で選択が無いときは介入せず PTY へ、貼り付けは macOS がネイティブ経路・Ctrl+Shift+V が自前ハンドラで `term.paste()` 経由、右クリックメニュー、`macOptionClickForcesSelection: true`） | 🚧 | v0.5.8 | 一部済（U13、2026-07-31。残り 4 点） |
| （UX トラック） | Phase 4 期に計画外で入った UX 改善: `mterm.yml` → `ptygrid.yml` リネームと用途別サンプル（`da40cb0`）、一括 cd（`cf42ced` / `77d0271`）、作業フォルダと設定探索の分離 + origin バッジ（`acbed94`）、設定なしフォールバック（`0530e3b`）、フォルダサジェスト（`a3a769a`）、終了ペインの明示と一括クローズ（`d8a3d8e`） | ✅ | v0.4.2 | 記録なし |
| （安定化） | docs/inside のバグ / セキュリティ調査への対応: backend 純バグ 12 件（`c6f31ad`）、frontend 純バグ 8 件（`7505bbe`）、S1 Queen `/mcp` の token + Host/Origin 認証（`3159263`）、S2/S4 autostart 信頼境界 + CSP（`f18bae6`）、手打ち claude の lead 帰属修正（`9c4ab67`）、認証トークン永続化（`0af8de4`） | ✅ | v0.4.3 | 記録なし |
| 5.0.0 | MVO: `workflows:` スキーマ + 検証 / `orchestrator.rs`（spawn + DAG 進行ドライバ、fail-fast、fan-out fresh-spawn）/ Queen MCP tools 22 本 / WorkflowPanel + 🔀 チップ / `close_on_exit`・`autoClose` | ✅ | v0.5.0（`b1b4f1f`） | pipeline 実走 済（U1、2026-07-30）/ fan-out 実走 済（U2、2026-07-31） |
| 5.0.1 | Workflow Resume: `workflow_runs` 永続化（`user_version` 2→3）+ write-through、`workflow-resume-pending` イベント + Y/N バナー、`resume_workflow` / `abandon_workflow` | ✅ | v0.5.1（`ac1b94b`） | 済（U5、2026-07-30） |
| 5.0.2 | `ptygrid init`: `ptygrid.yml` の自動生成（環境検出 → テンプレート生成 → 自己検査 → 既存ファイルがある場合は sidecar で差分提示）。backend（`init.rs` + Tauri command 3 本）と frontend（`InitPanel.svelte` + 入口 2 つ + i18n）が実装済みで自動テストは通過。2026-07-30、macOS 実機で全経路を確認済み（→ §2 U11）。spec: [spec-init-5.0.2.md](../spec/spec-init-5.0.2.md)（→ 脚注※）。ローカル LLM プローブの追補は次行 | ✅ | v0.5.7 | 済（U11、2026-07-30） |
| 5.0.2 追補 | ローカル LLM プローブ: `init_probe_llm`（4 本目の Tauri command）で既定 3 ポート + 手入力最大 4 本に `GET /v1/models` を当て、Anthropic Messages API 互換の確証が取れたときだけ有効な agent 定義を出す（それ以外はコメント行）。モデル選択 `<select>`・検出行への反映・`ANTHROPIC_AUTH_TOKEN` の出し分けを含む。ブランチ `feat/init-llm-probe-5.0.2` は `main` にマージ済み（→ §4） | 🚧 | v0.5.8 | 一部済（U12、2026-07-30。残り 3 点） |
| 5.0.3 | Queen MCP 登録の代行（バッジからコピペしている `claude mcp add` 等を ptygrid が代行。claude は CLI を代行実行、codex / grok は TOML を `toml_edit` で値単位編集。差分承認・冪等・登録解除を含む）。spec: [spec-registration-5.0.3.md](../spec/spec-registration-5.0.3.md)（→ 脚注※） | ⬜ | — | 該当なし |
| 5.0.4 | Orchestrator 実行層: `joinOn: reply` 完了判定 / `condition:` 評価 / `handoffTo` チェイン / `retry:` 再試行 / `timeoutMs` 強制 / supervisor・handoff の spawn ゲート撤去。続けて `fanOut` 黙殺による false green の解消と straggler（`any` / `N` join の敗者）協調キャンセル | 🚧 | v0.5.7（`5d3c1b5` → `3bd9833` = PR #1、`52de433` = PR #3 に同梱） | 基本の実走 済（U1）/ straggler 協調キャンセル 済（U2、2026-07-31）/ `joinOn: reply`・`condition:`・`handoffTo`・`retry:`・`timeoutMs` の実走は未（§2 に項番を立てていない） |
| 5.0.4 追補 | Orchestrator ハードニング: pane 上限の待ち行列化 / driver tick 軽量化（`session_states()`・registry evict）/ inbox mailbox の run 単位分離。wire 契約は無変更 | 🚧 | v0.5.7（`2dc5e40`、PR #3 = `4c02cbb`） | U3 済（2026-07-30）/ U4 未 |
| 5.0.5 | **Arena view**（`arena.rs` + `Arena.svelte`、`arena-open` イベント、`arena.vote` / `arena.list_votes`）。`arena: true` は現状パースだけ通り、書いても何も開かない | ⬜ | — | 該当なし |
| 5.0.6（案） | Orchestrator の計測とパイプライン化: `StepOutcome` に step 単位の終了時刻とペイン待ち時間を additive 追加 / 合成 workflow（直列・鎖分割・9 面待ち・fan-out + `joinOn: any` の 4 本）で orchestration の効きだけを測る / cold start（ペイン再利用 vs 毎回 spawn）の実測。**patch 番号は提案でありユーザー判断で確定**（→ 脚注※2） | 🚧 | v0.5.8 | 合成 workflow 4 本の実走と実測 済（2026-07-31、同じ回で U2 も消化）/ cold start 実測 済（2026-07-31、`example/measure-coldstart`）/ U4 は未 |
| 5.0.7（案） | ストリーミング依存 `onEach: reply` / `joinOn: stream`: 上流が生き続けたまま返信 1 本ごとに「unit」を送り、下流はその 1 本ごとにコピー `<id>#<k>` を 1 つ spawn する。番兵 `[[end]]` と上流終端の 2 層で stream を閉じ、unit 0 本は `Failed`、`STREAM_MAX_UNITS` = 64 で暴走を止める。load 時検証 V1〜V10 と resume 拒否。`StepOutcome` の wire は不変（unit 本文は `#[serde(skip)]`）で frontend も無変更。**patch 番号は提案でありユーザー判断で確定**（→ 脚注※2。5.0.6 が確定するまで 5.0.7 も確定しない） | 🚧 | v0.5.8 | 未（U14）。自動テストのみ（lib 459 / 統合 14） |
| 5.0.8（案） | 時刻で workflow を起こす `schedule:`: cron 式ではなく `every: day / weekday / hour` + `at:` の 3 語彙のみ。**アプリが起動している間だけ発火し、取りこぼしは追わない**（契約であって実装の限界ではない）。見送りは 2 条件（前の run が未終端 / 9 面に空きが無い）で、どちらも発火の前に判定して理由を残す。`schedule` を持つ workflow は `autoClose` の既定が `success` に変わる。連続 3 回失敗で自動停止（`ptygrid.yml` は書き換えない）。新 wire は読み取り専用の Tauri command `list_schedules` 1 本のみ。**patch 番号は提案でありユーザー判断で確定**（→ 脚注※2） | 🚧 | v0.5.8 | 未（U20）。自動テストのみ（lib 530 / 統合 14） |
| 5.5.0 | MCP 2026-07-28 RC 互換ルータ（`queen_compat`: header / route / capabilities / deprecation / initialize / meta、hot-swap 可能な `McpCompatHandle`、legacy 2025-06 併存） | 🚧 | v0.5.6（`21d1367`） | 記録なし（U10） |
| 5.5.1 | OTel GenAI 計装 + SQLite シンク（span の書き出し先） | ⬜ | — | 該当なし |
| 5.5.2 | Cost 計算 + `agent-cost` イベント | ⬜ | — | 該当なし |
| 5.5.3 | Agent Status Rings（通知リング / 要承認ハイライト。出自は competitive-landscape の「次に取る UX」で、4.0 の teammate permission 表示の汎用化。設計は spec-phase5-5.md §2.3 / §3.7） | ⬜ | — | 該当なし |
| 5.5.4 | Trace Waterfall + Cost Dashboard | ⬜ | — | 該当なし |
| （無番号） | escalation: retry 枯渇時に外部へ通知する経路（4.4.2 の `notifications:` 基盤への配線）。枯渇判定は 5.0.4 で発火するようになったが配送経路が無い | ✅（2026-08-13、Stage A-4。→ §6.16） | v0.5.8 | 未（U18） |
| （無番号） | `workflow_runs` の retention: project ごと **終端 run 500 件**の上限と DELETE。終端していない run は数えも消しもしない（resume を守る）。掃除は「run が終端に到達した書き込み」と abandon のときだけで、200ms tick には SQL を足さない。`user_version` は消費しない（3 のまま） | ✅（2026-08-13、Stage A-6。→ §6.17） | v0.5.8 | 未（U19） |
| 6.0.0 | Security Foundation: `user_version` 4 の 3 テーブル（`replays` / `secrets_audit` / `sandbox_events`）同時導入 | ⬜ | — | 該当なし |
| 6.0.1 | Sandbox filesystem-only プロファイル | ⬜ | — | 該当なし |
| 6.0.2 | Sandbox strict プロファイル | ⬜ | — | 該当なし |
| 6.0.3 | Secrets keychain backend | ⬜ | — | 該当なし |
| 6.0.4 | Secrets derived + proxy | ⬜ | — | 該当なし |
| 6.0.5 | Replay UI + Export（spec-phase6-0 §9 は「6.0.5 完了で v1.0.0 昇格を検討」としている） | ⬜ | — | 該当なし |

> **※ 脚注: Phase 5.0 の patch 採番が 3 資料で食い違っていた経緯と、その決着（2026-07-29）**
> [spec-phase5-0.md](../spec/spec-phase5-0.md) §9 = 「5.0.0 Provider 基盤 / 5.0.1 Memory 保存経路 /
> 5.0.2 Memory embedding / 5.0.3 Orchestrator pipeline+supervisor / 5.0.4 Orchestrator
> fan-out+handoff / 5.0.5 Arena」、CONTRACT.md「Phase 5.0 追加契約」冒頭 = 「5.0.0 MVO /
> 5.0.1 Memory FTS5 / 5.0.2 Memory embedding / 5.0.3 Provider / 5.0.4 Orchestrator
> supervisor+handoff / 5.0.5 Arena」、`v0.5.6` のタグメッセージ = 「5.0.2 Workflow Reliability」
> という第 3 の呼び方。**実際に消化されたのは 5.0.0 = MVO、5.0.1 = Workflow Resume、
> 5.0.4 = Orchestrator 実行層**で、5.0.2 / 5.0.3 は長く欠番のままだった（タグメッセージが 5.0.2 と
> 呼んだ内容は後の 5.0.4 として着地した）。**2026-07-29、ユーザー判断で決着**: 5.0.2 =
> `ptygrid init`（[spec-init-5.0.2.md](../spec/spec-init-5.0.2.md)）、5.0.3 = Queen MCP 登録の代行
> （[spec-registration-5.0.3.md](../spec/spec-registration-5.0.3.md)）に充てる。Memory + Provider は 5.0.6 以降へ回し、番号は着手時に確定する。なお
> `orchestrator.rs` のコード内コメントが 5.0.4 追補を「phase 5.0.5」と書いているが、`5.0.5` は
> Arena view 用に予約済み（本表の 5.0.5 行）のため**本断面には採番しない**（コメント表記の削除は
> §3 バックログ）。

> **※2 脚注: 5.0.6 の採番は未確定（2026-07-30 時点の提案）**
> ※ の決定は「Memory + Provider は 5.0.6 以降へ回し、番号は着手時に確定する」で止まっている。本表の
> 「5.0.6（案）」は、その空いている 5.0.6 を**orchestrator の計測とパイプライン化に充てるという提案**
> であり、**確定はユーザー判断による**。この案を採る場合 Memory + Provider は 5.0.7 以降へずれる。
> 番号が決まるまで本表の行は「（案）」表記のままとする（v0.5.8 のタグ内容そのものは §4 に書いてあり、
> patch 番号の確定を待たない）。

**いま特に効く読み方**: 2026-07-30 に `smoke` を実機で流し切ったことで、**「workflow は一度も実走していない」という一括の未知は消えた**（U1 完了）。続けて同日中に U3（pane 上限待ち）と U5（resume バナー）も実機で完了した。同日さらにローカル LLM プローブの追補が入り実機 1 回目を通し、翌 2026-07-31 にターミナルのコピー & ペーストが入って実機 1 回目を通した。**同じ 2026-07-31 に合成 workflow で U2（fan-out + straggler 協調キャンセル）も完了**し、5.0.0 は実機側の積み残しが無くなって ✅ になった一方、その計測作業そのものである「5.0.6（案）」が実装 + 実測まで進んで残作業ありの状態になったため、**🚧 は 8 行のまま**。同日さらに cold start の実測（`example/measure-coldstart`）も済み、「5.0.6（案）」に残るのは **U4 だけ**になったが、それが残っている以上 ✅ にはならない（→ §6.12）。残る理由は個別機能ごとに分かれている: Linux 常用（U7）、host モード（U6）、コピー & ペーストの残り 4 点（U13）、プローブ追補の残り 3 点（U12）、5.0.4 の残る固有機能（`joinOn: reply` / `condition:` / `handoffTo` / `retry:` / `timeoutMs` の実走。§2 に項番を立てていない）、mailbox 分離（U4。5.0.4 追補と 5.0.6（案）の両方がこれを待っている）、5.5.0 の実機記録なし（U10）。未タグ成果へのタグ付けは v0.5.7（2026-07-30）と v0.5.8（2026-08-13）で消化した（→ §4・§6.23）。**ただし v0.5.8 は実機検証を 1 つも消さずに打ったタグ**で、U14（残 3 点）/ U17 / U18 / U19 / U20 がそのまま残っている。

**補足: 主要モジュールの所在**（状態は上表を見ること）

- `orchestrator.rs`: workflow DAG ドライバ（200ms tick）。spawn / 完了判定 3 経路 / `condition` /
  `retry` / `timeout` / straggler キャンセル / pane 上限の待ち行列化（`WORKFLOW_DEFER_MAX_MS` = 5 分）/
  `WorkflowRegistry`（終端 run は `REGISTRY_TERMINAL_CAP` = 100 で evict）。
- `queen_compat/`: MCP 2026-07-28 RC と legacy 2025-06 の両立ルータ（統合テスト 14 本）。
- `queen_store.rs`: SQLite 永続化。現行 `user_version` = 3。v4 以上は開かずに明示エラー（→ §5.1）。
- `token_store.rs`: Queen `/mcp` と teammate hooks の Bearer トークンを `auth-tokens.json`
  （0600・atomic write）に永続化。再起動後の再登録は不要（v0.4.3〜）。
- `src-tauri/teams-backend/`: CustomPaneBackend 提案（anthropics/claude-code#26572）準拠の JSON-RPC 2.0
  ソケットサーバ + tmux 互換シム（独立 workspace、テスト 30 件）。Phase 4.2 で app 本体へ配線済み
  （`teams_host.rs` の `PaneHost`・`__tmux-compat` サブコマンド経由）。
- **ソースが存在しないもの**: `memory.rs` / `memory_embed.rs` / `provider.rs` / `arena.rs` /
  `observability.rs` / `secrets.rs` / `sandbox.rs` / `replay.rs`。
- Defer / Skip 判定（u32 wrap 等の理論値、稀なレース、実験機能の DoS、S3 caller-id 等）は
  `docs/inside/evaluation-2026-07-16.md`（git 管理外の内部資料）に整理。

---

## 2. 未検証事項

**実機検証の状況を書くのはこの節だけ**（§1 の実機検証列は本節への参照）。下記はいずれも
「一度も実機で動かしていない」または「実施記録が残っていない」ものである。U8（Windows）と
U9（frontend チェック）だけは特定の patch に紐づかない横断項目なので、§1 の表からは参照されない。

| # | 未検証の内容 | 状況 |
|---|---|---|
| U1 | **実機での workflow 1 本流し**（GUI の 🔀 チップ または Queen tool `spawn_workflow` から実走し、ペインの挙動を目視） | **2026-07-30、macOS で完了**。`smoke`（`pattern: pipeline` / `autoClose: success` / `a`(t1) → `b`(t2)）を最後まで流し切り、step `a` の終了で step `b` が自動 spawn され、run 完了で 2 枚のペインが自動で閉じるところまで確認。これで「完了判定が実 PTY で発火するか」「DAG が進むか」「`autoClose` が効くか」「`workflow-state` が frontend に届くか」の 4 つの継ぎ目が実機で裏付けられた。**5.0.0 以来続いていた「一度も実走していない」状態はここで解消**。残る実機項目は個別機能ごと（U2〜U5）に分かれる。詳細な経緯は §6.5 |
| U2 | straggler 協調キャンセルの pane kill（fan-out レースの敗者ペインが GUI 上で実際に閉じること） | **2026-07-31、macOS で実施**（スクリーンショットで確認済み）。`example/measure-parallelism` の `measure-4-join-any`（`gate` → `race` が `fanOut: 3` + `joinOn: any` → `report`）を **3 回**実行（03:05:30 / 03:06:11 / 03:07:18）。3 回とも `race#0` が SUCCEEDED（5.1〜5.3 秒）、`race#1` / `race#2` が **CANCELLED**（同 5.1〜5.3 秒）で、敗者は 45 秒 sleep に入っていたため **`[race] loser end` の行が 1 度も出ていない**。これが「敗者が待ち切らずに kill された」ことの直接証拠になる（時間切れではなく kill）。敗者ペインはグリッドから消え、最終状態は gate / 勝者 / report の 3 枚（フッター `3/9 ペイン`）。`report` は勝者の終了直後に spawn され、run 全体は SUCCEEDED。**U2 は完了**。この回で敗者 kill 時のバナー誤表示（`session N not found`）を 1 件見つけて修正した（`58e9c95`）。詳細な経緯は §6.11 |
| U3 | pane 上限（9 面）到達時の待ち行列化が実機で `Pending` のまま待ち、空きで再開すること（`error` に `"waiting for a free pane slot (N/9 occupied)"`） | **2026-07-30、macOS で実施→不整合を発見・修正済み**。8 面埋まった状態から `smoke` を起動し step `a` が 9 枚目を占有→`close_on_exit` 未指定のため自然終了後も `Exited` のままセルを占有→次 step の判定は live 基準で空きありと誤認して spawn し、frontend は表示できないまま headless で走った。占有判定を `occupied_pane_count()`（グリッド全セル数、`Exited` 含む）へ修正（`0e9c5ba`、詳細 §6.6）。**加えて表示側の欠落も判明**: 待機理由は `outcome.error` に入っていたが、パネルが全ての error を ⚠ のツールチップに畳んでいたため待っている step と止まっている step が見分けられなかった。`Pending` で理由があるときは step 行にテキストで出すよう修正（`05799d5`）。修正後の再検証で、step 行に `waiting for a free pane slot (9/9 occupied)` が表示されることを**スクリーンショットで確認**。ペインを閉じると `Running` に遷移することは**ユーザー報告**。**U3 は完了**（詳細 §6.7） |
| U4 | 同名 workflow の並行 run が互いの返信を取りこぼさないこと（mailbox の run 単位分離） | 未実施。続報9 が名指しで「目視確認未了」としたもののうち、U2 の完了後に残る 1 件。消化の枠は v0.5.8 の項目 5（→ §4） |
| U5 | クラッシュ / 再起動後の resume Y/N バナー（5.0.1） | **2026-07-30、macOS で実施**。`smoke` 実行中にアプリを再起動したところ、「前回のワークフロー run『smoke』が途中で中断されています。再開しますか？」のバナーが出ることを確認し、**再開後に run が `Succeeded`（step `a` / `b` とも `Succeeded`）まで到達するところまでスクリーンショットで確認**。中断からの復帰が実際に完走することの実証。再起動後のパネルは永続化された中断前の状態を表示するため（`error` は wire フィールドとして残るが `deferred_since_ms` は `#[serde(skip)]` のため復元されない）、ペインが 1 枚しかない状態でも `9/9 occupied` のような古い理由が見えることがある（矛盾ではない）。**U5 は完了**（詳細 §6.7） |
| U6 | host モード（Phase 4.2）の Claude Code 実機検証（spec-claude-teams-panes §10.3 の手順） | 実装は入っているが実機手順は未消化。macOS 必須 / Linux はベストエフォート |
| U7 | Linux 実機での常用 | build / `.deb` / AppImage は Ubuntu 22.04 CI で検証済み（Phase 3.9）。実機常用は beta 表記のまま |
| U8 | Windows | [porting.md](porting.md) の「Windows 対応チェックリスト」が全項目未着手。`process_name()` が `None` を返すため foreground 名解決 / agent-status / ssh 接続先表示が機能しない |
| U9 | frontend チェック（`svelte-check` / `npm run build`） | **2026-08-04、実測済み**。`npm install` からやり直して `npm run check` = **136 files / 0 errors / 0 warnings**、`npm run build` = **成功**。これで「v0.5.1 時点の 0 errors から変わっていないはず」という推測は実測に置き換わった。なお実行環境は Linux コンテナなので、macOS 限定のメニュー定義（`#[cfg(target_os = "macos")]`）はこのチェックの対象外である |
| U10 | 5.5.0（RC 互換ルータ）の実機検証 | **記録が無く判定不能**。CONTRACT.md の実装状況節も自動テスト（unit 35 + 統合 14）しか挙げていない。実機で RC / legacy 双方のクライアントを繋いだ記録は見当たらない |
| U11 | `ptygrid init`（5.0.2）の実機検証 | **2026-07-30、macOS で実施**（すべてスクリーンショットで確認済み）。(1) 設定の無いフォルダで起動→シェル 1 枚→「設定を作る」ボタンが出て、検出結果（opencode/claude/codex/gemini/qwen/grok/aider の 7 体・npm・git あり・ローカル LLM ルータ未検出・既存設定なし）が実環境と一致することを確認、(2) 通常生成で `ptygrid.yml`（2,060 バイト）が生成され agents チップ 7 体が並び、生成物は autostart 全 false のため trust プロンプトは出ずペインも自動起動しないことを確認、(4) 既存設定ありの状態では副入口の書き込み先が `ptygrid.init.yml` に切り替わり、書き込み後も既存 `ptygrid.yml` は mtime・内容とも無変更であることを確認（上書き禁止の実測裏付け）、(5) 書き込み直後に init 自身の通知と watcher `config-changed` による再読み込みトーストが二重に出る競合を実測（spec §9 で推測としていた箇所が確認され、直後に自己書き込みエコー抑制（`ui.selfWrite` + 3 秒窓）を別コミットで修正済み）。(3) プレビューを手編集して `autostart: true` にしてから書き込むと**今度は trust プロンプトが出て**、「信頼して起動」で当該エージェントが実際に起動することを確認（`init_write` → `loadConfig` → `maybeAutostart` の順序の実証）。**U11 は完了**。Global 選択時の `~/.ptygrid/` 作成のみ今回の範囲外（必要になった時点で確認する）。詳細な経緯は §6.4 |
| U12 | ローカル LLM プローブ（5.0.2 追補）の実機検証 | **2026-07-30、macOS で 1 回目を実施**（スクリーンショットで確認済み）。検出フォルダ `~/works/tmp/ptygrid`、PATH 上の CLI 7 体（opencode / claude / codex / gemini / qwen / grok / aider）、プロジェクト種別 npm、git リポジトリあり、既存設定ありのため書き込み先が `ptygrid.init.yml` に切り替わることを確認。プローブは 1234 / 3456 / 11434 を叩き、3456 は無応答、**11434 で `Ollama 0.32.1` が応答して「Anthropic API 確証あり」バッジが出てモデル 20 件を取得**（先頭は `x/flux2-klein:latest`）。**まだ確認していないことが 3 点**: (1) モデル選択 `<select>` の実機動作（実装は 2 つ目のコミット `8931464` で入ったが押していない）、(2) 生成された `local-11434` の定義で実際に Claude Code が起動するか、(3) LM Studio を上げたときに未確証の分岐（コメント行出力）へ落ちるか。**U12 は一部済**（この 3 点が残る）。詳細な経緯は §6.9 |
| U13 | ターミナルのコピー & ペーストの実機検証 | **2026-07-31、macOS で 1 回目を実施**（下記はすべてスクリーンショットで確認済み）。(1) **ペインをまたいだコピー & ペースト**: 1 枚目のペインでファイル名を範囲選択 → Cmd+C → 2 枚目の zsh ペインで Cmd+V し、同じ文字列が入ることを確認。(2) **右クリックメニューの 2 状態**: 選択があるときは「コピー ⌘C」「貼り付け ⌘V」がどちらも有効、**選択が無いときはコピーが無効表示**になり、ツールチップに「選択範囲がありません — ドラッグで選択してください / TUI がマウスを使っている間は macOS なら Option ドラッグ、それ以外は Shift ＋ドラッグ」が出ることを確認。**まだ確認していないことが 4 点**: (1) TUI（Claude Code や vim）がマウスレポートを有効にしている状態での Option ドラッグ選択、(2) 複数行の貼り付けが bracketed paste 対応シェルで Enter を押すまで実行されないこと、(3) Linux / Windows の Ctrl+Shift+C / Ctrl+Shift+V（U7 / U8 の範囲）、(4) macOS のメニューバーに Edit メニューが実際に出ていること（貼り付けが動いた以上は出ている可能性が高いが、**目視の記録は無い**ので未確認扱い）。**U13 は一部済**（この 4 点が残る）。詳細な経緯は §6.10 |
| U14 | **`onEach: reply` / `joinOn: stream`（5.0.7）の実機検証** | **2026-08-05、macOS で 1 回目を実施（一部済）**。`_OUTPUTS/u14-verify` の `u14-streaming`（git 管理外の使い捨て設定）を流し、run が SUCCEEDED まで到達することを確認: `coder` 37.4 秒で 5 unit を送り番兵 `[[end]]` で Succeeded、`reviewer#0`〜`#4` が**到着順・欠番なし**で 5 つ生え（所要 22.7 / 15.7 / 11.0 / 6.3 / 33.8 秒）、全コピーが終端したあと `summary` が **1 度だけ** 28.4 秒動いた。採番・番兵での閉じ・`dep_satisfied` の stream 節が実機で裏づけられた。**この回で 1 件の設計不備を発見し修正**（→ §6.14）: コピーが agent 定義名の mailbox を共有しており、あるレビュアーのペインが「待機で 3 件届いたので、いちばん新しい id=341 を選び」と報告した。**まだ確認していないことが 3 点**: (1) 「上流が 3 本目を送る前に 1 人目が動いている」瞬間の目視証拠（今回のスクリーンショットは完走後のもので、この 3 つ — `coder` が Running / `reviewer#0` が Running・Succeeded / `reviewer#2` の行が無い — が同時に写った 1 枚がまだ無い。`coder` は 37 秒動くので ▶ の 15〜20 秒後が狙い目）、(2) `u14-no-sentinel`（番兵を送り忘れても `timeoutMs` で run が終端に到達すること）、(3) `u14-queue`（9 面上限で待ち行列ができ、兄弟が走っている間は 5 分を超えても失敗しないこと）。**2 回目（同日 17:19、修正後のバイナリ）で per-copy mailbox の実効を確認**: reviewer のペインが `sender=wf/wfr_18c8daebafa1e87800000000/reviewer#2` / `#3` と自分専用の mailbox 名で返信しており、1 回目に出ていた「待機で N 件届いたので、いちばん新しいものを選び」という報告が消えた。`summarizer` も「reviewer#0〜#3 の 4 件（id 384〜387）」と自分の run の返信だけを数えている。**この回の unit は 4 本**（設定は「5 つ」のまま。1 回目は 5 本）で、実装は本数に関わらず正しく追随したが、**「1 単位ごとに自発的に返信を刻む」挙動がモデル依存であることが 2 回で 2 通りの本数として出た**。残るのは上記 3 点のうち (1)(2)(3) すべてで、いずれも未実施 |
| U15 | **resume 拒否ガード（`condition:` / `handoffTo:` の carry 喪失、2026-08-07）の実機検証** | 未実施。`condition:` を持つ workflow（例: `gate`(kickoff あり・`joinOn: reply`) → `apply`(`condition:`)）を流し、**`gate` が返信して Succeeded になった直後・`apply` が走り出す前にアプリを落として再起動**する。期待は「再開バナーが失敗表示になり、理由に `cannot be resumed` と `condition:` と `gate` が出る」こと。`handoffTo:` 版（`draft` → `polish`）も同じ形で 1 本。裏づけは現状 unit test 5 本のみ（→ CONTRACT.md 続報12） |
| U16 | **複数 `handoffTo:` の合流（2026-08-07）の実機検証** | 未実施。`pattern: supervisor` で `implement` → `reviewA` / `reviewB`（2 体とも `joinOn: reply` + `handoffTo: verdict`）→ `verdict`（`dependsOn: [implement, reviewA, reviewB]`）を組み、**判定ペインの kickoff に 2 体ぶんの本文が宣言順で前置されていること**を目視する。1 体だけが返信した場合にその 1 本が運ばれることも同じ回で。裏づけは現状 unit test 4 本のみ（→ CONTRACT.md 続報13） |
| U17 | **cancel / abandon 時の未 ack kickoff 掃除（Stage A-5、2026-08-13）の実機検証** | 未実施。手順は §6.14 で穴が出たときの逆をたどる: (1) `kickoff:` を持つ step の workflow を起動し、エージェントが返信する前に **⏹ で cancel** する。(2) 同じ workflow をもう一度起動し、ペインに mailbox の中身を数えさせて **前の run の kickoff が未 ack の一覧に出ないこと**を確認する（`await` が古い kickoff を返さないこと、が実際に見たいもの）。(3) abandon 側は「実行中にアプリを落として再起動 → 再開バナーで『破棄』」を選び、同じく次の run で残っていないことを確認する。(4) 並行 run 版（同名 workflow を 2 本走らせ、片方だけ cancel しても**もう片方のペインが自分の kickoff を受け取れる**こと）も同じ回で見たい。裏づけは現状 unit test 4 本のみ（→ CONTRACT.md 続報15） |
| U18 | **retry 枯渇の escalation 通知（Stage A-4、2026-08-13）の実機検証** | 未実施。手順: (1) `ptygrid.yml` に `notifications:`（`enabled: true`、`channels:` に `os` と、可能なら Slack の incoming webhook を 1 本）を書く。`level` は**既定の `critical` のまま**にする — 「既定でも届く」ことがこの回で見たいことの半分だから。(2) 必ず失敗する step（例 `cmd: /bin/false`、あるいは短い `timeoutMs` で必ず超過する step）に `retry: { max: 1, backoffMs: 500 }` を付けた workflow を 1 本流す。(3) 期待は **escalation が 1 通だけ**届き、本文に workflow 名 / run id / step id / `2 attempts` / 最後のエラーが入っていること。**枯渇の瞬間に 1 通で、200ms ごとの連投にならないこと**が最重要の観測点（`escalated` フラグの実効）。(4) 同じ回で**ペイン exit 由来の通知も別に届く**ことを確認する（仕様どおりの二重で、バグではない）。(5) 余力があれば `level: silent` にして 1 通も出ないことも見る。裏づけは現状 unit test 5 本のみ（→ CONTRACT.md 続報16 / §6.16） |
| U19 | **`workflow_runs` の retention（Stage A-6、2026-08-13）の実機検証** | 未実施。**上限が 500 件なので「本物の 500 run を流す」のは現実的でない**。見たいのは件数そのものではなく (a) 200ms tick に SQL が増えていないこと、(b) 生きている run が消えないこと、の 2 点なので、手順は次のとおり: (1) `queen.sqlite3` に `state = 'succeeded'` のダミー行を 500 件超（`sqlite3` で直接 INSERT。`run_id` は `done-00001` のような固定幅で）仕込んだ状態でアプリを起動し、**起動時には何も消えない**ことを確認する（開いた直後の `count(*)` が仕込んだ値のまま）。(2) その状態で workflow を 1 本流して完走させ、**完走した瞬間に 500 件へ縮む**ことと、いちばん古い行から消えていることを確認する。(3) 同じ回で、**実行中にアプリを落として再起動 → 再開バナーが出る**ことを確認する（`state = 'running'` の行が (2) の掃除に巻き込まれていないことの実証。これが最も重要）。(4) `steps_json` の実サイズを 1 行 SELECT して測り、CONTRACT.md 続報17 の「1 KiB/行と仮定」を実測値に置き換える。(5) 余力があれば、run 実行中に `PRAGMA` や `sqlite3` で書き込み待ちが増えていないこと（tick が重くなっていないこと）を体感で見る。裏づけは現状 unit test 5 本のみ（→ CONTRACT.md 続報17 / §6.17） |
| U20 | **`schedule:`（5.0.8）の実機検証** | 未実施。実装と自動テスト（2026-08-13 のレビュー修正後で lib 530 / 統合 14）は入っているが、**実機では一度も時刻を待っていない**。手順は [spec-schedule-5.0.8.md](../spec/spec-schedule-5.0.8.md) §7.2 の 4 項目に、§6.20 の修正ぶん 4 項目と §6.21 の 1 項目を足した 9 つ: (1) `at` を 2 分後にして保存し、**保存直後に発火しないこと**と 2 分後に発火することを確認（設定を書いている最中に走り出さないという規則と、発火そのものを 1 回で見る）、(2) その run を走らせたまま次の発火時刻を迎えさせ、見送りと理由の表示を確認、(3) 8 面を手で埋めた状態で発火時刻を迎えさせ、発火せず理由が出ることを確認、(4) 失敗する設定で 3 回発火させ自動停止を確認、(5) **(4) の続きで、kickoff を直して保存し、自動停止が解除されて次回に発火することを確認**（自動テストは `set_for_test` 経由なので、`notify` の watcher → 再読み込み → 表の作り直しという実経路は未検証）、(6) **`every: hour` の設定で、発火時刻の 10 分前にスリープさせ 30 分後に復帰させて、発火せず「予定時刻を過ぎていた」が出ることを確認**（猶予 5 分 / 15 分は実測値ではない。ここで「日常のゆらぎまで見送る」ようなら数字を上げる。→ spec §8.1）、(7) **復帰直後に `chrono::Local::now()` が正しい値を返すか**を (6) のついでに見る（spec §8.2 の未確認事項）、(8) **macOS の「システム設定 → 一般 → 日付と時刻」でタイムゾーンを切り替え、実行中のアプリの次回時刻が新しいゾーンの時刻に付け替わることを確認**（`chrono::Local` が実行中の TZ 変更を拾うかどうかは未確認で、拾わなければ再アンカーは発動しない。→ spec §8.2）、(9) **§6.21（M5）ぶん: 言語を日本語にして (2)〜(4) の行を目で読み、`example/scheduled-review/ptygrid.yml` の説明文と 1 字ずつ一致することを確認**（「見送り（前の run が終わっていません）」「自動停止（3 回連続で失敗）」「連続失敗 1/3（あと 2 回で自動停止）」。日本語で出ること自体はコードを読んだ帰結であって実測ではない）。**ただし日時スタンプ（「最終 8/5 09:00」の 「8/5 09:00」の部分）は 1 字一致の対象から外す** — `fmtSchedStamp` は `toLocaleString(undefined, …)` すなわち**システムロケール**で書式化するので、英語環境では「8/5, 09:00 AM」になり、例文と一致させることが原理的にできない（→ §6.21 残件 (4)。例文側にも 2026-08-13 に注記を足した）。一致を見るのは**語のほう**（「最終」「成功」「見送り」など）である。**手順全体を通しての注意: パネルのポーリングは 60 秒間隔なので、`ptygrid.yml` を保存してから schedule 行の表示が変わるまで最大 60 秒かかる。**変化が無いと判断する前に 1 分待つこと（設定の再読み込み自体は watcher が即座に行う。遅れるのは表示だけである）。**最大の未確認は依然として語彙の妥当性**で、想定利用者（cron を書けない層）がこの 3 語彙で実際に書けるかは検証していない |

---

## 3. 次の作業

優先順の根拠は「**未検証のまま積み上がっている量**」→「**リリース規律の負債**」→「**未着手の
新機能**」の順。新機能を足す前に足元を確定させる。各項目の状態は §1 の表を見ること（再掲しない）。

### P1. 実機での workflow 1 本流し（`smoke` workflow）— 最優先

**なぜ今それか**: 自動テストが全部通っているのに実走実績がゼロというギャップ（U1）を埋めるのが
いちばん費用対効果が高いから。ここが通らない限り、以降の実装はすべて「動くはずのコード」の上に
積み上がる。CONTRACT.md 続報8 / 続報9 / 続報10 が同じ未解除項目を繰り返し記録している。

- 対象は `ptygrid.yml` の `smoke` workflow（`pattern: pipeline` / `autoClose: success` /
  step `a`(agent `t1`) → `b`(agent `t2`)、各 30 秒 sleep の shell）。この最小構成を GUI の 🔀
  チップまたは Queen tool `spawn_workflow {name: "smoke"}` から流し切る。
- 確認したい最小項目: (1) run が `Running` → `Succeeded` まで進む、(2) `autoClose: success` でペインが
  実際に閉じる、(3) `workflow-state` で WorkflowPanel が追随、(4) resume Y/N バナー（U5）が出る。
- 続けてハードニング固有の挙動（U3 / U4）を実機で見る。`joinOn: reply` / `condition:` /
  `handoffTo` / `retry:` を使う workflow は `smoke` が通ってから別途 1 本ずつ。straggler の
  pane kill（U2）は v0.5.8 の合成 workflow の回で消えたので、この枠に残るのは U4（並行 run）
  だけになった（項目と順序は §4）。
- **（2026-08-13 追記）U4 は v0.5.8 のタグに間に合わなかった**。§4 の実装項目 5 に置いたまま
  未消化で `v0.5.8` を打っている（→ §6.23）。同じタグで実機検証が必要な項目が 4 つ増えた
  （U17 / U18 / U19 / U20）ので、**この枠に積まれているのは U4 / U14（残 3 点）/
  U17 / U18 / U19 / U20 の 6 件**である。次のタグより前に消す順序はまだ決めていない。
- 実走で分かったことは CONTRACT.md の続報として追記し、§6 に日付つきで残す。

### P2. 未タグ成果のリリース（次のタグ）— 完了

2026-07-30、`v0.5.7` としてリリース済み（詳細は §4・§6.8）。**2026-08-13、続けて `v0.5.8` を
リリース済み**（詳細は §4・§6.23）。以降の次の作業は P3 から。

### P3. retry 枯渇時の外部通知経路（escalation）— 完了（コード上）

**2026-08-13、Stage A-4 として実装した**（→ §6.16 / CONTRACT.md 続報16）。step が `retry:` の
予算を使い切った瞬間に、4.4.2 の通知経路へ `error` として 1 通出る。見込みどおり**新しい配送機構は
不要**で、`orchestrator::take_escalations`（純関数）＋ `notify_escalation` を `advance_run` の末尾に
足しただけである。config には何も足していない（ptygrid-yml-guide.md §1 の
「(config には書かない)」は現状のまま正しい）。**実機検証は未実施**（→ §2 の U18）なので、
U18 が済むまでこの節は「コード上は完了」として残す。以下は着手前の記述:

> **2026-08-05 追記**: `schedule:`（5.0.8）が入ったことで、この項目は「あると便利」から**「無いと 5.0.8 が半分しか成立しない」**に変わった。無人で時刻起動する run の失敗を外部へ知らせる経路が無いと、スケジューラは失敗を静かに溜めるだけの装置になる。4.4.2 の配送機構（OS 通知 / Slack ほか）は既にあり、workflow のイベントを流し込む配線だけが欠けている。

**（2026-08-13 追記）この前提は Stage A-4 の実装で満たされた。**

**なぜ今それか**: 4.4.2 の通知基盤が既にあるので**配線するだけ**で済み、労力に対して自主運用の
安全性の伸びが大きいから。

**根拠**: [ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md) §1 の実装マトリクスで ❌ のまま
残っているのは 2 行だけで、その 1 つが `escalation`（もう 1 つは `arena: true` で、こちらは P6 の
5.0.5 で解消する）。自主運用（[autonomous-operation-guide.md](../guide/autonomous-operation-guide.md)）は
「人間が気づく」ことに依存しており、通知が無いと夜間・離席中の失敗が滞留する。4.4.2 の配送機構
（OS 通知 / Slack / Mattermost / Discord / Telegram）をそのまま使えるので、**新しい配送機構は
要らず workflow 側のイベントを既存経路へ流すだけ**で済む見込み。

### P4. 5.0.2 `ptygrid init` / 5.0.3 登録代行（入口の自動化）

**なぜ今それか**: エンジン（5.0.4 まで実装済み）より**入口**が律速になっている。設定を手書きする
限り使う回数が増えず、他人にも渡せない（作者本人の個人設定が 790 行 → 棚卸しで 506 行に減った、
という実データがそれを裏づける）。

- 5.0.2 `ptygrid init`（[spec-init-5.0.2.md](../spec/spec-init-5.0.2.md): 環境検出 → テンプレート生成 →
  自己検査 → 既存ファイルがある場合は sidecar で差分提示）は backend（`init.rs` + Tauri command 3 本）と
  frontend（`InitPanel.svelte` + 入口 2 つ + i18n）を実装済み。実機での操作確認（U11）も消化済みで、
  追補（ローカル LLM プローブ）のブランチも `main` にマージ済み（→ §4）なので、残っているのは
  U12 の 3 点のみ。
- 5.0.3（Queen MCP 登録の代行）は spec のみ（[spec-registration-5.0.3.md](../spec/spec-registration-5.0.3.md)）
  で実装は未着手。claude は CLI を代行実行、codex / grok は `toml_edit` で値単位編集し、差分承認・冪等・
  登録解除までを含む。**着手前の gate として「docs と実装の食い違いの確定」がある**（README / userguide
  は grok を CLI と案内しているが、実装は codex と同一の TOML を出している）。
- **5.0.3 の着手は P1（実機での workflow 1 本流し）より前には置かない**: 入口だけ自動化しても、その先の
  workflow が実走未確認のままでは効果が薄いため。5.0.2 の残作業（U11）は軽量なので先に消してよい。

### P5. 実タスクでの並列化ベースライン測定と、その先の機能の spec（v0.5.8 のタグ内容には数えない）

**なぜ今それか**: 合成 workflow による計測（v0.5.8 = §4）は orchestration の効きしか測らない。実運用で
効くかどうかは実エージェントで測るしかないが、所要がばらつくので**タグの完了条件には入れられない**。
そのためタグとは切り離してここに置く。計測フィールドと cold start 実測（v0.5.8 の項目 2〜4）は消化済み
なので、道具は揃っている。

- **実タスクでのベースライン測定と改良構成の比較**: 同じ課題を直列構成と改良構成で流し、所要を比べる。
  実エージェントの所要はばらつくので**同じ課題を 2 回ずつ**流して比べる（1 回では差が判定できない）。
  比較の前提として U4（同名 workflow の並行 run が互いの返信を取りこぼさないこと）が要る。
- **`onEach: reply` の spec を先に起こす（`mode: serve` は後）**: 順序は v0.5.8 の cold start 実測
  （→ §4 の項目 4・§6.12）で決まった。cold start は約 3.4 秒（ただしこれは下限で、実タスクでは
  文脈の読み直しぶんだけ大きくなる）。`mode: serve` が節約するのは 1 step あたりこの 3.4 秒 + 文脈の
  読み直し分で、10 step 回しても数十秒にしかならない。対して `onEach: reply` が節約するのは「上流が
  全部終わるまで下流が待つ」という工程まるごとの待ち時間で、実タスクなら数分単位になる。orchestration
  自体は 1 依存あたり 200ms しか食っていない（→ §6.11）ので、削るべきは待ち時間のほう。**`mode: serve`
  を捨てるわけではなく順番が後**で、常駐が実機で成立すること自体は同じ回で確認できている（→ §6.12）。
  文脈の読み直しコストを別途測ってから改めて判断してもよい。**spec は起こし済み**
  （[spec-oneach-reply-5.0.7.md](../spec/spec-oneach-reply-5.0.7.md)、2026-07-31）、**実装も入った**
  （2026-08-04、→ §6.13）。当初「実装は v0.5.9 以降」と書いていたが前倒しになったので、どのタグに
  載せるかは §4 の判断待ちだった。**2026-08-13、`v0.5.8` に載せて決着**（→ §4・§6.23。U14 は
  未消化のまま）。この項目に残っているのは**実タスクでのベースライン測定**のほうで、
  それには U4（同名 workflow の並行 run）と U14（5.0.7 の実機検証）が前提になる。

### P6. 未着手フェーズ（着手順の案）

**なぜ今それか**: P1〜P5 で足元が固まるまでは着手しない。以下は「固まったあとの順番」の案。

| 順 | 内容 | 根拠 |
|---|---|---|
| 1 | **5.5.1 OTel 計装 + SQLite シンク** → **5.5.2 Cost 計算 + `agent-cost`** | 5.5.0 で RC ルータと `_meta.traceparent` の受け口だけ作って**エクスポート先が無い**（span を落としているだけ）。半端な状態を先に閉じる。バックエンド完結で UI 変更が要らず、P1/P2 と衝突しにくい |
| 2 | **5.0.5 Arena view** | fan-out + `joinOn: any` の straggler キャンセルが Arena の前提。spec-phase5-0 §2.4 が要求する「敗者が自動 CANCELLED」は 5.0.4 で満たされているので、いま作れば既存基盤の上に乗る |
| 3 | **Memory + Provider** | ptygrid 単体で完結せず、embedding backend（Ollama / LM Studio 等）と `sqlite-vec` の配布方式が未決（spec-phase5-0 §10）。外部依存が最も重い。5.0.6 以降に付け直す（§1 の脚注※） |
| 4 | **5.5.3 Agent Status Rings / 5.5.4 Trace Waterfall + Cost Dashboard** | どちらも frontend 中心で、5.5.1/5.5.2 のデータが無いと表示するものが無い。順序として後ろ |
| 5 | **Phase 6.0 Security（6.0.0〜6.0.5）** | `user_version` 4 の 3 テーブル同時導入を伴い、`session.rs`（PTY hot path）に tee tap を入れる最も侵襲的な変更。§5.2 の規律どおり人手レビュー枠が要る。macOS/Linux の sandbox 実装差も大きい |

### P7. Windows 移植 / Linux 実機検証の継続

**なぜ今それか**: 優先度は低いが、beta 表記を外す前提条件なので落とさずに持っておく（U7 / U8）。

- Windows: 最優先は `process_name()` の Windows 実装。次いで `/bin/cat`・`/bin/sh` に依存する既存
  テストの `#[cfg]` 分岐と Windows CI（詳細は [porting.md](porting.md)）。Linux は実機常用を継続。

### 継続ウォッチ / バックログ

いずれも「優先度は P1〜P7 より下だが忘れると困る」もの。完了・失効した項目はここから削除し、
実績は §1 の表と §4 のタグ表に残す。

- **返信せずに終わった run の kickoff は誰も ack しない（A-5 の入口が 2 つしかない）**
  （2026-08-13、Stage A の最終レビューで確認 → §6.15 / CONTRACT.md 続報15 の既知の限界）。
  `retire_run_kickoffs` の呼び出し元は `cancel_workflow` と `abandon_workflow` の 2 か所だけで、
  `Succeeded` / `Failed` で終端した run は掃かない。**滞留するほうが多数派**である:
  `joinOn: reply` でない step は route 1（PTY exit）/ route 2（semantic done）で完了するので、
  返信が無く kickoff は未 ack のまま残る。**A-6（retention）はここに効かない** —
  `prune_terminal_workflow_runs` は `workflow_runs` を DELETE するだけで `inbox_messages` には
  SQL を 1 本も投げないため、run 行が消えても kickoff は別テーブルに残る。**帰結**: ack しても
  行は消えないので `inbox_messages` は増え続け、`MAX_MESSAGES_PER_PROJECT` = 50,000 に達すると
  `enforce_limit` が `send_inbox` を `Err` で拒否する = **新規 kickoff を送れなくなる**
  （`workflow_runs` と違い、この表は削除ではなく拒否側）。直し方は 2 つ: A-5 の入口を終端書き込みに
  も広げるか、5.6.x（スキーマ分割）でまとめて扱うか。**A-5 が作った問題ではない**（5.0.0 からの
  挙動で、A-5 は cancel / abandon ぶんだけを塞いだ）。
- **`workflow_runs` の retention は入ったが、数字と掃除範囲に穴が残る**
  （2026-08-13、Stage A-6 → §6.17 / CONTRACT.md 続報17）。上限 `MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT`
  = 500 は下限（`REGISTRY_TERMINAL_CAP` = 100）と最悪ファイルサイズの両側から導いた値だが、
  **1 行あたりの `steps_json` の実サイズは未実測**（1 KiB/行と仮定した）。`STREAM_MAX_UNITS = 64`
  と同じ扱いで、実測（→ §2 の U19 (4)）と 5.6.0 のスキーマ分割のあとに見直す。
  残る穴は 3 つ: (1) **起動時の一括掃除をしていない**ので、A-6 以前のビルドが太らせた DB は
  そのプロジェクトで次に run が 1 本終わるまで縮まない（もう run しないプロジェクトなら永久に残る）、
  (2) **`VACUUM` はしない**ので SQLite のファイル自体は縮まず空きページの再利用にとどまる、
  (3) 実機検証が未実施（→ §2 の U19）。**5.6.0 が retention を引き取るときの申し送りは
  CONTRACT.md 続報17 の末尾**（run 単位で数える / 終端していない run は触らない /
  掃除の起動点は tick ではなく終端書き込み）。U19 が済むまでここに残す。
- **cancel / abandon された run の kickoff が agent の mailbox に未 ack で残り続ける**
  （2026-08-05 発見、→ §6.14）— **2026-08-13、コード上は解消（Stage A-5、→ §6.15 /
  CONTRACT.md 続報15）。実機検証は未実施（→ §2 の U17）なので、U17 が済むまでここに残す。**
  以下は発見当時の記述: 次の run のエージェントが死んだ run の指示を読んで実行しうる。
  相関は thread root なので step を誤完了させることはなく、5.0.7 が作った問題でもない
  （5.0.0 からの挙動）。直し方は 2 つあり、(a) `cancel_workflow` / `abandon_workflow` に
  未 ack kickoff の ack を足す、(b) 5.0.7 がコピーに入れた run スコープの mailbox
  （`wf/<run_id>/<step_id>`）を全 step に広げる。(b) は既存の全サンプルの `cmd` が
  `mailbox=$PTYGRID_MAILBOX` を使う形に揃っていることが前提になる。**採ったのは (a)**
  （既存設定に一切影響しないため）。ただし ack の選択キーは message id ではなく
  **sender**（`queen:workflow/<name>/<run_id>`）で、これにより `kickoff_root_msg_id` が
  `#[serde(skip)]` であることに起因する「読み戻した run には id が無い」問題を回避している。
- **cancel と driver tick の lost-update race（既存の穴に A-5 が新しい失敗形を足した）**
  （2026-08-13、Stage A の最終レビューで確認。コード読みのみで**実機再現は未実施**）。
  `WorkflowRegistry` は `get` → 変更 → `put` を**呼び出しをまたいで排他していない**
  （`orchestrator.rs:363-415`。ロックは 1 回の `get` / `put` の中でしか握られない）。driver は
  `start_driver` → `std::thread::spawn` の独立 OS スレッドなので、driver が run を local に
  clone した直後に `cancel_workflow` が走ると、cancel が `Cancelled` を `put` →
  `persist_run('cancelled')` → **kickoff を全部 ack** したあとで、driver が古い `Running`
  スナップショットを `put` し直し、**run が Running に復活して DB も `'running'` に戻る**。
  競合窓は `advance_run` 1 回ぶん。**この lost update 自体は A-5 以前からある**が、以前は
  「復活しただけ」で済んでいたところ、いまは復活した run の step が `await` している mailbox が
  空にされているので、その step は**来ない返信を `timeoutMs` まで待って固まる**という失敗形が
  足された。直すには registry に per-run ロックか cancel フラグが要る（`get`/`put` の粒度を
  変える話なので、Stage A の範囲では直さない）。
- **`onEach` / `fanOut` のコピーが一斉枯渇すると escalation がコピー数ぶん出る**
  （2026-08-13、Stage A の最終レビューで確認 → CONTRACT.md 続報16 の既知の限界）。
  `escalated` は **step（コピー）単位**のフラグなので、`reviewer#0`〜`#63` が同一 tick で
  枯渇すると **1 tick で 64 通**出る。上限は `STREAM_MAX_UNITS` = 64（`fanOut` はペイン枠
  `WORKFLOW_SESSION_CAP` = 9 で実質頭打ち）。しかも `send_os`（`notifications.rs:363`）は
  **driver スレッド上でインライン**で、detached thread に出るのは webhook だけなので、
  OS トースト 64 発がそのまま 200ms tick をブロックする。続報16 の「枯渇 1 回につき
  escalation は 1 通」は step 単位の話で、run 単位のバーストには触れていない。
  spec-notifications.md は §7 で事故防止を掲げつつ**スロットリング / ダイジェストは §9 で
  v2 送り**にしているので、対処するならそこ。**A-4 が作った問題ではない**（`onEach` × `retry` を
  併用する設定でだけ起きる、通知層の v1 が既知で残している穴のほう）。今回は記録のみ。
- **`feat/terminal-copy-paste` が push 未・PR 未**: ターミナルのコピー & ペースト（→ §4 の v0.5.8
  項目 7）はローカルのブランチにしか無い。push と PR を出し、U13 の残り 4 点を消す
- **`fanOut` を持つ step を root に置けない**: `spawn_workflow` の root ループは全コピーに枝番なしの
  同じ `step_id` を付ける一方、`spawn_ready` 経由のコピーだけが `race#0` のような枝番を持つ。パネルの
  step 一覧は `stepId` をキーにした keyed each なので、root fan-out だと同一キーが並ぶ。
  `example/measure-parallelism` では sleep なしの `gate` step を 1 段挟んで回避したが、これは設定側の
  工夫であって修正ではない（2026-07-31 に判明、未修正 → §6.11）
- **cancel された straggler は workflow の `autoClose` ではなく agent の `close_on_exit` に従う**:
  kill 時に `outcome.session_id` が消えるので frontend が workflow 所属を判定できなくなるため。
  2026-07-31 の設定では偶然それが望みどおりだったが、意図した挙動ではない（未修正 → §6.11）
- **2 体レビューの「突き合わせ」を設定で書けない（cross-model review）** — **2026-08-07、原因 (1) は解消。残りは (2) のみ**（`handoff_bodies` は同一 target を指す全 source の本文を宣言順に連結するようになった。→ CONTRACT.md 続報13）。以下は発見当時の記述で、(1) の段落は歴史的経緯として残す: 「実装 → 別モデル 2 体が
  並行レビュー → 結果を突き合わせて判定」のうち、**並行レビューまでは今日の実装で書ける**
  （`pattern: supervisor` の制約は root ちょうど 1 つ + 他は全員 root 依存だけなので、レビュー 2 体を
  並べ、判定 step を `dependsOn: [root, reviewA, reviewB]` の 3 本依存にしても root を含む限り通る
  = `config.rs:971-999`）。書けないのは突き合わせのほうで、原因は 2 つ。(1) `handoff_bodies` は
  ターゲット 1 つにつき本文 1 本しか運ばない（`orchestrator.rs:1906-1926` の
  `if bodies.contains_key(target) { continue; }`）ため、レビュアー 2 体が両方 `handoffTo: verdict` を
  宣言しても、設定に先に書いたほうの本文だけが判定 step の kickoff に前置され、もう 1 本は
  エラーにも警告にもならず捨てられる。(2) `condition_targets` は `depends_on.first()` しか見ない
  （`orchestrator.rs:2075-2093`）ので、依存が複数ある step に `condition:` を書いても評価対象は
  1 本目だけで、「両方が ACCEPT なら進む」を `condition` で表現できない。**今日できる回避策**:
  workflow 上は `joinOn: reply` で「2 体とも返信した」ことだけを同期に使い、中身の受け渡しは
  固定名の mailbox（`send_inbox`）かファイル経由にする — `reply_inbox` は返信の宛先を元メッセージの
  sender に固定する（`queen_store.rs:838` の `original.sender`）ため workflow の返信は
  `queen:workflow/<name>/<runId>` に戻り、run id を知らない判定 step からは読めない。**直すなら**
  (1) は `HashMap<String, String>` を `HashMap<String, Vec<String>>` にして kickoff に連結する話、
  (2) は「全依存の AND を許すか」という設計判断が要る。どちらも
  [spec-oneach-reply-5.0.7.md](../spec/spec-oneach-reply-5.0.7.md) と同じ層（依存と完了判定の
  表現力）なので、着手するならまとめて検討する。v0.5.9 以降の候補（2026-07-31 に調査、未修正）
- **`arena: true` が実装を伴わない**: 誤解を招くので、Arena 実装（P6）までの間は
  [ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md) §1 の ❌ 表記を維持する
- **`orchestrator.rs` のコード内コメントの「phase 5.0.5」表記**: 整理コミットで削除する
  （5.0.5 は Arena view 用に予約済みで、採番の食い違い自体は §1 の脚注※で決着済み）
- **`src-tauri/src/orchestrator.rs.bak` が git に追跡されたまま**: live source ではないが、ガイド §1 が
  「commit 済みの `.bak` に旧コードが残るが実行系とは無関係」と注記せざるを得ない。次の整理コミットで削除
- **anthropics/claude-code#26572**（CustomPaneBackend 公式化）: 採用されたら
  シム撤去 + `CLAUDE_PANE_BACKEND_SOCKET` 広告へ移行（teams-backend はそのまま使える）
- 残りの Defer 項目（backend M5/M8/L3/L4/L6/L7/L11 系、frontend BUG-8/10、
  security S3 caller-id・Low 群）は evaluation の推奨ロードマップに従い順次

---

## 4. バージョニングとリリース

**タグとバージョンの話を書くのはこの節だけ**（§1 のリリース列はタグ名のみを持つ）。当初は 3 ファイル
とも `0.1.0` のままで実態とズレていたため、次の規約を導入した。

> **注意**: 下記のタグは device 側で実在を確認した事実だが、**この作業ツリーには git tag が 1 本も無い**
> （履歴を 1 コミットに圧縮した baseline のため）。ここで `git tag` を叩いても 0 件しか出ず、
> 個別コミットハッシュも同様にローカルでは追えない。

### 規約（SemVer 0.y.z、1.0 まで）

- **y（minor）= Phase 番号**。Phase 4 系の間は `0.4.z`、Phase 5 系に入ったら `0.5.0` から。過去に
  当てはめると Phase 3.9 時点 ≒ `0.3.9` 相当（遡及タグは付けない）。
- **z（patch）= その Phase 内のリリース連番**。機能追加・修正の区別はしない
  （pre-1.0 の SemVer では minor が破壊的変更の単位のため、これで矛盾しない）。
- **破壊的変更**（config スキーマ・IPC/MCP 契約・保存データ）は pre-1.0 でも「CONTRACT.md への契約
  追記 + 互換パス（例: mterm.yml フォールバック）」を必須とし、やむを得ず互換を切る場合は y を上げる。
- **1.0.0 の条件**: ~~License 決定~~（`d3eac32` で MIT 確定済み）、macOS 安定 + Linux beta 卒業、
  teams host（4.2）の実機安定、config スキーマ凍結。残りは実機検証系（U6 / U7）とスキーマ凍結の 3 点。
- 原則 **1 リリース = 1 patch**。v0.1.0 のまま Phase 4.2 まで進めたため遡及タグ（v0.4.0 / v0.4.1）は
  付けず、v0.4.2 を最初のリリースタグに集約した。

### タグ実績（13 本）

実タグは `v0.4.2`〜`v0.4.9` / `v0.5.0` / `v0.5.1` / `v0.5.6` / `v0.5.7` / `v0.5.8` の **13 本**。
**`v0.5.2`〜`v0.5.5` は存在しない**（`v0.5.6` のタグメッセージが Phase 5.0.2〜5.0.5 用に予約と宣言した
まま実装が別の順序で進んだため）。次のタグは `v0.5.9`（未作成。
[spec-phase5-5.md](../spec/spec-phase5-5.md) §9 の予約では Phase 5.5.1）。`v0.5.7` と `v0.5.8` を続けて
Phase 5.0 系に充てたため、同 §9 の「バージョン割り当て」表の予約は
**2 度繰り下がり**、現在は 5.5.1 = `v0.5.9` / 5.5.2 = `v0.5.10` / 5.5.3 = `v0.5.11` /
5.5.4 = `v0.5.12` である（同 spec 側で対応済み。以前の対応表はもう有効でない）。作成日は
v0.4.2〜v0.4.6 が 2026-07-16〜17、v0.4.7〜v0.4.9 が 2026-07-18、v0.5.0 / v0.5.1 / v0.5.6 が
2026-07-23、`v0.5.7` が 2026-07-30、`v0.5.8` が 2026-08-13。

| バージョン | 内容 |
|---|---|
| ~~v0.4.0 / v0.4.1~~ | 個別タグは打たず **v0.4.2 に集約** |
| v0.4.2 | Phase 4.0（hooks 受信基盤）〜 4.1（observe）〜 4.2（host モード実験）+ UX トラック一式（最初のリリースタグ） |
| **v0.4.3** | 調査対応の安定化リリース: バグ修正 20 件（backend 12 / frontend 8）+ セキュリティ 4 件（S1 Queen 認証 / S2 trust / S4 CSP）+ 手打ち claude の lead 帰属修正 + 認証トークンの永続化。cargo test 159 / teams-backend 30 |
| **v0.4.4** | Phase 4.4.0 / 4.4.1 + `QUEEN_TOKEN` の各ペイン注入と Queen バッジへの codex/grok 登録追加 + Queen 登録コマンドの冪等化（remove→add）+ 手動起動 / node 起動エージェント（grok）の foreground 名解決 + フッターのサイドバー開閉トグル（右上チップ廃止）。タグメッセージは `chore: release v0.4.4` のみで、内容は `v0.4.3..v0.4.4` の 6 コミットから判定 |
| **v0.4.5** | 左ドックのタブ化と Git の統合（フロート廃止）+ ツールバーの Git ボタン撤去・状態表示のフッター集約。タグメッセージは `chore: release v0.4.5` のみで、内容は `v0.4.4..v0.4.5` の 2 コミットから判定。README のスクリーンショットはこの断面（`docs/screenshot-phase0.4.5.png`） |
| **v0.4.6** | Phase 4.3（team preset）。Phase 4.4.2（アプリ外通知）もこの断面に含まれる。cargo test 210 / svelte-check 0 |
| **v0.4.7** | UI 多言語化（en/ja。型付き辞書 `i18n.svelte.ts`、⚙ 設定メニューで 自動/English/日本語 切替、既定=OS 言語に自動追従・英語ベース）。フロントのみ、backend 文言・ログは対象外。svelte-check 0 / build 成功 |
| **v0.4.8** | Phase 4.4.3 ssh 接続先表示（`session-resources` の foreground に `detail?` を追加し argv から宛先抽出。ヘッダーとサイドバーに `ssh user@host` を表示。`.ssh/config` alias・`-l` 畳み込み対応）。cargo test 214 / clippy 0 / svelte-check 0 |
| **v0.4.9** | フォアグラウンド名解決の汎用化（opencode 等の node / python 起動エージェントを実体名で表示、`81ade5a`）+ 接続先表示ドキュメントの追随（sftp / scp / mosh / telnet / kubectl / docker、`e5b72d8`）。`v0.4.8..v0.4.9` は release コミット込みで 3 コミット |
| **v0.5.0** | Phase 5.0.0 MVO（`0182988` + `b1b4f1f`）。cargo test 246。同区間にはクラウド LLM の API キー利用ドキュメント（`665ee82` / `461d2a9`）も含まれる |
| **v0.5.1** | Phase 5.0.1 Workflow Resume。cargo test 251。**frontend（`src/`）はこの断面が最後の変更** |
| **v0.5.6** | Phase 5.5.0 RC 互換ルータ。lib 286 + 統合 14 tests / clippy `-D warnings` clean。タグメッセージは「`v0.5.2`〜`v0.5.5` は Phase 5.0.2〜5.0.5 用に予約」と宣言している |
| **v0.5.7** | Phase 5.0.2 `ptygrid init`（環境検出→テンプレート生成→自己検査、実機確認済み）+ Phase 5.0.4 Orchestrator 実行層（`retry:` / `timeoutMs` / `condition:` / `handoffTo` / `joinOn: reply`、supervisor・handoff の spawn ゲート撤去）+ `fanOut` 黙殺解消・straggler 協調キャンセル + ハードニング（pane 上限待ち行列化 / driver tick 軽量化 / inbox mailbox の run 単位分離）+ docs 公開/内部分離 + MIT license 宣言。lib 402 + 統合 14 tests |
| **v0.5.8** | 3 系統。(1) `v0.5.7` 以降に `main` へ入っていたぶん: Phase 5.0.7 `onEach: reply` / `joinOn: stream`（per-copy mailbox の修正込み）+ 5.0.2 追補のローカル LLM プローブ + 5.0.6（案）の計測フィールドと合成 workflow + ターミナルのコピー & ペースト + タイトルバーのバージョン表示。(2) Stage A（`docs/design/next-implementation-2026-08.md` §3、新採番なし）: A-2/A-3 resume の carry 喪失ガードと `handoffTo` 合流 / A-4 retry 枯渇の escalation 配線 / A-5 cancel・abandon の kickoff ack / A-6 `workflow_runs` の retention。(3) Phase 5.0.8 `schedule:`（`every: day / weekday / hour` + `at:`、レビュー是正 3 巡ぶんを含む）。version 3 ファイルを `0.5.8` に揃えたコミットに打った最初のタグ。lib 530 + 統合 14 tests。**実機検証は 1 つも消えていない**（U14 残 3 点 / U17 / U18 / U19 / U20）、**CI は未確認**（→ §6.23） |

> **v0.5.6 タグメッセージの注意**: 同メッセージは「untagged 5.0.2 Workflow Reliability
> (retry/timeoutMs/joinOn:reply/escalation) と integrator エージェントを同梱」と書いているが、
> `v0.5.1..v0.5.6` の 9 コミットはすべて 5.5.0（`queen_compat`）関連で、`retry:` / `condition:` /
> `handoffTo` のスキーマが `config.rs` に入るのは v0.5.6 **より後**の `5d3c1b5` である（v0.5.6 断面に
> あるのは 5.0.0 由来の `timeout_ms` のみ）。integrator エージェントは gitignore 対象の
> `ptygrid.yml` 側の定義でありタグの内容としては追跡できない。

### v0.5.7 として release 済み

`v0.5.6..main` の 45 コミット + 未マージだった 5.0.2 `ptygrid init`（docs 1 件）をまとめて
`v0.5.7` としてリリース済み（2026-07-30。60 files changed, +12,673 −1,005）。個別コミットの内訳は
タグメッセージと `git log v0.5.6..v0.5.7` に残るためここには重複させない。当時の検証と注記は §6.8。

番号は連番を優先する **(a) `v0.5.7`** を採用した（予約どおり `v0.5.4` を後追いする (b) 案は不採用）。

### v0.5.8 として release 済み

**2026-08-13 にリリース済み**（経緯と検証値は §6.23）。`v0.5.8` は spec-phase5-5.md §9 の予約では
Phase 5.5.1（OTel + SQLite シンク）だったが、**`v0.5.7` のときと同じ判断（タグ順と時系列を一致させる
= 先に完成した成果へ先の番号を与える）を通し**、下記の内容に割り当てた。5.5.1〜5.5.4 は
`v0.5.9`〜`v0.5.12` へもう 1 つ繰り下げた（同 spec 側で対応済み）。なお項目 2 は Phase 5.5.1 の前段
そのもの（5.5.1 の `observability.rs` が読む値を先に揃える）なので、繰り下げても 5.5.1 の着手は
遅くならない。

**タグに載ったのは 3 系統**である: (1) 下記の実装項目 1〜8 のうち消化したぶん（= `v0.5.7` 以降に
`main` へ入っていたもの。5.0.7 を含む）、(2) **予定に無かった Stage A**（項目 9）、
(3) **予定に無かった `schedule:`（Phase 5.0.8）**（項目 10）。**予定していて消化しなかったのは
項目 5（U4）の 1 つ**で、未消化のままタグを打っている（→ §6.23）。

**実装項目 1〜8（当初この順序で進める計画だったもの）**。順序の根拠は「決定論的なものから」
「安いものから」「あとの判断を分岐させるものを先に」:

1. **5.0.2 追補: ローカル LLM プローブ**（**PR #11 / #12 でマージ済み**。`main` に `17860e0` /
   `b8300a4`）。`init_probe_llm`（4 本目の Tauri
   command。既定 11434 / 1234 / 3456 + 手入力最大 4 本に `GET /v1/models`、1 ポート 1 秒・全体 3 秒・
   応答 64KB・モデル 20 件上限）、確証は `GET /api/version` が 0.14.0 以上のときだけ、確証ありは有効な
   agent 定義・それ以外はコメント行、`autostart` は常に false、モデル選択の `<select>`（既定は
   埋め込み/画像/音声/再ランクらしい名前を除いた先頭）、検出行への反映、`ANTHROPIC_AUTH_TOKEN` の
   出し分け。ブランチ側のコミットは `63af84f` と `8931464`。**前提の訂正**: Ollama v0.14.0 以降と LM Studio が Anthropic Messages API
   互換になったため、旧設計が想定していた translation 層（coderouter を挟む）は不要になった。
   lib 402 → 419、統合 14 不変、svelte-check 122 files 0/0。
2. **計測フィールドの追加**（additive）— **実装済み + 実測済み**（`9442758`）。`orchestrator.rs` の
   `StepOutcome` は `started_at_ms` しか持たず step 単位の終了時刻が無く、`deferred_since_ms` は
   `#[serde(skip)]` で persist されないため、**fan-out の各コピーの所要と、ペイン待ちに使った時間が
   事後に出せない**状態だった。`ended_at_ms`（Option、終端到達時に 1 度だけ押す）と
   `waited_for_pane_ms`（累積。再 spawn をまたいで残る）を additive 追加し、所要と待ちを別々に出す。
   下の項目 3 で実機の数字が取れたので、**この 2 フィールドが実機で機能することは裏づけ済み**。
   Phase 5.5.1 の前段。
3. **合成 workflow**（`sh -c 'sleep N'` でエージェントを置き換え、think time を排除して orchestration
   の効き = tick 間隔・spawn コスト・9 面上限の待ち・依存の解け方だけを測る）— **実装済み + 実測済み**
   （`af723ca`。`example/measure-parallelism/ptygrid.yml`）。当初「3 本」としていたが、9 面上限の待ちを
   測る版を分けたので **実際は 4 本**（`measure-1-serial` / `measure-2-split` / `measure-3-pane-queue` /
   `measure-4-join-any`）。2026-07-31 に macOS 実機で実測（各回スクリーンショットで確認済み。詳細と
   時刻は §6.11、U2 の消化は §2）:
   - `measure-1-serial`: run は 31 秒（理想 30 秒）。step の所要は先頭 5.2 秒・残り 5 つが 5.1 秒。
     依存エッジ 5 本に対しオーバーヘッド 1 秒 = **1 エッジあたり 0.2 秒**で、これは driver tick
     （200ms）ちょうど 1 回分にあたる。
   - `measure-2-split`: step の所要は 5.2 / 5.1 / 5.2 / 5.1 / 5.2 / 5.1 秒で **直列版と変わらない**。
     同時 3 枚でも spawn は重くならない。（run 全体の壁時計はスクリーンショットに写っておらず**未記録**）
   - `measure-3-pane-queue`: `gate` 0.2 秒、`wave-a#0`〜`#5` が各 5.1 秒、`wave-b#0`〜`#5` が各
     5.1 秒（**待ち 8.2 秒**）。サンプルの予測値は待ち約 8.0 秒だった。
   - **結論**: orchestration のコストは 1 依存あたり 200ms、spawn は 0.1 秒程度。実タスクの workflow が
     遅いとすれば**原因はここではない**（→ §3 P5 の実タスク測定へ）。
   - `measure-4-join-any` の 3 回で **U2（straggler 協調キャンセルのペイン kill）を消した**（→ §2）。
     その過程で **frontend の誤バナーを 1 件修正**（`58e9c95`）: 敗者 kill のたびに「ペインの停止に
     失敗しました (kill_pty #N): session N not found」が出ていた。`cancel_stragglers` が
     `outcome.session_id` を `None` にする（kill 済みペインは再利用も回収もしない）ため、その id で
     workflow 所属を判定している `autoCloseModeFor` が所属を見失い、agent の `close_on_exit: always`
     にフォールバックして 3 秒後に `closePane` → `kill_pty` を呼ぶが、backend ではスロットが既に無い、
     という経路。`check_timeouts` も同じく `session_id` を消すので同じ潜在経路を持っていた。修正は
     **`session N not found` だけを握り潰す**（`kill failed: …` は本物の孤児プロセスなので従来どおり
     バナーに出す = BUG-5 の意図を維持）。
4. **cold start の実測**（同一 step を、既存ペインを再利用する場合と毎回 spawn する場合で比較）—
   **実測済み**（`example/measure-coldstart`）。ここだけは実エージェントが必要だった。2026-07-31、
   macOS 実機で `measure-coldstart` を実行（3:49:38 開始、run は SUCCEEDED。スクリーンショットで
   確認済み。詳細は §6.12）:
   - step の所要は `s1-cold` **7.7 秒**（fresh spawn）、`s2-warm` **4.1 秒**（同じペインを再利用）、
     `s3-warm` **4.5 秒**（同上）。warm の平均 4.3 秒に対し **cold start は約 3.4 秒**。
     ばらつき（s2 と s3 の差）は 0.4 秒なので、3.4 秒はノイズの 8 倍以上あり有意。
   - 内訳の解釈: warm の 4.3 秒はほぼ**モデルの 1 往復**（kickoff が inbox に入る → await が起きる →
     モデルが読んで `reply_inbox` を呼ぶ → 次の tick で検出）。cold はそれに加えて**プロセス起動 +
     CLI のブート + MCP 接続 + 最初の await 設定**がかかり、その差が 3.4 秒。
   - **但し書き: この 3.4 秒は cold start の下限である**。計測用プロンプトが「考えず、調べず、
     ファイルも読まず」と明示的に禁じているため、**実タスクの cold start に含まれるはずの
     CLAUDE.md / リポジトリ / pins の読み直しが一切入っていない**。測れたのは起動の事務コストだけで、
     実タスクではこれより大きくなる。
   - **副産物: 常駐の実現可能性**。3 段とも返信で完了した（route 3）。エージェントの recap も
     「3 通すべてに ok を返し、そのあと 2 回続けて timedOut したので降りた」と自己申告している。
     つまり**走り続けているエージェントが 2 通目・3 通目の kickoff を実際に拾えた**。`mode: serve`
     （常駐ワーカー）が必要とする挙動そのものが実機で成立している。
   - **判断（次に作るもの）**: **`onEach: reply`（ストリーミング依存）を先に作り、`mode: serve`
     （常駐ワーカー）は後**。根拠は取り分の桁の差で、`mode: serve` が節約するのは 1 step あたり
     3.4 秒 + 文脈の読み直し分（10 step 回しても数十秒）なのに対し、`onEach: reply` が節約するのは
     「上流が全部終わるまで下流が待つ」という**工程まるごとの待ち時間**で、実タスクなら数分単位に
     なる。合成 workflow の実測で orchestration 自体は 1 依存あたり 200ms しか食っていないことが
     分かっている（→ §6.11）ので、削るべきは待ち時間のほう。`mode: serve` を捨てるわけではなく
     順番が後で、文脈の読み直しコストを別途測ってから判断してもよい。spec 執筆は §3 P5。
5. **U4 の消化**（同名 workflow の並行 run が互いの返信を取りこぼさないこと）。2 つの構成を
   同時に流して比較する場合の前提になる。**（2026-08-13）この項目は未消化のまま `v0.5.8` を打った。**
   予定に入れた項目を消さずにタグを打つのは、v0.5.7 までの「タグには実機で見たものを入れる」運びとは
   食い違う。`next-implementation-2026-08.md` §3 の A-1 も「v0.5.8 を出す。**U4 の消化を含む**」と
   書いているので、その完了ゲートも満たしていない。U4 は §3 P1 の枠に残る（→ §6.23）。
6. **リリース雑務 — version ファイルの食い違いの記録**: タグ `v0.5.7` が指すコミット（`4e9afb3`）
   では 3 つの version ファイルが `0.5.6` のままで、`0.5.7` に上げたリリースコミット（`27ecd90`）は
   ブランチ側に残っていた。**公開済みタグは動かさない**方針とし、`v0.5.8` は version を揃えた
   コミットに打つ。**（2026-08-13）消化した**: `package.json` / `src-tauri/Cargo.toml` /
   `src-tauri/tauri.conf.json` の 3 つを `0.5.8` に揃え（`src-tauri/Cargo.lock` は `cargo check` で
   追従）、そのコミットに `v0.5.8` を打っている。（`.gitignore` への `src-tauri/target-basemain/`（2.2GB）と `ptygrid.yml-20260729`
   の追加は、ユーザーが `89411b9` で `main` に直接コミット済みのため本項目からは落とした。）
7. **（後から入った実装済みの項目）ターミナルのコピー & ペースト**。上の 1〜6 は依存関係の順に
   並んでいるが、本項目はその並びが決まったあとに入った成果で、先行項目の前提にも依存先にも
   なっていないため末尾に置く。**背景**: ターミナルペインで範囲選択したテキストをコピーできず
   貼り付けもできなかった。原因は 4 つで、(a) 選択自体は阻害されていなかった（`user-select: none`
   は toolbar / dock / statusbar / pane-header のみで、xterm は自前の選択モデルを持つ）が、
   (b) `terminals.ts` の xterm 生成がテーマ・フォント・scrollback しか渡しておらず、キーハンドラも
   右クリックメニューもクリップボード呼び出しも無かった（xterm の選択は DOM の選択ではないので
   WebView の Cmd+C にはコピー対象が見えない）、(c) `src-tauri` にメニューが 1 つも定義されておらず、
   macOS の WKWebView では Edit メニューが無いと Cmd+V が `paste` イベントにならない、(d) TUI が
   マウスレポートを有効にすると選択できず逃げ道も無かった。**入ったもの**: macOS 限定
   （`#[cfg(target_os = "macos")]`）のアプリメニュー（App / Edit / Window。Edit に標準の Undo /
   Redo / Cut / Copy / Paste / Select All）/ `tauri-plugin-clipboard-manager`（Rust 2.3.2 /
   JS 2.3.2、capability の許可は **`clipboard-manager:allow-read-text` の 1 つだけ**。書き込みは
   既存の `navigator.clipboard.writeText` 経路をそのまま使うので足していない）/ コピーは macOS が
   Cmd+C・それ以外が Ctrl+Shift+C で、**選択が無いときは介入せず PTY へ流す**（素の Ctrl+C の
   SIGINT を壊さない）/ 貼り付けは **macOS ではネイティブ経路に一本化**（メニューのアクセラレータが
   keydown より先に Cmd+V を食うことがあり、発火済みのアクセラレータは `preventDefault` で
   取り消せないため、自前でも読むと二重に入る）、Ctrl+Shift+V はネイティブ `paste` が出ないので
   自前ハンドラが唯一の経路 / 貼り付けは `term.paste()` を通す（bracketed paste は xterm 任せ、
   `ignoreBracketedPasteMode` は既定の false のまま）/ 右クリックメニュー（コピー / 貼り付け。
   選択が無いときコピーは無効表示 + 理由のツールチップ、Esc / 外側 mousedown / window blur で閉じ、
   リスナは全て一緒に外れる）/ `macOptionClickForcesSelection: true`。コミット `8a83032`、ブランチ
   `feat/terminal-copy-paste`（`main` から分岐、**push 未・PR 未**）、13 ファイル。**仕様どおりに
   できなかった点**: ユーザーは「TUI 中の選択は Option ドラッグ」を選んだが、**xterm.js の実装では
   Option ドラッグは macOS 限定**で、バンドルの実物が `isMac ? altKey && macOptionClickForcesSelection
   : shiftKey` になっている。**Linux と Windows では Shift ドラッグ固定**でオプションでは変えられない
   ため、UI のヒントとコメントは「macOS は Option、それ以外は Shift」と書いてある。**自動テストの
   実測**: lib 419 / 統合 14 は不変、clippy は既存の `config.rs:834` の `nonminimal_bool` 1 件のみ、
   svelte-check 136 files 0 errors 0 warnings、`npm run build` 成功（メニューは macOS 限定で
   この作業環境の Linux では `cfg` で落ちるため、一時的に `cfg(all())` へ書き換えて実際にコンパイルと
   lint を通してから元に戻している）。実機検証は U13（一部済）。
8. **（後から入った実装済みの項目）タイトルバーに実行中のバージョンを出す**。コミット `c74997f`
   （`feat/schedule-5.0.8` の `1a48b1d` からの cherry-pick、`src-tauri/src/lib.rs` の 25 行のみ）。
   ウィンドウのタイトルが `ptygrid` だけで、「いま見ているのはどのビルドか」が画面のどこにも
   出ていなかった。`setup` フックで `window.set_title("{name} {version}")` を呼び、
   **`app.package_info()` から取る**（= `generate_context!` がビルド時に焼き込む値。`name` は
   `tauri.conf.json` の `productName`、`version` は同ファイルの `version`。後者が未設定のときだけ
   `CARGO_PKG_VERSION` にフォールバックする）。`tauri.conf.json` の**静的 `title: "ptygrid"` は
   そのまま残してある**: ウィンドウはまずその題で作られ、setup が直後に上書きする。main ウィンドウが
   取れないときは何もしない（best effort）ので、静的 `title` はそのときの表示でもある。
   **バージョン文字列を 4 つ目のファイルに増やさない**ためにこの形にした — version を持つファイルは
   既に 3 つあり（`package.json` / `Cargo.toml` / `tauri.conf.json`）、`v0.5.7` のタグが 3 つとも
   `0.5.6` のままのコミットを指している（上の項目 6）という前科がある。
   **この項目は schedule 機能とは独立**である（cherry-pick 元の `feat/schedule-5.0.8` は
   Stage A の後に回す判断で、時刻起動のコードは本ビルドに 1 行も入っていない）。
   wire 契約に変更は無いので CONTRACT.md には追記していないが、**ユーザーに見える挙動変更**なので
   ここに記録する。frontend 無変更、テスト数も不変。**実機での見え方は未確認**。

**（2026-08-13 追記）上の 1〜8 を決めたあとに、予定に無かった 2 系統が入ってタグに載った。**
どちらも [next-implementation-2026-08.md](next-implementation-2026-08.md)（2026-08-07）が書かれた
あとに順序が決まったもので、項目 1〜8 の並びの外側にある:

9. **（予定に無かった実装）Stage A — 足元固め**（A-2/A-3 / A-4 / A-5 / A-6。2026-08-13、
   `main` へマージ = `63ef201`。→ §6.15〜§6.18、CONTRACT.md 続報12〜17）。
   next-implementation-2026-08.md §3 が **「新採番なし。`v0.5.8` と直後の patch で消化」**と
   定義した枠なので、**まさにこのタグに載るべきもの**である。中身は A-2/A-3（step 間の carry を
   失った run の resume 拒否 + 同一 target を指す全 `handoffTo` source の合流）/ A-4（retry 枯渇の
   escalation を 4.4.2 の通知経路へ 1 通）/ A-5（cancel・abandon した run の未 ack kickoff を
   sender 選択で ack）/ A-6（`workflow_runs` を project ごと終端 run 500 件で刈る）。
   `user_version` は 3 のまま消費していない。lib 461 → 492 passed、統合 14 不変。
   **実機検証は 3 件とも未実施**（U17 / U18 / U19。A-2/A-3 ぶんの U15 / U16 も未実施）。
   なお同 §3 の A-1 は「v0.5.8 を出す（U4 の消化を含む）」なので、**A-1 の完了ゲートは
   半分しか満たしていない**（→ 項目 5）。
10. **（予定に無かった実装）`schedule:`（Phase 5.0.8）**（2026-08-13、`main` へマージ =
    `047a6ed`。→ §6.19〜§6.22、CONTRACT.md 続報18〜21）。実装そのものは 2026-08-05 に
    `feat/schedule-5.0.8` で書かれていたが、**当初は「Stage A の後に回す」判断**でブランチに
    留め置いた（項目 8 の但し書きがその判断の記録）。載せたのは A-4（escalation の配線）が
    入って前提が満たされたためである — 無人で時刻起動する run の失敗を外部へ知らせる経路が
    無い状態では、スケジューラは失敗を静かに溜めるだけの装置になる（§6.19 が最大の残件として
    挙げ、CONTRACT.md 続報21 (C) が続報18 の既知の限界 (d)「未配線」を失効と宣言した）。
    Stage A の上へ載せ替えたあと 3 巡のレビューで是正 7 + 1 + 11 件（→ §6.20〜§6.22）。
    新 wire は読み取り専用の Tauri command `list_schedules` 1 本のみ。lib 492 → 530 passed、
    統合 14 不変。**実機検証は未実施**（U20。実機では一度も時刻を待っていない）。

**タグの内容には数えないもの**: 実タスクでのベースライン測定と改良構成の比較、`mode: serve` の
spec 執筆（どちらも §3 P5。順序は項目 4 の実測で `onEach: reply` 先行に決まった）。

**（2026-08-04 追記）`onEach: reply` の実装が前倒しで入った。** 上の一文はもともと「`onEach: reply` /
`mode: serve` の spec 執筆」を数えないものとして並べ、実装を v0.5.9 以降に置いていた。実際には spec
（[spec-oneach-reply-5.0.7.md](../spec/spec-oneach-reply-5.0.7.md)）に続いて実装・自動テスト・CONTRACT
先行追記・`example/review-as-you-go` まで一度に入っている（→ §6.13）。**これを v0.5.8 に含めるか、
v0.5.9 として切るかはユーザー判断**であり、本文書はどちらとも決めない。判断材料は 2 つだけ:

- **含める場合**: v0.5.8 は「計測して、その結果で決めた機能まで」という 1 本の筋になる。ただし
  **実機検証（U14）が未消化のまま**タグに入ることになり、v0.5.7 までの「タグには実機で見たものを
  入れる」運びとは食い違う。
- **分ける場合**: v0.5.8 は計測とターミナルのコピー & ペーストで閉じ、5.0.7 は U14 を消してから
  v0.5.9 として切れる。項目 5（U4 の消化）は依然 v0.5.8 の残作業として残る。

**（2026-08-13）決着: 5.0.7 は `v0.5.8` に含まれる**（= 上の「含める場合」）。判断ではなく
**事実として決まった**もので、根拠は 5.0.7 が `v0.5.7`（2026-07-30）より後に `main` へマージ済み
だったこと（`b2e72ab` = PR #18、2026-08-05。その前に PR #16 / #17）である。`v0.5.8` は `main` の
先端に打つので、5.0.7 は含まれる以外にない。**「分ける場合」の論拠だった「U14 を消してから
v0.5.9 として切る」は取られなかった** — U14 は残 3 点のまま `v0.5.8` に入っており、上の一文が
食い違いとして予告したとおり、**v0.5.7 までの「タグには実機で見たものを入れる」運びとは
食い違っている**。同じ食い違いは項目 5（U4）と、あとから載った項目 9 / 10（U17 / U18 / U19 / U20）
にも及ぶ（→ §6.23）。

### リリース手順（タグ付けの作法）

1. `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` の `version` を一致させて
   更新（`Cargo.lock` は `cargo check` で追従）
2. 全チェック（`cargo test` / `clippy` / `npm run check` / `npm run build`）通過を確認。
   **v0.5.8 断面の注意**: 項目 7（ターミナルのコピー & ペースト）で JS 側の依存が 1 つ増えたので、
   この断面を取り込んだら**チェックの前に `npm install` が必要**（未実行だと `vite` が
   `Failed to resolve import "@tauri-apps/plugin-clipboard-manager"` で落ちる。実機で実際に踏んだ）
3. `git tag -a vX.Y.Z -m "<リリース概要>"` → push（annotated タグのみ。軽量タグは使わない）
4. 変更履歴は当面 CHANGELOG.md を作らず「タグメッセージ + `git log` + 本文書の表」で代替。License は
   `d3eac32` で MIT 確定済みなので、本格的な公開に踏み切るタイミングで CHANGELOG.md 化を再検討する
5. 将来課題: 3 ファイルの version 同期を `scripts/` の bump スクリプトにする（未着手）

---

## 5. 設計上の予約（変更しにくい取り決め）

先に決めてしまうと後から動かしにくいもの（DB スキーマ番号・branch 命名・並列化の枠組み）をここに
集める。詳細は 3 spec（[spec-phase5-0.md](../spec/spec-phase5-0.md) /
[spec-phase5-5.md](../spec/spec-phase5-5.md) / [spec-phase6-0.md](../spec/spec-phase6-0.md)）と
`docs/inside/phase5-6.md`（git 管理外）を参照。

### 5.1 SQLite `PRAGMA user_version` 予約表

migration は additive、既存 `queen.sqlite3` を壊さない。version bump は Phase 単位で予約する:

| user_version | Phase | 追加テーブル | patch |
|---|---|---|---|
| 1 | Phase 3.6 | pins / notes | 3.6 |
| 2 | Phase 3.7 | inbox / reply | 3.7 |
| **3** | **Phase 5.0** | `workflow_runs`（`queen_store.rs`: v0→v3 の新規作成、v1→v3、v2→v3 のいずれの経路も `WORKFLOW_RUNS_SCHEMA_SQL` を適用して `PRAGMA user_version = 3` に到達）／ `memory` + `memory_fts` + `memory_vec` | 5.0.0 / 5.0.1 / 5.0.2 |
| **4** | **Phase 6.0** | `replays`、`secrets_audit`、`sandbox_events` | 6.0.0 |

> 実装値は `user_version` = 3。`queen_store.rs` は `version > 3` を「unsupported Queen database
> version」で開かずに弾く（v4 の予約は表のみ）。なお「`workflow_runs` と `memory` を同じ v3 で
> 導入し 5.0.0 で skeleton・5.0.1 で本格実装」という下の規律は、実際には **5.0.1 が Workflow
> Resume に充てられ memory は着手されなかった**ため、v3 は `workflow_runs` のみで確定している。
> memory 系テーブルを追加する場合は additive migration を v3 内で行うか v4 を切るかを、
> 着手時に決め直す必要がある（§3 P6）。

**規律**: 未知の新 version は黙って開かない（明示 error でユーザーに再インストールを促す。Phase 3.6
の規律を継承）/ migration は transactional で既存の pins/notes/inbox データを壊さない /
Phase 5.0 の `workflow_runs` と `memory` は同じ v3 で導入し 5.0.0 で skeleton・5.0.1 で本格実装
（2 patch にまたがる migration は 1 回のみ）/ Phase 6.0 の 3 テーブルは同じ v4 で同時導入（6.0.0）。

### 5.2 Track 別 branch 命名規則

MVO（5.0.0）完成後、Track A/B/C/D を並列に走らせる。branch は 1 patch = 1 branch を基本とし、
以下の prefix を強制する:

| Track | prefix | 例 | 対応 patch |
|---|---|---|---|
| Track A(UI) | `track/a-ui-*` | `track/a-ui-5.5.3-status-rings` | 5.5.3 / 5.5.4 / 5.0.5 / 6.0.5 |
| Track B(MCP+観測) | `track/b-mcp-*` | `track/b-mcp-5.5.0-rc-router` | 5.5.0 / 5.5.1 / 5.5.2 |
| Track C(Memory+Provider+Orch完成) | `track/c-memory-*` | `track/c-memory-5.0.1-fts5` | 5.0.1 / 5.0.2 / 5.0.3 / 5.0.4 |
| Track D(Security) | `track/d-security-*` | `track/d-security-6.0.2-strict-sandbox` | 6.0.0〜6.0.4 |
| MVO(先行、Track に属さない) | `mvo/*` | `mvo/5.0.0-orchestrator` | 5.0.0 |
| その他 | `main`(直マージ不可)、`bug/*` / `docs/*` | | |

**コーディネーション制約**:

- `CONTRACT.md` の Phase 節は additive のみ。異なる Track が同時に同じ Phase 節を触ると merge 競合が
  起きるので、各 patch はその patch 用の subsection を先に予約する（スケルトンは用意済み）。
- `queen.rs` は薄いディスパッチャに保ち、各 tool 実装は別 module（`orchestrator.rs` / `memory.rs` /
  `secrets.rs` / `sandbox.rs` / `replay.rs` / `provider.rs` 等）に閉じる。
- `session.rs`（PTY hot path）は Track A(UI)/D(sandbox tee tap) の両方が触るので、Track D が先に
  tee tap を入れて、Track A は tap 済み event を購読するだけにする。
- GitHub Actions の concurrency group を Track 別に切る。merge queue を利用して直列化。
- 人手レビューは Track D(Security) を最優先。Sandbox / Secrets は毎日固定 2 時間のレビュー枠を
  確保、他 Track は Opus adversarial verify で 8 割済ませる。

### 5.3 実装 dev workflow 用の agent / workflow 定義

`ptygrid.yml` の `agents:` に 4 種（`opus-planner` / `sonnet-coder` / `opus-reviewer` / `sonnet-docs`）、
`workflows:` に 4 track（`track-a-ui` / `track-b-mcp-otel` / `track-c-memory` / `track-d-security`）を
定義済み。`spawn_workflow {name: "track-b-mcp-otel"}` の Queen tool 呼び出しで各 Track の 1 patch
サイクルが回る想定（design → implement → verify → docs、Track D は verify → redteam → docs）。

---

## 6. 進捗記録（日付つきアーカイブ）

各記録は**その時点で書かれたまま**で後から更新していない。完了/未完了の判定はここには書かない（→ §1）。

### 6.1 2026-07-22: Phase 5.0.0 MVO

詳細な経緯。現在地は §1。

- 入ったもの: `workflows:` スキーマ + 検証（config.rs）/ orchestrator.rs（spawn + DAG 進行ドライバ、
  完了判定 2 経路、fail-fast、fan-out fresh-spawn）/ Queen MCP tools 22 本（`spawn_workflow` /
  `join_workflow` / `cancel_workflow` 追加）/ Tauri commands 3 本 + `workflow-state` イベント /
  WorkflowPanel.svelte + 🔀 チップ。CONTRACT.md「Phase 5.0 追加契約」に確定契約を追記。
- 当時の検証: cargo test 246 / clippy 0 / svelte-check 0 / vite build 成功 / 実機で config 読み込みと
  チップ表示を確認。
- 当時の注記: run registry は in-memory（app 再起動で消える）。SQLite `workflow_runs` +
  user_version 2→3 は 5.0.1 へ。supervisor / handoff / retry / timeout / join_on reply|N は 5.0.4 へ。
- 当時は「v0.5.0 タグは workflow 実走スモークテスト通過後」という方針だったが、**実際には
  v0.5.0 は 2026-07-23 に実走前にタグ付けされた**（§4 のタグ表が正）。

### 6.2 2026-07-23: Phase 5.0.1 Workflow Resume

詳細な経緯。現在地は §1。

- 入ったもの: `workflow_runs` 永続化（user_version 2→3）+ write-through、`workflow-resume-pending`
  イベント + Y/N バナー、`resume_workflow` / `abandon_workflow` commands。
- 当時の検証: cargo test 251 / clippy 0 / svelte-check 0。

### 6.3 2026-07-28〜29: Orchestrator ハードニング（pane 上限 / driver tick / mailbox）

詳細な経緯。現在地は §1。

設計メモ（作業中はリポジトリ直下の `DESIGN-refactor-5.0.5.md` として書き、コミット時に
[refactor-pane-cap-5.0.5.md](refactor-pane-cap-5.0.5.md) へ移して `2dc5e40` に同梱）に沿って
5.0.4 Orchestrator の非機能面を
3 点リファクタ。**wire 契約は無変更**につき CONTRACT.md は追記のみ（続報10）で対応した。

- pane 上限（9 面）が埋まっている間、spawn できない step は `Failed` ではなく `Pending` のまま
  待ち行列化（最大 `WORKFLOW_DEFER_MAX_MS` = 5 分、超過で従来どおり `Failed`）。`timeoutMs` は
  待ち時間を含まない仕様として確定。
- `PtyManager::session_states()` / `live_session_count()` を新設し、driver tick / `team_presets` /
  `queen list_agents` の内部計算が `ps` fork を伴う `list_sessions()` を呼ばなくなった
  （`list_agents` の返り値自体は不変）。`WorkflowRegistry` に終端 run の evict（`REGISTRY_TERMINAL_CAP` = 100）を追加。
- workflow の inbox mailbox を `queen:workflow/<name>` から `queen:workflow/<name>/<runId>` へ変更し、
  同名 workflow の並行 run がもう mailbox を共有しないようにした（新規制約: workflow 名は 84 バイト以下）。
- 当時の検証: cargo test 374 passed（lib）+ 14 passed（統合）。svelte-check は backend のみのため対象外。
- **2026-07-29 未明**: レビュー指摘（F1/F4/F5/F6/F7/F8）の反映を経て `2dc5e40` に squash され、
  PR #3（`4c02cbb`、2026-07-29 01:35 +0900）で `main` にマージ。CONTRACT.md 続報10 が書いている
  「`main` 未マージも変わらない」はこの時点で失効。lib テストは 374 → 375 に増えた（§4 の実測）。
- **当時の結論**: wire 契約が不変の内部ハードニングであり §4 の規約上は単独タグを必須としない。
  `5.0.5` は Arena view 用の予約なので本断面には採番しない（→ §1 の脚注※）。

### 6.4 2026-07-30: 5.0.2 init の実機検証

詳細な経緯。現在地は §1・§2（U11）。

- 入ったもの: 実機検証そのもの（macOS）に加え、主入口の表示条件のバグ修正（`ffd32c3`）。従来は
  起動時に `not_found:` を踏んだかのフラグに紐づけていたため、(a) 過去に ptygrid を使い前回の
  作業フォルダが復元されると設定が見つかり主入口が一度も出ない、(b) 目標フォルダ指定読み込みが
  既定設定で成功した瞬間にボタンが消える、という二重の不具合があった。条件を「いま設定ファイルが
  効いているか」（`configInfo` が無い、または origin が `default`）に変更して解消した。
- 当時の検証: 設定の無いフォルダで起動→シェル 1 枚→「設定を作る」表示、検出結果（opencode/
  claude/codex/gemini/qwen/grok/aider の 7 体・npm・git あり・ローカル LLM ルータ未検出・既存設定
  なし）が実環境と一致、通常生成で `ptygrid.yml`（2,060 バイト・agents 7 体）を生成し trust
  プロンプトなし・ペイン自動起動なしを確認、副入口（⚙→設定ファイル→設定を作る）で既存設定あり
  時の書き込み先が `ptygrid.init.yml` に切り替わり既存 `ptygrid.yml` の mtime・内容が無変更である
  ことを確認。すべてスクリーンショットで確認済み。
- 当時の注記: 書き込み直後に init 自身の通知と watcher `config-changed` による再読み込みトーストが
  二重に出る競合を実測した。spec-init-5.0.2.md §9 で「推測であり未実測」としていた watcher と
  `loadConfig()` の競合がここで実測により確認され、直後に自己書き込みエコー抑制（`ui.selfWrite` +
  3 秒の窓）を別コミットで追加した。仕上げに `autostart: true` へ手編集して書き込むと trust プロンプトが出て、
  「信頼して起動」で当該エージェントが起動することまで確認し、U11 を完了とした。Windows（U8）と
  Global 選択時の `~/.ptygrid/` 作成は範囲外のまま。

### 6.5 2026-07-30: P1 — `smoke` workflow の実機 1 本流し

詳細な経緯。現在地は §1・§2（U1）。

- 入ったもの: コード変更なし。**実機検証のみ**。`smoke`（`pattern: pipeline` / `autoClose: success` /
  step `a` = agent `t1` → step `b` = agent `t2`、各 `sh -c 'echo …; sleep 30; echo …'`）を
  `~/works/project/ptygrid` の設定で GUI から起動した。
- 当時の検証: step `a` のペインが立ち上がって出力し、30 秒後の exit 0 で step `a` が完了して
  **step `b` が自動 spawn**、さらに 30 秒後に run 全体が完了して **2 枚のペインが自動で閉じる**まで
  を目視で確認。これで (1) 完了判定が実 PTY の終了で発火する、(2) DAG が依存関係どおりに進む、
  (3) `autoClose: success` が効く、(4) `workflow-state` が frontend に届いてパネルが追随する、の
  4 つの継ぎ目が実機で裏付けられた。trust プロンプトは出ない（`orchestrator.rs` 冒頭が明言する
  「workflow は新しい信頼境界を作らない」の実証）。
- 当時の注記: 5.0.0 以来 CONTRACT.md 続報8〜続報10 が繰り返し「解除されないもの」として記録して
  いた項目がこれで解除された。ただし `smoke` は pipeline かつ kickoff 無しなので、fan-out と
  straggler キャンセル（U2）、`joinOn: reply` / `condition:` / `handoffTo` / `retry:` / `timeoutMs`
  といった 5.0.4 固有機能、pane 上限待ち（U3）、mailbox の run 単位分離（U4）、resume バナー（U5）
  はいずれも**別途 1 本ずつ確認が要る**。P2（未タグ成果へのタグ付け）の前提条件は満たした。

### 6.6 2026-07-30: U3 で見つかった pane 上限の数え方の不整合

詳細な経緯。現在地は §1・§2（U3）。

- 入ったもの: U3（pane 上限の待ち行列化）の実機 1 本流し中に見つかった不整合の修正。8 面埋まった
  状態で `smoke` の step `a`(t1) が 9 枚目を占有し、`close_on_exit` 未指定のため自然終了後も
  `Exited` のままセルを占有し続けた。続報10 決定 A-7 の「`state != Exited` の数」基準では次
  step の判定が live=8 と見て空きありと誤認して spawn し、frontend は `ui.panes.length`
  （グリッドの全セル数）でしか描画できず、セッションが表示できないまま headless で走った。
  占有判定を `PtyManager::occupied_pane_count()`（全 state、`Exited` 含む）へ変更し
  `live_session_count()` は削除、`team_presets.rs` の判定も同じ基準へ揃えた。
  `teams_hooks.rs` / `teams_host.rs` の `GRID_MAX_PANES` 判定は元から `sessions.len()`
  （全 state）基準だったため、これで orchestrator / team_presets / teams 系 / frontend の
  `MAX_PANES` の 4 経路が同じ基準に揃った。
- 当時の検証: lib **402 passed**（追加2本 `spawn_ready_counts_an_exited_pane_against_the_budget` /
  `an_exited_pane_still_fills_the_team_pane_cap`、更新1本 `live_session_count_excludes_exited_slots`
  → `occupied_pane_count_includes_exited_slots`）、統合14は不変。
- 当時の注記: 修正後の再検証（8 面埋まった状態から再度 `smoke` を流し、`Pending` のまま待って
  空きで再開することの目視）は未実施。CONTRACT.md は続報10 への訂正+追記で対応し、設計メモは
  [refactor-pane-cap-5.0.5.md](refactor-pane-cap-5.0.5.md) A-7 に追記した。

### 6.7 2026-07-30（続き）: U3 / U5 の消化と、その過程で出た 3 件の修正

詳細な経緯。現在地は §1・§2（U3・U5）。

- 入ったもの: `0e9c5ba` pane 上限判定をグリッド占有基準（`occupied_pane_count()`）へ変更（A-7 の
  判断を反転させたもので、経緯は §6.6）。`22e090c` テストの macOS 移植性: fixture が使う
  `/bin/true` は macOS に存在しないため `/bin/echo` へ置換。`f0bee39` テストの fd 枯渇修正:
  macOS で毎回 60 件規模の失敗が出ていた。原因は (a) グリッドを埋めるテストヘルパーが 1 テストごとに
  実 PTY を 8〜9 個開いていたこと、(b) 後片付けがテスト末尾にあり panic 時には走らず reader
  スレッドが master fd を保持し続けるため 1 本の失敗が後続を道連れにしていたこと。フィラーを
  PTY なし論理セッションへ、kill 処理を `Drop` ガードへ変更し、ピーク fd は 12 スレッドで
  270 → 71 に低下（macOS 既定のソフトリミット 256 を超えていたのが直接原因）。付随して Linux
  限定で sysinfo が fd を大量に保持する交絡も 1 件見つかった。`05799d5` 待機理由の可視化:
  §6.6 の A-6 は「パネルが表示する」としていたが、実際は全 error が ⚠ のツールチップに畳まれ
  待機と停止の区別が付かなかった。`Pending` かつ理由ありのときは step 行にテキスト表示するよう修正。
- 当時の検証: U3 は再検証で 8 面埋まった状態から `smoke` を起動し、step 行に
  `waiting for a free pane slot (9/9 occupied)` が出ることをスクリーンショットで確認。ペインを
  閉じると `Running` に遷移することはユーザー報告。U5 は `smoke` 実行中にアプリを再起動し、
  resume Y/N バナーが出ることをスクリーンショットで確認。再開後に run が `Succeeded` まで到達するところまで確認。
- 当時の注記: 再起動後のパネルは永続化された中断前の状態を表示する（`error` は wire フィールド
  として残るが `deferred_since_ms` は `#[serde(skip)]` のため復元されない）ため、ペイン 1 枚の
  状態でも `9/9 occupied` という古い理由が見えることがあるが矛盾ではない。今日の一連では自動
  テストでは捕まらない不具合が 5 件出た（主入口の表示条件、トーストの二重表示、pane 上限の
  数え方、テストの fd 枯渇と macOS 移植性、待機理由の不可視）。

### 6.8 2026-07-30: v0.5.7 リリース

詳細な経緯。現在地は §1・§4。

- 入ったもの: docs の公開/内部分離（`3883128`）+ MIT license 宣言（`d3eac32`）/ Phase 5.0.4
  Orchestrator 実行層（`5d3c1b5` → `3bd9833`: `retry:` / `timeoutMs` / `condition:` / `handoffTo` /
  `joinOn: reply`、supervisor・handoff の spawn ゲート撤去、スキーマ検証）/ docs 再編（`dd7a135`:
  spec / guide / design、3 spec の公開）/ `fanOut` 黙殺による false green の解消 + straggler 協調
  キャンセル（`52de433`）/ Orchestrator ハードニング（`2dc5e40`: pane 上限の待ち行列化 / driver
  tick 軽量化 / inbox mailbox の run 単位分離）/ Phase 5.0.2 `ptygrid init`（`283839c` /
  `c6528d3` の backend + UI、実機修正 `ffd32c3` / `944ff46`）/ pane 上限をグリッド占有基準へ
  （`0e9c5ba`）+ 待機理由の可視化（`05799d5`）/ テストの fd 枯渇修正（`f0bee39`）+ macOS 移植性
  （`22e090c`）/ 実機検証（U11・U1・U3・U5）の記録（`7883d80` / `9384c3c` / `bbb0ba2`）。
  計 45 コミット + 未マージだった docs 1 件、60 files changed / +12,673 / −1,005。
- 当時の検証: lib **402 passed** + 統合 **14 passed**。clippy は `config.rs` の既知 1 件のみ
  （5.0.4 由来、非回帰）。svelte-check 0 / build 成功。実機検証は U1（workflow 1 本流し）・
  U3（pane 上限待ち行列化）・U5（resume バナー）・U11（`ptygrid init`）がいずれも完了。
- 当時の注記: `v0.5.2`〜`v0.5.5` の予約（Phase 5.0.2〜5.0.5 用）は使わないまま残る。`v0.5.7` を
  Phase 5.0.2 + 5.0.4 のリリースに充てたことで、spec-phase5-5.md §9 の「バージョン割り当て」表が
  予約していた 5.5.1 以降を `v0.5.8`〜`v0.5.11` へ 1 つずつ繰り下げた。

### 6.9 2026-07-30（続き）: 5.0.2 追補 — ローカル LLM プローブ

詳細な経緯。現在地は §1・§2（U12）・§4。

- 入ったもの: ローカル LLM プローブを 2 コミットで実装（`63af84f` = `init_probe_llm` と検出行への
  反映、`8931464` = モデル選択の `<select>` と既定モデルの選び方）。ブランチ
  `feat/init-llm-probe-5.0.2` を origin に push した時点で、PR は未作成。プローブは既定
  11434 / 1234 / 3456 と手入力最大 4 本に `GET /v1/models` を当て（1 ポート 1 秒・全体 3 秒・応答
  64KB・モデル 20 件上限）、Anthropic Messages API 互換の確証は `GET /api/version` が 0.14.0 以上の
  ときだけとし、確証ありなら有効な agent 定義・それ以外はコメント行を出す（`autostart` は常に
  false、`ANTHROPIC_AUTH_TOKEN` も確証の有無で出し分け）。
- 当時の検証: lib 402 → **419 passed**、統合 14 は不変、svelte-check 122 files 0 errors / 0 warnings。
  実機（macOS）1 回目は検出フォルダ `~/works/tmp/ptygrid` で、CLI 7 体・npm・git あり・既存設定あり
  （書き込み先が `ptygrid.init.yml` に切り替わる）を確認。プローブは 1234 / 3456 / 11434 を叩き、
  3456 は無応答、**11434 で `Ollama 0.32.1` が応答して「Anthropic API 確証あり」バッジとモデル
  20 件（先頭 `x/flux2-klein:latest`）を取得**するところまでスクリーンショットで確認した。
- 当時の注記: 実機 1 回目で自動テストに掛からない不具合が 2 件出た。(1) 既定モデルに `models[0]` を
  そのまま使っていたため画像生成モデル（`x/flux2-klein:latest`）が選ばれた → 埋め込み/画像/音声/
  再ランクらしい名前を除いた先頭を採る `<select>` を入れた。(2) 確証が取れていても検出行のヘッダーが
  「未検出」のままだった → 反映を修正。**前提の訂正**として、旧設計は Anthropic 互換が無い前提で
  coderouter を挟む translation 層を想定していたが、Ollama v0.14.0 以降と LM Studio が Anthropic
  Messages API 互換になっていたため translation 層は不要と判明した。運用上の事実として、接続フォルダ
  経由の git がロックファイルを削除できず（デバイスブリッジ側の制約）、`_to_delete/gitlocks-20260730/`
  へ退避して作業を続けた。実機の残り 3 点（`<select>` の実操作、生成された `local-11434` 定義での
  Claude Code 起動、LM Studio での未確証分岐）は U12 に未消化として残る。

### 6.10 2026-07-31: ターミナルのコピー & ペースト

詳細な経緯。現在地は §1・§2（U13）・§4。

- 入ったもの: ターミナルペインのコピー & ペースト一式を 1 コミット（`8a83032`、13 ファイル）で実装。
  ブランチ `feat/terminal-copy-paste` は `main` から分岐し、**push も PR も未**。着手時に切り分けた
  原因は 4 つ。(a) 選択自体は阻害されていなかった（`user-select: none` は toolbar / dock /
  statusbar / pane-header のみで、xterm は自前の選択モデルを持つ）、(b) `terminals.ts` の xterm
  生成がテーマ・フォント・scrollback しか渡しておらず、キーハンドラも右クリックメニューも
  クリップボード呼び出しも無かった（xterm の選択は DOM の選択ではないので WebView の Cmd+C には
  コピー対象が見えない）、(c) `src-tauri` にメニューが 1 つも定義されておらず、macOS の WKWebView
  では Edit メニューが無いと Cmd+V が `paste` イベントにならない、(d) TUI がマウスレポートを
  有効にすると選択できず逃げ道も無かった。これに対して、macOS 限定
  （`#[cfg(target_os = "macos")]`）のアプリメニュー（App / Edit / Window。Edit に標準の Undo /
  Redo / Cut / Copy / Paste / Select All）、`tauri-plugin-clipboard-manager`（Rust 2.3.2 /
  JS 2.3.2。capability の許可は `clipboard-manager:allow-read-text` の 1 つだけで、書き込みは既存の
  `navigator.clipboard.writeText` 経路をそのまま使うため足していない）、コピー（macOS = Cmd+C /
  それ以外 = Ctrl+Shift+C。**選択が無いときは介入せず PTY へ流し**、素の Ctrl+C の SIGINT を
  壊さない）、貼り付け（`term.paste()` 経由。bracketed paste は xterm に任せ
  `ignoreBracketedPasteMode` は既定の false のまま）、右クリックメニュー（コピー / 貼り付け。
  選択が無いときコピーは無効表示 + 理由のツールチップ、Esc / 外側 mousedown / window blur で閉じ、
  リスナは全て一緒に外れる）、`macOptionClickForcesSelection: true` を入れた。
- 当時の検証: lib 419 / 統合 14 は不変、clippy は既存の `config.rs:834` の `nonminimal_bool` 1 件
  のみ、svelte-check 136 files 0 errors / 0 warnings、`npm run build` 成功。メニューは macOS 限定で
  この作業環境の Linux では `cfg` により落ちてしまうため、担当が一時的に `cfg(all())` へ書き換えて
  実際にコンパイルと lint を通してから元に戻している。実機（macOS）1 回目では、1 枚目のペインで
  ファイル名を範囲選択 → Cmd+C → 2 枚目の zsh ペインで Cmd+V し同じ文字列が入ること、右クリック
  メニューが選択の有無で 2 状態（選択ありは「コピー ⌘C」「貼り付け ⌘V」がどちらも有効、選択なしは
  コピーが無効表示 + 理由のツールチップ）になることをスクリーンショットで確認した。残りは U13。
- 当時の注記: **二重貼り付けの回避**として、macOS では貼り付けをネイティブ経路に一本化する判断を
  した。メニューのアクセラレータが keydown より先に Cmd+V を食うことがあり、発火済みの
  アクセラレータは `preventDefault` で取り消せないため、自前でもクリップボードを読むと同じ内容が
  2 回入る。Ctrl+Shift+V は逆にネイティブの `paste` イベントが出ないので、自前ハンドラが唯一の
  経路になる。**仕様どおりにできなかった点**: ユーザーは「TUI 中の選択は Option ドラッグ」を選んだ
  が、xterm.js の実装では Option ドラッグは **macOS 限定**で、バンドルの実物が
  `isMac ? altKey && macOptionClickForcesSelection : shiftKey` になっている。Linux と Windows は
  **Shift ドラッグ固定**でオプションでは変えられないため、UI のヒントとコメントは「macOS は
  Option、それ以外は Shift」と書き分けた。**運用上の事実**: JS 側の依存が 1 つ増えたので、この断面を
  取り込む側は `npm install` が要る。実機でこれを踏み、`vite` が
  `Failed to resolve import "@tauri-apps/plugin-clipboard-manager"` で落ちた（→ §4 のリリース手順）。

### 6.11 2026-07-31: 5.0.6（案）の計測フィールドと合成 workflow、U2 の消化

詳細な経緯。現在地は §1・§2（U2）・§4。

- 入ったもの: 3 コミット。`9442758` = `StepOutcome` への計測フィールド追加（`ended_at_ms` は終端到達
  時に 1 度だけ押し、再 spawn では `started_at_ms` の上書きと対で `None` に戻す / `waited_for_pane_ms`
  はペイン待ちの累積で再 spawn をまたいで残る。所要と待ちを別々の数として出す）。`af723ca` =
  合成 workflow 一式（`example/measure-parallelism/ptygrid.yml`。エージェントを `sh -c 'sleep N'` に
  置き換えた `measure-1-serial` / `measure-2-split` / `measure-3-pane-queue` / `measure-4-join-any` の
  4 本）。`58e9c95` = 敗者 kill 時の誤バナー修正（`App.svelte` の `closePane` が `session N not found`
  **だけ**を握り潰す。`kill failed: …` は本物の孤児プロセスなので従来どおりバナーに出す = BUG-5 の意図を
  維持）。ブランチ `feat/step-timing-5.0.6` は `main` から分岐し、**push も PR も未**。
- 当時の検証: すべて macOS 実機、スクリーンショットで確認済み。
  - `measure-1-serial`: run は 03:02:09 開始 → 03:02:40 終了で **31 秒**（理想 30 秒）。step の所要は
    先頭 5.2 秒、残り 5 つが 5.1 秒。依存エッジ 5 本に対しオーバーヘッドは 1 秒、すなわち
    **1 エッジあたり 0.2 秒**で、これは driver tick（200ms）ちょうど 1 回分にあたる。
  - `measure-2-split`: step の所要は 5.2 / 5.1 / 5.2 / 5.1 / 5.2 / 5.1 秒で、**直列版と変わらない**。
    同時 3 枚でも spawn は重くならないことが分かった。run 全体の壁時計はスクリーンショットに写って
    おらず**未記録**。
  - `measure-3-pane-queue`: `gate` 0.2 秒、`wave-a#0`〜`#5` が各 5.1 秒、`wave-b#0`〜`#5` が各
    **5.1 秒（待ち 8.2 秒）**。サンプルに書いておいた予測値は待ち約 8.0 秒だったので、ほぼ予測どおり。
    これは `waited_for_pane_ms` が実機で機能することの裏づけでもある。
  - `measure-4-join-any`: **3 回**実行（03:05:30 / 03:06:11 / 03:07:18）。3 回とも `race#0` が
    SUCCEEDED（5.1〜5.3 秒）、`race#1` / `race#2` が **CANCELLED**（同 5.1〜5.3 秒）。敗者は 45 秒
    sleep に入っていたので **`[race] loser end` の行が 1 度も出ていない**。これが kill が実際に
    起きたことの直接証拠になる。敗者ペインはグリッドから消え、最終状態は gate / 勝者 / report の
    3 枚（フッター `3/9 ペイン`）。`report` は勝者の終了直後に spawn され、run 全体は SUCCEEDED。
    これで U2 を完了とした。
  - **計測の結論**: orchestration のコストは 1 依存あたり 200ms、spawn は 0.1 秒程度。実タスクの
    workflow が遅いとすれば、原因はここではない。
- 当時の注記: 誤バナーの経路は「`cancel_stragglers` が敗者を kill して `outcome.session_id` を `None`
  にする（kill 済みペインは再利用も回収もしないため）→ frontend の `autoCloseModeFor` はその session id
  を持つ step があるかで workflow 由来かを判定しているので、**id が消えた瞬間に workflow 所属と分から
  なくなり**、agent の `close_on_exit: always` にフォールバックして 3 秒後に `closePane` → `kill_pty` を
  呼ぶ → backend ではスロットが既に消えているので `session N not found` が返る」というもの。中身は
  成功しているのに失敗に見える表示だった。`check_timeouts` も同じく `session_id` を消すので同じ潜在
  経路を持っていた。**今回の作業で分かったが直していないもの**が 2 件あり、§3 の継続ウォッチ /
  バックログへ回した: (1) `fanOut` を持つ step を root に置けない（`spawn_workflow` の root ループは
  全コピーに枝番なしの同じ `step_id` を付ける一方、`spawn_ready` 経由のコピーだけが `race#0` のような
  枝番を持つ。パネルの step 一覧は `stepId` をキーにした keyed each なので root fan-out だと同一キーが
  並ぶ。`example/measure-parallelism` では sleep なしの `gate` step を 1 段挟んで回避したが、これは
  設定側の工夫であって修正ではない）、(2) cancel された straggler は workflow の `autoClose` ではなく
  agent の `close_on_exit` に従う（上記のとおり `session_id` が消えるため。今回の設定では偶然それが
  望みどおりだった）。

### 6.12 2026-07-31（続き）: cold start の実測

詳細な経緯。現在地は §1・§4。

- 入ったもの: コード変更なし。**実機計測のみ**。`example/measure-coldstart` の `measure-coldstart`
  を macOS 実機で実行した（同じ agent を使う step を 1 本の pipeline に並べ、1 段目だけ fresh spawn・
  2 段目以降は同じペインの再利用になることを利用して差を取る構成）。
- 当時の検証: すべて macOS 実機、スクリーンショットで確認済み。run は **SUCCEEDED**（3:49:38 開始）。
  - step の所要は `s1-cold` **7.7 秒**（fresh spawn）、`s2-warm` **4.1 秒**（同じペインを再利用）、
    `s3-warm` **4.5 秒**（同上）。warm の平均 4.3 秒に対し **cold start は約 3.4 秒**。
    ばらつき（s2 と s3 の差）は 0.4 秒なので、**3.4 秒はノイズの 8 倍以上あり有意**。
  - 内訳の解釈: warm の 4.3 秒はほぼ**モデルの 1 往復**（kickoff が inbox に入る → await が起きる →
    モデルが読んで `reply_inbox` を呼ぶ → 次の tick で検出）。cold はそれに加えて**プロセス起動 +
    CLI のブート + MCP 接続 + 最初の await 設定**がかかっており、その差が 3.4 秒。
  - **副産物: 常駐の実現可能性**。3 段とも返信で完了した（route 3）。エージェントの recap も
    「3 通すべてに ok を返し、そのあと 2 回続けて timedOut したので降りた」と自己申告している。
    つまり**走り続けているエージェントが 2 通目・3 通目の kickoff を実際に拾えた**ということで、
    `mode: serve`（常駐ワーカー）が必要とする挙動そのものが実機で成立している。
- 当時の注記: **この 3.4 秒は cold start の下限である**。計測用プロンプトが「考えず、調べず、
  ファイルも読まず」と明示的に禁じているため、**実タスクの cold start に含まれるはずの CLAUDE.md /
  リポジトリ / pins の読み直しが一切入っていない**。測れたのは起動の事務コストだけで、実タスクでは
  これより大きくなる。**1 回目は空振りした**（誤りではなく使い方の罠）: 最初の試行ではエージェントの
  ペインだけを起動して**ワークフローを ▶ Run していなかった**ため、inbox に kickoff が 1 件も無く、
  55 秒の await が 2 回 timedOut して終了した。fixture は Run して初めて意味を持つので、
  **エージェント単体を起動しても何も測れない**。**この回で決めたこと**: 次に作るのは
  `onEach: reply`（ストリーミング依存）が先で、`mode: serve`（常駐ワーカー）は後。根拠は取り分の
  桁の差で、`mode: serve` が節約するのは 1 step あたり 3.4 秒 + 文脈の読み直し分（10 step 回しても
  数十秒）なのに対し、`onEach: reply` が節約するのは「上流が全部終わるまで下流が待つ」という工程
  まるごとの待ち時間（実タスクなら数分単位）だから。合成 workflow の実測で orchestration 自体は
  1 依存あたり 200ms しか食っていないことが分かっている（→ §6.11）ので、削るべきは待ち時間のほう。
  `mode: serve` を捨てるわけではなく順番が後で、文脈の読み直しコストを別途測ってから判断してもよい。

### 6.13 2026-08-04: `onEach: reply` / `joinOn: stream` の実装（5.0.7 案）

詳細な経緯。現在地は §1・§2（U14）・§4。

- 入ったもの: `config.rs`（`JoinOnName::Stream` の 1 値追加、`WorkflowStep::on_each: Option<OnEach>`、
  検証 V1〜V10）と `orchestrator.rs`（`detect_reply_completions` の unit 切り出し、新設の
  `mint_stream_copies` / `close_stream_targets` / `stream_closed_for`、`spawn_ready` の 1 コピー
  1 スロット spawn、`fire_due_retries` の unit 再配送、`defer_step` の打ち切り条件、
  `resume_workflow` の拒否）。CONTRACT.md に**先行追記**（続報11b）、
  `example/review-as-you-go` を 1 本追加、`docs/guide/ptygrid-yml-guide.md` §1 に 2 行追加。
  `queen_store.rs` / `queen.rs` / frontend は**無変更**。
- **設計上、いちばん危なかった箇所**（いずれも spec §3.6 / §3.4.4 が名指ししていたもの）:
  - **ペイン再利用**: `agent_claimed_by_other_step` は `base_id(o.step_id) != step.id` で判定するため
    **同じ step の兄弟コピーを「他の step」に数えない**。既存の `copies == 1` 経路にそのまま載せると、
    2 つ目のコピーが 1 つ目のペインを黙って引き取り、1 つの session を 2 つの outcome が追跡し、
    `slots_needed` が 0 を返して 9 面上限を踏み越える。**黙って壊れる**種類の不具合なので、
    `reuse_existing: false` を固定したうえで回帰テスト
    （`on_each_copies_each_take_their_own_pane`）を置いた。
  - **`WORKFLOW_DEFER_MAX_MS`（5 分）の意味**: この定数の doc comment は「外部がグリッドを占有し
    続ける wedge から run を守る」ものだと明記しており、「run 自身はつねに全予算を使える」を前提に
    している。`onEach` はその前提を内側から壊す（自分のコピーが 9 面を埋める）ので、**兄弟コピーが
    1 つでも `Running` の間は打ち切らない**ことにした。`onEach` 以外では同じ述語が構造上つねに
    false なので既存挙動は変わらない。
  - **完了判定**: コピーが全部成功しても上流がまだ `Running` なら次の unit が来うるため、
    `dep_satisfied` / `all_terminal` に「stream が閉じていること」を足した。これが無いと、
    レビュー 2 件が返ってきた時点で `summary` step が走り出す。
- 当時の検証: **自動テストのみ**。lib **434 → 459 passed / 0 failed**（新規 25 本 = config 12 +
  orchestrator 13）、統合 **14 不変**、`cargo clippy --all-targets` は既存の `config.rs` の
  `nonminimal_bool` **1 件のみ**（V3 の判定を `is_none_or` で書いたのは、素直に
  `!…is_some_and(…)` と書くとその警告が 2 件になるため）。frontend は無変更だが**この回で初めて実測した**:
  `npm run check`（svelte-check）**136 files / 0 errors / 0 warnings**、`npm run build` **成功**
  （U9 が「本作業環境に `node_modules` が無く未実測」としていた項目。今回は `npm install` から通した）。
  **実機検証は未実施**で U14 として登録した。
- 当時の注記: **番兵 `[[end]]` は完全一致**で判定する。部分一致にすると unit 本文が番兵に言及した
  だけで stream が閉じるため、テスト（`stream_end_token_must_be_the_whole_message`）でそれを固定した。
  **unit の ack はコピー行を作ってから**行う（逆順だとクラッシュ窓で unit が永久に失われる。この順序に
  しても `persist_run` は tick 末尾なので窓は完全には塞がらず、そのぶんは resume 拒否で受けている）。
  **今回やっていないこと**: 実タスクでの取り分の測定（spec §1.2 の「実タスクなら分単位」は合成
  workflow と cold start からの**外挿であって未測定**）、`onEach` を含む run の resume、unit ごとの
  `condition:`、`mode: serve`。`STREAM_MAX_UNITS = 64` は**数字そのものの根拠が弱い初期値**で、
  実タスク測定（§3 P5）のあとに見直す。

### 6.14 2026-08-05: 5.0.7 の実機 1 回目と、そこで見つかった mailbox の共有

詳細な経緯。現在地は §1・§2（U14）・§4。

- **動いた**。`u14-streaming` が SUCCEEDED まで到達し、5 unit → コピー 5 つ（`#0`〜`#4`、
  欠番なし）→ 番兵で上流 Succeeded → 全コピー終端のあと `summary` が 1 度だけ、という
  一連が実機で確認できた。数字は §2 の U14 に置いた。
- **見つかったもの: コピーが mailbox を共有していた。** `onEach` のコピーは全部が同じ
  agent 定義から起動するので、kickoff の宛先を agent 定義名にしていると N 個のペインが
  1 つの mailbox を奪い合う。実機のレビュアーのペインが「待機で 3 件届いたので、いちばん
  新しい id=341 を選び」と報告したのがそれで、**設定を書く側からは見えない**種類の穴
  だった。返信は thread root で相関されるので 5 行とも正しく完了しており、run の結果は
  正しい。しかし (a) どのペインがどの unit を担当するかが到着タイミング次第になり、
  コピーの所要時間が「そのコピーの unit の所要」を意味しなくなる（今回 6.3 秒と 33.8 秒の
  ばらつきがそれ）、(b) 2 つのペインが同じ unit を取ると別の unit が誰にも拾われず
  `timeoutMs` に落ちる余地が残る。
- **是正**: kickoff の宛先を copy ごとに分けた。`onEach` のコピーは
  `wf/<run_id>/<step_id>` を宛先とし、それ以外の step は**従来どおり agent 定義名のまま**
  （既存設定は 1 バイトも影響を受けない）。`run_id` を含めるのは、同名 workflow の並行 run が
  どちらも `reviewer#0` を持つため。ペインが自分の mailbox 名を知る手段として
  `PTYGRID_MAILBOX` 環境変数を workflow が起動する全セッションに注入する — kickoff 本文では
  渡せない（エージェントは kickoff が届く前に await していなければならない）。`onEach` 以外では
  値が agent 定義名そのものなので、`mailbox=$PTYGRID_MAILBOX` の 1 つの書き方で全役割に使える。
  併せて load 時規則 **V11**（`onEach` step の id は 64 バイト以下）を追加した。
- 当時の検証: lib **459 → 461 passed / 0 failed**（`kickoff_recipient_is_per_copy_only_for_on_each`
  と `a_copy_completes_only_on_a_reply_to_its_own_mailbox` を追加。既存の
  `on_each_copies_each_take_their_own_pane` / `a_retried_copy_is_re_told_which_unit_it_is_about`
  は per-copy mailbox を読むように書き換えた）、統合 14 不変、clippy 新規警告ゼロ。
- 当時の注記: **設定 2 本に別の誤りもあった**。`example/review-as-you-go` と検証用 fixture の
  `cmd` を素の `claude` にしていたため、エージェントが起動画面のまま止まり step が Running から
  動かなかった（1 回目の空振り）。kickoff は durable inbox に置かれるだけで**ペインには何も
  打ち込まれない**ので、「起動直後に await を呼べ」は CLI の起動引数で渡すしかない。
  `example/measure-coldstart` は最初からそうしていた。spec §4.3 のサンプルは
  `cmd: "claude"` のままなので、**あれをそのまま写すと動かない**。
- **同日 17:19、修正後のバイナリで 2 回目**。per-copy mailbox が実機で効いていることを、
  reviewer のペインに出た `sender=wf/wfr_18c8daebafa1e87800000000/reviewer#2` / `#3` で確認した
  （1 回目に出ていた「N 件届いたので新しいものを選び」が消えている）。**判定に所要時間を使っては
  いけない**ことも分かった: 1 回目に「6.3 秒と 33.8 秒のばらつきが共有 mailbox の証拠」と書いたのは
  言い過ぎで、レビュー対象の長さが違えば同じばらつきは普通に出る。証拠はペインが名乗る mailbox 名のほう。
- **2 回目で見つかった別件（本 patch では直していない）: 古い run の kickoff が mailbox に滞留する。**
  `coder` のペインが「mailbox には kickoff が 3 通たまっていました。id=333 と id=334（いずれも
  2026-08-04 の古い run）は…返信もしていないため未 ack のまま残っています」と報告した。`coder` は
  `onEach` ではないので mailbox は agent 定義名のままで、**cancel / abandon された run の kickoff は
  誰も ack しないので残り続ける**。相関は thread root なので別 step を完了させることはないが、
  次の run のエージェントが**死んだ run の指示を読んで実行してしまう**余地がある。5.0.7 が作った
  問題ではなく 5.0.0 からの挙動で、今回コピーに入れた run スコープの mailbox を全 step に広げるか、
  `cancel_workflow` / `abandon_workflow` が未 ack の kickoff を ack するかのどちらかで消える
  （→ §3 の継続ウォッチ / バックログ）。

### 6.15 2026-08-13: Stage A-5 — cancel / abandon された run の kickoff を ack する

詳細な経緯。現在地は §1・§2（U17）・§3。

- **直したもの**: §6.14 の末尾で「本 patch では直していない」と書いた滞留そのもの。
  cancel / abandon された run の kickoff が agent の mailbox に**未 ack のまま残り**、次の run の
  ペインが `await` でそれを拾って**取り消された作業を実行してしまう**余地があった。2026-08-05 の
  実機で `coder` のペインが前日の run の kickoff 2 通（id=333 / id=334）を報告したのが観測点。
  **ただし、その 2 通が cancel / abandon 由来だったかは未確認**（§6.14 に引用したペインの報告は
  「2026-08-04 の古い run」「返信もしていないため未 ack」としか言っておらず、run がどう終わったかを
  示していない。返信せずに終わった run でも同じ見え方になる → 下の「未対応のもの」）。
  誤完了は起きない（返信は thread root で相関される）ので、症状は「run の結果が壊れる」ではなく
  「やらなくていい作業をやる」。5.0.7 が作った問題ではなく **5.0.0 からの挙動**。
- **入ったもの**: `QueenStore::ack_inbox_from_sender`（新設）と
  `orchestrator::retire_run_kickoffs`（新設、`cancel_workflow` / `abandon_workflow` から呼ぶ）。
  §3 に並べていた 2 案のうち **(a)** を採った。(b)（run スコープ mailbox を全 step に広げる）は
  既存の全サンプル・全ユーザー設定の `cmd` が `mailbox=$PTYGRID_MAILBOX` に揃っていることを
  前提にするので、既存設定に一切影響しない (a) のほうが安い。
- **設計判断 1: 選択キーは message id ではなく sender。** kickoff の sender は
  `workflow_mailbox` が作る `queen:workflow/<name>/<run_id>` で **run_id を含む**ため、
  文字列一致だけで「この run の kickoff」を正確に選べる（共有 mailbox のぶんも `onEach` コピーの
  専用 mailbox のぶんも同じ条件で拾え、並行する別 run を巻き込む余地が構造的に無い）。
  id 経由だとこれが成立しない: `StepOutcome::kickoff_root_msg_id` は `#[serde(skip)]` なので
  **DB から読み戻した run には id が 1 つも無い**。`abandon_workflow` が見るのは常にその読み戻した
  run なので、id 方式では abandon 側が丸ごと機能しないところだった。**「resume をまたぐと id が
  消えるので abandon では縮退する」という穴は、sender 方式では発生しない。**
- **設計判断 2: step の state を問わず全部 ack する。** 終わった run の kickoff は全部用済みで、
  かつ `Succeeded` の step の kickoff も未 ack で残っていることが多い（route 1 / route 2 は
  返信なしで step を完了させる）。次の run から見れば古さは同じ。
- **設計判断 3: best-effort。** store エラーは `eprintln!` に落として cancel / abandon 自体は
  成功させる。掃除の失敗で cancel が失敗するのは本末転倒で、失敗時の最悪ケースは修正前の挙動と
  同じだから。順序も「主目的が先」に固定した（cancel は `persist_run` の後、abandon は
  `mark_workflow_abandoned` の成功後）。
- 検証: `cargo test` **lib 475 → 479 passed / 統合 14 passed / 0 failed**（新規 4 本 —
  `acking_by_sender_closes_only_that_senders_unacknowledged_messages` /
  `cancelling_a_run_acks_the_kickoff_its_agent_never_answered` /
  `abandoning_a_run_acks_kickoffs_whose_ids_the_persisted_run_has_lost` /
  `retiring_one_runs_kickoffs_leaves_a_concurrent_runs_alone`）。clippy は既存の `config.rs` の
  `nonminimal_bool` 1 件のみで新規警告ゼロ。frontend 無変更。
- **未実測のもの**（推測で埋めないこと）:
  - **実機検証は一切していない**。上の裏づけは unit test 4 本だけで、実機で「次の run の
    ペインが古い kickoff を数えなくなった」ところは**見ていない** → §2 の U17。
  - メッセージは削除ではなく ack なので、`list_inbox(includeAcknowledged: true)` には残る。
    「古い kickoff を読ませない」保証が及ぶのは `await` と未 ack 一覧まで。
- **未対応のもの**:
  - **`Failed` / `Succeeded` で終端した run は掃いていない。** 入口は cancel / abandon の
    2 つだけ（`retire_run_kickoffs` の呼び出し元はこの 2 か所しかない）。しかも**滞留するほうが
    多数派**である: `joinOn: reply` でない step は route 1（PTY exit）/ route 2（semantic done）で
    完了するので、**返信が無く kickoff は未 ack のまま残る**。
    **A-6（retention）はここに効かない** — `prune_terminal_workflow_runs` が触るのは
    `workflow_runs` の DELETE だけで、`inbox_messages` には SQL を 1 本も投げないので、
    run 行が消えたあとも kickoff は inbox に残る（別テーブル）。**帰結**: ack しても行は消えない
    ため `inbox_messages` は増え続け、`MAX_MESSAGES_PER_PROJECT` = 50,000 に達すると
    `enforce_limit` が `send_inbox` を `Err` で拒否する = **新規 kickoff を送れなくなる**。
    直すなら A-5 の入口を終端書き込みにも広げるか、5.6.x で扱う（→ §3 のバックログ）。

### 6.16 2026-08-13: Stage A-4 — retry を使い切った step が外へ 1 通出す（escalation の配線）

詳細な経緯。現在地は §1・§2（U18）・§3（P3）。

- **直したもの**: §3 P3 そのもの。4.4.2 の通知基盤（OS トースト / Slack / Mattermost / Discord /
  Telegram）は 5.0.4 の retry 実行系より前からあったのに、**workflow 側からの入口が無かった**。
  step が `retry:` の予算を使い切っても `Failed` で終端して run が red になるだけで、
  アプリの外へは 1 通も出ない。自主運用は「人間が気づく」ことに依存しているので、
  離席中・夜間の失敗がそのまま滞留する。ptygrid-yml-guide.md §1 で escalation 行が ❌ のまま
  残っていた理由でもある。
- **入ったもの**: `orchestrator::take_escalations`（純関数、枯渇 step の収集）と
  `notify_escalation`（`notifications::dispatch_ctx` の呼び出し）を新設し、`advance_run` の末尾に
  配線した。通知側は `NotifyContext` に任意フィールド `origin`（`WorkflowOrigin`）と、
  文脈をまとめて渡せる `dispatch_ctx` を追加。**新しい配送機構はゼロ**で、P3 の見込み
  （「workflow 側のイベントを既存経路へ流すだけ」）はそのまま成立した。
- **設計判断 1: エッジ化は orchestrator 側でやる。** notifications.rs は冒頭で「イベント源は
  すべてエッジなので、この層はポーリングも重複除去もしない」と宣言している。ところが
  workflow driver は **200ms の tick（ポーリング）**で、「この step は retry を使い切った」は
  以後の全 tick で真であり続ける**レベル**である。そのまま流すと同じ枯渇で **5 通/秒**になる。
  `StepOutcome` に `#[serde(skip)] escalated: bool` を足し、それを立てた tick の分だけ返す形にした
  （`next_retry_at_ms` が「Failed」を「Failed だが再試行待ち」に変えているのと同じ発想）。
- **設計判断 2: イベントは `Error`（`NeedsAttention` ではない）。** 既定の
  `notifications.level` は `critical` で、`critical` が購読するのは `error` **だけ**である。
  `NeedsAttention` にすると、**level を書き換えていない利用者＝既定の設定には 1 通も届かない**。
  escalation が最も要るのは「誰も画面を見ていない」設定であり、それが既定の設定でもあるので、
  ここを外すと A-4 の completion gate 自体を満たさない。
- **設計判断 3: config には何も足さない。** 宛先と音量は既存の `notifications:` ブロックの
  `level` / `channels` が既に表現している。escalation 専用のスイッチを足すと、
  「2 つの設定が食い違う」状態を作れるようになるだけである。ptygrid-yml-guide.md §1 の
  「(config には書かない)」は変えていない。
- **設計判断 4: メッセージは session ではなく step を名乗る。** `NotifyContext` は
  `session_id: u32` が必須の session 中心の型だが、枯渇した step は**ペインを持たないことがある**
  （spawn できないまま枯渇した / `check_timeouts` に kill 済み）。`session_id` を `Option` に
  すると既存 2 源へ波及するので、任意フィールド `origin` を足して、あるときだけ整形を分岐させた。
  **既存 2 源の出力はバイト単位で不変**である。
- **二重通知は抑止していない（許容）。** 枯渇した step の最後の試行がペイン付きだったなら、
  その exit で `session::handle_eof` 由来の `error` が別途 1 通出る（変更前から、しかも
  **試行のたびに**出ていた通知）。両者は別のことを言っている: 前者は「プロセスが落ちた」、
  後者は「この run のこの step はもう自動では戻らない」で、workflow / run / step を名乗るのは
  後者だけ。抑止するには通知層が session と workflow の対応を横断で知る必要があり、
  4.4.2 の前提を壊す。
- 検証: `cargo test` **lib 479 → 484 passed / 統合 14 passed / 0 failed**（新規 5 本 —
  `an_exhausted_retry_budget_escalates_once_and_never_again` /
  `a_retry_that_still_has_budget_left_does_not_escalate` /
  `a_step_with_no_retry_policy_never_escalates_however_hard_it_failed` /
  `escalation_names_the_workflow_run_step_and_attempt_count` /
  `escalation_reaches_a_channel_left_at_the_default_critical_level`）。clippy は既存の
  `config.rs` の `nonminimal_bool` 1 件のみで新規警告ゼロ。frontend 無変更。wire 契約も無変更
  （`escalated` は `#[serde(skip)]`）。
- **未実測のもの**（推測で埋めないこと）:
  - **実機検証は一切していない。** OS トーストにも Slack にも、実際に届いたところは
    **見ていない**。「200ms の連投にならない」ことも unit test（同じ run を 6 回 tick 相当で
    回して 2 回目以降が空）で固定しただけで、**実機では未確認** → §2 の U18。
  - **通知の所要・遅延は未計測。** 枯渇から着信までどれだけかかるかは測っていない。
  - **`joinOn: any` の敗者が枯渇したあとに兄弟が勝つ場合**、run は緑で終わるのに escalation は
    既に飛んでいる（同一 tick 内なら `cancel_stragglers` が先に走るので出ない）。
    この競合が実際にどのくらい起きるかは**未実測**。
  - **resume すると同じ枯渇がもう 1 通出る**（`escalated` を永続化していないため）。
    意図した挙動だが、実機では未確認。
  - **run 全体の失敗は依然として通知しない。** 入口は step の retry 枯渇 1 つだけで、
    `retry:` を書いていない workflow は red になっても escalation を出さない。

### 6.17 2026-08-13: Stage A-6 — `workflow_runs` に retention を入れる

詳細な経緯。現在地は §1・§2（U19）・§3（バックログ）。

- **直したもの**: `workflow_runs` は `queen.sqlite3` の中で**唯一、件数上限も DELETE も
  持たない表**だった（pins / notes / inbox_messages は `enforce_limit` を通る）。
  5.0.1 で表が入って以来、実行した workflow の run 行が**インストールの寿命ぶん単調増加**する。
  しかもこの表の終端行を読む経路は現時点で**存在しない**（唯一の SELECT は
  `state = 'running'` で絞る `list_running_workflow_runs`）ので、溜まっているのは
  **書き込み専用の重さ**である。いま直す実質的な理由は 2 つ: 別ブランチ
  `feat/schedule-5.0.8`（時刻起動。**本ビルドには入っていない**）が入ると `every: hour`
  1 本で 24 行/日を無人で積むこと、そして 5.6.0（スキーマ分割）が**太った表を移行する
  羽目になる**こと。next-implementation-2026-08.md の依存グラフでも
  A-6 → 5.6.0 → 5.6.1 → 5.6.2 → 5.7.0 がクリティカルパスになっている。
- **入ったもの**: `queen_store.rs` だけ。定数
  `MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT = 500` と `TERMINAL_WORKFLOW_STATES`、
  関数 `is_terminal_workflow_state` / `prune_terminal_workflow_runs` を新設し、
  `upsert_workflow_run`（終端 snapshot のときだけ）と `mark_workflow_abandoned` から呼ぶ。
  `orchestrator.rs` / `lib.rs` / frontend / DDL / wire は**すべて無変更**。
- **設計判断 1: 件数であって日数ではない。** 隣の 3 つ（256 / 10,000 / 50,000）が全部件数で
  揃っているのに加えて、**日数だけではファイルサイズが有界にならない**（1 日で何百 run でも
  書ける）。逆に日数だけにすると 1 か月放置したプロジェクトを開いた人が**まさに見たい履歴を
  全部失う**。件数なら最悪値が決まり、放置では減らない。両方入れる案は、消える条件が 2 つに
  なるぶん「なぜ消えたか」の説明が難しくなるので採らなかった。
- **設計判断 2: 500 という数字の根拠。** 2 つの境界から挟んで決めた。**下限は 100** =
  `orchestrator::REGISTRY_TERMINAL_CAP`（メモリ上の registry が保持する終端 run 数）。
  永続ストアが揮発ストアより狭いのは背理なので、DB はこれ以上でなければならない。
  **上限はファイル増加**で、1 行が run 全体の `steps_json` を丸ごと持つため 1 行が重い。
  **悲観的に 1 KiB/行と置いて 500 行 ≒ 0.5 MiB/project**、隣が許す最悪値
  （`MAX_NOTES_PER_PROJECT` × `MAX_NOTE_BODY_BYTES` だけで ≒ 640 MiB）に比べれば十分保守的。
  時間軸では時間起動 ≒ 20 日、日次起動 ≒ 1 年以上に当たる。
  **ただし 1 KiB は仮定で、実サイズは未実測**（→ 下の「未実測のもの」）。
- **設計判断 3: 消さない run の条件は「終端していないこと」で、しかも許可リストで書く。**
  削除対象は `state` が `succeeded` / `failed` / `cancelled` の行だけ。`'running'` の否定では
  なく明示の許可リストにしたのは、**このビルドが知らない state 値を「まだ生きている」側へ
  倒す**ため。retention は間違えるなら残す方向に間違えなければならない —
  誤って消す行は `list_running_workflow_runs` が拾う行、すなわち「再開しますか」バナー
  （5.0.1）の実体であり、消せばクラッシュからの復帰が**黙って**不可能になるからである。
  終端していない run は**上限にもカウントしない**ので、履歴が溜まっても生きている run を
  押し出せず、生きている run が何本あっても履歴の窓は狭まらない。
  abandon された run は `cancelled` なので削除対象**に含める**（操作者が既に「再開しない」と
  答えており、`error` の abandon マーカーがそのバナーより長生きする必要は無い）。
  並び順は `orchestrator::evict_terminal` と同一にした
  （`COALESCE(ended_at_ms, started_at_ms) DESC, run_id DESC`）。`COALESCE` は防御ではなく必須で、
  `spawn_workflow` は全 root の spawn に失敗すると **`ended_at_ms` が NULL のまま終端した run**
  を publish しうる。これを 0 扱いにすると**いちばん新しい run から消える**。
- **設計判断 4: 走るのは「run が終端に到達した書き込み」だけ。200ms tick には何も足さない。**
  driver が呼ぶ `upsert_workflow_run` は run が生きている間つねに `state = 'running'` の
  snapshot なので、**Rust 側の文字列判定を SQL の前に置く**だけで hot path のコストは
  **3 要素の許可リスト照合 1 回**（`TERMINAL_WORKFLOW_STATES.contains` = `[&str; 3]` の線形走査。
  hot path の `"running"` は 3 つ全部と比較して外れる）に収まる（SQL は 1 本も増えない）。
  run が終端に到達するのは 1 回、
  かつ `advance_all` はその直後からその run を tick しないので、実際の掃除は
  **完了 1 run あたり約 1 回**。`mark_workflow_abandoned`（`upsert_workflow_run` を通らない
  もう 1 つの終端経路）でも同じ関数を呼ぶ。**起動時の一括掃除は入れていない**（下記）。
- **設計判断 5: `enforce_limit` と違って拒否ではなく削除。** pins / notes / inbox は上限で
  **書き込みを拒否**する（利用者の要求なので「上限です」と答えられる）が、`workflow_runs` の
  書き込みは要求ではなく**既に起きたことの記録**（`persist_run`。しかも `Err` を握り潰す）。
  拒否しても run は止まらず、**その run の永続記録と resume 可能性が黙って消えるだけ**になる。
- **設計判断 6: project スコープ。** 既存 `enforce_limit` と同じ単位。全体で 1 つの上限に
  すると、忙しいプロジェクトが静かなプロジェクトの履歴を追い出せてしまう。
- **`user_version` は消費していない（3 のまま）。** `count(*)` も `DELETE` も絞り込みは
  `(project_dir, state)` で、これは既存 `workflow_runs_project_state` の先頭 2 列である。
  §5.1 のとおり **`user_version` 4 は 5.6.0 と 6.0.0 で未決**なので、A-6 はそこに触らない。
- 検証: `cargo test` **lib 484 → 489 passed / 統合 14 passed / 0 failed**（新規 5 本 —
  `finished_workflow_runs_past_the_cap_lose_the_oldest_rows_first` /
  `an_unfinished_workflow_run_survives_any_amount_of_history_written_after_it` /
  `workflow_run_retention_deletes_nothing_while_the_project_is_under_its_cap` /
  `workflow_run_retention_gives_every_project_its_own_window` /
  `abandoning_a_run_prunes_the_history_it_has_just_joined`）。clippy は既存の
  `config.rs` の `nonminimal_bool` 1 件のみで新規警告ゼロ。frontend 無変更。wire 契約も無変更。
- **未実測のもの**（推測で埋めないこと）:
  - **実機検証は一切していない。** 裏づけは unit test 5 本だけで、実機の `queen.sqlite3` が
    実際に縮むところも、resume バナーが掃除後も出るところも**見ていない** → §2 の U19。
  - **`steps_json` の 1 行あたりの実サイズは測っていない。** 上限値の導出に使った
    「悲観的に 1 KiB/行」は**仮定**であり、実測すれば 500 が過大にも過小にもなりうる。
  - **掃除の所要時間も未計測。** 「完了 1 run あたり `count(*)` 1 回 + 超過時のみ DELETE 1 回」
    は設計上そうなるというだけで、実機で tick が重くならないことは**確認していない**。
  - **起動時の一括掃除は入れていない。** A-6 以前のビルドが太らせた DB は、そのプロジェクトで
    次に run が 1 本終わるまで縮まない（もう run しないプロジェクトなら永久に残る）。
    既知かつ許容の穴。
  - **`VACUUM` はしない。** 行は消えるが SQLite のファイルサイズ自体は縮まず、
    空きページが再利用されるだけである。
  - **終端 run 行を読む機能は依然として無い。** A-6 は「上限を決めて DELETE を入れる」までで、
    履歴 UI は 5.6.0 以降の話。したがって**この変更で失われる利用者向けの機能は現時点で無い**。

### 6.18 2026-08-13: Stage A 4 件の最終レビューと、その是正（記述の誤り + テストの穴）

A-2/A-3・A-4・A-5・A-6 を別々に実装したあと、まとめて 1 回レビューした。**コードの機能バグは
出ていない**ので、是正は記述とテストだけ。

- **記述の誤り 3 件**:
  - 「返信せずに終わった run の kickoff の滞留は **A-6（retention）の範囲**」（続報15 / §6.15）は
    **事実として誤り**だった。`prune_terminal_workflow_runs` は `workflow_runs` を DELETE する
    だけで `inbox_messages` には SQL を投げない。「未対応」に書き換え、帰結
    （`MAX_MESSAGES_PER_PROJECT` = 50,000 で `send_inbox` が拒否されはじめる）を足し、
    §3 のバックログに 1 項目として残した。
  - §6.15 が「2026-08-05 に `coder` のペインが報告した 2 通が A-5 の直した対象」と読めた点。
    §6.14 に引用した報告は「古い run」「未 ack」としか言っておらず、**cancel / abandon 由来である
    証拠は無い**ので、その旨を 1 行足した（§7 の「推測を断定で書かない」）。
  - 「hot path が払うのは**文字列比較 1 回**」（続報17 (4) / §6.17 の設計判断 4）は言い過ぎ。
    実体は `TERMINAL_WORKFLOW_STATES.contains` = `[&str; 3]` の線形走査なので、
    **「3 要素の許可リスト照合 1 回（SQL は 1 本も増えない）」**に直した。
  - あわせて §4 に**項目 8（タイトルバーのバージョン表示、`c74997f`）**を足した。cherry-pick で
    入っていたのに CONTRACT.md にも plan.md にも記録が無く、wire は変わらないが**ユーザーに
    見える挙動変更**であるため。
- **テストの穴 3 件**（→ コミット `test: pin the escalation wiring, ...`）:
  - `advance_run_escalates_a_step_that_has_run_out_of_retries`: A-4 の**配線**回帰。既存の
    escalation テストは `take_escalations` を直接呼ぶものだけで、`advance_run` の呼び出しを
    消しても全テストが通る状態だった（先例は
    `advance_run_cancels_a_straggler_once_the_any_join_is_won`）。**配線を潰すとこのテストだけが
    落ちることを確認**してから元に戻している。
  - `an_escalation_mark_does_not_survive_the_persisted_round_trip`: 続報16 の
    「`escalated` は `steps_json` にも `workflow-state` にも出ない」を assert で固定（A-5 が
    `abandoning_a_run_acks_kickoffs_whose_ids_the_persisted_run_has_lost` でやっているのと同じ形）。
  - `a_workflow_run_state_this_build_does_not_know_is_never_pruned`: A-6 の許可リスト方式
    （続報17 (3)）を、`'weird'` という未知 state の行で固定。既存 5 本は `'running'` と
    `'succeeded'` しか使っておらず、「`'running'` の否定」に書き換えても全部緑のままだった。
- **記録だけして直さなかったもの**（→ §3 のバックログ）: cancel と driver tick の lost-update
  race、`onEach` / `fanOut` の一斉枯渇による escalation バースト（最大 64 通）。**どちらも
  Stage A が作った問題ではない**。
- 検証: `cargo test` **lib 489 → 492 passed / 統合 14 passed / 0 failed**。clippy は既存の
  `config.rs` の `nonminimal_bool` 1 件のみで新規警告ゼロ。`cargo fmt` は**走らせていない**
  （既存コードが現行 rustfmt で未整形のため、無関係な差分が大量に出る）。frontend 無変更、
  wire 契約も無変更。**実機検証は依然として未実施**（U17 / U18 / U19 はそのまま残る）。

### 6.19 2026-08-05: `schedule:` — 時刻で workflow を起こす（5.0.8 案）

> **採番注**: この作業は 2026-08-05 に行われたが、`feat/schedule-5.0.8` ブランチに留め置かれ、
> Stage A（§6.15〜§6.18）の完了後にその上へ載せ替えた。時系列と節番号が前後するのはそのため。

詳細な経緯。現在地は §1・§2（**U20**）・§3 P3・§4。

- **出発点はハーネスとしての使い勝手**。「cron などを利用した時刻指定で回せると使い道が良くなる。
  通常の cron で回すと調査が面倒」というのが最初の動機で、そこに**想定利用者の指定**が加わった:
  cron の設定が分からない初級エンジニアや、プログラムは書かないがバイブコーディングで開発を
  始めた層。「エラーが出るなら、そのエラーが出にくい構造にすればよい」。
- **[spec-phase5-0.md](../spec/spec-phase5-0.md) §8 の却下を覆した。** あの一文
  （「cron は OS 側の launchd / systemd / cron を使ってもらう」）は**責務の線引き**として
  書かれており、そこは今でも筋が通っている。覆した理由は 3 つで、(a) 想定利用者が変わった —
  あの文は launchd を操作できる利用者を暗黙に前提しており、対象読者にとって OS cron は
  書式・PATH・無言の失敗の 3 つで詰まる壁である、(b) 失敗したときに残る資産が ptygrid 側に
  しか無い（ペイン・step 所要・run 履歴・inbox スレッド）、(c) 実装が新しいサブシステムに
  ならない — `driver_loop` は既に 200ms で回っており、足したのは述語 1 つと tick 1 段である。
- **いちばん効いた決定は「取りこぼしを追わない」こと。** アプリが起動していない間は発火せず、
  起動時のキャッチアップもしない。ユーザーの指定でもあり（「起動してないと動かないは当たり前」）、
  設計としても正しかった: キャッチアップを入れると「朝アプリを開いた瞬間に 5 本が同時に走り出し、
  9 面を埋め、課金が積み上がる」が起きる。取りこぼす側に倒したことで、**キャッチアップ機構が
  丸ごと不要になった**。代わりに「次回」と「最終実行」を常時表示し、最終実行が数日前で止まって
  いること自体が「開いていなかった」の表示になるようにした。
- **エラーを報告するのではなく、エラーになる前に見送る形にした。** 見送りは 2 条件で、
  前の run が未終端（積むと 9 面と掛け算になる）と、グリッドに root ぶんの空きが無い
  （始めてから 5 分待って赤くする形だと「勝手に始まって勝手に赤くなった」にしか見えない）。
  どちらも**発火の前**に判定して理由を残す。併せて `schedule` を持つ workflow の `autoClose`
  既定を `success` に変えた — 昨日のペインが残っていて今日の発火が枠不足で飛ぶ、が原因の
  いちばん説明しづらい形だったため。
- **語彙を 3 つに絞った。** `every: day / weekday / hour` + `at:` だけで、cron 式は書けない。
  自由文字列にしないのはタイプミスを実行時まで運ばないためで、`every: dayly` は serde が、
  `at: "9時"` は検証 S2 が load 時に落とす。**書けないことが機能**という位置づけで、
  代償として「毎週火木」のような表現は書けない（spec §8.1）。
- 当時の検証: **自動テストのみ**。lib **461 → 475 passed / 0 failed**（新規 14 本。うち 7 本が
  `next_fire_at` の時刻計算 — 日跨ぎ・週末飛び・毎時の時跨ぎ・月末年末・DST の 2 つの縁・
  「保存した瞬間に発火しない」）、統合 14 不変、`svelte-check` 136 files 0 errors 0 warnings、
  `npm run build` 成功、clippy 新規警告ゼロ。**実機検証は未実施**で U20 として登録した。
- 当時の注記: **自動停止の状態はメモリ上にしか無い**ので、再起動すると連続失敗カウンタが 0 に
  戻る（毎朝失敗する設定は毎朝 3 回試される）。永続化には新テーブルか `user_version` の更新が
  要り、Phase 5.5.1 の 3 テーブル導入と衝突しうるので本 patch では持たなかった。
  **そして最大の残件は P3（通知）**である。無人で時刻起動する run の失敗を外部へ知らせる経路が
  無いままなので、いまの 5.0.8 は「失敗を静かに溜めうる」状態にある。§3 P3 の優先度を
  上げたのはそのため。

### 6.20 2026-08-13: 5.0.8 のレビューで出た 7 件の是正

§6.19 の実装を Stage A の上へ載せ替えたあとのレビューで、欠陥が 7 件出た。**新しい Phase では
なく 5.0.8 の修正**である。契約の差分は [CONTRACT.md](../../CONTRACT.md) 続報19（続報18 の
うち何が変わったかを書く形にした。続報18 は「2026-08-05 時点のまま」という但し書き付きで
移設されているので、そちらを書き換えるより時系列の性格に合う）。仕様側は
[spec-schedule-5.0.8.md](../spec/spec-schedule-5.0.8.md) の §3.1.1 / §3.1.2 / §3.2 / §3.5.1 /
§3.7 / §4.2 S8 / §8.1 / §8.2 を更新した。現在地は §2（**U20** に 4 項目追加）。

- **【高】自動停止を解除する手段が実質「アプリ再起動」しかなかった。** `stopped_reason` は
  セットする経路と作り直し時に引き継ぐ経路しか無く、**クリアする経路が 1 つも無かった**。
  `list_schedules` は読み取り専用で、再開コマンドも UI ボタンも無い。しかも fingerprint が
  `name` / `summary()` / `enabled` / `max` しか見ていないので、`kickoff` を直しても**表の
  作り直しすら起きない**。spec §3.5 の「再開はユーザーの明示操作（または設定の再読み込み）」と
  `example/scheduled-review/ptygrid.yml` の「直したら設定を保存し直せば、また動き出す」は
  どちらも満たされておらず、CONTRACT だけがこの一文を持たないため実装と整合していた
  ——**仕様・example・契約の三者が食い違っていた**。
  直し方は fingerprint を「表全体 1 本」から「**スケジュールごと 1 本**」に分け、
  `ScheduleState` に自分の宣言 fingerprint を持たせる形。**fingerprint が見る範囲は
  `schedule:` ブロックではなく、その workflow の宣言全体 + その step が名指しする
  agent / process の定義**とした。根拠は 2 つで、(a) 停止の原因はほぼ常に `schedule:` の
  外（指示文、消えた agent、`cmd`）にあるので、狭い fingerprint では「直して保存する」が
  解除にならず H1 がそのまま残る、(b) 広すぎて誤って解除した場合の代償は
  「`maxConsecutiveFailures` 回ぶん余計に試す」だけで、しかもパネルに回数が見えている。
  非対称なので広いほうへ倒した。
- **副次効果（レビューの読みどおりだった）。** 表全体の作り直しは、無関係な workflow の
  編集で**その tick で due だった発火を黙って 1 回捨てて**いた（作り直しのたびに
  `next_fire_at` が strictly-after-now に切り直されるため）。宣言が変わっていない行は
  次回時刻をそのまま持ち越すようにしたので、これも消えた。テスト
  `an_unrelated_edit_does_not_eat_a_fire_that_is_due_in_the_same_tick` で固定した。
- **【高】スリープ復帰後に「過ぎた発火」を 1 回だけ遅れて実行していた。** due 判定は
  「予定時刻が現在時刻以下」の一方向比較で**猶予の上限が無かった**。08:00 に蓋を閉じて
  17:00 に開くと、今日の 09:00 は過去なので復帰した最初の tick で発火する。しかも
  アプリを終了して起動し直した場合はキャッチアップしない（表が strictly-after で作り直される。
  テスト `tick_schedules_builds_the_table_and_does_not_fire_on_the_first_look` が固定）ので、
  **「終了→起動」では追わず「サスペンド→復帰」では 1 回だけ追う**という非対称が生まれて
  いた。CONTRACT 続報18 (2) はこの経路で破れていた。`every: hour` は 5 分、`day` /
  `weekday` は 15 分の猶予を入れ、超えたら発火せず理由を立てて次回へ送る。**数字は実測では
  ない**（5 分は `WORKFLOW_DEFER_MAX_MS` の再利用、15 分はその 3 倍）ので、U20 (6) で見る。
- **【高】CONTRACT に書いた DST の 2 規則が、CI で 1 度も実行されていなかった。**
  `resolve_local` が `chrono::Local` にハードコードされており、CI（`macos-14` /
  `ubuntu-22.04`、`TZ` 指定なし = UTC）には DST が無いので `LocalResult::Ambiguous` /
  `None` の分岐に一度も入らない。該当テストが主張していたのは `resolved.naive_local() >= naive`
  だけで、`Single` に対して自明に成り立つ（テスト自身のコメントも「host zone may have
  neither」と認めていた）。つまり**実測ではなく宣言**であり、実測を書く場である CONTRACT に
  対して過大記述だった。`next_fire_at` / `resolve_local` を `Tz: TimeZone` でジェネリックにし、
  dev-dependency に `chrono-tz` を足して `America/New_York` の 2026-03-08（春の飛び）/
  2026-11-01（秋の重なり）で固定した。`std::env::set_var("TZ", …)` は並列テストで unsound
  なので採らない。アプリ本体は従来どおり `driver_loop` が `Local` を渡すだけである。
- **あわせて、直さない限界を 1 つ固定した。** 秋の DST の日、`every: hour` はその日の発火が
  1 回減る（25 時間の日に 24 回）。繰り返される壁時計の読みを常に早いほうに解決するのは
  日次の「1 日 1 回」を守る規則そのものなので、直すと別立ての規則が要る。テスト
  `known_limitation_an_hourly_schedule_loses_one_fire_on_the_autumn_day` で固定し、
  CONTRACT 続報19 の既知の限界 (f) に書いた。
- **【中】200ms tick ごとに Config 全体をディープクローンしていた。** 変更前、run が 1 本も
  走っていないアイドル時の driver tick は `registry.active_run_ids()` の mutex 1 回で終わって
  いた。5.0.8 は tick ごとに無条件で `agents` / `processes` / `workflows`（全 step の
  `kickoff` を含む）/ `team_presets` を丸ごと clone し、さらに fingerprint の `String` を
  毎回組み立てていた。**5 回/秒 × 常時。** spec §3.7 は「200ms ごとに時刻計算をしない。
  tick では比較 1 回で済ませる」と決めているので、これは挙動ではなく決定に反していた。
  **レビューが挙げた 2 案の両方**を採った: (a) `tick_schedules` を 5 tick に 1 回（1 秒）に
  する、(b) `ConfigManager::current_arc()` を足して clone を避ける（`current()` は無変更）。
  加えて `ConfigManager` に世代カウンタを持たせ、fingerprint は設定が実際に差し替わった
  ときにしか組み立てないようにした。(a) だけでも 5 分の 1 になるが、(b) の変更範囲が
  `ConfigStateInner` の 1 フィールドと 3 箇所の代入だけで収まったので両方入れた。
  **どちらも未実測**である（→ spec §8.2）。
- **【中】同一 tick で複数が due のとき、発火順が非決定的だった。** 表が `HashMap` で、
  `view()` は `sort_by` していたが `due` はソートしていなかった。両方が `at: "09:00"` で
  グリッドの空きが片方ぶんしか無いとき、どちらが 2 面を取るかが実行ごとに変わる
  （負けたほうに `no room` が付く）。`pane_budget` はループ内で毎回読み直されるので**上限は
  守られていた**——壊れていたのは順序だけである。表を `BTreeMap` にして名前の昇順に固定し、
  `view()` の `sort_by` は不要になったので落とした。
- **【中】`Schedule` に `deny_unknown_fields` が無かった。** `maxConsecutiveFailure: 10`
  （`s` 落ち）や `enable: false`（`d` 落ち）は serde に黙って捨てられ、既定値（3 / 有効）の
  まま動く——**「止めたつもりのスケジュールが毎日発火する」**。`Every` の enum は closed で
  テスト済み、`at` の値も検証 S2 で落ちるので、**穴はキー名だけ**だった。リポジトリ全体の
  「未知フィールドは黙って無視」という慣習からは外れるが、その慣習こそこの機能の設計思想
  （書けないことが機能）と衝突している。`Schedule` は 5.0.8 の新規型なので既存設定への
  非回帰リスクはゼロ。**なぜここだけ例外にしたか**は CONTRACT 続報19 (E) と spec §3.2 に
  書いた。
- **【中】システム時計の巻き戻し / TZ 変更に対する再アンカーが無かった。** `next_fire_at_ms` を
  更新する経路は「宣言の変更による作り直し」と「発火/見送り後の再計算」の 2 つだけだったので、
  (a) 時計を 3 時間戻すとその日の発火が無言で 3 時間遅れ（パネルは健全なカウントダウンを
  出し続け、見送り理由も立たない）、(b) JST → GMT へ移動すると **1 回だけ GMT 00:00 に
  発火**していた。前 tick の時刻とオフセットを保持し、**5 秒を超える巻き戻し**または
  **オフセットの変化**で全スケジュールを再計算する。**前方向の跳びは扱わない** ——
  前方向の跳びとサスペンド復帰は観測上区別できず、そちらは猶予の担当である。レビューの
  指摘どおり H2 と同じ場所に置いた。
- **今回の検証: 自動テストのみ。** lib **506 → 520 passed / 0 failed**（新規 14 本。
  DST の 3 本、自動停止の解除 2 本、due 発火の巻き添え 1 本、猶予 2 本、発火順 1 本、
  キー名タイポ 1 本、`enabled: false` 1 本、`maxConsecutiveFailures: 1` 1 本、時計巻き戻し
  1 本、TZ 変更 1 本）、統合 14 不変、`cargo clippy --all-targets` は既存の `config.rs` の
  `nonminimal_bool` 1 件のみ。frontend は無変更。**実機検証は依然として未実施**で、U20 に
  4 項目（停止の解除の実経路、スリープ復帰、復帰直後の `Local::now()`、TZ 切り替え）を
  足した。
- **残件（次の担当への申し送り）。** (1) backend が返す `lastSkipReason` /
  `stoppedReason` は**英語の自由文字列**のままで、frontend がそれをそのまま表示するため
  日本語 UI でも英語が出る。本作業ではフィールドの型も構成も変えておらず（増減ゼロ）、
  今回足した「予定時刻超過」の文言が 1 種類増えただけである。構造化して i18n する担当は
  別に立っている。(2) **P3（escalation）は Stage A-4（2026-08-13、→ §6.16 / CONTRACT
  続報16）で配線済み**である。§6.19 の最後の一文「無人で時刻起動する run の失敗を外部へ
  知らせる経路が無い」は 2026-08-05 時点の記述で、いまは**失効している**（§6.19 は
  アーカイブなので §7 の append-only の規律どおり書き換えず、ここで失効を宣言する）。
  ただし**配線されたのは `retry:` の枯渇時だけ**で、`retry:` を書いていない step が
  失敗しても escalation は出ない — schedule が起こす workflow の step は多くがそれに
  当たるので、通知が欲しければ step に `retry:` を書く必要がある。なお run のペインが
  異常終了した場合の通知は 4.4.2 の経路から別に出る（続報16 (4) / U18 (4)）ので、
  「失敗を静かに溜める」は二重に不正確だった。(3) 自動停止の永続化（再起動で 0 に戻る）は
  §6.19 のまま未着手である。

---

### 6.21 2026-08-13: 5.0.8 の M5 — schedule 行を日本語で出す

§6.20 の申し送り (1)（**backend の英語文字列が日本語 UI にそのまま出る**）の是正。
新しい Phase ではなく 5.0.8 の修正で、`schedule:` を書かない設定への影響はゼロ。
wire が変わるので CONTRACT.md に**続報20**を立てた。spec は §6.1 を追記。
現在地は §2（**U20** に 1 項目追加）。

- **何が壊れていたか。** `ScheduleView` の `summary` / `lastSkipReason` /
  `stoppedReason` / `lastResult` は Rust 側で組み立てた**英語の文**で、
  `WorkflowPanel` はそれを `join(" · ")` するだけだった。言語を日本語にして 3 連続
  失敗させると 1 行が
  `🕒 every day 09:00 · stopped after 3 consecutive failures · 最終 8/5 9:00 failed`
  になる。`example/scheduled-review/ptygrid.yml` は「飛ばした理由はパネルに出る
  （**前の run が終わっていません**）」「パネルに**自動停止**と出る」と書き、spec §6 の
  表示例も全部日本語だったので、**サンプルと spec が約束した画面が実装から出ていなかった**。
  H1（spec と example と実装の食い違い）と同じ種類の欠陥である。
- **直し方: 判定は backend、文言は frontend。** wire は「宣言」と「タグ + 数値」だけを
  運ぶ。`summary` は削除して `every` / `at` に置き換え（`Schedule::summary()` も削除。
  唯一の呼び出し元がここだった）、3 つの理由は内部タグ付き enum
  （`{kind:"overlap"}` / `{kind:"noRoom",occupied,cap,needed}` /
  `{kind:"late",lateMinutes}` / `{kind:"consecutiveFailures",failures}` /
  `{kind:"succeeded"|"failed"|"cancelled"|"spawnFailed"}`）にした。
  タグ付けの書き方は `project_state.rs` の `LogicalSession` に揃えている。
  **文字列 + 任意の optional フィールド**（`TeammateLifecyclePayload` の形）も候補だったが、
  「`noRoom` のときだけ 3 つの数値がある」を型で言えるほうを採った — frontend に
  テストランナーが無い以上、`switch` の網羅性を TypeScript に見てもらえる形が要る。
- **`"failed: {err}"` を `spawnFailed` として分けた。** 旧実装はこの 1 つの文字列に
  「run が走って失敗した」と「そもそも起動しなかった（run が存在しない）」を詰めていた。
  `err` は spawn 経路の任意の文字列で翻訳できないので、生で運んで panel が
  「起動に失敗しました: {err}」と包む。**1 行の中で英語のまま残るのはこの例外文だけ**である。
- **あわせて直した低優先の指摘。** L12（`toLocaleTimeString` に日付が無く「次回 09:00」が
  今日か明日か分からない）→ 今日でなければ日付も出す。L13（`formatDurationMs(left) ?? "0s"`
  の `??` が無意味）→ カウントダウンが `formatDurationMs` を通らなくなったので消滅。
  L14（`consecutiveFailures` が wire にあるのに未使用）→ 停止前に
  「連続失敗 1/3（あと 2 回で自動停止）」を出す。無人運用では「あと 1 回で止まる」が
  最も価値のある情報で、そのために `maxConsecutiveFailures` を wire に足した。
  L15（ポーリングが 60 秒なので設定保存から表示更新まで最大 60 秒）→ コードではなく
  U20 の手順に 1 行足した。
- **`formatDurationMs` は変えていない。** カウントダウンだけがそれを使うのをやめて、
  分に丸めた「時・分」を i18n に渡す形にした（日本語で「あと 3 時間 20 分」）。
  他の 2 つの呼び出し元（step の実行時間、ペイン待ち）は 0.1 秒の解像度が要る別用途なので、
  共通化せず**分けた**。
- **今回の検証: 自動テストのみ。** lib **520 → 523 passed / 0 failed**（新規 3 本:
  `schedule_reasons_reach_the_wire_as_tags_and_numbers_not_sentences`（JSON の
  タグ名・フィールド名・camelCase を固定）、
  `a_schedule_view_carries_the_declaration_and_the_stop_threshold`（`summary` が
  wire に無いこと、`maxConsecutiveFailures` が出ること）、
  `a_padded_at_is_trimmed_before_it_reaches_the_panel`）。既存の schedule テスト 6 本は
  文字列の `contains` 比較から enum の等値比較に置き換えたので、**文言を変えても
  テストは通るが、タグを変えたら落ちる**という正しい向きになった。統合 14 不変、
  `cargo clippy --all-targets` は既存の `config.rs` の `nonminimal_bool` 1 件のみ。
  `svelte-check` 136 files 0 errors 0 warnings、`npm run build` 成功。
  i18n は en / ja 両方に 11 本追加。
- **未実測。** **画面は一度も見ていない。** 日本語の文言が例文どおりに出ることは
  i18n テーブルと `scheduleLine` を読んだ帰結であって、実機での確認ではない（→ U20）。
- **残件。** (1) P3（escalation）は Stage A-4 で配線済み（→ §6.20 残件 (2)）。残るのは
  「`retry:` を書いていない step の失敗は外へ出ない」ことだけである。
  (2) 自動停止の永続化（再起動で 0 に戻る）も未着手。(3) `spawnFailed` の `error` は
  英語のまま出る（元の例外文なので、これは仕様）。(4) **日付・時刻の書式はアプリの言語
  設定ではなくシステムロケールに従う**（`toLocaleString(undefined, …)`）。schedule 行
  だけ `currentLocale()` を渡すと同じパネルの run 行と書式が食い違うので、パネル全体の
  話として分けた。日本語 OS ならそのまま日本語書式になるので、実害は「英語 OS で日本語
  UI」の組み合わせに限られる。

---

### 6.22 2026-08-13: 5.0.8 の最終レビューで出た 11 件の是正

§6.19〜§6.21 を通した最終レビューで 11 件出た。**新しい Phase ではなく 5.0.8 の修正**で、
**wire は 1 バイトも動いていない**（frontend も無変更）。契約の差分は
[CONTRACT.md](../../CONTRACT.md) 続報21、仕様は
[spec-schedule-5.0.8.md](../spec/spec-schedule-5.0.8.md)（§3.1.1 / §3.1.2 / §3.5 / §3.7 /
§4.1 / §6.1）。現在地は §1・§2（**U20** の (9) を 1 点緩めた）。

- **【高】fingerprint が `HashMap` の `Debug` を混ぜており、同じ設定でも毎回変わっていた。**
  §6.20 で入れた「スケジュール 1 本ごとの fingerprint」は宣言の `Debug` 表現をハッシュして
  おり、コメントは「Deterministic: `WorkflowDef` holds no maps」と書いていた。**それは
  同じ関数の半分にしか当てはまらない** — もう半分は step が名指しする `AgentDef` を
  ハッシュしており、`AgentDef` には `env: Option<HashMap<String, String>>` がある。
  Rust の `RandomState` はインスタンスごとに違うキーを使うので、**同じバイト列を 2 回
  パースしただけで `Debug` の並び順が変わる**（レビュー担当の実測で env 4 キー・20 回中
  19 回）。結果、自動停止した schedule は**無関係な workflow を保存しただけで 6 回に 5 回
  解除され**、連続失敗カウンタも次回時刻もリセットされていた。§6.20 が主張した 2 つの
  性質が両方とも成立していなかったことになる。発現条件は `env:` 2 個以上の agent、
  すなわち **`ptygrid init` が自分で書く形**（`ANTHROPIC_BASE_URL` +
  `ANTHROPIC_AUTH_TOKEN`）で、`example/adaptive-orchestration` も該当する。既存テストが
  見逃したのは fixture が全部 `cmd: /bin/cat` だけの agent だったため。是正は
  `serde_json::to_value()` 経由でのハッシュ（`serde_json::Map` は `preserve_order` 無効時
  `BTreeMap` なのでキー順が確定する）。**`serde_json::to_string()` を struct に直接かけると
  `HashMap` をそのまま辿るので直らない**ことは、コメントにも spec §3.5 にも明記した。
  `Err` 時は `Debug` に落とす — map を含む値では不安定なので毎回作り直す側に倒れるが、
  §6.20 が書いたとおり「広めに倒すほうが安い」ので意図どおりである。
- **【中】春の DST の日、再アンカーが「まさに due だった正当な発火」を飲んでいた。**
  §6.20 で入れた TZ 変更の再アンカーは `now.offset()` を見るので、**DST の遷移で必ず
  発動する**（巻き戻し判定は `timestamp_millis()` = UTC 単調なので DST では発動しない）。
  `America/New_York` の `{ every: day, at: "02:30" }` は 2026-03-08 に 02:30 が存在しない
  ため §6.19 の規則で 03:00 EDT（= 07:00:00.000 UTC）に着地するが、**その瞬間がオフセットの
  変わり目**なので、07:00:00.000 UTC 以降の最初の tick で「オフセットが動いた」と
  「`next_fire_at <= now`」が同時に成立する。再アンカーが「fire what is due」より前に
  あったため、その日の発火が消え、`lastSkipReason` も立たなかった（spec §1.3 が「絶対に
  作らない」と言っている形）。`every: hour` でも春の飛びの直後の 1 回が消えていた。
  spec §3.1.2 は自分で「**前方向の跳びは扱わない。ここで再アンカーすると、正当に due な
  発火まで飲み込む**」と書いており、その理由づけを TZ 側に適用し忘れていた形である。
  **レビューが挙げた 2 案のうち差分の小さい (1) を採った**: 再アンカーのループで
  `next_fire_at_ms <= now` の行をスキップする。ブロックごと後ろへ移す案 (2) は、
  「due でない行の再アンカー」と「count the outcome」の順序まで変えてしまうので採らない。
  なお H2 × H3 の「猶予の誤爆」（春の 1 時間の飛びが `lateMs` に化ける）はレビューで
  起きないことが確認済み（`next_fire_at_ms` は `resolve_local` を通った実在する絶対時刻で
  ある）ため、そちらは触っていない。
- **【中】「P3 は未配線」という記述が 4 箇所あった。** escalation は Stage A-4
  （2026-08-13、→ §6.16 / CONTRACT 続報16）で配線済みで、`ptygrid-yml-guide.md` に
  至っては**同じ表の中で 2 行が矛盾**していた（`schedule:` 行が「いまも ❌」、escalation
  行が「✅ 2026-08-13」）。4 箇所とも「配線済み。ただし出るのは `retry:` を使い切った
  step だけで、`retry:` を書いていない step の失敗は外へ出ない」に改めた。ペイン異常終了
  由来の通知も別に出るので、「失敗を静かに溜める」は二重に不正確だった。**§6.19 は
  2026-08-05 のアーカイブなので §7 の append-only の規律どおり書き換えず**、§6.20 の
  残件 (2) で失効を宣言する形にした。CONTRACT 側は続報18 の既知の限界 (d) が続報19 の
  上書き 5 点に入っておらず現行の記述として生きていたので、続報21 で明示的に上書きした。
- **【中】§1 の通し進捗表の 5.0.8 行が採番修正から漏れていた。** マージ時に U15 → U20 へ
  振り直したが §1 だけ古く、U15（現在は「resume 拒否ガードの実機検証」）という別物を
  指していた。テスト本数も 475（マージ前の系列）。`未（U20）。自動テストのみ
  （lib 530 / 統合 14）` に直した。
- **【低】7 件。** (1) `SCHEDULE_GRACE_HOURLY_MS` が `WORKFLOW_DEFER_MAX_MS` と同じ数字の
  **別リテラル**で、doc コメント / CONTRACT 続報19 (A) / spec §3.1.1 の「再利用」「同じ
  問いに 2 つ目の答えを作らない」がコード上は嘘だった → 定数そのものを参照する。
  (2) `lateMinutes` が切り捨てで、**見送りが起きる最小の遅れ**（猶予 +1ms = 900,001ms）が
  「15」と出ていた。「猶予 15 分と書いてあるのに 15 分遅れで見送られた」と読めるので
  `div_ceil` にした。 (3) `current_arc()` が唯一の呼び出し側が捨てている `PathBuf` を
  毎秒クローンしており、spec §8.2 / 続報19 (F) の「`Arc` のクローン 1 回と整数の比較
  1 回」が厳密には偽だった → 返り値から外した（`None` の意味は不変）。 (4)
  `CLOCK_STEP_TOLERANCE_MS` の根拠文が「driver tick 25 回ぶん」のままだった。
  `last_tick_ms` を書くのは `tick_schedules` だけで、それは §6.20 (M4a) 以降 5 tick に
  1 回なので実際の標本間隔は 1 秒 = 5 標本である（結論は不変、根拠の数字だけ古かった）。
  (5) spec §4.1 の `Schedule` スニペットに `deny_unknown_fields` が無く、同じ文書の
  §4.2 S8 と矛盾していた（実装には付いている）。 (6) 続報20 の上書き宣言に**続報19 (G) が
  抜けていた** — (G) の「フィールドは 1 つも増減していない」「`lastSkipReason` は既に
  自由文字列だったので契約の変更ではない」は M5 で両方とも偽になっている → 続報20 の
  冒頭を補った。 (7) `example/scheduled-review/ptygrid.yml` の画面例の `最終 8/5 09:00` は
  `fmtSchedStamp` が `toLocaleString(undefined, …)` を使う以上 en-US では
  `8/5, 09:00 AM` になり、U20 (9) の「例文と 1 字ずつ一致」は**原理的に達成できない** →
  例文に注記を足し、U20 (9) の 1 字一致の対象から日時スタンプを外した（一致を見るのは
  語のほう）。
- **テスト。** lib **523 → 530 passed / 0 failed**（追加 7 本）、統合 **14 不変**、
  `cargo clippy --all-targets` は既存の `config.rs` の `nonminimal_bool` **1 件のみ**、
  `svelte-check` **136 files 0 errors 0 warnings**。**追加 7 本のうち 5 本は、修正前の
  コミットで実際に落ちることを確認してから直した**:
  `the_same_declaration_hashes_to_the_same_fingerprint_every_time`（20 回ループ。1 回では
  6 回に 1 回の確率で通ってしまうため）、
  `editing_an_unrelated_workflow_does_not_restart_a_stopped_one_with_env`（既存の同種
  テストの fixture は `cmd:` だけなので env 版を別に立てた）、
  `saving_the_same_file_again_moves_nothing_in_the_table`（世代カウンタと fingerprint の
  交点）、`a_fire_due_in_the_very_tick_the_offset_moves_still_happens`（既存の TZ テストは
  `FixedOffset` で移動時点に due な発火が無く、再アンカーが発火を飲むかを一切問うて
  いなかった。DST の既存 3 本も `next_fire_at` の純関数側しか見ていない）、
  `the_grace_window_is_closed_at_the_top_and_reports_a_whole_minute`（`lateMs == graceMs` と
  `+1ms` の 2 点）。残る 2 本は既に正しかった挙動を tick 経由で固定したもの:
  `deleting_the_schedule_block_removes_the_row`（既存の inert テストは最初から schedule
  無しで始まるので「あった schedule を消す」経路を通っていない）、
  `the_hour_that_happens_twice_loses_one_fire_and_never_gains_one`（秋の DST を
  `tick_schedules` 経由で 4 時間ぶん回し、発火が 3 回 — 増えも減りもしない — であることを
  固定）。
- **残件。** (1) **実機検証は依然として未実施**（→ U20）。今回の 2 件はどちらも実機で
  しか出ない条件（保存の実経路、DST の当日）に見えるが、**自動テストで再現・固定できる
  形に落としてある**ので U20 の負荷は増えていない。 (2) 猶予の 2 つの数字も時計跳びの
  閾値も実測ではない（続報19 (h) のまま）。 (3) 自動停止の永続化も未着手のまま。

### 6.23 2026-08-13: v0.5.8 リリース

詳細な経緯。現在地は §1・§4。

- **入ったもの（3 系統）**。(1) **`v0.5.7` 以降に `main` へ入っていたぶん**: Phase 5.0.7
  `onEach: reply` / `joinOn: stream`（`ee6e020` = PR #16、per-copy mailbox の修正 `0a5d4be` =
  PR #17、実機 1 回目の記録 `52fd0d9` = PR #18。→ §6.13 / §6.14）、5.0.2 追補のローカル LLM プローブ（PR #11 / #12。→ §6.9）、
  5.0.6（案）の計測フィールドと合成 workflow と cold start 実測（→ §6.11 / §6.12）、ターミナルの
  コピー & ペースト（PR #13。→ §6.10）、タイトルバーのバージョン表示（`c74997f`）。
  (2) **Stage A**（`63ef201` でマージ。A-2/A-3 resume の carry 喪失ガードと `handoffTo` 合流 /
  A-4 escalation 配線 / A-5 cancel・abandon の kickoff ack / A-6 `workflow_runs` の retention。
  → §6.15〜§6.18、CONTRACT.md 続報12〜17）。(3) **Phase 5.0.8 `schedule:`**（`047a6ed` でマージ。
  レビュー是正 3 巡ぶんを含む。→ §6.19〜§6.22、CONTRACT.md 続報18〜21）。(2)(3) はどちらも
  §4 の実装項目 1〜8 を決めたあとに順序が決まったもので、**当初の予定には無かった**（§4 の項目
  9 / 10 に内訳と、載せた理由を書いた）。
- **version 3 ファイルを揃えた**。`package.json` / `src-tauri/Cargo.toml` /
  `src-tauri/tauri.conf.json` を `0.5.8` にし、`src-tauri/Cargo.lock` は `cargo check` で追従させ、
  **そのコミットに `v0.5.8` を打った**。これは §4 の実装項目 6 が「`v0.5.7` が指すコミットは
  3 ファイルとも `0.5.6` のままだった」という前科に対して要求していたもので、**version を
  揃えたコミットにタグを打った最初の断面**になる。
- **当時の検証**: `cargo test` lib **530 passed** + 統合 **14 passed** / **0 failed**。
  `cargo clippy --all-targets` は既存の `config.rs` の `nonminimal_bool` **1 件のみ**で新規警告
  ゼロ（5.0.4 由来、非回帰）。`svelte-check` **136 files 0 errors 0 warnings**、
  `npm run build` **成功**。`cargo fmt` は §6.18 以降と同じ理由で**走らせていない**
  （既存コードが現行 rustfmt で未整形のため、無関係な差分が大量に出る）。
- **このタグ最大の注記: 実機検証は 1 つも消えていない**。**U14（残 3 点）/ U17 / U18 / U19 /
  U20 が全部未実施**のままタグに入っている（U15 / U16 も同様に未実施）。v0.5.7 までは
  「タグには実機で見たものを入れる」運びで、実際 §6.8 は U1 / U3 / U5 / U11 の 4 件を消して
  から打っている。**v0.5.8 はその運びと明確に食い違う**。§4 の「（2026-08-04 追記）」が
  「含める場合は U14 が未消化のままタグに入る」と予告した食い違いが、5.0.7 だけでなく
  Stage A と 5.0.8 にも広がった形である。裏づけは**自動テストだけ**であって、
  実機で動くことの確認ではない。
- **予定していて消化しなかった項目が 1 つある**。§4 の実装項目 5（**U4 の消化** =
  同名 workflow の並行 run が互いの返信を取りこぼさないこと）は未消化のままである。
  `next-implementation-2026-08.md` §3 の A-1 は「v0.5.8 を出す。**U4 の消化を含む**」を完了
  ゲートにしていたので、**A-1 のゲートは半分しか満たしていない**。U4 は §3 P1 の枠に残る。
- **CI は未確認**。§7 の運用メモはリリースの規律に「両プラットフォーム CI 通過」を挙げているが、
  **この断面の作業は Linux コンテナ上で行われ、macOS の CI は回していない**。したがって
  `#[cfg(target_os = "macos")]` のコード（項目 7 のアプリメニュー App / Edit / Window など）は
  上記チェックの対象外である（U9 が同じ限界を 2026-08-04 に記録している）。**Linux で通った
  という事実から macOS でも通ると書くことはしない**（§7 の「推測を断定で書かない」）。
- **その他の注記**: `v0.5.2`〜`v0.5.5` の予約（Phase 5.0.2〜5.0.5 用）は使わないまま残る。
  `v0.5.7` に続いて `v0.5.8` も Phase 5.0 系に充てたことで、spec-phase5-5.md §9 の
  「バージョン割り当て」表は**通算 2 度**繰り下がり、5.5.1 = `v0.5.9` になっている
  （同 spec 側で対応済み）。`user_version` は 3 のままで、このタグでは消費していない。

### 6.24 2026-08-14: v0.5.9 の負債返済 3 件と掃除（新機能ゼロ）

`next-release-v0.5.9.md` §2 の担当ぶん。**新機能は 1 つも足していない**。契約の先行追記は
CONTRACT.md **続報22**。ブランチは `fix/retire-kickoffs-on-terminal`。

- **入ったもの（コード 3 件）**。
  1. **終端した run の kickoff を ack する**（§2.1）。`retire_run_kickoffs`（続報15 の純関数）の
     呼び出し元が **cancel と abandon の 2 か所だけ**だったので、`joinOn: reply` でない step —
     route 1 / route 2 で完了する多数派 — の kickoff が**未 ack のまま `inbox_messages` に
     溜まり続けていた**。A-6 の retention は `workflow_runs` を DELETE するだけでこの表に
     効かない。`advance_run` が run を終端させた tick でも同じ関数を呼ぶようにした。
  2. **run 全体の失敗にも escalation を出す**（§2.3）。escalation の入口が step の retry 枯渇
     1 つだけだったので、`retry:` を書いていない step だけの workflow は run が red でも
     **1 通も出なかった**。純関数 `run_failure_escalation` を足し、`advance_run` の末尾と
     `spawn_workflow`（root が全部 spawn 失敗して**生まれた瞬間に `Failed`** の run）から呼ぶ。
     配送機構は新設せず、既存の `dispatch_ctx` にそのまま乗せた。
  3. **`send_os` を detached thread へ出す**（§2.2）。`escalated` は step 単位のフラグなので
     `onEach` のコピーが同一 tick で一斉に枯渇すると **1 tick で最大 64 通**（`STREAM_MAX_UNITS`）
     になり、同期のトースト呼び出しが **200ms tick を直接ブロックしていた**。webhook 側
     （`post_json`）と同じ形にしただけ。**run 単位のダイジェスト化はやっていない**
     （spec-notifications v2 のまま。1 tick で 64 通「出る」こと自体は変わらない）。

- **入ったもの（掃除 4 件）**。
  1. **retention の起動時一括掃除**。`QueenStore::open` から
     `prune_every_projects_terminal_workflow_runs` をプロセス起動につき 1 回。**`VACUUM` は
     入れていない**（起動時にファイル全体を書き直すことになるため）。
  2. **`src-tauri/src/orchestrator.rs.bak` を `git rm`**。5.0.0 から追跡されたままで v0.5.8 にも
     同梱されていた。live source ではないので**挙動は変わらない**。§3 の該当項目はこれで消える。
  3. **CONTRACT.md の「続報11」が 2 つあった問題**を、**続報11a（2026-07-30）/ 続報11b
     （2026-08-04）**の枝番で解消。参照側は CONTRACT 4 か所と本文書 §6.13 の 1 か所。
  4. **`ptygrid-yml-guide.md` の前文の陳腐化 2 件**（「`orchestrator.rs` の実行系配線は未完了」
     「`timeoutMs` / `retry:` 行の ❌ 判定は有効」）を訂正。表の当該行は 5.0.4 で既に ✅ で、
     前文と本文が矛盾していた。加えて §2.3 で内容が変わる §1 の 2 行（`schedule:` /
     escalation）と、`.bak` を前提にしていた supervisor 行の注記も追記した。
  5. **`PTYGRID_MAILBOX` の契約を CONTRACT に明文化**（続報22 (5)、**コード修正は無し**）。
     env 注入は `spawn_step` の fresh spawn 経路にしか無く、`autostart` / `spawn_agent` /
     `spawn_team` が立てたペインには**存在しない**（`$PTYGRID_MAILBOX` が空文字に展開される）。
     ペイン再利用経路も通らないので、再利用した step の値は前回 spawn 時のまま。現状実害が
     出ていないのは `onEach` のコピーが常に `reuse_existing: false` だからにすぎない。

- **設計判断（§2.3 で決めたもの）**。
  - **(a) 二重通知は「抑止」を採った**。step の枯渇 escalation が 1 通でも出た run では run 単位を
    出さない。step のメッセージは既に failing step / agent / 試行回数 / エラー本文を名乗って
    いるので「よって run が赤い」は操作を増やさず、`onEach` × `retry` は 1 tick で最大 64 通を
    出しうる側でもある。**新しい状態は足していない** — 既存の `StepOutcome::escalated` を読むだけ。
  - **(b) `Cancelled` は対象外**。`finalize_state` は `Cancelled` を返さず、書き手は
    `cancel_workflow` だけ ＝ 自分で止めた操作者に結果を通知することになるため。
  - **(c) `origin` は `scope: OriginScope::{ Step { step_id, attempts }, Run { failed_steps } }`
    に変えた**（Rust 内部型で wire には出ない）。run に試行回数は無く step に失敗一覧は無いので、
    空文字 `step_id` を番兵にすると「`demo/` exhausted its retries」が黙って出せてしまう。
    **step スコープの文面はバイト単位で不変**。
  - **(d) エッジの取り方**は §2.1 と共有した（`just_reached_terminal`）。**フラグは足していない** —
    「遷移前の state」がそのまま記憶で、`advance_all` / `advance_run` はどちらも終端 run を
    tick しない。`#[serde(skip)]` で resume 時に落ちる新フィールドを増やさない狙いもある。
  - **200ms tick への追加コストはゼロ**。§2.1 / §2.3 のどちらのゲートも Rust 側の enum 比較で、
    SQL が増えるのは「終わった run 1 本につき 1 文」だけ。A-6 の
    `prune_terminal_workflow_runs` が同じ形で終端書き込みに乗っているのに倣った。

- **検証値（実測）**。`cargo test` は **lib 530 → 543 passed / 0 failed**（新規 13 本）、
  **統合 14 → 14（不変）**。`cargo clippy --all-targets` は既存の `config.rs` の
  `nonminimal_bool` **1 件のみ**で、本作業起因の新規警告ゼロ。frontend 無変更のため
  `npm run check` は**走らせていない**。`cargo fmt` も走らせていない（規律どおり）。
- **修正前に落ちることを確認したテスト**（それぞれ親の断面で実行して確認した）:
  `a_run_the_driver_finishes_retires_its_own_kickoffs`（§2.1）、
  `a_run_that_fails_without_any_step_escalation_escalates_once` /
  `a_run_failure_notice_carries_the_first_failed_steps_error`（§2.3）。
  逆側（`a_run_the_driver_leaves_running_keeps_its_kickoffs_live` /
  `a_run_whose_step_already_escalated_is_not_escalated_again` /
  `a_succeeded_or_cancelled_run_is_never_escalated`）は**修正前から緑**で、これは正しい —
  「出るべきでないときに出ない」を固定するテストだからである。

- **未実測（推測で埋めていないもの）**。
  - **実機検証は 1 件も行っていない**。OS トーストが detached thread から実際に出るところも、
    run 単位の escalation が Slack / トーストに届くところも見ていない（→ §2 の U18）。
    本作業は Linux コンテナ上で行われ、**macOS の CI も回していない**。
  - `inbox_messages` が `MAX_MESSAGES_PER_PROJECT` = 50,000 に到達するまでの実時間は**未計測**。
    毎時 1 run × kickoff 3 step なら 72 行/日で約 690 日、という算数はできるが、返信ぶんが
    加算されるので実測が要る（→ U19 (4) のついでに行数を測ること）。
  - **起動時掃除が実際に何行削るか、その所要時間も未計測**。削除対象ゼロなら
    `SELECT DISTINCT` 1 文 + プロジェクト数ぶんの `count(*)` だが、これも実測していない。
  - **§2.2 の効果を数字で示していない**。「64 発のトーストが tick をブロックする」も
    「ブロックしなくなった」も、コードの形からの帰結であって計測ではない。

- **やっていないこと**。`next-release-v0.5.9.md` §2.5 の表にある項目（lost-update race、
  `condition:` の多依存 AND、root の `fanOut`、straggler の `close_on_exit`、`arena`、
  `workflow_runs.error`、resume 拒否の偽陽性、秋の DST、schedule の自動停止解除、
  `orchestrator.rs` の「phase 5.0.5」表記 17 か所）には一切手を付けていない。
  version 3 ファイルも触っていない。

---

## 7. 運用メモ

- 各リリースは `docs/inside/phase3.md` の規律を踏襲する: CONTRACT 先行追記、`lib.rs` / hot path に
  新ロジックを置かない、unit + integration テスト、両プラットフォーム CI 通過、userguide 更新。
- 本文書は Phase の完了・計画変更のたびに **§1 の通し進捗表と §3 の次の作業**を更新する。§6 の
  進捗記録は追記のみで、過去の記録は書き換えない。
- 推測を断定で書かない。実測していないものは「未実測」「判定不能」と明記する（例: U9 / U10）。
