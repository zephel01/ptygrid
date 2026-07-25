# Queen MCP「汎用コピー」設計案

現行パネルは `claude 登録コマンド` / `codex スニペット` / `grok スニペット` の3ボタン構成。
新しいエージェント CLI が出るたびにボタンを増やさなくても済むよう、
「どのツールにも貼れる汎用コピー」を用意するための設計。

---

## 0. 前提（ソースから確認した事実）

- Queen は `rmcp`（公式 Rust SDK）/ streamable HTTP。既定エンドポイントは `http://127.0.0.1:39237/mcp`。
- `/mcp` は **token + Host/Origin 検証**で保護。認証トークンは**2経路**を受け付ける:
  1. クエリ: `...?token=<token>`（claude ボタンが使用）
  2. ヘッダ: `Authorization: Bearer <token>`（codex/grok の `bearer_token_env_var = "QUEEN_TOKEN"` はこれを送っている）
- トークンは永続化され再起動後も有効。ただし**再生成すると URL 埋め込み型は貼り直しが必要**、
  env 参照型（`QUEEN_TOKEN`）はそのまま有効。

### 汎用化の肝

「認証の渡し方」は結局この2つしかない:

| 認証チャネル | 貼り直し | 対応フォーマット | 備考 |
|---|---|---|---|
| URL クエリ `?token=` | 再生成で必要 | URL / JSON | ヘッダも env も要らない**最も互換性が高い**渡し方 |
| env `QUEEN_TOKEN`（→ Bearer） | 不要（stale-proof） | TOML(`bearer_token_env_var`) | codex/grok 系 |

JSON の `headers` に `${QUEEN_TOKEN}` を書く方式は **採らない**。
claude-code では `.mcp.json` の headers 内 env 展開が効かない不具合報告があり
（[#6204](https://github.com/anthropics/claude-code/issues/6204),
[#51581](https://github.com/anthropics/claude-code/issues/51581)）、
ツール横断で信用できないため。JSON はトークンを URL に埋めるのが確実。

---

## 1. 最小構成（"最低限でよい" 案）

真に汎用な原始プリミティブは **トークン込み URL** 1本。
HTTP エンドポイント URL を受け付ける MCP クライアントなら、ヘッダも env も設定不要で通る。

```
http://127.0.0.1:39237/mcp?token=<token>
```

新ツールが来ても「MCP サーバーの HTTP URL を入れる欄」にこれを貼るだけ。
**汎用ボタンを1個だけ足すなら、これをコピーさせるのが正解。**
（デメリットは再生成で貼り直しが要る点。claude ボタンと同じ挙動なので既存ユーザーの理解と一致する）

---

## 2. 推奨構成（3コピー）

ユーザー選択の 標準JSON / 汎用TOML / 生の値 をそのまま採用。
`<token>` `39237` はバッジの実値で置換する（現行コードの `q.url ?? http://127.0.0.1:${q.port}/mcp`、`q.token`）。

### 2-1. 標準 mcpServers JSON（最も広く通る）

Cursor / Cline / VS Code / Gemini CLI / Claude Desktop 等が読む形。
env 展開に頼らずトークンを URL に埋める（stale-proof ではないが最も確実）:

```json
{
  "mcpServers": {
    "queen": {
      "type": "http",
      "url": "http://127.0.0.1:39237/mcp?token=<token>"
    }
  }
}
```

env で stale-proof にしたいツール向けの別案（ヘッダ + env、対応ツール限定）:

```json
{
  "mcpServers": {
    "queen": {
      "type": "http",
      "url": "http://127.0.0.1:39237/mcp",
      "headers": { "Authorization": "Bearer ${QUEEN_TOKEN}" }
    }
  }
}
```

> 注: 後者の `${QUEEN_TOKEN}` は展開できないツールがある。既定は前者（URL 埋め込み）を推奨。

### 2-2. 汎用 TOML テーブル（codex/grok 系そのまま）

env 参照なので**再生成後もそのまま有効**。TOML 系 CLI に流用可能:

```toml
[mcp_servers.queen]
url = "http://127.0.0.1:39237/mcp"
bearer_token_env_var = "QUEEN_TOKEN"
```

### 2-3. 生の値（手貼り用の素材）

新ツールがどの形式でも、この4つがあれば手で組める:

```
エンドポイント URL : http://127.0.0.1:39237/mcp
トークン           : <token>
env 変数名         : QUEEN_TOKEN
トークン込み URL   : http://127.0.0.1:39237/mcp?token=<token>
```

---

## 3. パネル UI 案（最小差分）

既存の footnote（`codex / grok は QUEEN_TOKEN env 参照…`）の下に「汎用」行を1つ足すだけ:

```
[claude 登録コマンド] [codex スニペット] [grok スニペット]
─────────────────────────────
汎用: [URL をコピー] [JSON をコピー] [生の値をコピー]
```

最小で行くなら `[URL をコピー]`（= 1章のトークン込み URL）1個でも実用十分。

### 実装スケッチ（App.svelte、既存関数と同じ体裁）

```ts
/** 汎用: トークン込み URL。HTTP URL を受け付ける任意の MCP クライアントに貼れる。 */
async function copyUniversalUrl(): Promise<void> {
  const q = ui.queenStatus;
  if (!isTauri() || !q || (!q.url && !q.port)) return;
  await navigator.clipboard.writeText(queenRegisterUrl(q)); // 既存関数を再利用（?token= 付き）
  addNotice(m.universalUrlCopied, queenRegisterUrl(q));
}

/** 汎用: 標準 mcpServers JSON（URL 埋め込み型。env 展開に依存しない）。 */
async function copyUniversalJson(): Promise<void> {
  const q = ui.queenStatus;
  if (!isTauri() || !q || (!q.url && !q.port)) return;
  const url = queenRegisterUrl(q);
  const snippet = JSON.stringify(
    { mcpServers: { queen: { type: "http", url } } }, null, 2,
  );
  await navigator.clipboard.writeText(snippet);
  addNotice(m.universalJsonCopied, snippet);
}
```

`copyRawValues()` は `queenBaseUrl(q)` / `q.token` / `"QUEEN_TOKEN"` を組み立てるだけ。
i18n は `universalUrlCopied` / `universalJsonCopied` / `rawValuesCopied` と
ボタンラベル `btnUniversalUrl` 等を追加（既存 `queenCmdCopied` 等と同じ並び）。

---

## 4. まとめ（意思決定）

- **最低限**: 汎用ボタン1個 = トークン込み URL（1章）。互換性最強・実装最小。
- **推奨**: それに 標準JSON（URL 埋め込み型）と 生の値 を足した3コピー（2章）。
- JSON の env 展開ヘッダ方式は**既定にしない**（ツール差で壊れる）。
  stale-proof が欲しいユーザーには TOML の `bearer_token_env_var` を案内。
- Host/Origin 検証があるため、リモート/プロキシ越しではなくローカル起動の CLI が前提。
