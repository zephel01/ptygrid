# 手動検証手順: Queen team preset（Phase 4.3 / v0.4.6）

対象: [spec-team-presets.md](spec-team-presets.md) の実装（v0.4.6）。
本書は **(A) 環境設定 → (B) 機能テスト T1〜T6 → (C) 実機偵察 R1〜R3（spec 8章）→
(D) 結果の記録** の順に実施する。機能テスト（B）は coderouter なしでも実施できる。
自動テスト（cargo test 210 / svelte-check 0 / build）は v0.4.6 タグ時点で通過済み。

---

## A. 環境設定

### A-1. ビルドと起動

```bash
cd ~/works/project/ptygrid
git status          # clean / v0.4.6 であること
npm install
npm run tauri dev   # 初回は Rust ビルドで数分
```

### A-2. coderouter（claude-code-router）— 偵察(C)で使用

ローカルLLM側の Claude Code は coderouter 経由で llama.cpp / ollama に向ける。
複数ローカルモデルの使い分けは **router 側の設定で行う**（ptygrid は env を渡すだけ）。

1. coderouter を起動し、llama.cpp / ollama の Provider とルーティングを router 側で設定
   （既定ポートは `3456`。詳細は claude-code-router の README を参照）
2. `curl http://127.0.0.1:3456/` 等で router が応答することを確認
3. **ptygrid を起動するシェルで** env をエクスポートしてから起動する
   （`${VAR}` はスポーン時にホスト環境から展開されるため）:

```bash
export CODEROUTER_URL="http://127.0.0.1:3456"
export CODEROUTER_TOKEN="dummy"     # router が認証不要なら任意の値
npm run tauri dev
```

> ⚠️ **コスト注意**: `CODEROUTER_URL` が未設定だと `${CODEROUTER_URL}` は**空文字**に
> 展開され、`local` ペインの Claude Code が**素の Anthropic API（課金）**に接続する。
> 偵察前に必ず `echo $CODEROUTER_URL` を確認すること。

### A-2b. settings.json 干渉への対策（重要）

Claude Code は env のほかに **`~/.claude/settings.json`（user）/ `.claude/settings.json`
（project）** も読む。優先順位は
`managed > CLI引数 > .claude/settings.local.json > project > user` で、settings の
`env` ブロックはセッションに適用される。**バージョンによってはプロセス env より
settings 側が勝つ**（v2.0.1 のリグレッション実績。issue は not planned でクローズ）ため、
ptygrid の `agents[].env` だけではルーティングが確実に効かないことがある。

- **推奨**: local エージェントは **`--settings` で専用ファイルを渡す**
  （CLI 引数スコープなので user / project 両方の settings に勝つ）:

  ```yaml
  - name: local
    cmd: "claude --settings router.settings.json"   # cwd からの相対パス
  ```

  `router.settings.json`（example/team-preset に同梱）:

  ```json
  { "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:3456",
             "ANTHROPIC_AUTH_TOKEN": "dummy" } }
  ```

- **やってはいけない**: プロジェクト直下に `.claude/settings.json` を置いて base URL を
  書く方法。**同じ cwd で動く `opus`（素の Claude Code）にも効いてしまい**、
  ローカル/クラウド混在チームが壊れる。
- `~/.claude/settings.json` に `env.ANTHROPIC_BASE_URL` 等を書いている場合は、
  `opus` 側がそれを拾って**逆にクラウドに繋がらない**事故も起きうる。user settings の
  env は素の状態（API/サブスク接続）を既定にしておくのが安全。
- 実際にどちらに繋がったかは **R1 の着弾確認**（coderouter のログ）で必ず検証する。

### A-3. 設定ファイル

```bash
# 試すだけなら: ツールバーの作業フォルダに example/team-preset を指定して読み込み
# 自プロジェクトで使うなら:
cp example/team-preset/ptygrid.yml ~/works/<対象プロジェクト>/ptygrid.yml
```

読み込み時に「このフォルダを信頼しますか？」が出たら「信頼して起動」を選ぶ
（preset 起動は autostart と同じ信頼ゲートの内側）。

### A-4. Queen 登録（初回のみ）

ツールバー右の「● Queen :39237」バッジをクリックし、コピーされた登録コマンドを実行:

```bash
claude mcp add -s user --transport http queen "http://127.0.0.1:39237/mcp?token=<token>"
grok mcp add -s user -t http queen "http://127.0.0.1:39237/mcp?token=<token>"
```

- `-s user` 登録1回で **local / opus 両方のペインに効く**（同じ claude バイナリのため）
- v0.4.3 以降トークンは永続化されているので、アプリ再起動での再登録は不要

---

## B. 機能テスト（coderouter 不要。`cmd: claude` を `cmd: /bin/cat` 等に
変えれば API 接続なしでも起動系のテストは全部できる）

### T1. 検証エラー（ロード拒否）

1. `ptygrid.yml` の preset の `agent: local` を `agent: typo` に変える → 保存 → Reload
2. **期待**: ロードがエラーになり、`team_presets.daily: member 'typo' is not defined
   under agents:` を含むメッセージが表示される。元に戻すと正常に読み込める
3. 追加で任意確認: `members: []` / 全員 `standby: true` / standby を `lead:` に指定 /
   同じ agent を2回宣言 — いずれも明確なエラーでロード失敗すること

### T2. 👥 一括起動

