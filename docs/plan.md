# ptygrid 作業計画 (plan.md)

更新日: 2026-07-17 / 実装基準: Phase 4.2（host モード）+ UXトラック完了時点

この文書は「いま何が終わっていて、次に何をやるか」と「バージョンの付け方」を
1か所にまとめる作業計画である。Phase 3.x の詳細な実績とリリース規律は
[phase3.md](phase3.md)、teams 機能の設計は
[spec-claude-teams-panes.md](spec-claude-teams-panes.md)、方向性の背景は
[competitive-landscape.md](competitive-landscape.md) を参照。

---

## 1. 現在地サマリ

### 完了済み

| Phase | 内容 | 状態 |
|---|---|---|
| 0 | 単一 PTY ペイン | ✅ |
| 1 | マルチペイン + config-as-code（現 `ptygrid.yml`）、autostart/restart | ✅ |
| 2〜2.1 | Queen（内蔵 MCP サーバー、基本5 tools）+ ドッグフーディング反映 | ✅ |
| 3.0〜3.8 | Git status/diff/stage/commit、opt-in worktree 分離、logical resume、リソース監視、Queen pins/notes/inbox/reply/await（18 tools） | ✅ |
| 3.9 | Linux テスト対応（PATH 復元、Ubuntu CI、`.deb`/AppImage） | ✅ |
| 4.0 | teammate hooks 受信基盤（`/hooks/v1/*`、token 認可、toast、Teammates バッジ、`teammates:` ブロック、settings.json 半自動登録） | ✅ |
| 4.1 | observe: `transcript` ペイン種別（PTY なし論理セッション）、SubagentStart で read-only tail 自動生成、`agents[].teams`、上限/9面/path 検証 | ✅ |
| 4.2 | host: tmux 互換シム + per-lead Unix socket RPC 配線、env/PATH 注入、実 PTY teammate ペイン、フォールバック検知→observe 降格、`teammate-focus`/`teammate-fallback`/`teams_host_status`、frontend（確認付き close・focus 強調・Teammates パネル host セクション） | ✅ |

### Phase 4 計画外で入った UX トラック（コミット済み）

| 内容 | コミット |
|---|---|
| 設定ファイル名を `ptygrid.yml` へ変更（`mterm.yml` は作業フォルダ内のみ互換） | da40cb0 |
| 用途別サンプル `example/{basic,multi-agent,web-dev,worktree,teammates}` | da40cb0 |
| 一括cd（ツールバー → のちに読み込みへ統合） | cf42ced, 77d0271 |
| プロジェクト欄を作業フォルダ化、設定探索を 作業→起動→`~/.ptygrid` に分離、origin バッジ | acbed94 |
| 設定なしフォールバック（既定設定で開く）+ 読み込み = シェルペイン一括cd | 0530e3b |
| cd…ボタン撤去、作業フォルダ入力のフォルダサジェスト（projects root 自動記憶） | a3a769a |
| 終了ペインの明示（「終了」タグ）と一括クローズ | d8a3d8e |

### v0.4.3 で入った調査対応・安定化（コミット済み）

docs/inside のバグ/セキュリティ調査（teammate 分担レビュー）を評価し、対応(Do)判定を実装:

| 内容 | コミット |
|---|---|
| backend 純バグ 12件（H1 teammate消失, M1 マルチバイト分割, M2/M3, M4 worktreeリーク, M6/M7, L1/L5/L8/L9/L12a） | c6f31ad |
| frontend 純バグ 8件（BUG-1 TermHandleリーク, BUG-2 ゾンビペイン, BUG-3 autorestart誤バナー, BUG-4/5/6/7/9） | 7505bbe |
| **S1 Critical**: Queen `/mcp` にトークン＋Host/Origin 認証（RCE 対策）＋定数時間比較 | 3159263 |
| **S2 High / S4 Medium**: autostart 信頼境界（trust.rs）＋ CSP | f18bae6 |
| 手打ち claude も observe lead 候補にする＋未マッチ通知（実機で判明した帰属漏れ） | 9c4ab67 |
| 認証トークンの永続化（`auth-tokens.json`、再起動後も再登録不要、再生成コマンド） | 0af8de4 |

Defer/Skip 判定（u32 wrap 等の理論値、稀なレース、実験機能の DoS、S3 caller-id 等）は
[docs/inside/evaluation-2026-07-16.md](inside/evaluation-2026-07-16.md) に整理（この文書は git 管理外）。

### 実装済みの基盤

- `src-tauri/teams-backend/`: CustomPaneBackend 提案（anthropics/claude-code#26572）
  準拠の JSON-RPC 2.0 ソケットサーバ + tmux 互換シム（テスト30件）。**Phase 4.2 で app 本体へ
  配線済み**（`teams_host.rs` の `PaneHost` 実装・`__tmux-compat` サブコマンド経由）。

