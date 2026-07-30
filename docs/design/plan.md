# ptygrid 作業計画 (plan.md)

更新日: 2026-07-31 / 実装基準: `main`（PR #10 のあと PR #11 / #12 = `17860e0` / `b8300a4` が
マージされ、`.gitignore` 追加の `89411b9` が直接コミットされている）。作業中のブランチは
`feat/step-timing-5.0.6` と `feat/terminal-copy-paste`（どちらも `main` から分岐、**origin へ
push 未・PR 未作成**）。
最新タグは `v0.5.7`（2026-07-30 作成）、次のタグは `v0.5.8`（未作成）。作業ツリーの
`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` の 3 ファイルはいずれも
`0.5.7` だが、**タグ `v0.5.7` が指すコミットはそうではない**（→ §4）。

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
| （UI 横断） | ターミナルのコピー & ペースト（macOS 限定のアプリメニュー App / Edit / Window + `tauri-plugin-clipboard-manager`、コピーは macOS が Cmd+C・それ以外が Ctrl+Shift+C で選択が無いときは介入せず PTY へ、貼り付けは macOS がネイティブ経路・Ctrl+Shift+V が自前ハンドラで `term.paste()` 経由、右クリックメニュー、`macOptionClickForcesSelection: true`） | 🚧 | v0.5.8（予定） | 一部済（U13、2026-07-31。残り 4 点） |
| （UX トラック） | Phase 4 期に計画外で入った UX 改善: `mterm.yml` → `ptygrid.yml` リネームと用途別サンプル（`da40cb0`）、一括 cd（`cf42ced` / `77d0271`）、作業フォルダと設定探索の分離 + origin バッジ（`acbed94`）、設定なしフォールバック（`0530e3b`）、フォルダサジェスト（`a3a769a`）、終了ペインの明示と一括クローズ（`d8a3d8e`） | ✅ | v0.4.2 | 記録なし |
| （安定化） | docs/inside のバグ / セキュリティ調査への対応: backend 純バグ 12 件（`c6f31ad`）、frontend 純バグ 8 件（`7505bbe`）、S1 Queen `/mcp` の token + Host/Origin 認証（`3159263`）、S2/S4 autostart 信頼境界 + CSP（`f18bae6`）、手打ち claude の lead 帰属修正（`9c4ab67`）、認証トークン永続化（`0af8de4`） | ✅ | v0.4.3 | 記録なし |
| 5.0.0 | MVO: `workflows:` スキーマ + 検証 / `orchestrator.rs`（spawn + DAG 進行ドライバ、fail-fast、fan-out fresh-spawn）/ Queen MCP tools 22 本 / WorkflowPanel + 🔀 チップ / `close_on_exit`・`autoClose` | ✅ | v0.5.0（`b1b4f1f`） | pipeline 実走 済（U1、2026-07-30）/ fan-out 実走 済（U2、2026-07-31） |
| 5.0.1 | Workflow Resume: `workflow_runs` 永続化（`user_version` 2→3）+ write-through、`workflow-resume-pending` イベント + Y/N バナー、`resume_workflow` / `abandon_workflow` | ✅ | v0.5.1（`ac1b94b`） | 済（U5、2026-07-30） |
| 5.0.2 | `ptygrid init`: `ptygrid.yml` の自動生成（環境検出 → テンプレート生成 → 自己検査 → 既存ファイルがある場合は sidecar で差分提示）。backend（`init.rs` + Tauri command 3 本）と frontend（`InitPanel.svelte` + 入口 2 つ + i18n）が実装済みで自動テストは通過。2026-07-30、macOS 実機で全経路を確認済み（→ §2 U11）。spec: [spec-init-5.0.2.md](../spec/spec-init-5.0.2.md)（→ 脚注※）。ローカル LLM プローブの追補は次行 | ✅ | v0.5.7 | 済（U11、2026-07-30） |
| 5.0.2 追補 | ローカル LLM プローブ: `init_probe_llm`（4 本目の Tauri command）で既定 3 ポート + 手入力最大 4 本に `GET /v1/models` を当て、Anthropic Messages API 互換の確証が取れたときだけ有効な agent 定義を出す（それ以外はコメント行）。モデル選択 `<select>`・検出行への反映・`ANTHROPIC_AUTH_TOKEN` の出し分けを含む。ブランチ `feat/init-llm-probe-5.0.2` は `main` にマージ済み（→ §4） | 🚧 | v0.5.8（予定） | 一部済（U12、2026-07-30。残り 3 点） |
| 5.0.3 | Queen MCP 登録の代行（バッジからコピペしている `claude mcp add` 等を ptygrid が代行。claude は CLI を代行実行、codex / grok は TOML を `toml_edit` で値単位編集。差分承認・冪等・登録解除を含む）。spec: [spec-registration-5.0.3.md](../spec/spec-registration-5.0.3.md)（→ 脚注※） | ⬜ | — | 該当なし |
| 5.0.4 | Orchestrator 実行層: `joinOn: reply` 完了判定 / `condition:` 評価 / `handoffTo` チェイン / `retry:` 再試行 / `timeoutMs` 強制 / supervisor・handoff の spawn ゲート撤去。続けて `fanOut` 黙殺による false green の解消と straggler（`any` / `N` join の敗者）協調キャンセル | 🚧 | v0.5.7（`5d3c1b5` → `3bd9833` = PR #1、`52de433` = PR #3 に同梱） | 基本の実走 済（U1）/ straggler 協調キャンセル 済（U2、2026-07-31）/ `joinOn: reply`・`condition:`・`handoffTo`・`retry:`・`timeoutMs` の実走は未（§2 に項番を立てていない） |
| 5.0.4 追補 | Orchestrator ハードニング: pane 上限の待ち行列化 / driver tick 軽量化（`session_states()`・registry evict）/ inbox mailbox の run 単位分離。wire 契約は無変更 | 🚧 | v0.5.7（`2dc5e40`、PR #3 = `4c02cbb`） | U3 済（2026-07-30）/ U4 未 |
| 5.0.5 | **Arena view**（`arena.rs` + `Arena.svelte`、`arena-open` イベント、`arena.vote` / `arena.list_votes`）。`arena: true` は現状パースだけ通り、書いても何も開かない | ⬜ | — | 該当なし |
| 5.0.6（案） | Orchestrator の計測とパイプライン化: `StepOutcome` に step 単位の終了時刻とペイン待ち時間を additive 追加 / 合成 workflow（直列・鎖分割・9 面待ち・fan-out + `joinOn: any` の 4 本）で orchestration の効きだけを測る / cold start（ペイン再利用 vs 毎回 spawn）の実測。**patch 番号は提案でありユーザー判断で確定**（→ 脚注※2） | 🚧 | v0.5.8（予定） | 合成 workflow 4 本の実走と実測 済（2026-07-31、同じ回で U2 も消化）/ cold start 実測 済（2026-07-31、`example/measure-coldstart`）/ U4 は未 |
| 5.5.0 | MCP 2026-07-28 RC 互換ルータ（`queen_compat`: header / route / capabilities / deprecation / initialize / meta、hot-swap 可能な `McpCompatHandle`、legacy 2025-06 併存） | 🚧 | v0.5.6（`21d1367`） | 記録なし（U10） |
| 5.5.1 | OTel GenAI 計装 + SQLite シンク（span の書き出し先） | ⬜ | — | 該当なし |
| 5.5.2 | Cost 計算 + `agent-cost` イベント | ⬜ | — | 該当なし |
| 5.5.3 | Agent Status Rings（通知リング / 要承認ハイライト。出自は competitive-landscape の「次に取る UX」で、4.0 の teammate permission 表示の汎用化。設計は spec-phase5-5.md §2.3 / §3.7） | ⬜ | — | 該当なし |
| 5.5.4 | Trace Waterfall + Cost Dashboard | ⬜ | — | 該当なし |
| （無番号） | escalation: retry 枯渇時に外部へ通知する経路（4.4.2 の `notifications:` 基盤への配線）。枯渇判定は 5.0.4 で発火するようになったが配送経路が無い | ⬜ | — | 該当なし |
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

