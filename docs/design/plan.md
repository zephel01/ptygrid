# ptygrid 作業計画 (plan.md)

更新日: 2026-07-29 / 実装基準: **`v0.5.6` + 未タグ 2 件**（5.0.4 Orchestrator 実行層 `52de433`、
同ハードニング `2dc5e40`）。`main` = `4c02cbb`（PR #3 マージ済み / 2026-07-29 01:35 +0900）。

この文書は「いま何が終わっていて、次に何をやるか」と「バージョンの付け方」を 1 か所にまとめる
作業計画である。**現在地だけ知りたいなら §1 の「ひとことサマリ」と「Phase 5 系 実装状況」で足りる。**
Phase 3.x の詳細な実績とリリース規律は `docs/inside/phase3.md`（git 管理外の内部資料）、
teams 機能の設計は [spec-claude-teams-panes.md](../spec/spec-claude-teams-panes.md)、方向性の背景は
[competitive-landscape.md](competitive-landscape.md) を参照。

---

## 1. 現在地サマリ

### ひとことサマリ

- **バージョン**: 最新タグは `v0.5.6`（= Phase 5.5.0、2026-07-23）。`main` = `4c02cbb` はそこから
  **24 コミット先**にあり、`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json`
  の 3 ファイルはいずれも `0.5.6` で揃っている。
- **直近で入ったもの**: Phase 5.0.4 の Orchestrator 実行層（`joinOn: reply` / `condition:` /
  `handoffTo` / `retry:` / `timeoutMs` + straggler 協調キャンセル、`52de433`）と、その非機能
  ハードニング（pane 上限の待ち行列化・driver tick 軽量化・mailbox の run 単位分離、`2dc5e40`）。
  **どちらも未タグ**。
- **次にやるべきこと**: 実機で workflow を 1 本流す（§2 P1）→ 未タグ成果にタグを打つ（§2 P2）。
- **最大のリスク**: 自動テストは lib **375** + 統合 **14** が green なのに、**GUI / Queen から
  `spawn_workflow` して最後まで回した実績が 5.0.4 以来ゼロ**。実行層もハードニングも、現時点では
  「動くはずのコード」でしかない（→「未検証事項」）。
- **未着手**: Memory / Provider / Arena UI / OTel / Phase 6.0 Security は spec だけがあり、
  対応するソースが存在しない。

### Phase 5 系 実装状況

> 各断面の詳しい実装記録・検証範囲・既知ギャップは **§5.4〜§5.6**（進捗記録・履歴）と
> CONTRACT.md「Phase 5.0 追加契約」の続報7〜続報10 にある。本表は要約であり、食い違ったときは
> CONTRACT.md（wire 契約）と §5 の日付つき記録を正とする。

| patch | 内容 | 状態 | タグ / コミット | 実機検証 |
|---|---|---|---|---|
| 5.0.0 | MVO: `workflows:` スキーマ + 検証 / `orchestrator.rs`（spawn + DAG 進行ドライバ、fail-fast、fan-out fresh-spawn）/ Queen MCP tools 22 本 / WorkflowPanel + 🔀 チップ / `close_on_exit`・`autoClose` | ✅ 実装済み | `v0.5.0`（`b1b4f1f`） | ⚠️ config 読み込みとチップ表示のみ。**workflow 実走は未** |
| 5.0.1 | Workflow Resume: `workflow_runs` 永続化（`user_version` 2→3）+ write-through、`workflow-resume-pending` イベント + Y/N バナー、`resume_workflow` / `abandon_workflow` | ✅ 実装済み | `v0.5.1`（`ac1b94b`） | ❌ クラッシュ→再開の実機テストは未実施（§5.5 の「継続」がそのまま） |
| 5.0.2 | **欠番／未確定**。指すものが資料間で割れている（spec-phase5-0 §9 = Memory embedding、CONTRACT.md「Phase 5.0 追加契約」冒頭 = Memory embedding、`v0.5.6` タグメッセージ = Workflow Reliability）。実装としては何も消化していない | ⬜ 未着手（採番の要整理） | — | — |
| 5.0.3 | **欠番／未確定**。同様に割れている（spec-phase5-0 §9 = Orchestrator pipeline + supervisor、CONTRACT.md = Provider）。実装としては未消化 | ⬜ 未着手（採番の要整理） | — | — |
| 5.0.4 | Orchestrator 実行層: `joinOn: reply` 完了判定 / `condition:` 評価 / `handoffTo` チェイン / `retry:` 再試行 / `timeoutMs` 強制 / supervisor・handoff の spawn ゲート撤去。続けて `fanOut` 黙殺による false green の解消と straggler（`any` / `N` join の敗者）協調キャンセル | ✅ 実装済み | **未タグ**（スキーマ `5d3c1b5` → 実行層 `3bd9833` = PR #1、仕上げ `52de433` = PR #3 に同梱） | ❌ **未実施（最大の穴）** |
| 5.0.4 追補 | Orchestrator ハードニング（今日の作業）: pane 上限の待ち行列化 / driver tick 軽量化（`session_states()`・registry evict）/ inbox mailbox の run 単位分離。wire 契約は無変更。詳細は §5.6 | ✅ 実装済み | **未タグ**（`2dc5e40`、PR #3 = `4c02cbb`） | ❌ 未実施 |
| 5.0.5 | **Arena view**（`arena.rs` + `Arena.svelte`、`arena-open` イベント、`arena.vote` / `arena.list_votes`）。`arena: true` は**パースだけ通り、書いても何も開かない** | ⬜ 未着手（`arena.rs` が存在しない） | — | — |
| 5.5.0 | MCP 2026-07-28 RC 互換ルータ（`queen_compat`: header / route / capabilities / deprecation / initialize / meta、hot-swap 可能な `McpCompatHandle`、legacy 2025-06 併存） | ✅ 実装済み | `v0.5.6`（`21d1367`） | ❓ 記録なし。CONTRACT.md の実装状況節は自動テスト（unit 35 + 統合 14）しか挙げていない |
| 5.5.1 | OTel GenAI 計装 + SQLite シンク（span の書き出し先） | ⬜ 未着手（`observability.rs` が存在しない） | — | — |
| 5.5.2 | Cost 計算 + `agent-cost` イベント | ⬜ 未着手 | — | — |
| 5.5.3 | Agent Status Rings（通知リング / 要承認ハイライト） | ⬜ 未着手 | — | — |
| 5.5.4 | Trace Waterfall + Cost Dashboard | ⬜ 未着手 | — | — |
| 6.0.x | Security: Sandbox / Secrets / Replay（`user_version` 4 の 3 テーブル同時導入） | ⬜ 未着手（`secrets.rs` / `sandbox.rs` / `replay.rs` が存在しない） | — | — |

