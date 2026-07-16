# Troubleshooting

## Queen MCPから別ペインへ接続できない(2026-07-16の事例)

Claude Code(バックグラウンドジョブ)から queen の `send_message` / `read_output` で
codex にレビュー依頼を送ろうとした際、そのままではツールを呼べなかった。
原因は複数あり、それぞれ以下のように切り分け・対処した。
初回調査はsession内のworkaroundで切り分け、その後の実装対応は末尾にまとめている。

### 1. queen の MCP ツールがセッションに存在しない

- **症状**: ToolSearch で `mcp__queen__send_message` / `mcp__queen__read_output` が
  「No matching deferred tools found」になる。
- **原因**: Queen serverがClaude Codeの別project scopeに登録されていた。
  対象sessionのprojectは`~/works/project/ptygrid`で、そこにはQueenの登録がなく、
  グローバルには `hiveterm` しか登録されていない。プロジェクトスコープが違うため
  MCP クライアントとして接続されず、ツールが読み込まれなかった。
- **対処(今回)**: 登録情報から queen のエンドポイント
  `http://127.0.0.1:39237/mcp`(Streamable HTTP)を特定し、`curl` で MCP プロトコルを直接叩いた。
- **恒久対応**: directoryをまたいで使う場合は
  `claude mcp add -s user --transport http queen http://127.0.0.1:39237/mcp`で
  user scopeへ登録する。projectだけで共有するなら`-s project`を使う。
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

### 4. 手動起動したCodexのsession IDが分からない

- **症状**: `send_message` の宛先に `codex` を指定できると思っていたが、
  `list_agents` の結果は definitions が空で、セッションは `/bin/zsh` が 4 つ
  (#1, #2, #4, #5)あるだけだった。
- **原因**: codex は mterm.yml のエージェント定義ではなく、zsh ペイン内で
  手動起動された CLI だった(セッション #2)。
- **対処(今回)**: pane headerと`list_agents`を確認し、Codexが動いているIDを特定。
  宛先は`"#2"`のようなsession ID形式を使用する。現在はforeground process名も返すため、
  shell内で手動起動したCLIを発見できる。

### 5. `agent: "codex"`がambiguous errorになる

- **症状**: `use one of: #3, #5`のようなerrorで送信されない。
- **原因**: 同名の定義、またはforeground processが複数実行中。
- **対処**: pane headerまたは`list_agents`で相手を確認し、`agent: "#3"`のように厳密指定する。
- **設計意図**: 最新sessionを推測して別ペインへ誤送信しないための正常な安全動作。

### 6. 入力欄に未送信のテキストが残っていた

- **症状**: #2 のペインを読むと、composer に
  `› src/session.rs をレビューして問題点を挙げて` が既にタイプ済み・未送信で残っていた。
- **リスク**: そのまま `send_message` で同じテキストを送ると入力が二重になる。
- **対処(今回)**: `send_message` を `{"text": "", "submit": true}` で呼び、
  Enter のみを送って既存テキストを送信した。
- **教訓**: queen 経由で送信する前に必ず `read_output` で composer の状態を確認する。

### その他のメモ

- TUIのraw出力はANSI制御や`\r`上書きによるspinner残骸で汚れる。通常の`read_output`は
  ANSI除去とterminal準拠の上書き畳み込みを行う。調査時だけ`raw: true`を使う。
- 完了判定は「末尾に `esc to interrupt` が含まれず、かつ出力が 2 回連続で変化しない」
  ことをポーリング(10 秒間隔)で確認する方式が安定した。
- Codex は依頼受領後、リポジトリ内の RTK.md の指示も読み込んでから作業を開始していた。

---

## 実装側の対応状況（2026-07-16、Phase 3.6）

上記の調査を受けてptygrid本体に以下を反映した:

- **1. スコープ問題** → Queen バッジがコピーする登録コマンドを `claude mcp add -s user --transport http ...` に変更（README にも罠として明記）。
- **4. 手動起動CLIの識別** → `list_agents` / `list_sessions`がforeground process名
  (`foreground`)を返す。宛先は`#<id>`を最優先、次に一意な定義/session名、最後に一意な
  foreground名で解決する。複数matchは候補IDを返して拒否する。
- **その他メモ（スピナー残骸）** → `read_output` は ANSI 除去に加えて `\r` 上書きをターミナル準拠のオーバーレイ方式で畳み込み、各行の最終状態だけを返すようになった（`raw: true` で従来の生出力）。
- **6. composer 二重入力** → `send_message`のtool descriptionに「送信前にread_outputで
  composer確認を推奨。`text: ""` + `submit: true`でEnterのみ送出可」を明記。
- **2. サンドボックスの localhost 制限** → アプリ側では対処不能。Claude Code 側で `/sandbox` から `127.0.0.1` を許可するのが恒久対応（README に記載）。
- **完了判定のポーリング（その他メモ）** → Phase 3.8のcancellable Queen `await`で
  cursor-based waitとbounded timeoutを提供する予定。

---

## Pins / Notesで`conflict`になる

- **症状**: `set_pin` / `update_note` / delete系toolがrevision conflictを返す。
- **原因**: 読み取った後に別agentが同じrecordを更新または削除した。
- **対処**: `list_pins`または`get_note`で最新版とrevisionを取得し、自分の変更をmergeしてから
  新しい`expectedRevision`でretryする。
- **注意**: conflict時のmutationはtransactionごとrollbackされる。別agentの新しい内容を
  上書き・削除していないため、errorを無視してblind retryしない。
