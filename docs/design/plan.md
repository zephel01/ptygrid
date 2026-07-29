# ptygrid 作業計画 (plan.md)

更新日: 2026-07-29 / 実装基準: `main` = `4c02cbb`（PR #3 マージ済み / 2026-07-29 01:35 +0900）。
最新タグは `v0.5.6` で、そこから **24 コミットが未タグ**。`package.json` / `src-tauri/Cargo.toml` /
`src-tauri/tauri.conf.json` の 3 ファイルはいずれも `0.5.6` で揃っている。

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
| （UX トラック） | Phase 4 期に計画外で入った UX 改善: `mterm.yml` → `ptygrid.yml` リネームと用途別サンプル（`da40cb0`）、一括 cd（`cf42ced` / `77d0271`）、作業フォルダと設定探索の分離 + origin バッジ（`acbed94`）、設定なしフォールバック（`0530e3b`）、フォルダサジェスト（`a3a769a`）、終了ペインの明示と一括クローズ（`d8a3d8e`） | ✅ | v0.4.2 | 記録なし |
| （安定化） | docs/inside のバグ / セキュリティ調査への対応: backend 純バグ 12 件（`c6f31ad`）、frontend 純バグ 8 件（`7505bbe`）、S1 Queen `/mcp` の token + Host/Origin 認証（`3159263`）、S2/S4 autostart 信頼境界 + CSP（`f18bae6`）、手打ち claude の lead 帰属修正（`9c4ab67`）、認証トークン永続化（`0af8de4`） | ✅ | v0.4.3 | 記録なし |
| 5.0.0 | MVO: `workflows:` スキーマ + 検証 / `orchestrator.rs`（spawn + DAG 進行ドライバ、fail-fast、fan-out fresh-spawn）/ Queen MCP tools 22 本 / WorkflowPanel + 🔀 チップ / `close_on_exit`・`autoClose` | 🚧 | v0.5.0（`b1b4f1f`） | config 読み込みとチップ表示のみ / 実走は未（U1） |
| 5.0.1 | Workflow Resume: `workflow_runs` 永続化（`user_version` 2→3）+ write-through、`workflow-resume-pending` イベント + Y/N バナー、`resume_workflow` / `abandon_workflow` | 🚧 | v0.5.1（`ac1b94b`） | 未（U5） |
| 5.0.2 | `ptygrid init`: `ptygrid.yml` の自動生成（環境検出 → テンプレート生成 → 自己検査 → 既存ファイルがある場合は sidecar で差分提示）。backend（`init.rs` + Tauri command 3 本）と frontend（`InitPanel.svelte` + 入口 2 つ + i18n）が実装済みで自動テストは通過。2026-07-30、macOS 実機で全経路を確認済み（→ §2 U11）。spec: [spec-init-5.0.2.md](../spec/spec-init-5.0.2.md)（→ 脚注※） | ✅ | 未タグ | 済（U11、2026-07-30） |
| 5.0.3 | Queen MCP 登録の代行（バッジからコピペしている `claude mcp add` 等を ptygrid が代行。claude は CLI を代行実行、codex / grok は TOML を `toml_edit` で値単位編集。差分承認・冪等・登録解除を含む）。spec: [spec-registration-5.0.3.md](../spec/spec-registration-5.0.3.md)（→ 脚注※） | ⬜ | — | 該当なし |
| 5.0.4 | Orchestrator 実行層: `joinOn: reply` 完了判定 / `condition:` 評価 / `handoffTo` チェイン / `retry:` 再試行 / `timeoutMs` 強制 / supervisor・handoff の spawn ゲート撤去。続けて `fanOut` 黙殺による false green の解消と straggler（`any` / `N` join の敗者）協調キャンセル | 🚧 | 未タグ（`5d3c1b5` → `3bd9833` = PR #1、`52de433` = PR #3 に同梱） | 未（U1 / U2） |
| 5.0.4 追補 | Orchestrator ハードニング: pane 上限の待ち行列化 / driver tick 軽量化（`session_states()`・registry evict）/ inbox mailbox の run 単位分離。wire 契約は無変更 | 🚧 | 未タグ（`2dc5e40`、PR #3 = `4c02cbb`） | 未（U3 / U4） |
| 5.0.5 | **Arena view**（`arena.rs` + `Arena.svelte`、`arena-open` イベント、`arena.vote` / `arena.list_votes`）。`arena: true` は現状パースだけ通り、書いても何も開かない | ⬜ | — | 該当なし |
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