---

## 2. 次の作業（優先順）

### Phase 4.2 — host モード ✅ 完了

tmux 互換シム + per-lead Unix socket RPC の配線、env/PATH 注入、実 PTY teammate ペイン、
フォールバック検知（→ observe 降格）、frontend（host teammate ペイン・確認付き close・
focus 強調・Teammates パネル host セクション・paneless 昇格・孤立 teammate 停止）まで実装済み。
残タスクは実機検証のみ:

- Claude Code 実機での手動検証（spec 10.3 の手順）: macOS 必須・Linux はベストエフォート。
  互換 Claude Code で teammate がネイティブ対話ペイン化し、シム未使用時に observe へ降格する
  ことの実地確認。自動テスト（cargo test 119 / teams-backend 30 / svelte-check 0 / build）は通過済み。

### Phase 4.3 — Queen team preset（方式C、Claude Code 内部に非依存）【v0.4.6 リリース済み】

方針決定・実装・**リリース（v0.4.6 タグ）・実機偵察まで完了（2026-07-17）**。
詳細仕様・設計差分・偵察ログは [spec-team-presets.md](spec-team-presets.md)、
wire 契約は CONTRACT.md「Phase 4.3 追加契約」、検証手順は
[verify-team-preset.md](verify-team-preset.md)。

- 実装: config 検証 + `team_presets.rs` + Queen tool `spawn_team`（19本目）+ Tauri command +
  👥 チップ UI + example/team-preset。cargo test 210 / svelte-check 0 errors / build 通過
- 偵察結果: coderouter 経由のローカルLLMで Queen 協調・混在・エスカレーション機構は成立。
  **エージェント発の自動エスカレーションのみ不合格**（制約違反の自力修正を観測）→
  **既定パターンは人間発**（spec 8章 偵察ログ ①〜⑤）
- 残: E2E-1 日常編の実タスク検証（任意）、`git push --follow-tags`（ローカルにのみ
  コミット/タグがある状態）、v0.4.7 候補（instructions 重複配送抑止・`spawn_agent` reuse）

- `ptygrid.yml` に `team_presets:` を追加。メンバーは `agents:` 定義の参照のみ
  （allowlist 完全整合・新しい信頼境界なし）、`standby: true` で待機層を宣言、
  `instructions` / `kickoff` は Queen の永続 inbox で配送（`#id` 限定・推測拒否を維持）
- 起動経路はツールバー UI + Queen tool `spawn_team` の両輪（実体は同一 backend 関数）。
  冪等 skip・9面上限は部分起動+明示レポート
- 主想定はローカルLLM混在のコスト階層型チーム（普段はローカル、難問のみ Opus/Grok 等）。
  CLI は Claude Code のまま coderouter 経由で llama.cpp / ollama に接続し、**ptygrid の
  責務は per-agent env の注入まで**。複数ローカルモデルの切替は coderouter 以降
  （ユーザー側で実装予定・ptygrid 非スコープ）
- エスカレーションは機構化せず、instructions 規約として userguide に文書化
- **実装前ゲート**: coderouter 構成で Queen 接続とエスカレーション一連
  （spawn_agent → inbox → await）の実機偵察（spec 8 章）→ 結果を spec に反映してから着手。
  リリースは v0.4.4 を想定

### 継続ウォッチ / バックログ

- **anthropics/claude-code#26572**（CustomPaneBackend 公式化）: 採用されたら
  シム撤去 + `CLAUDE_PANE_BACKEND_SOCKET` 広告へ移行（teams-backend はそのまま使える）
- 通知リング / 要承認ハイライト（competitive-landscape の「次に取る UX」。
  4.0 の teammate permission 表示を汎用化）
- ~~hook token の固定化~~ → v0.4.3 で対応済み（トークン永続化）
- 残りの Defer 項目（backend M5/M8/L3/L4/L6/L7/L11 系、frontend BUG-8/10、
  security S3 caller-id・Low 群）は evaluation の推奨ロードマップに従い順次
- Linux 実機検証の継続、Windows 移植（[porting.md](porting.md)）
- License 決定（公開前に必須）

---

## 3. バージョニング規約

現状の `version` は 3 ファイルとも `0.1.0` のまま実態とズレているため、次の規約で運用する。

### 規約（SemVer 0.y.z、1.0 まで）

- **y（minor）= Phase 番号**。Phase 4 系の間は `0.4.z`、Phase 5 系に入ったら `0.5.0` から。
  過去に当てはめると Phase 3.9 時点 ≒ `0.3.9` 相当（遡及タグは付けない）。
- **z（patch）= その Phase 内のリリース連番**。機能追加・修正の区別はしない
  （pre-1.0 の SemVer では minor が破壊的変更の単位のため、これで矛盾しない）。