### 未検証事項（いまの最大リスク）

自動テストは green だが、**下記はいずれも「一度も実機で動かしていない」**。5.0.4 以降のコードは
すべてこの上に積まれているので、ここが今のプロジェクトで一番大きい未知である。

| # | 未検証の内容 | 状況 |
|---|---|---|
| U1 | **実機での workflow 1 本流し**（GUI の 🔀 チップ または Queen tool `spawn_workflow` から実走し、ペインの挙動を目視） | **5.0.4 以来ずっと未実施**。CONTRACT.md 続報8 / 続報9 / 続報10 がいずれも「解除されないもの」として同じ内容を記録している。§2 P1 |
| U2 | straggler 協調キャンセルの pane kill（fan-out レースの敗者ペインが GUI 上で実際に閉じること） | 続報9 が名指しで「目視確認未了」。U1 の後続 |
| U3 | pane 上限（9 面）到達時の待ち行列化が実機で `Pending` のまま待ち、空きで再開すること | `2dc5e40` の中核挙動。unit テストのみ |
| U4 | 同名 workflow の並行 run が互いの返信を取りこぼさないこと（mailbox の run 単位分離） | 同上 |
| U5 | クラッシュ / 再起動後の resume Y/N バナー（5.0.1） | §5.5 の時点から「継続」のまま |
| U6 | host モード（Phase 4.2）の Claude Code 実機検証（spec-claude-teams-panes §10.3 の手順） | 実装は完了しているが実機手順は未消化。macOS 必須 / Linux はベストエフォート |
| U7 | Linux 実機での常用 | build / `.deb` / AppImage は Ubuntu 22.04 CI で検証済み（Phase 3.9）。実機常用は beta 表記のまま |
| U8 | Windows | [porting.md](porting.md) の「Windows 対応チェックリスト」が全項目未着手。`process_name()` が `None` を返すため foreground 名解決 / agent-status / ssh 接続先表示が機能しない |
| U9 | frontend チェック（`svelte-check` / `npm run build`） | 本作業環境に `node_modules` が無く未実測。`src/` は v0.5.1 の `ac1b94b` 以降変更されておらず、`v0.5.6..main` の diff も 0 件なので状態は変わっていないはずだが、**実測はしていない** |

### 完了済み Phase（0〜4.x）

ここは畳んである。各リリースに何が入ったかは §3「直近のバージョン割り当て」、Phase 3.x の詳細は
`docs/inside/phase3.md`（git 管理外）、teams 系の設計は
[spec-claude-teams-panes.md](../spec/spec-claude-teams-panes.md) を参照。

| Phase | 内容 | リリース |
|---|---|---|
| 0 | 単一 PTY ペイン | — |
| 1 | マルチペイン + config-as-code（現 `ptygrid.yml`）、autostart / restart | — |
| 2〜2.1 | Queen（内蔵 MCP サーバー、基本 5 tools）+ ドッグフーディング反映 | — |
| 3.0〜3.8 | Git status/diff/stage/commit、opt-in worktree 分離、logical resume、リソース監視、Queen pins/notes/inbox/reply/await（18 tools） | — |
| 3.9 | Linux テスト対応（PATH 復元、Ubuntu CI、`.deb` / AppImage） | — |
| 4.0 | teammate hooks 受信基盤（`/hooks/v1/*`、token 認可、toast、Teammates バッジ、`teammates:` ブロック、settings.json 半自動登録） | v0.4.2 |
| 4.1 | observe: `transcript` ペイン種別（PTY なし論理セッション）、SubagentStart で read-only tail 自動生成、`agents[].teams` | v0.4.2 |
| 4.2 | host: tmux 互換シム + per-lead Unix socket RPC 配線、env/PATH 注入、実 PTY teammate ペイン、フォールバック検知→observe 降格、frontend 一式 | v0.4.2（実機検証は U6 として未消化） |
| 4.3 | Queen team preset（`team_presets:` + Queen tool `spawn_team`（19 本目）+ 👥 一括起動 UI + example/team-preset） | v0.4.6 |
| 4.4.0〜4.4.1 | エージェント意味的状態の検出（working / blocked / done / idle）+ 左ステータスサイドバー | v0.4.4 |
| 4.4.2 | アプリ外通知（セッション終了・blocked/done エッジを OS 通知 / Slack / Mattermost / Discord / Telegram へ中継、`notifications:`） | v0.4.6 に同梱 |
| 4.4.3 | ssh 接続先表示（`session-resources.foreground.detail?` を additive 追加）+ フォアグラウンド名解決の汎用化 | v0.4.8 / v0.4.9 |
| （UI 横断） | UI 多言語化 en/ja（型付き辞書 `i18n.svelte.ts` + ⚙ 設定メニュー、既定は OS 言語追従） | v0.4.7 |
| （UX トラック） | Phase 4 期に計画外で入った UX 改善: `mterm.yml` → `ptygrid.yml` リネームと用途別サンプル（`da40cb0`）、一括 cd（`cf42ced` / `77d0271`）、作業フォルダと設定探索の分離 + origin バッジ（`acbed94`）、設定なしフォールバック（`0530e3b`）、フォルダサジェスト（`a3a769a`）、終了ペインの明示と一括クローズ（`d8a3d8e`） | v0.4.2 |
| （安定化） | docs/inside のバグ / セキュリティ調査への対応: backend 純バグ 12 件（`c6f31ad`）、frontend 純バグ 8 件（`7505bbe`）、S1 Queen `/mcp` の token + Host/Origin 認証（`3159263`）、S2/S4 autostart 信頼境界 + CSP（`f18bae6`）、手打ち claude の lead 帰属修正（`9c4ab67`）、認証トークン永続化（`0af8de4`） | v0.4.3 |

Defer / Skip 判定（u32 wrap 等の理論値、稀なレース、実験機能の DoS、S3 caller-id 等）は
`docs/inside/evaluation-2026-07-16.md`（git 管理外の内部資料）に整理。

### 実装済みの基盤

- `src-tauri/teams-backend/`: CustomPaneBackend 提案（anthropics/claude-code#26572）
  準拠の JSON-RPC 2.0 ソケットサーバ + tmux 互換シム（テスト30件）。**Phase 4.2 で app 本体へ
  配線済み**（`teams_host.rs` の `PaneHost` 実装・`__tmux-compat` サブコマンド経由）。
