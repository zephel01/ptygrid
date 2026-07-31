# example/ — 用途別のサンプル設定

用途ごとの `ptygrid.yml` スターター集。使いたい構成のファイルを自分のプロジェクト
ルートへ `ptygrid.yml` としてコピーし、ツールバーから読み込んでください
（ディレクトリ指定欄に各サンプルのフォルダを直接指定して試すこともできます）。

旧ファイル名 `mterm.yml` も互換のため読み込めますが、`ptygrid.yml` が優先されます。
全フィールドの注釈付きリファレンスはリポジトリ直下の
[ptygrid.example.yml](../ptygrid.example.yml) を参照してください。

| サンプル | 用途 |
|---|---|
| [basic/](basic/ptygrid.yml) | 最小構成。shell + Claude Code を1〜2ペインで |
| [multi-agent/](multi-agent/ptygrid.yml) | Claude / Codex / Grok を並行実行し、Queen で協調させる |
| [web-dev/](web-dev/ptygrid.yml) | Web開発。エージェント + dev server / テストwatchを autorestart で常駐 |
| [worktree/](worktree/ptygrid.yml) | エージェントごとに linked worktree で作業ツリーを分離 |
| [teammates/](teammates/ptygrid.yml) | Claude Code の subagent/teammate をペインで観測（Phase 4.0/4.1） |
| [team-preset/](team-preset/ptygrid.yml) | ローカルLLM主体 + クラウド standby のチームを 👥 で一括起動（Phase 4.3） |
| [adaptive-orchestration/](adaptive-orchestration/ptygrid.yml) | タスクを分類して worker を事前選抜し、Verifier 合格まで反復させる（Phase 5.7.0）。router + routing_hints 表 + plan-build-verify + 総当たり bakeoff |
| [review-starter/](review-starter/ptygrid.yml) | 実装 → レビュー → ジャッジの3段 workflow。指示 / チェック観点 / 判定基準の**3か所だけ**埋めれば動く雛形（[README](review-starter/README.md)） |
| [cross-model-review/](cross-model-review/ptygrid.yml) | 実装1体 → **別モデル2体が並行レビュー** → 突き合わせ（`pattern: supervisor`）。レビュアーは worktree で分離し、レビュー本文は `handoffTo` ではなく**ファイル経由**で渡す（`handoffTo` はターゲット1つにつき本文1本しか運ばず、2体目が黙って捨てられるため）。`joinOn: reply` は「2体とも返信した」という同期にだけ使う |
| [measure-parallelism/](measure-parallelism/ptygrid.yml) | 計測用。エージェントを `sleep` に置き換えた合成 workflow 4本で、並列化の効きと orchestration のコスト（tick・spawn・9面上限の待ち・joinOn: any の敗者kill）を決定論的に測る |
| [measure-coldstart/](measure-coldstart/ptygrid.yml) | 計測用。同じ agent を3段並べた pipeline（1段目=spawn / 2・3段目=同じペインの再利用）で、エージェントを毎回 spawn し直す代金（プロセス起動 + CLIブート + Queen接続 + inbox読み直し）を `1段目 − 2段目` の引き算で実測する。`joinOn: reply` + 返信極小。ローカルLLM代替あり |

各ファイルは省略可能なフィールドをコメントで残しています。既定値は
[docs/guide/userguide.md](../docs/guide/userguide.md) の設定リファレンスを参照してください。