**いま特に効く読み方**: ⬜ の多さより、**🚧 が 7 行あることのほうが重要**。5.0.0 / 5.0.1 / 5.0.4 /
5.0.4 追補 の 4 行はいずれも同じ理由（実機で workflow を 1 本も流していない = U1）で 🚧 に留まり、
5.0.4 以降のコードはすべてその上に積まれている。workflow 系は「動くはずのコード」のまま 4 断面ぶん
積み上がった状態であり、新機能（⬜ 群）より先に U1 を消すのが効く（→ §3 P1）。未タグ成果が
24 コミット溜まっている点も同根で、P1 を通してからタグを打つ（→ §3 P2、§4）。

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
| U1 | **実機での workflow 1 本流し**（GUI の 🔀 チップ または Queen tool `spawn_workflow` から実走し、ペインの挙動を目視） | **5.0.4 以来ずっと未実施**。CONTRACT.md 続報8〜続報10 がいずれも「解除されないもの」として同じ内容を記録している。ただし続報8 は macOS 実機での QA を**部分的に**実施したとも書いており、「完全にゼロ」ではなく「1 本流し切った実績が無い」が正確。→ §3 P1 |
| U2 | straggler 協調キャンセルの pane kill（fan-out レースの敗者ペインが GUI 上で実際に閉じること） | 続報9 が名指しで「目視確認未了」。U1 の後続 |
| U3 | pane 上限（9 面）到達時の待ち行列化が実機で `Pending` のまま待ち、空きで再開すること（`error` に `"waiting for a free pane slot (N/9 occupied)"`） | `2dc5e40` の中核挙動。unit テストのみ |
| U4 | 同名 workflow の並行 run が互いの返信を取りこぼさないこと（mailbox の run 単位分離） | 同上 |
| U5 | クラッシュ / 再起動後の resume Y/N バナー（5.0.1） | 5.0.1 完了時点から「継続」のまま |
| U6 | host モード（Phase 4.2）の Claude Code 実機検証（spec-claude-teams-panes §10.3 の手順） | 実装は入っているが実機手順は未消化。macOS 必須 / Linux はベストエフォート |
| U7 | Linux 実機での常用 | build / `.deb` / AppImage は Ubuntu 22.04 CI で検証済み（Phase 3.9）。実機常用は beta 表記のまま |
| U8 | Windows | [porting.md](porting.md) の「Windows 対応チェックリスト」が全項目未着手。`process_name()` が `None` を返すため foreground 名解決 / agent-status / ssh 接続先表示が機能しない |
| U9 | frontend チェック（`svelte-check` / `npm run build`） | 本作業環境に `node_modules` が無く**未実測**。`src/` は v0.5.1 の `ac1b94b` 以降変更されておらず `v0.5.6..main` の diff も 0 件なので v0.5.1 時点の「0 errors」から変わっていない**はず**だが、これは推測であって実測ではない |
| U10 | 5.5.0（RC 互換ルータ）の実機検証 | **記録が無く判定不能**。CONTRACT.md の実装状況節も自動テスト（unit 35 + 統合 14）しか挙げていない。実機で RC / legacy 双方のクライアントを繋いだ記録は見当たらない |
| U11 | `ptygrid init`（5.0.2）の実機検証 | **2026-07-30、macOS で実施**（すべてスクリーンショットで確認済み）。(1) 設定の無いフォルダで起動→シェル 1 枚→「設定を作る」ボタンが出て、検出結果（opencode/claude/codex/gemini/qwen/grok/aider の 7 体・npm・git あり・ローカル LLM ルータ未検出・既存設定なし）が実環境と一致することを確認、(2) 通常生成で `ptygrid.yml`（2,060 バイト）が生成され agents チップ 7 体が並び、生成物は autostart 全 false のため trust プロンプトは出ずペインも自動起動しないことを確認、(4) 既存設定ありの状態では副入口の書き込み先が `ptygrid.init.yml` に切り替わり、書き込み後も既存 `ptygrid.yml` は mtime・内容とも無変更であることを確認（上書き禁止の実測裏付け）、(5) 書き込み直後に init 自身の通知と watcher `config-changed` による再読み込みトーストが二重に出る競合を実測（spec §9 で推測としていた箇所が確認され、直後に自己書き込みエコー抑制（`ui.selfWrite` + 3 秒窓）を別コミットで修正済み）。(3) プレビューを手編集して `autostart: true` にしてから書き込むと**今度は trust プロンプトが出て**、「信頼して起動」で当該エージェントが実際に起動することを確認（`init_write` → `loadConfig` → `maybeAutostart` の順序の実証）。**U11 は完了**。Global 選択時の `~/.ptygrid/` 作成のみ今回の範囲外（必要になった時点で確認する）。詳細な経緯は §6.4 |

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
  pane kill（U2）もここで消す。
