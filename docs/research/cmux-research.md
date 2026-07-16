# 調査レポート2: cmux (manaflow-ai/cmux) 調査（2026年7月, Sonnetエージェント調査）

出典: github.com/manaflow-ai/cmux（README.md / README.ja.md）、cmux.com/docs（Claude Code Teams / oh-my-codex / Notifications）、cmux.com/blog/cmux-claude-teams、Issues #123 / #6447 / #2618 / #3414 / #4310、Discussions #1872 / #884、YC launch page。

## 1. 概要
- 「Open source Ghostty-based macOS terminal with vertical tabs and notifications for AI coding agents.」
- macOS 専用ネイティブアプリ。Electron ではなく Swift/AppKit + libghostty（GPU レンダリング）。~/.config/ghostty/config を読む。
- 言語構成: Swift ~81%, Python ~10.5%, TS ~3.7%, Shell ~1.8%, Go ~1.6%（Go relay が SSH 機能）。
- ライセンス: GPL-3.0-or-later + 商用ライセンス販売 + 有償 Founder's Edition。
- ★ ~19.3k（2026年7月時点）。2026年3月 YC ローンチ、HN #2。活発（v0.64.10 が 2026-05-23、~3,247 commits）。

## 2. エージェントの実行とペイン構成
- エージェントは workspace 内の実端末ペイン/スプリットで動く（隠しバックグラウンドではない）。
- サイドバーにペイン毎の git branch、PR 番号/状態、cwd、listen ポート、最新通知を表示。縦タブ + 水平/垂直分割（⌘D / ⌘⇧D）。
- git worktree は組み込み自動機能ではない（Issue #3414 が open の機能要望。ユーザーが cmux.json のシェルスニペットで手動運用）。
- コンテナ/サンドボックス分離は無し。ペイン/プロセスレベルの分離のみ。
- ペインの自動生成は teams/teammate 系統合に限る。通常の単発エージェントはユーザーが手動起動。

## 3. Claude Code 統合（cmux claude-teams）
- `cmux claude-teams` 一発で Claude Code の teammate モードを起動。teammate がネイティブスプリット（サイドバーメタデータ・通知付き）として現れる。ユーザー側に tmux 不要。
- 技術メカニズム: tmux 互換シムを `~/.cmuxterm/claude-teams-bin/tmux` に配置し `cmux __tmux-compat` に exec。環境変数 `TMUX`, `TMUX_PANE`, `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` を設定して Claude Code に「本物の tmux 内にいる」と誤認させる。teammate モードが発行する tmux コマンドをシムが横取りし、cmux のローカルソケット API（`CMUX_SOCKET_PATH`）に変換: split-window → ペイン分割、send-keys → テキスト送信、capture-pane → テキスト読み取り、select-pane → フォーカス。
- `cmux claude-teams` の実体: env 設定 → tmux シムを PATH に → `claude --teammate-mode auto` 実行。teammate ペインは右カラムに積まれ、spawn/exit で自動リサイズ。
- 脆さの実例: Issue #6447 — Claude Code 2.1.183 が teammate ペインの起動方法を変え、シムのコードパスを通らなくなり split-window が呼ばれず、teammate が in-process に静かにフォールバック（ペインが出ない）。ペイン生成は「Claude Code が tmux 形式のコマンドを呼ぶこと」に完全依存。
- Issue #123: 元の機能要望。~/.claude/teams/{team-name}/config.json や teammateMode("in-process"/"tmux") に言及。cmux を tmux 相当プロバイダとして登録する upstream 提案。
- マルチモデル: cmux 自体はエージェント非依存。oh-my-opencode / oh-my-codex（OMX, Codex CLI 用 30+ ロールのオーケストレーション層）/ oh-my-pi / oh-my-claudecode の各統合も同じ tmux シムトリック。`cmux claude-teams --model sonnet` の例あり。

## 4. 通知・レビュー UX
- 専用 diff ビューア/承認 UI は無し。レビューは端末ペインの内容（エージェント CLI 自身の表示）で行う。
- cmux の付加価値は注意喚起: 通知リング/サイドバー点灯、通知パネル（⌘⇧I）、⌘⇧U で最新未読へジャンプ、macOS ネイティブ通知（対象にフォーカス中は抑制）。
- エージェントは OSC エスケープシーケンスまたは `cmux notify` CLI で通知を発火。

## 5. 「subagent spawn でペイン自動追加」のメカニズム確認
- 実在する。ただし Claude-Teams/OMX 統合の中核であって cmux の汎用挙動ではない。
- トリガー: Claude Code 自身が tmux の split-window を呼ぶ（tmux 内にいると信じている）。
- 横取り: PATH 上の tmux シムが受けて `cmux __tmux-compat` として cmux アプリのローカルソケット API に転送。
- 公開 REST API ではなく CLI+socket/IPC。`cmux` CLI とソケット API で workspace/tab 作成・ペイン分割・キー送信も汎用スクリプト可能。
- Issue #2618: claude-teams / omo / omx / omc のサブエージェントペインが自動生成される（隠す手段の要望）。