- `src-tauri/src/orchestrator.rs`: workflow DAG ドライバ（200ms tick）。spawn / 完了検出 3 経路 /
  `condition` / `retry` / `timeout` / straggler キャンセル / pane 上限の待ち行列化 / `WorkflowRegistry`
  （終端 run は `REGISTRY_TERMINAL_CAP` = 100 で evict）。
- `src-tauri/src/queen_compat/`: MCP 2026-07-28 RC と legacy 2025-06 の両立ルータ（統合テスト
  `tests/queen_compat_integration.rs` 14 本）。
- `src-tauri/src/queen_store.rs`: SQLite 永続化。**現行 `user_version` = 3**（pins / notes /
  inbox / reply / `workflow_runs`）。v4 以上は開かずに明示エラー。
- `src-tauri/src/token_store.rs`: Queen `/mcp` と teammate hooks の Bearer トークンを
  `auth-tokens.json`（0600・atomic write）に永続化。**再起動後の再登録は不要**（v0.4.3〜）。

**未実装（ソースが存在しない）**: `memory.rs` / `memory_embed.rs`（Memory）、`provider.rs`（Local
Provider）、`arena.rs`（Arena view）、`observability.rs`（OTel）、`secrets.rs` / `sandbox.rs` /
`replay.rs`（Phase 6.0）。spec だけがある状態。

### 現時点の自動チェック実測（2026-07-29、`main` = `4c02cbb` 相当の作業ツリー）

| チェック | 結果 |
|---|---|
| `cargo test`（`src-tauri`、lib） | **375 passed** / 0 failed |
| `cargo test`（統合 `queen_compat_integration`） | **14 passed** / 0 failed |
| `cargo test`（`src-tauri/teams-backend`、独立 workspace） | **30 passed** / 0 failed（18 + 8 + 4） |
| `cargo clippy --all-targets --all-features -- -D warnings` | **失敗 1 件**: `config.rs:834` の `nonminimal_bool`（`joinOn: reply` の kickoff 必須チェック）。`git log -S` で追うと 5.0.4 実行層の `3bd9833` で入ったもので、今日のハードニング（`52de433` / `2dc5e40`）に起因する新規警告ではない（CONTRACT.md 続報10 §(5) と同じ状況が未解消）。**なお CI（`.github/workflows/ci.yml`）は `cargo clippy --all-targets --locked` を `-D warnings` なしで実行しており `RUSTFLAGS` の deny も無いため、この 1 件で CI が赤くなることはない**。落ちるのは README「開発時のチェック」と本文書の規律どおり手元で `-D warnings` を付けたときだけ。修正案（`is_none_or`）は Rust 1.82+ を要求するので、直すなら最低対応 toolchain の確認とセットで |
| `svelte-check` / `npm run build` | 未実測（本作業環境に `node_modules` が無い）。`src/`（frontend）は v0.5.1 の `ac1b94b` 以降変更されておらず、`v0.5.6..main` の diff も 0 件のため v0.5.1 時点の「0 errors」から変わっていないはずだが、**実測はしていない**（U9） |

---

## 2. 次の作業（優先順）

優先順の根拠は「**未検証のまま積み上がっている量**」→「**リリース規律の負債**」→
「**未着手の新機能**」の順。コードは既に 5.0.4 まで積んであるのに実走した実績が無いので、
新機能を足す前に足元を確定させる。

### P1. 実機での workflow 1 本流し（`smoke` workflow）— 最優先

**なぜ今それか**: 自動テストは全部通っているのに実走実績がゼロ、というギャップを埋めるのが
いちばん費用対効果が高いから（U1）。

**根拠**: 5.0.0（§5.4）以来「workflow 実走の実機確認は継続」と書き続けており、CONTRACT.md の
続報8・続報9・続報10 がいずれも「解除されないもの: 実機での workflow 1 本流しは未実施」を
繰り返し記録している。5.0.4 実行層（reply join / condition / handoffTo / retry / timeout）、
straggler 協調キャンセル、pane 上限の待ち行列化、mailbox の run 単位分離は**すべて unit テスト
だけで検証されており、実 PTY・実 GUI を通していない**。ここが通らない限り、以降の全ての
実装は「動くはずのコード」の上に積み上がる。

- 対象は `ptygrid.yml` の `smoke` workflow（`pattern: pipeline` / `autoClose: success` /
  step `a`(agent `t1`) → `b`(agent `t2`)、各 30 秒 sleep の shell）。まずこの最小構成を
  GUI の 🔀 チップまたは Queen tool `spawn_workflow {name: "smoke"}` から流し切る。
- 確認したい最小項目: (1) run が `Running` → `Succeeded` まで進む、(2) `autoClose: success`
  でペインが実際に閉じる、(3) `workflow-state` イベントで WorkflowPanel が追随する、
  (4) アプリ再起動での resume Y/N バナー（5.0.1）が実際に出る。
- そのうえで、ハードニング固有の挙動を実機で見る: 9 面を埋めた状態で step が `Failed` に
  ならず `Pending` のまま待ち、空きが出たら再開すること（`error` に
  `"waiting for a free pane slot (N/9 occupied)"` が出る）。同名 workflow の並行 run が
  互いの返信を取りこぼさないこと（mailbox の run 単位分離）。
- `joinOn: reply` / `condition:` / `handoffTo` / `retry:` を使う workflow は、`smoke` が
  通ってから別途 1 本ずつ。とくに `cancel_stragglers` の pane kill（fan-out レースの敗者
  ペインが GUI 上で閉じること）は続報9 が名指しで「目視確認未了」としている。
- 実走で分かったことは CONTRACT.md の続報として追記し、§5 に日付つきで残す。

### P2. 未タグ成果のリリース（次のタグ）

**なぜ今それか**: 未タグの成果が 24 コミット積み上がっており、これ以上増やすと
「どのバイナリに何が入っているか」が追えなくなるから。

**根拠**: `v0.5.6` 以降、`main` には 24 コミット（46 ファイル / +7,949 −729）が未タグで積んで
ある。§3 のリリース手順は「1 リリース = 1 patch」を規律としており、この量を未タグのまま
放置すると「どのバイナリに何が入っているか」が追えなくなる。ただし P1 の実機確認を経ずに
タグを打つと「未検証のものをリリースした」記録が残るので、**P1 の後**に置く。