**いま特に効く読み方**: 2026-07-30 に `smoke` を実機で流し切ったことで、**「workflow は一度も実走していない」という一括の未知は消えた**（U1 完了）。続けて同日中に U3（pane 上限待ち）と U5（resume バナー）も実機で完了した。同日さらにローカル LLM プローブの追補が入り実機 1 回目を通し、翌 2026-07-31 にターミナルのコピー & ペーストが入って実機 1 回目を通した。**同じ 2026-07-31 に合成 workflow で U2（fan-out + straggler 協調キャンセル）も完了**し、5.0.0 は実機側の積み残しが無くなって ✅ になった一方、その計測作業そのものである「5.0.6（案）」が実装 + 実測まで進んで残作業ありの状態になったため、**🚧 は 8 行のまま**。同日さらに cold start の実測（`example/measure-coldstart`）も済み、「5.0.6（案）」に残るのは **U4 だけ**になったが、それが残っている以上 ✅ にはならない（→ §6.12）。残る理由は個別機能ごとに分かれている: Linux 常用（U7）、host モード（U6）、コピー & ペーストの残り 4 点（U13）、プローブ追補の残り 3 点（U12）、5.0.4 の残る固有機能（`joinOn: reply` / `condition:` / `handoffTo` / `retry:` / `timeoutMs` の実走。§2 に項番を立てていない）、mailbox 分離（U4。5.0.4 追補と 5.0.6（案）の両方がこれを待っている）、5.5.0 の実機記録なし（U10）。未タグ成果へのタグ付けは v0.5.7 で一度消化し、次は v0.5.8（→ §4）。

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
| U9 | frontend チェック（`svelte-check` / `npm run build`） | 本作業環境に `node_modules` が無く**未実測**。`src/` は v0.5.1 の `ac1b94b` 以降変更されておらず `v0.5.6..main` の diff も 0 件なので v0.5.1 時点の「0 errors」から変わっていない**はず**だが、これは推測であって実測ではない |
| U10 | 5.5.0（RC 互換ルータ）の実機検証 | **記録が無く判定不能**。CONTRACT.md の実装状況節も自動テスト（unit 35 + 統合 14）しか挙げていない。実機で RC / legacy 双方のクライアントを繋いだ記録は見当たらない |
| U11 | `ptygrid init`（5.0.2）の実機検証 | **2026-07-30、macOS で実施**（すべてスクリーンショットで確認済み）。(1) 設定の無いフォルダで起動→シェル 1 枚→「設定を作る」ボタンが出て、検出結果（opencode/claude/codex/gemini/qwen/grok/aider の 7 体・npm・git あり・ローカル LLM ルータ未検出・既存設定なし）が実環境と一致することを確認、(2) 通常生成で `ptygrid.yml`（2,060 バイト）が生成され agents チップ 7 体が並び、生成物は autostart 全 false のため trust プロンプトは出ずペインも自動起動しないことを確認、(4) 既存設定ありの状態では副入口の書き込み先が `ptygrid.init.yml` に切り替わり、書き込み後も既存 `ptygrid.yml` は mtime・内容とも無変更であることを確認（上書き禁止の実測裏付け）、(5) 書き込み直後に init 自身の通知と watcher `config-changed` による再読み込みトーストが二重に出る競合を実測（spec §9 で推測としていた箇所が確認され、直後に自己書き込みエコー抑制（`ui.selfWrite` + 3 秒窓）を別コミットで修正済み）。(3) プレビューを手編集して `autostart: true` にしてから書き込むと**今度は trust プロンプトが出て**、「信頼して起動」で当該エージェントが実際に起動することを確認（`init_write` → `loadConfig` → `maybeAutostart` の順序の実証）。**U11 は完了**。Global 選択時の `~/.ptygrid/` 作成のみ今回の範囲外（必要になった時点で確認する）。詳細な経緯は §6.4 |
| U12 | ローカル LLM プローブ（5.0.2 追補）の実機検証 | **2026-07-30、macOS で 1 回目を実施**（スクリーンショットで確認済み）。検出フォルダ `~/works/tmp/ptygrid`、PATH 上の CLI 7 体（opencode / claude / codex / gemini / qwen / grok / aider）、プロジェクト種別 npm、git リポジトリあり、既存設定ありのため書き込み先が `ptygrid.init.yml` に切り替わることを確認。プローブは 1234 / 3456 / 11434 を叩き、3456 は無応答、**11434 で `Ollama 0.32.1` が応答して「Anthropic API 確証あり」バッジが出てモデル 20 件を取得**（先頭は `x/flux2-klein:latest`）。**まだ確認していないことが 3 点**: (1) モデル選択 `<select>` の実機動作（実装は 2 つ目のコミット `8931464` で入ったが押していない）、(2) 生成された `local-11434` の定義で実際に Claude Code が起動するか、(3) LM Studio を上げたときに未確証の分岐（コメント行出力）へ落ちるか。**U12 は一部済**（この 3 点が残る）。詳細な経緯は §6.9 |
| U13 | ターミナルのコピー & ペーストの実機検証 | **2026-07-31、macOS で 1 回目を実施**（下記はすべてスクリーンショットで確認済み）。(1) **ペインをまたいだコピー & ペースト**: 1 枚目のペインでファイル名を範囲選択 → Cmd+C → 2 枚目の zsh ペインで Cmd+V し、同じ文字列が入ることを確認。(2) **右クリックメニューの 2 状態**: 選択があるときは「コピー ⌘C」「貼り付け ⌘V」がどちらも有効、**選択が無いときはコピーが無効表示**になり、ツールチップに「選択範囲がありません — ドラッグで選択してください / TUI がマウスを使っている間は macOS なら Option ドラッグ、それ以外は Shift ＋ドラッグ」が出ることを確認。**まだ確認していないことが 4 点**: (1) TUI（Claude Code や vim）がマウスレポートを有効にしている状態での Option ドラッグ選択、(2) 複数行の貼り付けが bracketed paste 対応シェルで Enter を押すまで実行されないこと、(3) Linux / Windows の Ctrl+Shift+C / Ctrl+Shift+V（U7 / U8 の範囲）、(4) macOS のメニューバーに Edit メニューが実際に出ていること（貼り付けが動いた以上は出ている可能性が高いが、**目視の記録は無い**ので未確認扱い）。**U13 は一部済**（この 4 点が残る）。詳細な経緯は §6.10 |

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
- 実走で分かったことは CONTRACT.md の続報として追記し、§6 に日付つきで残す。