- 実走で分かったことは CONTRACT.md の続報として追記し、§6 に日付つきで残す。

### P2. 未タグ成果のリリース（次のタグ）

**なぜ今それか**: 未タグの成果が 24 コミット積み上がっており、これ以上増やすと「どのバイナリに
何が入っているか」が追えなくなるから。「1 リリース = 1 patch」という規律（§4）に照らして負債が大きい。

- ただし P1 の実機確認を経ずにタグを打つと「未検証のものをリリースした」記録が残るので、**P1 の後**。
- タグ番号・前提条件・手順はすべて §4 に書いてある。**打つかどうかを決めるのはユーザー**。
- `svelte-check` / `npm run build` を実測してから（U9）3 ファイルの version を同期する。

### P3. retry 枯渇時の外部通知経路（escalation）

**なぜ今それか**: 4.4.2 の通知基盤が既にあるので**配線するだけ**で済み、労力に対して自主運用の
安全性の伸びが大きいから。

**根拠**: [ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md) §1 の実装マトリクスで ❌ のまま
残っているのは 2 行だけで、その 1 つが `escalation`（もう 1 つは `arena: true` で、こちらは P5 の
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
  frontend（`InitPanel.svelte` + 入口 2 つ + i18n）を実装済み。残りは実機での操作確認（U11）のみ。
- 5.0.3（Queen MCP 登録の代行）は spec のみ（[spec-registration-5.0.3.md](../spec/spec-registration-5.0.3.md)）
  で実装は未着手。claude は CLI を代行実行、codex / grok は `toml_edit` で値単位編集し、差分承認・冪等・
  登録解除までを含む。**着手前の gate として「docs と実装の食い違いの確定」がある**（README / userguide
  は grok を CLI と案内しているが、実装は codex と同一の TOML を出している）。
- **5.0.3 の着手は P1（実機での workflow 1 本流し）より前には置かない**: 入口だけ自動化しても、その先の
  workflow が実走未確認のままでは効果が薄いため。5.0.2 の残作業（U11）は軽量なので先に消してよい。

### P5. 未着手フェーズ（着手順の案）

**なぜ今それか**: P1〜P4 で足元が固まるまでは着手しない。以下は「固まったあとの順番」の案。

| 順 | 内容 | 根拠 |
|---|---|---|
| 1 | **5.5.1 OTel 計装 + SQLite シンク** → **5.5.2 Cost 計算 + `agent-cost`** | 5.5.0 で RC ルータと `_meta.traceparent` の受け口だけ作って**エクスポート先が無い**（span を落としているだけ）。半端な状態を先に閉じる。バックエンド完結で UI 変更が要らず、P1/P2 と衝突しにくい |
| 2 | **5.0.5 Arena view** | fan-out + `joinOn: any` の straggler キャンセルが Arena の前提。spec-phase5-0 §2.4 が要求する「敗者が自動 CANCELLED」は 5.0.4 で満たされているので、いま作れば既存基盤の上に乗る |
| 3 | **Memory + Provider** | ptygrid 単体で完結せず、embedding backend（Ollama / LM Studio 等）と `sqlite-vec` の配布方式が未決（spec-phase5-0 §10）。外部依存が最も重い。5.0.6 以降に付け直す（§1 の脚注※） |
| 4 | **5.5.3 Agent Status Rings / 5.5.4 Trace Waterfall + Cost Dashboard** | どちらも frontend 中心で、5.5.1/5.5.2 のデータが無いと表示するものが無い。順序として後ろ |
| 5 | **Phase 6.0 Security（6.0.0〜6.0.5）** | `user_version` 4 の 3 テーブル同時導入を伴い、`session.rs`（PTY hot path）に tee tap を入れる最も侵襲的な変更。§5.2 の規律どおり人手レビュー枠が要る。macOS/Linux の sandbox 実装差も大きい |