- 前提として `cargo clippy --all-targets --all-features -- -D warnings` を green にする
  （現状 `config.rs:834` の `nonminimal_bool` 1 件で落ちる。`git log -S` で追うと 5.0.4 実行層の
  `3bd9833` 由来で、今日のハードニングが持ち込んだものではない）。**CI は `-D warnings` を
  付けていないので main は赤くなっていない**が、規律としては手元 green を先に取る。修正案の
  `is_none_or` は Rust 1.82+ 要求のため、最低対応 toolchain を決めてから入れること。
- タグ番号は未確定。候補と得失は §3「v0.5.6 以降の未タグ成果」を参照。**打つかどうかを決めるのは
  ユーザー**であり、本文書は提案までにとどめる。
- `svelte-check` / `npm run build` を実測してから（U9）3 ファイルの version を同期する。

### P3. retry 枯渇時の外部通知経路（現状 ❌）

**なぜ今それか**: 4.4.2 の通知基盤が既にあるので**配線するだけ**で済み、労力に対して
自主運用の安全性の伸びが大きいから。

**根拠**: [ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md) §1 の実装マトリクスで ❌ のまま
残っているのは 2 行だけで、その 1 つが `escalation`（retry 枯渇時の外部通知）である
（もう 1 つは `arena: true`＝ Arena UI 未実装で、こちらは §2 P4 の 5.0.5 で解消する）。5.0.4 で retry 実行系が
配線され枯渇判定は発火するようになったが、**枯渇時に外部へ通知する経路が無く、step が
`Failed` で終端し run が red になるだけ**。自主運用（[autonomous-operation-guide.md](../guide/autonomous-operation-guide.md)）
は「人間が気づく」ことに依存しており、通知が無いと夜間・離席中の失敗が滞留する。
4.4.2 のアプリ外通知（`notifications:`、OS 通知 / Slack / Mattermost / Discord / Telegram）が
既にあるので、**新しい配送機構は要らず、workflow 側のイベントを既存経路へ流すだけ**で済む
見込み。小さいわりに運用価値が大きいので、未着手フェーズより前に置く。

### P4. 未着手フェーズ（着手順の案）

**なぜ今それか**: P1〜P3 で足元が固まるまでは着手しない。以下は「固まったあとの順番」の案。

いずれも spec のみで、対応するソース（`memory.rs` / `provider.rs` / `arena.rs` /
`observability.rs` / `secrets.rs` / `sandbox.rs` / `replay.rs`）がまだ存在しない。

| 順 | 内容 | 根拠 |
|---|---|---|
| 1 | **5.5.1 OTel 計装 + SQLite シンク** → **5.5.2 Cost 計算 + `agent-cost`** | 5.5.0 で RC ルータと `_meta.traceparent` の受け口だけ作って**エクスポート先が無い**（span を落としているだけ）。半端な状態を先に閉じる。バックエンド完結で UI 変更が要らず、P1/P2 と衝突しにくい |
| 2 | **5.0.5 Arena view** | fan-out + `joinOn: any` の straggler キャンセル（5.0.4 で実装済み）が Arena の前提。spec-phase5-0 §2.4 が要求する「敗者が自動 CANCELLED」は既に満たされているので、いま作れば実装済み基盤の上に乗る |
| 3 | **Memory + Provider** | ptygrid 単体で完結せず、embedding backend（Ollama / LM Studio 等）と `sqlite-vec` の配布方式が未決（spec-phase5-0 §10）。外部依存が最も重い。**patch 番号が空いていない**（下記の注記を参照） |
| 4 | **5.5.3 Agent Status Rings / 5.5.4 Trace Waterfall + Cost Dashboard** | どちらも frontend 中心で、5.5.1/5.5.2 のデータが無いと表示するものが無い。順序として後ろ |
| 5 | **Phase 6.0 Security（Sandbox / Secrets / Replay）** | `user_version` 4 の 3 テーブル同時導入を伴い、`session.rs`（PTY hot path）に tee tap を入れる最も侵襲的な変更。§5.2 の規律どおり人手レビュー枠が要る。macOS/Linux の sandbox 実装差も大きい |

> **patch 番号の齟齬（未解決・要整理）**: Phase 5.0 の patch 採番が資料ごとに食い違っている。
> [spec-phase5-0.md](../spec/spec-phase5-0.md) §9 は「5.0.0 = Provider 統合の基盤 / 5.0.1 =
> Memory 保存経路 / 5.0.2 = Memory embedding / 5.0.3 = Orchestrator pipeline+supervisor /
> 5.0.4 = Orchestrator fan-out+handoff / 5.0.5 = Arena」、CONTRACT.md「Phase 5.0 追加契約」冒頭は
> 「5.0.0 = MVO / 5.0.1 = Memory FTS5 / 5.0.2 = Memory embedding / 5.0.3 = Provider /
> 5.0.4 = Orchestrator supervisor+handoff / 5.0.5 = Arena」と書いている。
> **実際に消化されたのは 5.0.0 = MVO、5.0.1 = Workflow Resume、5.0.4 = Orchestrator 実行層**で、
> **5.0.2 / 5.0.3 は欠番**（`v0.5.6` のタグメッセージだけが「untagged 5.0.2 Workflow Reliability」
> という第 3 の呼び方をしているが、その内容は後の 5.0.4 として着地している）。
> 結果、Memory + Provider に割り当てられる patch 番号が無い。**どう付け直すかは未確定**で、
> 着手時に「空いた 5.0.2 / 5.0.3 を再利用する」か「5.0.6 以降に付け直す」かを決める必要がある。

### P5. Windows 移植 / Linux 実機検証の継続

**なぜ今それか**: 優先度は低いが、beta 表記を外す前提条件なので落とさずに持っておく。

- Windows: [porting.md](porting.md) の「Windows 対応チェックリスト」が全項目未着手。最優先は
  `process_name()` の Windows 実装（現状 `None` を返すため foreground 名解決・agent-status・
  ssh 接続先表示がすべて機能しない）。次いで `/bin/cat`・`/bin/sh` に依存する既存テストの
  `#[cfg]` 分岐と Windows CI。
- Linux: build / `.deb` / AppImage は Ubuntu 22.04 CI で検証済み（Phase 3.9）だが、実機での
  常用検証は継続中（beta 表記のまま）。

### 継続ウォッチ / バックログ

いずれも「優先度は P1〜P5 より下だが忘れると困る」もの。完了・失効した項目はここから削除し、
実績は §1 の完了済み表と §3 のバージョン表に残す。

