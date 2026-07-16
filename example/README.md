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

各ファイルは省略可能なフィールドをコメントで残しています。既定値は
[docs/userguide.md](../docs/userguide.md) の設定リファレンスを参照してください。