### P6. Windows 移植 / Linux 実機検証の継続

**なぜ今それか**: 優先度は低いが、beta 表記を外す前提条件なので落とさずに持っておく（U7 / U8）。

- Windows: 最優先は `process_name()` の Windows 実装。次いで `/bin/cat`・`/bin/sh` に依存する既存
  テストの `#[cfg]` 分岐と Windows CI（詳細は [porting.md](porting.md)）。Linux は実機常用を継続。

### 継続ウォッチ / バックログ

いずれも「優先度は P1〜P6 より下だが忘れると困る」もの。完了・失効した項目はここから削除し、
実績は §1 の表と §4 のタグ表に残す。

- **`arena: true` が実装を伴わない**: 誤解を招くので、Arena 実装（P5）までの間は
  [ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md) §1 の ❌ 表記を維持する
- **`orchestrator.rs` のコード内コメントの「phase 5.0.5」表記**: 整理コミットで削除する
  （5.0.5 は Arena view 用に予約済みで、採番の食い違い自体は §1 の脚注※で決着済み）
- **`src-tauri/src/orchestrator.rs.bak` が git に追跡されたまま**: live source ではないが、ガイド §1 が
  「commit 済みの `.bak` に旧コードが残るが実行系とは無関係」と注記せざるを得ない。次の整理コミットで削除
- **spec-phase5-5.md §9 の「バージョン割り当て」表が未修正**: §4 の次タグ案 (a) を採る場合に
  繰り下げが要る（数字と得失は §4）。現状は 2 資料が違うことを言っている状態
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

### タグ実績（11 本）

実タグは `v0.4.2`〜`v0.4.9` / `v0.5.0` / `v0.5.1` / `v0.5.6` の **11 本**。**`v0.5.2`〜`v0.5.5` は存在
しない**（`v0.5.6` のタグメッセージが Phase 5.0.2〜5.0.5 用に予約と宣言したまま実装が別の順序で
進んだため）。作成日は v0.4.2〜v0.4.6 が 2026-07-16〜17、v0.4.7〜v0.4.9 が 2026-07-18、
v0.5.0 / v0.5.1 / v0.5.6 が 2026-07-23。

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

> **v0.5.6 タグメッセージの注意**: 同メッセージは「untagged 5.0.2 Workflow Reliability
> (retry/timeoutMs/joinOn:reply/escalation) と integrator エージェントを同梱」と書いているが、
> `v0.5.1..v0.5.6` の 9 コミットはすべて 5.5.0（`queen_compat`）関連で、`retry:` / `condition:` /
> `handoffTo` のスキーマが `config.rs` に入るのは v0.5.6 **より後**の `5d3c1b5` である（v0.5.6 断面に
> あるのは 5.0.0 由来の `timeout_ms` のみ）。integrator エージェントは gitignore 対象の
> `ptygrid.yml` 側の定義でありタグの内容としては追跡できない。

### v0.5.6 以降の未タグ成果（`main` = `4c02cbb`）

`v0.5.6..main` は 24 コミット（46 ファイル / +7,949 −729）+ 本ブランチの 5.0.2 実装。内訳:

| コミット | 経路 | 内容 |
|---|---|---|
| `5d3c1b5` + `3bd9833` | PR #1（`4981ba6`） | 5.0.4 スキーマ + 実行層本体 |
| `dd7a135` | PR #2（`784f7e9`） | docs を spec / guide / design に再編、3 spec の公開 |
| `52de433` | PR #3 に同梱 | 5.0.4 実行層の仕上げ: `fanOut` 黙殺による false green の解消（`fan-out` 以外での `fanOut` 宣言を load 時に拒否、`joinOn: N` を実効コピー数で検査）+ straggler cancellation。CONTRACT.md 続報9 |
| `2dc5e40` | PR #3（`4c02cbb`） | Orchestrator ハードニング。CONTRACT.md 続報10、設計は [refactor-pane-cap-5.0.5.md](refactor-pane-cap-5.0.5.md)、経緯は §6 |
| `3883128` / `d3eac32` | `main` 直コミット | docs の公開/内部分離 / MIT license 宣言 |
| 未確定（本ブランチ） | 未マージ | 5.0.2 `ptygrid init`: backend `init.rs`（1402 行）+ Tauri command 3 本 + frontend `InitPanel.svelte`（858 行）+ 入口 2 つ + i18n 53 キー。lib テスト 375→400、統合 14 は不変、clippy は既存の `config.rs` 1 件のみ、`npm run check`/`build` 成功。CONTRACT.md に「Phase 5.0.2 追加契約」を追記済み |