### P2. 未タグ成果のリリース（次のタグ）— 完了

2026-07-30、`v0.5.7` としてリリース済み（詳細は §4・§6.8）。以降の次の作業は P3 から。

### P3. retry 枯渇時の外部通知経路（escalation）

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
  文脈の読み直しコストを別途測ってから改めて判断してもよい。spec は `docs/spec/` に起こす（本文書は
  ファイル名を決め打ちしない）。実装は v0.5.9 以降。

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
- **2 体レビューの「突き合わせ」を設定で書けない（cross-model review）**: 「実装 → 別モデル 2 体が
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

### タグ実績（12 本）

実タグは `v0.4.2`〜`v0.4.9` / `v0.5.0` / `v0.5.1` / `v0.5.6` / `v0.5.7` の **12 本**。**`v0.5.2`〜`v0.5.5`
は存在しない**（`v0.5.6` のタグメッセージが Phase 5.0.2〜5.0.5 用に予約と宣言したまま実装が別の順序で
進んだため）。次のタグは `v0.5.8`（未作成 → 後述）。`v0.5.7` と `v0.5.8` を続けて Phase 5.0 系に
充てたため、[spec-phase5-5.md](../spec/spec-phase5-5.md) §9 の「バージョン割り当て」表の予約は
**2 度繰り下がり**、現在は 5.5.1 = `v0.5.9` / 5.5.2 = `v0.5.10` / 5.5.3 = `v0.5.11` /
5.5.4 = `v0.5.12` である（同 spec 側で対応済み。以前の対応表はもう有効でない）。作成日は
v0.4.2〜v0.4.6 が 2026-07-16〜17、v0.4.7〜v0.4.9 が 2026-07-18、v0.5.0 / v0.5.1 / v0.5.6 が
2026-07-23、`v0.5.7` が 2026-07-30。

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