- **破壊的変更**（config スキーマ・IPC/MCP 契約・保存データ）は pre-1.0 でも
  「CONTRACT.md への契約追記 + 互換パス（例: mterm.yml フォールバック）」を必須とし、
  やむを得ず互換を切る場合は y を上げて README に移行手順を書く。
- **1.0.0 の条件**: License 決定、macOS 安定 + Linux beta 卒業、teams host（4.2）の
  実機安定、config スキーマ凍結。

### 直近のバージョン割り当て

| バージョン | 内容 |
|---|---|
| ~~v0.4.0 / v0.4.1~~ | 個別タグは打たず **v0.4.2 に集約** |
| v0.4.2 | Phase 4.0（hooks 受信基盤）〜 4.1（observe）〜 4.2（host モード実験）+ UXトラック一式（最初のリリースタグ） |
| **v0.4.3** | 調査対応の安定化リリース: バグ修正 20件（backend 12 / frontend 8）+ セキュリティ 4件（S1 Queen認証 / S2 trust / S4 CSP）+ 手打ち claude の lead 帰属修正 + **認証トークンの永続化**。cargo test 159 / teams-backend 30。**現時点の main** |
| **v0.4.6** | Phase 4.3（Queen team preset: `team_presets` 宣言 + `spawn_team`（19本目）+ 👥 一括起動 UI + example/team-preset）。cargo test 210 / svelte-check 0 |
| **v0.4.7** | UI 多言語化（en/ja。型付き辞書 `i18n.svelte.ts`、⚙ 設定メニューで 自動/English/日本語 切替、既定=OS言語に自動追従・英語ベース）。フロントのみ、backend 文言・ログは対象外。svelte-check 0 / build 成功 |
| **v0.4.8** | ssh 接続先表示（Phase 4.4.3: `session-resources` の foreground に `detail?` を追加し argv から宛先抽出。ヘッダー/サイドバーに `ssh user@host`。`.ssh/config` alias・`-l` 畳み込み対応）。cargo test 214 / clippy 0 / svelte-check 0。**現時点の main** |
| v0.4.7 / v0.5.0 | 残 Defer 消化 or 通知リング系 UX。Phase 5 系の扱いは実機偵察の結果を見て決定 |

> 注: v0.1.0 のまま Phase 4.2 まで進めたため、遡及タグ（v0.4.0/v0.4.1）は付けず、
> v0.4.2 を最初のリリースタグに集約した。以降は原則 1 リリース = 1 patch。

### リリース手順（タグ付けの作法）

1. `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` の
   `version` を一致させて更新（`Cargo.lock` は `cargo check` で追従）
2. 全チェック（`cargo test` / `clippy` / `npm run check` / `npm run build`）通過を確認
3. `git tag -a vX.Y.Z -m "<リリース概要>"` → push（annotated タグのみ。軽量タグは使わない）
4. 変更履歴は当面 CHANGELOG.md を作らず「タグメッセージ + `git log` + 本文書の表」で代替。
   公開（License 決定）のタイミングで CHANGELOG.md 化を再検討
5. 将来課題: 3 ファイルの version 同期を `scripts/` の bump スクリプトにする（未着手）

---

## 4. 運用メモ

- 各リリースは phase3.md の規律を踏襲する: CONTRACT 先行追記、`lib.rs`/hot path に
  新ロジックを置かない、unit + integration テスト、両プラットフォーム CI 通過、
  該当挙動のみ userguide 更新。
- 本文書は Phase の完了・計画変更のたびに「現在地サマリ」と「次の作業」を更新する。

---

## 5. Phase 5.0 / 5.5 / 6.0 の予約（未実装・設計のみ）

詳細は [phase5-6.md](phase5-6.md) と 3 spec([spec-phase5-0.md](spec-phase5-0.md) / [spec-phase5-5.md](spec-phase5-5.md) / [spec-phase6-0.md](spec-phase6-0.md)) を参照。**先行実装は Phase 5.0 の MVO(Minimum Viable Orchestrator)**、それ以降は Track A/B/C/D の 4 並列(`ptygrid.yml` の `workflows:` セクション参照)。

### 5.1 SQLite `PRAGMA user_version` 予約表

migration は additive、既存 `queen.sqlite3` を壊さない。version bump は Phase 単位で予約する:

| user_version | Phase | 追加テーブル | patch |
|---|---|---|---|
| 1 | Phase 3.6 | pins / notes | 3.6 |
| 2 | Phase 3.7 | inbox / reply | 3.7 |
| **3** | **Phase 5.0**(未実装) | `workflow_runs`(5.0.0)、`memory` + `memory_fts` + `memory_vec`(5.0.1-2) | 5.0.0 / 5.0.1 / 5.0.2 |
| **4** | **Phase 6.0**(未実装) | `replays`、`secrets_audit`、`sandbox_events` | 6.0.0 |

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
