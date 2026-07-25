# docs/ ドキュメント索引

ptygrid のドキュメント置き場。**日本語版が正**で、主要ドキュメントには英語版（同名の
`.en.md`）を併置しています。リポジトリ全体の入口は [../README.md](../README.md) を、
backend ⇄ frontend / Queen の正確な wire 仕様は [../CONTRACT.md](../CONTRACT.md) を
参照してください。

## フォルダ構成

| フォルダ | 役割 |
|---|---|
| [guide/](guide/) | 使う人向け。操作手順、設定の書き方、運用規約、検証手順 |
| [spec/](spec/) | 仕様書。実装済み機能の仕様と、未実装 Phase の設計仕様 |
| [design/](design/) | 設計と計画。アーキテクチャ、移植状況、競合調査、作業計画 |

`docs/note/` / `docs/inside/` / `docs/research/` はローカル専用（`.gitignore` 対象）。
詳細は下の「git 管理外のフォルダ」節を参照。

## まず読む（使う人向け）

| ドキュメント | 内容 |
|---|---|
| [guide/userguide.md](guide/userguide.md) | 全機能の操作ガイド。インストール、`ptygrid.yml` リファレンス、Queen（19ツール）の登録と使い方、Teammates（observe/host）、チームプリセット、状態バッジ、worktree 分離、セッション復元、エージェント間協調の実践レシピまで。**何か操作で迷ったらまずここ** |
| [guide/troubleshooting.md](guide/troubleshooting.md) | 実機で踏んだ罠と対処の事例集。Queen MCP の登録スコープ問題（`-s user` 必須）、「ウィンドウが勝手に落ちる」の正体が `tauri dev` のファイル監視だった件（`--no-watch`）、Inbox が見つからない、Grok TUI で応答判定が遅れる件など。**詰まったら最初に検索する場所** |

## 設定と運用（guide/）

| ドキュメント | 内容 |
|---|---|
| [guide/ptygrid-yml-guide.md](guide/ptygrid-yml-guide.md) | `ptygrid.yml` 執筆マニュアル（agents / workflows / team_presets 編）。userguide.md に無い `workflows:` ブロックの書き方、フィールド網羅表、**「書けるが動かないフィールド」の実装状況の線引き**、実運用で踏んだ落とし穴 |
| [guide/autonomous-operation-guide.md](guide/autonomous-operation-guide.md) | 自主運用ガイド。エージェントに日々の実装を任せるときの原則。基本サイクル、統合担当（integrator）の inbox 応対規約。実運用で実際に起きたことに基づく運用マニュアル |
| [guide/secrets.md](guide/secrets.md) | 秘密（APIキー）の書き方 3系統マニュアル。複数の AI CLI（aider / opencode / claude / pi 等）を1つの `ptygrid.yml` で束ねるときの、キーの置き場所と GUI 起動時の env の扱い |
| [guide/verify-team-preset.md](guide/verify-team-preset.md) | チームプリセットの手動検証手順書。ゴール定義（G1–G4）、起動順序チェックリスト、機能テスト T1–T6、実機偵察 R1–R3、E2E 受け入れシナリオ |

## 仕様（spec/）