- **retry 枯渇時に外部へ通知する経路が無い**: step が `Failed` で終端して run が red になるだけ。
  → §2 P3 に格上げ済み。ここでは「未解決の穴」として名前だけ残す
- **`arena: true` が実装を伴わない**: パースは通るが Arena UI（`arena.rs` / `Arena.svelte`）が
  存在せず、書いても何も開かない。[ptygrid-yml-guide.md](../guide/ptygrid-yml-guide.md) §1 の
  ❌ 行。誤解を招くので、Arena 実装（§2 P4）までの間はガイドの ❌ 表記を維持する
- **`5.0.5` の採番衝突**: `orchestrator.rs` のコード内コメントが今日のハードニングを
  「phase 5.0.5」と書いているが、`5.0.5` は §5.1 / spec-phase5-0.md で Arena view 用に予約済み。
  §5.6 の結論どおり**本断面には `5.0.5` を正式採番しない**。コメント表記の削除は次の整理コミットで
- **Phase 5.0 の patch 採番そのものが資料間で割れている**（5.0.2 / 5.0.3 が欠番）: §2 P4 の注記を参照。
  Memory / Provider の着手前に決め直す必要がある
- **`src-tauri/src/orchestrator.rs.bak` が git に追跡されたまま**（`git ls-files` で確認）。
  live source ではないが、ガイド §1 が「commit 済みの `.bak` に旧コードが残るが実行系とは無関係」と
  注記せざるを得ない状態になっている。次の整理コミットで削除する
- **`cargo clippy -D warnings` が 1 件で落ちる**（`config.rs:834` の `nonminimal_bool`）。
  §2 P2 のタグ付け前提条件
- **anthropics/claude-code#26572**（CustomPaneBackend 公式化）: 採用されたら
  シム撤去 + `CLAUDE_PANE_BACKEND_SOCKET` 広告へ移行（teams-backend はそのまま使える）
- 通知リング / 要承認ハイライト（competitive-landscape の「次に取る UX」。
  4.0 の teammate permission 表示を汎用化）→ 設計は Phase **5.5.3 Agent Status Rings**
  （[spec-phase5-5.md](../spec/spec-phase5-5.md) §2.3 / §3.7）に吸収済み。§2 P4 で追う
- 残りの Defer 項目（backend M5/M8/L3/L4/L6/L7/L11 系、frontend BUG-8/10、
  security S3 caller-id・Low 群）は evaluation の推奨ロードマップに従い順次

---

## 3. バージョニング規約

当初は 3 ファイルとも `0.1.0` のままで実態とズレていたため、次の規約を導入した。
**2026-07-29 現在、`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` は
いずれも `0.5.6` で一致している**（= 最新タグと同じ。未タグ成果 2 件はまだ version に反映していない）。

### 規約（SemVer 0.y.z、1.0 まで）

- **y（minor）= Phase 番号**。Phase 4 系の間は `0.4.z`、Phase 5 系に入ったら `0.5.0` から。
  過去に当てはめると Phase 3.9 時点 ≒ `0.3.9` 相当（遡及タグは付けない）。
- **z（patch）= その Phase 内のリリース連番**。機能追加・修正の区別はしない
  （pre-1.0 の SemVer では minor が破壊的変更の単位のため、これで矛盾しない）。
- **破壊的変更**（config スキーマ・IPC/MCP 契約・保存データ）は pre-1.0 でも
  「CONTRACT.md への契約追記 + 互換パス（例: mterm.yml フォールバック）」を必須とし、
  やむを得ず互換を切る場合は y を上げて README に移行手順を書く。
- **1.0.0 の条件**: ~~License 決定~~（`d3eac32` で MIT 確定済み）、macOS 安定 + Linux beta 卒業、
  teams host（4.2）の実機安定、config スキーマ凍結。残りは実機検証系（§1 の U6 / U7）と
  スキーマ凍結の 3 点。

### 直近のバージョン割り当て

実タグは `git tag -n99` で確認できる **11 本**（`v0.4.2` / `v0.4.3` / `v0.4.4` / `v0.4.5` /
`v0.4.6` / `v0.4.7` / `v0.4.8` / `v0.4.9` / `v0.5.0` / `v0.5.1` / `v0.5.6`）。
**`v0.5.2`〜`v0.5.5` は存在しない**（`v0.5.6` のタグメッセージが Phase 5.0.2〜5.0.5 用に予約と
宣言したまま、実装が別の順序で進んだため空いている）。作成日は v0.4.2〜v0.4.6 が 2026-07-16〜17、
v0.4.7〜v0.4.9 が 2026-07-18、v0.5.0 / v0.5.1 / v0.5.6 が 2026-07-23。