1. 正しい設定を読み込む → ツールバーに **👥 daily** チップが出る
2. ▶ をクリック
3. **期待**:
   - `local` のペインが1面追加される（`opus` / `grok` は起動しない = standby）
   - トーストに「チーム daily / 起動 1 / 待機 2、kickoff を lead へ送信」相当の要約
   - `local` ペインの Claude Code で「`list_inbox` で mailbox `local` を読んで」と頼むと、
     `queen:preset/daily` からの **instructions と kickoff の2通**が見える
   - `opus` の mailbox にも instructions が1通入っている（standby でも配送される）:
     ペイン外から確認するなら `python3 scripts/queen-send.py` ではなく
     Claude Code 経由で `list_inbox {mailbox: "opus", includeAcknowledged: true}` を呼ぶ

### T3. 冪等性（再クリック）

1. チーム稼働中にもう一度 👥 daily の ▶ を押す
2. **期待**: 新しいペインは増えない。トーストは「起動 0 / 既存 1 / 待機 2」相当で、
   kickoff 送信の表記が**出ない**。inbox の件数も増えていない（重複配送なし）

### T4. standby の個別起動

1. ツールバーの `opus` エージェントチップの ▶ で起動（または T6 の spawn_agent）
2. **期待**: opus のペインが開く。opus に「自分の mailbox (`opus`) を `list_inbox` で
   読んで」と頼むと、チーム起動時に配送済みの役割指示が読める

### T5. ペイン上限（部分起動）

1. 一時的に `grok` の `standby: true` を削除して Reload（非 standby を2体にする）
2. 全ペインを閉じ、シェルを**8面**開く（「+」またはツールバーの一括オープン）
3. 👥 daily の ▶ を押す
4. **期待**: `local` が最後の1枠（9面目）で起動し、`grok` は起動されず
   **失敗(pane limit)** としてトースト/バナーに表示される。起動できた分はそのまま
   使える（全体が拒否されない = 部分起動）
5. 確認後、`grok` の `standby: true` を戻して Reload

### T6. Queen tool `spawn_team`

1. 全ペインを閉じて claude を1面だけ手動起動（または既存 local ペインを使用）
2. その Claude Code に「**Queen の spawn_team を preset "daily" で呼んで、結果の JSON を
   そのまま見せて**」と頼む
3. **期待**: UI と同じ挙動（ペイン追加・トースト）+ ツールの戻り値に
   `{preset, lead, members[{agent,standby,status,id}], kickoffDelivered}` が入っている。
   `preset: "nope"` では invalid_params エラーになる

---

## C. 実機偵察 R1〜R3（spec 8章 — coderouter 構成）

目的: エスカレーションの**既定パターン（エージェント発 / 人間発）を決める**こと。

### R1. ローカルLLMの Queen 接続品質（+ ルーティング着弾確認）

1. A-2 / A-2b の設定で ptygrid を起動し、👥 daily で `local` を起動
2. **着弾確認（最初に必ず）**: coderouter のログ（またはコンソール）を見ながら `local` に
   何か1つ話しかけ、**リクエストが router に着弾している**ことを確認する。
   着弾していなければ settings.json 干渉を疑い、A-2b の `--settings` 方式に切り替える。
   逆に `opus` を起動したときは router に**着弾しない**ことも確認する
3. `local` に順に頼む: 「`list_agents` を呼んで」「`list_inbox` で自分の mailbox を読んで」
   「`set_pin` で key=test を書いて」
4. **記録**: 各ツール呼び出しが1発で通るか / 引数の組み立てを間違えるか / 何回言い直したか

### R2. エスカレーション一連（エージェント発）

1. `local` にそのまま頼む: 「**難問が来たと想定して、instructions の手順どおり
   opus にエスカレーションして**」
2. **期待する一連**: `spawn_agent {name:"opus"}` → opus ペイン追加 →
   `send_inbox {recipient:"opus", ...}` → `await {mailbox:"local"}` で待機 →
   （opus 側で `list_inbox` → `reply_inbox`）→ local が回答を受けて要約
3. **記録**: どこで詰まるか（spawn までは行くが await を使わずポーリングする、等）。
   モデルを替えて（router 側で切替）差も見る

### R3. 混在の確認

1. R2 の状態で `opus`（素の Claude Code）と `local`（router 経由）が同時に動いていることを確認
2. **期待**: 両ペインとも Queen ツールが使える（登録1回で両方に効く）。ペインヘッダーは
   定義名（local / opus）で区別され、`#id` 指定も通常どおり機能する

### 判定

- R2 が安定して成立 → **エージェント発**を既定パターンとして spec 7章に確定
- 成立しない/不安定 → **人間発**（standby を UI ▶ で起動し、依頼文を人間が送る）を
  既定にし、instructions の書き方を「依頼が来たら reply_inbox で返す」側に寄せる

---

## D. 結果の記録

1. R1〜R3 の結果と判定を [spec-team-presets.md](spec-team-presets.md) 8章の末尾に追記
   （使ったローカルモデル名・router 設定の要点も一行残す）
2. エスカレーション既定パターンを spec 7章に反映（必要なら
   `example/team-preset/ptygrid.yml` の instructions も追随）
3. [plan.md](plan.md) の Phase 4.3 残タスクから「実機偵察」を消し込む
4. 挙動修正が必要になった場合は v0.4.7 として通常のリリース規律
   （CONTRACT → 実装 → テスト → タグ）で対応

## 回帰チェック（コード変更を伴った場合のみ）

```bash
npm run check && npm run build
cd src-tauri && cargo check && cargo test && cargo clippy --all-targets --all-features
```
