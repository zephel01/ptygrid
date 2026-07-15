# Troubleshooting

## Claude Code から queen MCP 経由で codex ペインに接続できなかった件(2026-07-16)

Claude Code(バックグラウンドジョブ)から queen の `send_message` / `read_output` で
codex にレビュー依頼を送ろうとした際、そのままではツールを呼べなかった。
原因は複数あり、それぞれ以下のように切り分け・対処した。
**恒久的な設定変更は行っていない**(すべてセッション内のワークアラウンド)。

### 1. queen の MCP ツールがセッションに存在しない

- **症状**: ToolSearch で `mcp__queen__send_message` / `mcp__queen__read_output` が
  「No matching deferred tools found」になる。
- **原因**: queen サーバーは `~/.claude.json` の **`/Users/h.yamamoto`(ホーム)プロジェクトのスコープ**に
  登録されていた(`projects./Users/h.yamamoto.mcpServers.queen`)。
  このセッションのプロジェクトは `~/works/project/multi-terminal` で、そこには queen の登録がなく、
  グローバルには `hiveterm` しか登録されていない。プロジェクトスコープが違うため
  MCP クライアントとして接続されず、ツールが読み込まれなかった。
- **対処(今回)**: 登録情報から queen のエンドポイント
  `http://127.0.0.1:39237/mcp`(Streamable HTTP)を特定し、`curl` で MCP プロトコルを直接叩いた。
- **恒久対応の候補**: multi-terminal プロジェクトでも使うなら
  `claude mcp add --transport http queen http://127.0.0.1:39237/mcp` を
  このプロジェクト(またはユーザースコープ `-s user`)で実行して登録する。
  ※ queen のポートが固定でない場合は起動時にポートが変わる点に注意。

### 2. サンドボックスのネットワーク制限で curl が失敗

- **症状**: `curl http://127.0.0.1:39237/mcp` が exit code 7(接続失敗)。
- **原因**: Claude Code の Bash サンドボックスは `allowedHosts` が空で、
  localhost を含む全ホストへのネットワーク接続がブロックされる。
- **対処(今回)**: 当該 curl コマンドのみサンドボックスを無効化して実行。
- **恒久対応の候補**: `/sandbox` コマンドで `127.0.0.1` / `localhost` を許可ホストに追加する。

### 3. MCP を直接叩く際のプロトコル手順

queen は rmcp 1.8.0 / Streamable HTTP(SSE レスポンス)。curl で使う場合の手順:

```sh
# 1) initialize — レスポンスヘッダの mcp-session-id を控える
curl -s -D - -X POST http://127.0.0.1:39237/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"1.0"}}}'

# 2) initialized 通知(以降すべてのリクエストに mcp-session-id ヘッダが必要)
curl -s -X POST http://127.0.0.1:39237/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: $SID" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'

# 3) tools/call(レスポンスは SSE。`data:` 行の JSON を取り出す)
curl -s -X POST http://127.0.0.1:39237/mcp ... \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_output","arguments":{"agent":"#2","lines":300}}}'
```

注意点:

- `Accept: application/json, text/event-stream` がないと拒否される。
- レスポンスは `text/event-stream` なので、`data:` 行を抽出して JSON パースする必要がある。

### 4. 「codex」という名前のエージェントは存在しない

- **症状**: `send_message` の宛先に `codex` を指定できると思っていたが、
  `list_agents` の結果は definitions が空で、セッションは `/bin/zsh` が 4 つ
  (#1, #2, #4, #5)あるだけだった。
- **原因**: codex は mterm.yml のエージェント定義ではなく、zsh ペイン内で
  手動起動された CLI だった(セッション #2)。
- **対処(今回)**: 各ペインを `read_output` で確認し、Codex CLI(v0.144.1)が
  動いている #2 を特定。宛先は `"#2"`(セッション ID 形式)を使用。

### 5. 入力欄に未送信のテキストが残っていた

- **症状**: #2 のペインを読むと、composer に
  `› src/session.rs をレビューして問題点を挙げて` が既にタイプ済み・未送信で残っていた。
- **リスク**: そのまま `send_message` で同じテキストを送ると入力が二重になる。
- **対処(今回)**: `send_message` を `{"text": "", "submit": true}` で呼び、
  Enter のみを送って既存テキストを送信した。
- **教訓**: queen 経由で送信する前に必ず `read_output` で composer の状態を確認する。

### その他のメモ

- TUI のペイン出力は ANSI 制御によるスピナー残骸(`Working…` の連打など)で汚れる。
  `\r` を改行に置換し、末尾数千文字だけ見ると読みやすい。
- 完了判定は「末尾に `esc to interrupt` が含まれず、かつ出力が 2 回連続で変化しない」
  ことをポーリング(10 秒間隔)で確認する方式が安定した。
- Codex は依頼受領後、リポジトリ内の RTK.md の指示も読み込んでから作業を開始していた。

---

## 実装側の対応状況（2026-07-15 反映済み）

上記の調査を受けて multi-terminal 本体に以下を反映した:

- **1. スコープ問題** → Queen バッジがコピーする登録コマンドを `claude mcp add -s user --transport http ...` に変更（README にも罠として明記）。
- **4. 「codex」を名前で指せない** → `list_agents` / `list_sessions` がフォアグラウンドプロセス名（`foreground`）を返すようになり、`read_output` / `send_message` の宛先解決が「定義名 → セッション名 → **fgプロセス名** → `#<id>`」に拡張された。zsh内で手動起動した codex も `"codex"` で指せる。エラー時の一覧にも `(fg: codex)` が出る。
- **その他メモ（スピナー残骸）** → `read_output` は ANSI 除去に加えて `\r` 上書きをターミナル準拠のオーバーレイ方式で畳み込み、各行の最終状態だけを返すようになった（`raw: true` で従来の生出力）。
- **5. composer 二重入力** → `send_message` のツール説明文に「送信前に read_output で composer 確認を推奨。`text: ""` + `submit: true` で Enter のみ送出可」を明記（エージェントが自力で気づけるように）。
- **2. サンドボックスの localhost 制限** → アプリ側では対処不能。Claude Code 側で `/sandbox` から `127.0.0.1` を許可するのが恒久対応（README に記載）。
- **完了判定のポーリング（その他メモ）** → 将来の `run_task` ツール（`codex exec` 等の非対話実行で完了まで待って綺麗な結果を返す）で解消予定。
