**日本語** · [English](troubleshooting.en.md)

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
- **追加症状**: 「`grok #2で作業させて`」と頼んでも、Claudeがheadless実行や
  ユーザーによる別タブへの貼り付けを提案する。
- **判定**: モデル能力の問題ではなく、`claude mcp list`に`queen`が表示されないため
  `#2`をptygrid session IDとして解決するtoolが見えていない。
- **復旧**: 上記のuser scope登録後、**Claude Codeセッションを再起動またはresume**する。
  MCP tool一覧は起動時に読み込まれるため、登録前から動いているsessionには即時反映されない。

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
  ペイン寸法に合わせてcursor/erase/alternate screenを再構成する。調査時だけ`raw: true`を使う。
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
- **その他メモ（スピナー残骸）** → `read_output`は当初ANSI除去と`\r`畳み込みを実装し、その後Grok TUIの事例を受け、ペイン寸法を使ったcursor/erase/alternate screen再構成へ拡張した（`raw: true`で従来の生出力）。
- **6. composer 二重入力** → `send_message`のtool descriptionに「送信前にread_outputで
  composer確認を推奨。`text: ""` + `submit: true`でEnterのみ送出可」を明記。
- **2. サンドボックスの localhost 制限** → アプリ側では対処不能。Claude Code 側で `/sandbox` から `127.0.0.1` を許可するのが恒久対応（README に記載）。
- **完了判定のポーリング（その他メモ）** → Phase 3.8のcancellable Queen `await`で
  cursor-based waitとbounded timeoutを提供済み。Inbox待機には`await`を使い、terminal出力の
  完了判定とは区別する。

---

## `queen-send.py`が止まったように見える（Grok TUI、2026-07-16の事例）

Phase 3.8のNote原稿をGrok `#2`へ依頼した際、Claude Code側ではbackground commandが
長時間実行中に見えた。調査したログでは次の順に処理されていた:

- `sent to #2`が記録され、依頼は最初の`send_message`で到達していた。
- `no activity — sent extra Enter`は記録されておらず、追加Enterのnudgeは発動していない。
- Grokは約1分後にファイルを書き、`執筆完了`を出力した。
- `queen-send.py`はその後、出力が2回連続で静止したことを確認して正常終了した。

したがって、この事例は**コマンド未送信ではなく、出力監視と完了判定の遅延**だった。
Grokのalternate-screen TUIはspinner、経過時間、画面全体の再描画を大量に出すため、
`read_output`の結果が約45 KiBまで膨らみ、見た目上の更新も続く。ANSI除去と`\r`畳み込み後にも
描画断片が残る場合があり、「出力が静止した」という判定が応答本文の完成より遅れる。

切り分け時は以下を確認する:

1. stderrに`sent to #<id>`があるか。あればQueenへの送信呼び出しは成功している。
2. `no activity — sent extra Enter`があるか。あれば初回送信後に出力変化がなく、nudgeが発動した。
3. 宛先ペインの`read_output`だけでなく、依頼された成果物の更新時刻・内容も確認する。
4. 完了後も待っている場合は、送信問題と決めつけずTUIの継続再描画を疑う。

この事例を受け、`read_output`はANSIを単純に削除する方式から、ペインのrows/colsを使って
cursor移動、画面/行消去、save/restore cursor、alternate screenを適用する軽量VT再構成へ
変更した。これにより過去の全画面再描画が巨大な1行へ連結される問題を防ぐ。

なお`queen-send.py`はterminal出力の静止を汎用的な完了シグナルとして使うため、現在画面の
内容自体を更新し続けるTUIでは遅延し得る。`await`はInbox用であり、terminal TUIの応答完了を
直接待つtoolではない。将来は明示的な完了通知、成果物確認、またはTUI別の完了条件を検討する。

---

## Pins / Notesで`conflict`になる

- **症状**: `set_pin` / `update_note` / delete系toolがrevision conflictを返す。
- **原因**: 読み取った後に別agentが同じrecordを更新または削除した。
- **対処**: `list_pins`または`get_note`で最新版とrevisionを取得し、自分の変更をmergeしてから
  新しい`expectedRevision`でretryする。
- **注意**: conflict時のmutationはtransactionごとrollbackされる。別agentの新しい内容を
  上書き・削除していないため、errorを無視してblind retryしない。

---

## Inboxにmessageが見つからない

- **`#3`をmailboxにした**: Inboxはapp再起動をまたぐためsession IDを拒否する。
  `codex-review`などのstable role名で`send_inbox` / `list_inbox`を呼ぶ。
- **ack済み**: `list_inbox`はdefaultで未ackだけを返す。履歴確認時は
  `includeAcknowledged: true`を指定する。
- **別project**: Inboxは読み込まれた`mterm.yml`のcanonical directory単位で分離される。
  senderとrecipientが同じでも別projectからは見えない。
- **replyできない**: `reply_inbox.sender`は元messageのrecipientと完全一致する必要がある。
  messageの`recipient`を確認し、表示名やsession IDへ置き換えない。

### `await`が空で返る

- `timedOut: true`ならerrorではなく正常なdeadline到達。返された`nextCursor`で再度待てる。
- `afterId`より小さいIDは返らない。履歴確認は`list_inbox`で`afterId: 0`を使う。
- defaultではack済みmessageを除外する。必要なら`includeAcknowledged: true`を指定する。
- cancellation errorはMCP clientがrequestをcancelした結果。messageやack状態は変更されていない。