| バージョン | 内容 |
|---|---|
| ~~v0.4.0 / v0.4.1~~ | 個別タグは打たず **v0.4.2 に集約** |
| v0.4.2 | Phase 4.0（hooks 受信基盤）〜 4.1（observe）〜 4.2（host モード実験）+ UXトラック一式（最初のリリースタグ） |
| **v0.4.3** | 調査対応の安定化リリース: バグ修正 20件（backend 12 / frontend 8）+ セキュリティ 4件（S1 Queen認証 / S2 trust / S4 CSP）+ 手打ち claude の lead 帰属修正 + **認証トークンの永続化**。cargo test 159 / teams-backend 30 |
| **v0.4.4** | Phase 4.4.0 / 4.4.1（エージェント意味的状態の検出・可視化）+ `QUEEN_TOKEN` の各ペイン注入と Queen バッジへの codex/grok 登録追加 + Queen 登録コマンドの冪等化（remove→add）+ 手動起動エージェント / node 起動エージェント（grok）の foreground 名解決 + フッターのサイドバー開閉トグル（右上チップ廃止）。タグメッセージは `chore: release v0.4.4` のみで、内容は `v0.4.3..v0.4.4` の 6 コミットから判定 |
| **v0.4.5** | 左ドックのタブ化と Git の統合（フロート廃止）+ ツールバーの Git ボタン撤去・状態表示のフッター集約。タグメッセージは `chore: release v0.4.5` のみで、内容は `v0.4.4..v0.4.5` の 2 コミットから判定。README のスクリーンショットはこの断面（`docs/screenshot-phase0.4.5.png`） |
| **v0.4.6** | Phase 4.3（Queen team preset: `team_presets` 宣言 + `spawn_team`（19本目）+ 👥 一括起動 UI + example/team-preset）。Phase 4.4.2（アプリ外通知 `notifications:`）もこの断面に含まれる。cargo test 210 / svelte-check 0 |
| **v0.4.7** | UI 多言語化（en/ja。型付き辞書 `i18n.svelte.ts`、⚙ 設定メニューで 自動/English/日本語 切替、既定=OS言語に自動追従・英語ベース）。フロントのみ、backend 文言・ログは対象外。svelte-check 0 / build 成功 |
| **v0.4.8** | ssh 接続先表示（Phase 4.4.3: `session-resources` の foreground に `detail?` を追加し argv から宛先抽出。ヘッダー/サイドバーに `ssh user@host`。`.ssh/config` alias・`-l` 畳み込み対応）。cargo test 214 / clippy 0 / svelte-check 0。macOS 実機確認済み |
| **v0.4.9** | エージェント CLI のフォアグラウンド名解決を汎用化（opencode 等の node / python 起動エージェントを実体名で表示、`81ade5a`）+ 接続先表示ドキュメントの Phase 4.4.3 追随（sftp / scp / mosh / telnet / kubectl / docker、`e5b72d8`）。`v0.4.8..v0.4.9` は release コミット込みで 3 コミット |
| **v0.5.0** | Phase 5.0.0 MVO: `workflows:`（pipeline / fan-out）+ DAG ドライバ + Queen 22 tools + WorkflowPanel + `close_on_exit` / `autoClose`（`0182988` + `b1b4f1f`）。cargo test 246（§5.4）。同区間にはクラウド LLM の API キー利用ドキュメント（`665ee82` / `461d2a9`）も含まれる |
| **v0.5.1** | Phase 5.0.1: クラッシュ / 再起動後の workflow resume（Y/N プロンプト）。`workflow_runs` 永続化（`user_version` 2→3）。cargo test 251（§5.5）。**frontend（`src/`）はこの断面が最後の変更** |
| **v0.5.6** | Phase 5.5.0: MCP 2026-07-28 RC 互換ルータ（`queen_compat`: header / route / capabilities / deprecation / initialize / meta、hot-swap 可能な `McpCompatHandle`、legacy 2025-06 併存）。lib 286 + 統合 14 tests / clippy `-D warnings` clean。タグメッセージは「`v0.5.2`〜`v0.5.5` は plan.md の Phase 5.0.2〜5.0.5 用に予約」と宣言している |

> **v0.5.6 タグメッセージの注意**: 同メッセージは「untagged 5.0.2 Workflow Reliability
> (retry/timeoutMs/joinOn:reply/escalation) と integrator エージェントを同梱」と書いているが、
> `v0.5.1..v0.5.6` の 9 コミットはすべて 5.5.0（`queen_compat`）関連で、`retry:` /
> `condition:` / `handoffTo` のスキーマが `config.rs` に入るのは v0.5.6 **より後**の
> `5d3c1b5`（Phase 5.0.4 として）である。v0.5.6 断面の `config.rs` にあるのは 5.0.0 由来の
> `timeout_ms: Option<u64>` フィールドのみ。integrator エージェントは gitignore 対象の
> `ptygrid.yml` に定義されているためタグの内容としては追跡できない。

> 注: v0.1.0 のまま Phase 4.2 まで進めたため、遡及タグ（v0.4.0/v0.4.1）は付けず、
> v0.4.2 を最初のリリースタグに集約した。以降は原則 1 リリース = 1 patch。

### v0.5.6 以降の未タグ成果（`main` = `4c02cbb`）

`v0.5.6..main` は 24 コミット（46 ファイル / +7,949 −729）。主なものは次の 2 件で、
いずれも**タグが付いていない**:

| コミット | 内容 |
|---|---|
| `52de433` | **5.0.4 実行層の仕上げ**: `fanOut` の黙殺による false green の解消（`fan-out` 以外のパターンでの `fanOut` 宣言を load 時に拒否、`joinOn: N` を実効コピー数で検査）+ straggler cancellation（`any` / `N` join が満たされた時点で同一 step の残 copy を kill / `Cancelled` 降格）。CONTRACT.md 続報9 |
| `2dc5e40` | **Orchestrator ハードニング**: pane 上限の待ち行列化 / driver tick 軽量化 / inbox mailbox の run 単位分離。**PR #3（`4c02cbb`）で `main` にマージ済み**。CONTRACT.md 続報10、設計は [refactor-pane-cap-5.0.5.md](refactor-pane-cap-5.0.5.md)。詳細は §5.6 |

このほか `3883128`（docs の公開/内部分離）と `d3eac32`（MIT license 宣言）が `main` 直コミット、
`5d3c1b5` + `3bd9833`（5.0.4 スキーマ + 実行層本体）が PR #1（`4981ba6`）、
`dd7a135`（docs を spec / guide / design に再編、3 spec の公開）が PR #2（`784f7e9`）で入っており、
いずれも未タグのまま `main` にある（上表の 2 件は PR #3 = `4c02cbb` に同梱）。

**次のタグ（提案）**: 上記をまとめて 1 本のリリースにすることを提案する。**タグを打つかどうか、
どの番号にするかを決めるのはユーザーであり、本文書は選択肢と得失を並べるところまで**。番号は
未確定で、次の 2 案がある:

- **(a) `v0.5.7` を割り当てる**。連番が単調増加し、`git tag` の並びと時系列が一致する。
  ただし [spec-phase5-5.md](../spec/spec-phase5-5.md) §9 の「バージョン割り当て」表と v0.5.6 の
  タグメッセージが **`v0.5.7` = Phase 5.5.1（OTel + SQLite）を予約している**ため、
  5.5.1〜5.5.4 を 1 つずつ繰り下げる（`v0.5.8`〜`v0.5.11`）修正が要る。
- **(b) 予約どおり `v0.5.4` を後追いで打つ**（`v0.5.2`〜`v0.5.5` = Phase 5.0.2〜5.0.5 の予約を
  守る）。予約表は無傷だが、`v0.5.6` より後に `v0.5.4` を作ることになり、タグ順と時系列が
  逆転する。

文書としての推しは **(a)**（タグ順と時系列が一致するほうが後から追いやすく、予約表の付け替えは
ドキュメント修正だけで済むため）。どちらを採るにせよ、**§2 P1（実機での workflow 1 本流し）を
通してからタグを打つ**ことを提案する。`5.0.5` を今回の断面に採番しないという §5.6 の結論は
維持する（Arena view 用に予約済み）。

