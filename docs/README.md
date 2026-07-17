# docs/ ドキュメント索引

ptygrid のドキュメント置き場。**日本語版が正**で、主要ドキュメントには英語版（同名の
`.en.md`）を併置しています。リポジトリ全体の入口は [../README.md](../README.md) を、
backend ⇄ frontend / Queen の正確な wire 仕様は [../CONTRACT.md](../CONTRACT.md) を
参照してください。

## まず読む（使う人向け）

| ドキュメント | 内容 |
|---|---|
| [userguide.md](userguide.md) | 全機能の操作ガイド。インストール、`ptygrid.yml` リファレンス、Queen（19ツール）の登録と使い方、Teammates（observe/host）、チームプリセット、状態バッジ、worktree 分離、セッション復元、エージェント間協調の実践レシピまで。**何か操作で迷ったらまずここ** |
| [troubleshooting.md](troubleshooting.md) | 実機で踏んだ罠と対処の事例集。Queen MCP の登録スコープ問題（`-s user` 必須）、「ウィンドウが勝手に落ちる」の正体が `tauri dev` のファイル監視だった件（`--no-watch`）、Inbox が見つからない、Grok TUI で応答判定が遅れる件など。**詰まったら最初に検索する場所** |

## 設計と仕様（開発者向け）

| ドキュメント | 内容 |
|---|---|
| [design.md](design.md) | 設計ドキュメント。フロント（Svelte 5）+ backend（Rust）のモジュール構成、技術スタックの採用理由、Session/PTY モデル、Queen、SQLite 永続化と revision 競合検出、変更時に守る設計原則 |
| [../CONTRACT.md](../CONTRACT.md) | IPC / MCP 契約の時系列記録（Phase 0 〜 4.x の追加契約、19章）。コマンド・イベント・スキーマの**現行仕様の正**。機能追加はここへの契約追記が先 |
| [spec-claude-teams-panes.md](spec-claude-teams-panes.md) | Claude Code の teammate/subagent をペイン自動追加する仕様。方式A（tmux シム host）/ B（hooks 観測 observe）/ C（Queen 自前）の比較と採用判断、シムの JSON-RPC、フォールバック検知。A/B は Phase 4.1–4.2 で実装済み |
| [spec-team-presets.md](spec-team-presets.md) | チームプリセット（方式C・Phase 4.3 = v0.4.6 実装済み）の仕様。`team_presets:` スキーマと検証、起動セマンティクス（冪等 skip・部分起動・inbox 配送）、ローカルLLM主体+クラウド standby のコスト階層構成、**実機偵察ログ**（エージェント発エスカレーション不採用の経緯を含む） |
| [spec-agent-status.md](spec-agent-status.md) | 意味的状態検出（working / blocked / done / idle）の仕様。herdr 由来の出力ヒューリスティック、内蔵既定パターンと `agent_status:` 上書き、hot path 分離。策定時の計画文書で、Phase 4.4.0（検出基盤）として実装済み |
| [spec-notifications.md](spec-notifications.md) | アプリ外通知（Phase 4.4.2 実装済み）の仕様。セッション終了と blocked/done エッジを OS 通知・Slack / Mattermost / Discord / Telegram へ中継。イベント×レベルのマトリクスと `notifications:` スキーマ |

## 計画・運用・検証

| ドキュメント | 内容 |
|---|---|
| [plan.md](plan.md) | 作業計画。「いま何が終わっていて次に何をやるか」の現在地サマリ、バージョニング規約（y = Phase 番号）、リリース手順。**現在地を知りたいときはここ** |
| [phase3.md](phase3.md) | Phase 3.0–3.9 の段階リリース記録と、以後も踏襲しているリリース規律（CONTRACT 先行追記・テスト・両OS CI） |
| [verify-team-preset.md](verify-team-preset.md) | チームプリセットの手動検証手順書。ゴール定義（G1–G4）、起動順序チェックリスト、機能テスト T1–T6、実機偵察 R1–R3、E2E 受け入れシナリオ |
| [porting.md](porting.md) | 移植状況。Linux（beta）の build / `.deb` / AppImage と CI、Windows 対応の未着手チェックリスト |
| [competitive-landscape.md](competitive-landscape.md) | 類似ツールの競合調査（cmux, Claude Squad, Conductor 等）。worktree 隔離系 / 同一画面協調系の分類と ptygrid のポジショニング、やらないことの整理 |

## 英語版について

`userguide` / `design` / `troubleshooting` / `porting` / `competitive-landscape` には
英語版（`*.en.md`）があります。各ファイル先頭の言語スイッチャーで行き来できます。
仕様書（spec-*）・計画（plan / phase3）・検証手順は日本語のみです。

## git 管理外のフォルダ（ローカルのみ）

`docs/note/`（note 記事の下書き）、`docs/inside/`（内部レビュー・調査ログ）、
`docs/research/`（競合・外部仕様の調査メモ）は個人メモとして `.gitignore` 対象です。
リポジトリをクローンした環境には存在しません。

## その他

- [screenshot-phase0.4.5.png](screenshot-phase0.4.5.png) — README 掲載のスクリーンショット（v0.4.5 時点の 4 ペイン構成）