### 次のタグ: v0.5.8（未作成）

`v0.5.8` は spec-phase5-5.md §9 の予約では Phase 5.5.1（OTel + SQLite シンク）だったが、**`v0.5.7`
のときと同じ判断（タグ順と時系列を一致させる = 先に完成した成果へ先の番号を与える）を通し**、下記の
内容に割り当てる。5.5.1〜5.5.4 は `v0.5.9`〜`v0.5.12` へもう 1 つ繰り下げた（同 spec 側で対応済み）。
なお項目 2 は Phase 5.5.1 の前段そのもの（5.5.1 の `observability.rs` が読む値を先に揃える）なので、
繰り下げても 5.5.1 の着手は遅くならない。

**実装項目（この順序で進める）**。順序の根拠は「決定論的なものから」「安いものから」「あとの判断を
分岐させるものを先に」:

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
5. **（次はここ）U4 の消化**（同名 workflow の並行 run が互いの返信を取りこぼさないこと）。2 つの構成を
   同時に流して比較する場合の前提になる。
6. **リリース雑務 — version ファイルの食い違いの記録**: タグ `v0.5.7` が指すコミット（`4e9afb3`）
   では 3 つの version ファイルが `0.5.6` のままで、`0.5.7` に上げたリリースコミット（`27ecd90`）は
   ブランチ側に残っていた。**公開済みタグは動かさない**方針とし、`v0.5.8` は version を揃えた
   コミットに打つ。（`.gitignore` への `src-tauri/target-basemain/`（2.2GB）と `ptygrid.yml-20260729`
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

**タグの内容には数えないもの**: 実タスクでのベースライン測定と改良構成の比較、`onEach: reply` /
`mode: serve` の spec 執筆（どちらも §3 P5。順序は項目 4 の実測で `onEach: reply` 先行に決まった。
実装は v0.5.9 以降）。

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

---

## 7. 運用メモ

- 各リリースは `docs/inside/phase3.md` の規律を踏襲する: CONTRACT 先行追記、`lib.rs` / hot path に
  新ロジックを置かない、unit + integration テスト、両プラットフォーム CI 通過、userguide 更新。
- 本文書は Phase の完了・計画変更のたびに **§1 の通し進捗表と §3 の次の作業**を更新する。§6 の
  進捗記録は追記のみで、過去の記録は書き換えない。
- 推測を断定で書かない。実測していないものは「未実測」「判定不能」と明記する（例: U9 / U10）。