### リリース手順（タグ付けの作法）

1. `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` の
   `version` を一致させて更新（`Cargo.lock` は `cargo check` で追従）
2. 全チェック（`cargo test` / `clippy` / `npm run check` / `npm run build`）通過を確認
3. `git tag -a vX.Y.Z -m "<リリース概要>"` → push（annotated タグのみ。軽量タグは使わない）
4. 変更履歴は当面 CHANGELOG.md を作らず「タグメッセージ + `git log` + 本文書の表」で代替。
   License は `d3eac32` で MIT 確定済みなので、本格的な公開に踏み切るタイミングで
   CHANGELOG.md 化を再検討する
5. 将来課題: 3 ファイルの version 同期を `scripts/` の bump スクリプトにする（未着手）

---

## 4. 運用メモ

- 各リリースは inside/phase3.md の規律を踏襲する: CONTRACT 先行追記、`lib.rs`/hot path に
  新ロジックを置かない、unit + integration テスト、両プラットフォーム CI 通過、
  該当挙動のみ userguide 更新。
- 本文書は Phase の完了・計画変更のたびに「現在地サマリ」と「次の作業」を更新する。

---

## 5. Phase 5.0 / 5.5 / 6.0 の予約と進捗記録

**詳細な経緯は以下、現在地の要約は §1**（Phase 5 系 実装状況テーブル）を見ること。

> **本節の位置づけ（2026-07-29 時点）**: 元は「未実装・設計のみ」の予約表だったが、
> §5.4〜§5.6 に**日付つきの進捗記録**が積み上がっている。§5.1〜§5.3 は予約・規律、
> §5.4 以降は**その時点の状態を記録した履歴**であり、後から更新していない
> （例: §5.4 の「v0.5.0 タグは実走スモークテスト通過後」は当時の方針で、
> 実際には v0.5.0 は 2026-07-23 にタグ済み）。**現在地は §1、次にやることは §2 が正**。
> 未実装として残っているのは Memory / Provider / Arena / OTel / Phase 6.0 で、
> workflow 系（5.0.0 / 5.0.1 / 5.0.4）と 5.5.0 は実装済み。

詳細は 3 spec([spec-phase5-0.md](../spec/spec-phase5-0.md) / [spec-phase5-5.md](../spec/spec-phase5-5.md) / [spec-phase6-0.md](../spec/spec-phase6-0.md)) と `docs/inside/phase5-6.md`（git 管理外の内部資料）を参照。**先行実装は Phase 5.0 の MVO(Minimum Viable Orchestrator)**、それ以降は Track A/B/C/D の 4 並列(`ptygrid.yml` の `workflows:` セクション参照)。

### 5.1 SQLite `PRAGMA user_version` 予約表

migration は additive、既存 `queen.sqlite3` を壊さない。version bump は Phase 単位で予約する:

| user_version | Phase | 追加テーブル | patch |
|---|---|---|---|
| 1 | Phase 3.6 | pins / notes | 3.6 |
| 2 | Phase 3.7 | inbox / reply | 3.7 |
| **3** | **Phase 5.0**(部分実装) | `workflow_runs` ✅ **5.0.1 で実装済み**（`queen_store.rs`: v0→v3 の新規作成、v1→v3、v2→v3 のいずれの経路も `WORKFLOW_RUNS_SCHEMA_SQL` を適用して `PRAGMA user_version = 3` に到達する）／ `memory` + `memory_fts` + `memory_vec` ❌ **未実装**（`memory.rs` 自体が存在しない） | 5.0.0 / 5.0.1 / 5.0.2 |
| **4** | **Phase 6.0**(未実装) | `replays`、`secrets_audit`、`sandbox_events` | 6.0.0 |

> 2026-07-28 現在の実装値は **`user_version` = 3**。`queen_store.rs` は `version > 3` を
> 「unsupported Queen database version」で開かずに弾く（v4 の予約は表のみで未実装）。
> なお「`workflow_runs` と `memory` を同じ v3 で導入し 5.0.0 で skeleton・5.0.1 で本格実装」
> という下の規律は、実際には **5.0.1 が Workflow Resume に充てられ memory は着手されなかった**
> ため、v3 は `workflow_runs` のみで確定している。memory 系テーブルを追加する場合は
> additive migration を v3 内で行うか v4 を切るかを、着手時に決め直す必要がある（§2 P4 の注記）。

**規律**:
- 未知の新 version は黙って開かない(明示 error でユーザーに再インストールを促す。Phase 3.6 の規律を継承)。
- migration は transactional、既存の pins/notes/inbox データを壊さない。
- Phase 5.0 の `workflow_runs` と `memory` は同じ v3 で導入し、5.0.0 で skeleton、5.0.1 で本格実装(2 patch にまたがる migration は1回のみ)。
- Phase 6.0 の 3 テーブルは同じ v4 で同時導入(6.0.0 Foundation)。

### 5.2 Track 別 branch 命名規則

MVO(5.0.0)完成後、Track A/B/C/D を並列に走らせる。branch は 1 patch = 1 branch を基本とし、以下の prefix を強制する:

| Track | prefix | 例 | 対応 patch |
|---|---|---|---|
| Track A(UI) | `track/a-ui-*` | `track/a-ui-5.5.3-status-rings` | 5.5.3 / 5.5.4 / 5.0.5 / 6.0.5 |
| Track B(MCP+観測) | `track/b-mcp-*` | `track/b-mcp-5.5.0-rc-router` | 5.5.0 / 5.5.1 / 5.5.2 |
| Track C(Memory+Provider+Orch完成) | `track/c-memory-*` | `track/c-memory-5.0.1-fts5` | 5.0.1 / 5.0.2 / 5.0.3 / 5.0.4 |
| Track D(Security) | `track/d-security-*` | `track/d-security-6.0.2-strict-sandbox` | 6.0.0 / 6.0.1 / 6.0.2 / 6.0.3 / 6.0.4 |
| MVO(先行、Track に属さない) | `mvo/*` | `mvo/5.0.0-orchestrator` | 5.0.0 |
| その他 | `main`(直マージ不可)、`bug/*` / `docs/*` | | |