| ドキュメント | 内容 |
|---|---|
| [../CONTRACT.md](../CONTRACT.md) | IPC / MCP 契約の時系列記録。コマンド・イベント・スキーマの**現行仕様の正**。機能追加はここへの契約追記が先 |
| [spec/spec-claude-teams-panes.md](spec/spec-claude-teams-panes.md) | Claude Code の teammate/subagent をペイン自動追加する仕様。方式A（tmux シム host）/ B（hooks 観測 observe）/ C（Queen 自前）の比較と採用判断、シムの JSON-RPC、フォールバック検知。A/B は Phase 4.1–4.2 で実装済み |
| [spec/spec-team-presets.md](spec/spec-team-presets.md) | チームプリセット（方式C・Phase 4.3 = v0.4.6 実装済み）の仕様。`team_presets:` スキーマと検証、起動セマンティクス（冪等 skip・部分起動・inbox 配送）、ローカルLLM主体+クラウド standby のコスト階層構成、**実機偵察ログ** |
| [spec/spec-agent-status.md](spec/spec-agent-status.md) | 意味的状態検出（working / blocked / done / idle）の仕様。herdr 由来の出力ヒューリスティック、内蔵既定パターンと `agent_status:` 上書き、hot path 分離。Phase 4.4.0（検出基盤）として実装済み |
| [spec/spec-notifications.md](spec/spec-notifications.md) | アプリ外通知（Phase 4.4.2 実装済み）の仕様。セッション終了と blocked/done エッジを OS 通知・Slack / Mattermost / Discord / Telegram へ中継。イベント×レベルのマトリクスと `notifications:` スキーマ |
| [spec/spec-phase5-0.md](spec/spec-phase5-0.md) | Phase 5.0「Orchestrated & Remembering」仕様。宣言的 DAG workflow / 共有メモリ / Local Provider / AI Arena。workflow 実行層は Phase 5.0.x として段階実装中、memory・provider・arena は仕様のみ |
| [spec/spec-phase5-5.md](spec/spec-phase5-5.md) | Phase 5.5「Observable & Standards-Compliant」仕様（未実装）。MCP 2026-07-28 RC 追随、OTel GenAI 計装、Agent Status Rings |
| [spec/spec-phase6-0.md](spec/spec-phase6-0.md) | Phase 6.0「Secure & Auditable」仕様（未実装）。Sandboxed Execution Pane / Credential Proxy / Session Replay |

未実装 Phase の仕様書は**そのリンク先ソース（`memory.rs` / `sandbox.rs` 等）がまだ存在しない**
前提で読んでください。実装状況の現在地は [design/plan.md](design/plan.md) が正です。

## 設計と計画（design/）

| ドキュメント | 内容 |
|---|---|
| [design/design.md](design/design.md) | 設計ドキュメント。フロント（Svelte 5）+ backend（Rust）のモジュール構成、技術スタックの採用理由、Session/PTY モデル、Queen、SQLite 永続化と revision 競合検出、変更時に守る設計原則 |
| [design/plan.md](design/plan.md) | 作業計画。「いま何が終わっていて次に何をやるか」の現在地サマリ、バージョニング規約（y = Phase 番号）、リリース手順。**現在地を知りたいときはここ** |
| [design/porting.md](design/porting.md) | 移植状況。Linux（beta）の build / `.deb` / AppImage と CI、Windows 対応の未着手チェックリスト |
| [design/competitive-landscape.md](design/competitive-landscape.md) | 類似ツールの競合調査（cmux, Claude Squad, Conductor 等）。worktree 隔離系 / 同一画面協調系の分類と ptygrid のポジショニング、やらないことの整理 |
| [design/queen-universal-register.md](design/queen-universal-register.md) | Queen MCP「汎用コピー」設計案。CLI ごとにボタンを増やさず、どのツールにも貼れる登録スニペットを用意するための設計 |

## 設定例

`ptygrid.yml` のサンプルは [../example/](../example/) にあります（basic / multi-agent /
web-dev / worktree / teammates / team-preset / adaptive-orchestration）。
注釈付きの全体例は [../ptygrid.example.yml](../ptygrid.example.yml)。

## 英語版について

`userguide` / `troubleshooting`（guide/）と `design` / `porting` /
`competitive-landscape`（design/）には英語版（`*.en.md`）があります。各ファイル先頭の
言語スイッチャーで行き来できます。仕様書（spec/）・計画・運用ガイド・検証手順は
日本語のみです。

## git 管理外のフォルダ（ローカルのみ）

`docs/note/`（note 記事の下書き）、`docs/inside/`（内部レビュー・調査ログ・フェーズ記録）、
`docs/research/`（競合・外部仕様の調査メモ）は個人メモとして `.gitignore` 対象です。
リポジトリをクローンした環境には存在しません。本索引からこれらへのリンクは張っていません。

## その他

- [screenshot-phase0.4.5.png](screenshot-phase0.4.5.png) — README 掲載のスクリーンショット（v0.4.5 時点の 4 ペイン構成）