### 次タグの前提（打つ前に潰すもの）

- **`cargo clippy --all-targets --all-features -- -D warnings` が 1 件で落ちる**:
  `config.rs:834` の `nonminimal_bool`（`joinOn: reply` の kickoff 必須チェック）。`git log -S` で
  追うと 5.0.4 実行層の `3bd9833` で入ったもので、ハードニング（`52de433` / `2dc5e40`）が持ち込んだ
  新規警告ではない。**CI（`.github/workflows/ci.yml`）は `cargo clippy --all-targets --locked` を
  `-D warnings` なしで実行し `RUSTFLAGS` の deny も無いため、この 1 件で main が赤くなることはない**。
  落ちるのは README「開発時のチェック」と本文書の規律どおり手元で `-D warnings` を付けたときだけ。
  修正案（`is_none_or`）は Rust 1.82+ を要求するので、直すなら最低対応 toolchain の確認とセットで。
- **自動テスト実測（2026-07-29、`main` = `4c02cbb` 相当の作業ツリー）**: lib **375 passed** /
  統合 `queen_compat_integration` **14 passed** / `teams-backend` **30 passed**（18 + 8 + 4）。
  ただし**並列実行時に `git_service` のテスト（`reads_status_and_diff_from_a_real_repository`）が
  1 件 flaky に落ちることがある**（単体実行では pass）。regression ではなくテスト間の環境競合と
  見られるが、「375 / 0」は実行条件つきの値であり無条件の事実ではない。
- **frontend チェックは未実測**（U9）。3 ファイルの version 同期の前に実測すること。

そのうえで、未タグ成果をまとめて 1 本のリリースにすることを提案する。**打つかどうか・どの番号に
するかを決めるのはユーザー**であり、本文書は選択肢と得失を並べるところまで。番号は未確定で 2 案:

- **(a) `v0.5.7` を割り当てる**。連番が単調増加し `git tag` の並びと時系列が一致する。ただし
  [spec-phase5-5.md](../spec/spec-phase5-5.md) §9 の「バージョン割り当て」表と v0.5.6 のタグメッセージが
  **`v0.5.7` = Phase 5.5.1（OTel + SQLite）を予約している**ため、5.5.1〜5.5.4 を 1 つずつ繰り下げる
  （`v0.5.8`〜`v0.5.11`）文書修正が要る。
- **(b) 予約どおり `v0.5.4` を後追いで打つ**（`v0.5.2`〜`v0.5.5` = Phase 5.0.2〜5.0.5 の予約を守る）。
  予約表は無傷だが、`v0.5.6` より後に `v0.5.4` を作ることになり、タグ順と時系列が逆転する。

文書としての推しは **(a)**（タグ順と時系列が一致するほうが後から追いやすく、予約表の付け替えは
ドキュメント修正だけで済むため）。どちらを採るにせよ、**§3 P1 を通してから打つ**ことを提案する。

### リリース手順（タグ付けの作法）

1. `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` の `version` を一致させて
   更新（`Cargo.lock` は `cargo check` で追従）
2. 全チェック（`cargo test` / `clippy` / `npm run check` / `npm run build`）通過を確認
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
> 着手時に決め直す必要がある（§3 P5）。

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

---

## 7. 運用メモ

- 各リリースは `docs/inside/phase3.md` の規律を踏襲する: CONTRACT 先行追記、`lib.rs` / hot path に
  新ロジックを置かない、unit + integration テスト、両プラットフォーム CI 通過、userguide 更新。
- 本文書は Phase の完了・計画変更のたびに **§1 の通し進捗表と §3 の次の作業**を更新する。§6 の
  進捗記録は追記のみで、過去の記録は書き換えない。
- 推測を断定で書かない。実測していないものは「未実測」「判定不能」と明記する（例: U9 / U10）。