**コーディネーション制約**:
- `CONTRACT.md` の Phase 節は additive のみ。異なる Track が同時に同じ Phase 節を触ると merge 競合が起きるので、各 patch はその patch 用の subsection(§5.0.1 / §5.0.2 のような)を先に予約する(既に本節と CONTRACT.md 側でスケルトンを用意済み)。
- `queen.rs` は薄いディスパッチャに保ち、各 tool 実装は別 module(`orchestrator.rs` / `memory.rs` / `secrets.rs` / `sandbox.rs` / `replay.rs` / `provider.rs` 等)に閉じる。
- `session.rs`(PTY hot path)は Track A(UI)/D(sandbox tee tap)の両方が触るので、Track D が先に tee tap を入れて、Track A は tap 済み event を購読するだけにする。
- GitHub Actions の concurrency group を Track 別に切る。merge queue を利用して直列化。
- 人手レビューは Track D(Security)を最優先。Sandbox / Secrets は毎日固定 2 時間のレビュー枠を確保、他 Track は Opus adversarial verify で 8 割済ませる。

### 5.3 実装 dev workflow 用の agent と workflow は `ptygrid.yml` に定義済み

`ptygrid.yml` の `agents:` に 4 種(`opus-planner` / `sonnet-coder` / `opus-reviewer` / `sonnet-docs`)、`workflows:` に 4 track(`track-a-ui` / `track-b-mcp-otel` / `track-c-memory` / `track-d-security`)を定義済み。MVO(5.0.0)完成後、`spawn_workflow {name: "track-b-mcp-otel"}` の Queen tool 呼び出しで各 Track の1 patch サイクルが自動で回る(design → implement → verify → docs、Track D は verify → redteam → docs)。

### 5.4 進捗（2026-07-22): Phase 5.0.0 MVO 完了

- **実装完了**: `workflows:` スキーマ + 検証（config.rs）/ orchestrator.rs（spawn + DAG 進行
  ドライバ、完了判定 2 経路、fail-fast、fan-out fresh-spawn）/ Queen MCP tools 22 本
  （`spawn_workflow` / `join_workflow` / `cancel_workflow` 追加）/ Tauri commands 3 本 +
  `workflow-state` イベント / WorkflowPanel.svelte + 🔀 チップ。
- **検証**: cargo test 246 / clippy 0 / svelte-check 0 / vite build 成功 / 実機で
  config 読み込み・チップ表示を確認済み（workflow 実走の実機確認は継続）。
- **注記**: run registry は in-memory（app 再起動で消える）。SQLite `workflow_runs` +
  user_version 2→3 は 5.0.1 へ。supervisor / handoff / retry / timeout / join_on reply|N
  は 5.0.4。CONTRACT.md「Phase 5.0 追加契約」に確定契約を追記済み。
- **バージョン**: v0.5.0 タグは workflow 実走スモークテスト通過後（3 ファイルの
  version 同期 → 全チェック → annotated tag、§3 の手順どおり）。

### 5.5 進捗（2026-07-23): Phase 5.0.1 Workflow Resume 完了

- `workflow_runs` 永続化（user_version 2→3）+ write-through、`workflow-resume-pending`
  イベント + Y/N バナー、`resume_workflow` / `abandon_workflow` commands。
- cargo test 251 / clippy 0 / svelte-check 0。実機のクラッシュ→再開テストは継続。
- バージョン: v0.5.1 タグ候補（3 ファイル version 同期 → 全チェック → annotated tag）。

### 5.6 進捗（2026-07-28）: Orchestrator ハードニング（pane 上限 / driver tick / mailbox）

設計メモ（当時 `DESIGN-refactor-5.0.5.md`（リポジトリ直下）。`3c83384` で
[docs/design/refactor-pane-cap-5.0.5.md](refactor-pane-cap-5.0.5.md) へ移動）に沿って、5.0.4 Orchestrator の非機能面を
3点リファクタ。**wire 契約(config スキーマ/IPC/MCP)は無変更**につき CONTRACT.md は
「Phase 5.0 追加契約」節に追記のみ（続報10）で対応、破壊的変更の互換パス整備は不要。

- pane 上限（9面）が埋まっている間、spawn できない step は `Failed` ではなく `Pending`
  のまま待ち行列化（最大 `WORKFLOW_DEFER_MAX_MS` = 5分、超過で従来どおり `Failed`）。
  `timeoutMs` は待ち時間を含まない仕様として確定。
- `PtyManager::session_states()` / `live_session_count()` を新設し、driver tick /
  `team_presets` / `queen list_agents` の内部計算が `ps` fork を伴う `list_sessions()`
  を呼ばなくなった（`list_agents` の返り値自体は不変）。`WorkflowRegistry` に終端 run の
  evict（`REGISTRY_TERMINAL_CAP` = 100）を追加。
- workflow の inbox mailbox を `queen:workflow/<name>` から `queen:workflow/<name>/<runId>`
  へ変更し、同名 workflow の並行 run がもう mailbox を共有しないようにした（新規制約:
  workflow 名は 84 バイト以下）。
- cargo test 374 passed / 0 failed（lib）+ 14 passed（統合）。svelte-check は本リファクタが
  backend のみのため対象外。実機での workflow 1 本流し・GUI 目視確認は継続（Phase 5.0.4
  以来の既知の未検証事項）。
  - **追記（2026-07-29 未明）**: 本断面はレビュー指摘（F1/F4/F5/F6/F7/F8）の反映を経て
    `2dc5e40` に squash され、**PR #3（`4c02cbb`、2026-07-29 01:35 +0900）で `main` に
    マージ済み**。CONTRACT.md
    続報10 が書いている「`main` 未マージも変わらない」はこの時点で失効している。
    lib テストは 374 → **375** に増えている（§1 の実測表）。`cargo clippy -D warnings` が
    `config.rs:834` の `nonminimal_bool` で落ちる件は未解消のまま。
- **バージョン提案**: 本断面は wire 契約が不変の内部ハードニングであり、§3 規約上は
  「破壊的変更ではない」ため単独のリリースタグを必須としない。`orchestrator.rs` の
  コード内コメントは便宜上「phase 5.0.5」と書いているが、**`5.0.5` は既に §5.1/
  spec-phase5-0.md で Arena view 用に予約済み**であり衝突する。次のいずれかで解消する
  ことを提案する: (a) 本断面を「5.0.4 のフォローアップ」として無番号のまま次の
  リリースタグ（例: v0.5.4 の一部、または次回のまとめリリース）に含める、
  (b) Arena view の実装時に `5.0.5` はそのまま Arena に割り当て、本断面のコード
  コメントの「phase 5.0.5」表記は将来の整理コミットで削除する。いずれにせよ
  **`5.0.5` を本断面に正式採番しない**。
