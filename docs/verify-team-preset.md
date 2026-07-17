# 手動検証手順: Queen team preset（Phase 4.3 / v0.4.6）

対象: [spec-team-presets.md](spec-team-presets.md) の実装（v0.4.6）。

---

## 0. ゴール — この検証で「何ができる」を確認するのか

この機能のゴールはただ一つ:

> **「普段はローカルLLM、難しい問題だけ Opus/Grok」という毎日の作業体制を、
> 👥 を1回押すだけで再現できる。**

これまで手作業だった「ペインを並べる → 各エージェントに役割を貼り付ける →
最初のタスクを投げる → 難問のときだけクラウドを立ち上げて依頼文を書く」が
すべて宣言（ptygrid.yml）+ 1操作になる、が実現価値である。合格条件は4つ:

| # | ゴール | 判定基準 | 対応テスト |
|---|---|---|---|
| G1 | **ワンクリック再現** | 👥 1回で「local 起動 + 役割配布 + kickoff 着手」まで人手ゼロ。2回押しても壊れない | T2, T3, E2E-1 |
| G2 | **役割の自動配布** | 誰にも手で指示を貼らない。standby の opus も、後から起動した時点で自分の役割を知っている | T2, T4, E2E-1 |
| G3 | **コスト階層が機能** | 日常作業でクラウド呼び出しゼロ（router 着弾ログで確認）。クラウドが動くのは難問エスカレーション時だけ | R1, E2E-2 |
| G4 | **エスカレーションが回る** | 難問時に local → opus の依頼と回答が（エージェント発 or 人間発で）成立し、結果が local の作業に反映される | R2, E2E-2 |

B/C の個別テストは上記の分解であり、**最終判定は E2E シナリオ（G章）で行う**。
E2E が通れば個別テストの多少の飛ばしは許容してよい。逆に個別テストが全部通っても
E2E が通らなければこの機能は未完成である。

---

本書は **(A) 環境設定 → (B) 機能テスト T1〜T6 → (C) 実機偵察 R1〜R3（spec 8章）→
(G) E2E 受け入れシナリオ → (D) 結果の記録** の順に実施する。機能テスト（B）は
coderouter なしでも実施できる。
自動テスト（cargo test 210 / svelte-check 0 / build）は v0.4.6 タグ時点で通過済み。

---

## A. 環境設定

### A-0. 起動順序チェックリスト（毎回この順で）

**下から順に依存している**ため、必ずこの順で立ち上げる。各ステップの確認を
飛ばさないこと（次のステップの失敗原因が切り分けられなくなる）。

| # | 起動するもの | コマンド例 | 確認方法 |
|---|---|---|---|
| 1 | **ローカルLLMバックエンド**（llama.cpp / ollama） | `llama-server -m <model>.gguf --port 8080` / `ollama serve` | `curl http://127.0.0.1:8080/health` / `ollama list` が応答する |
| 2 | **coderouter**（1 に接続する設定済みであること） | `ccr start` 等 | `curl http://127.0.0.1:3456/` が応答し、router のログが流れ始める |
| 3 | **env のエクスポート**（ptygrid を起動する**その**シェルで） | `export CODEROUTER_URL="http://127.0.0.1:3456"`<br>`export CODEROUTER_TOKEN="dummy"` | `echo $CODEROUTER_URL` が空でない |
| 4 | **ptygrid**（3 と同じシェルから） | `npm run tauri dev` | ウィンドウが開き、シェルペインが1面出る |
| 5 | **作業フォルダの読み込み** | ツールバーに対象フォルダを入力 → 読み込み（初回は「信頼して起動」） | エージェントチップと **👥 daily** チップが並ぶ |
| 6 | **Queen 登録**（初回のみ。トークンは v0.4.3+ で永続） | Queen バッジをクリック → コピーされたコマンドを任意のターミナルで実行 | `claude mcp list` に queen が出る |
| 7 | **チーム起動** | 👥 daily の ▶ を1回 | local ペインが開き、トーストに起動サマリ。router ログに着弾（R1） |
| 8 | **opus / grok（standby）** | 起動しない（待機）。必要時に ▶ または local の `spawn_agent` | — |

**順序を守る理由**: local ペインの Claude Code は**起動した瞬間に** base URL へ接続する。
2 より先に 7 をやると router 不在で接続エラーになり、3 を飛ばすと `${CODEROUTER_URL}` が
空文字に展開されて**素の Anthropic API（課金）に接続**する（A-2 の警告）。

> ⚠️ **Dock / Finder から ptygrid.app を起動しない**（この検証では）。GUI 起動は
> シェルの export を継承しないため 3 が無効になる。必ず 3 と同じシェルから
> `npm run tauri dev`（またはターミナル経由で app）を起動する。なお A-2b の
> `--settings router.settings.json` 方式は**ファイルベースなので GUI 起動でも効く**。
> env に頼らない運用に固めるなら、こちらを正とするのが安全。

**終了は逆順**が安全: ペインを閉じる → ptygrid 終了 → coderouter → バックエンド。
（逆順にしない場合も壊れはしないが、router 切断エラーがペインに残って紛らわしい）

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

### R2. エスカレーション一連 — 「機構」と「発火」を分けて検証する

> **所見（2026-07-17）**: 「難しい問題を投げれば詰まってエスカレーションするはず」は
> 成立しない。ローカルモデルは**自信を持って普通に答えてしまう**（正しいかは別）ため、
> 難易度の自己判断はトリガーとして機能しない。よって検証を2段に分ける。

**R2a. 機構の検証（訓練モード = 強制委譲）**

1. `local` に貼る: 「**エスカレーション訓練です。質問の内容や自分が答えられるかに
   関係なく、必ず次の手順を実行**: `spawn_agent` で "opus" を起動 → `send_inbox` で
   recipient "opus" に質問文と自分の暫定回答を送る → `await` で mailbox "local" への
   返信を待つ → opus の回答と自分の暫定回答の差分を要約する。質問: 〜」
2. **期待する一連**: `spawn_agent` → opus ペイン追加 → `send_inbox` →
   `await` 待機 →（opus 側で `list_inbox` → `reply_inbox`）→ local が差分を要約
3. **記録**: どこで詰まるか（spawn までは行くが await を使わずポーリングする、等）。
   モデルを替えて（router 側で切替）差も見る

**R2b. 発火の検証（客観トリガー）**

instructions は「詰まったら」ではなく**客観条件**で書く（example/team-preset 参照:
①同一原因でテスト/ビルド2回連続失敗 ②公開API/保存データ/セキュリティ境界/並行処理に
触る変更 ③人間の一言）。検証は条件を**わざと成立させて**行う。

**条件①の仕込み方（重要）**: `assert_eq!(1, 2)` のような明らかに壊れたテストは
**使えない** — 賢いモデルは「テストが間違っている」と見抜いて直してしまい、失敗が
続かない。正しい仕込みは「**実装もテストも触ってはいけない制約の下で、満たせない
要求**」にすること。2回試行しても必ず失敗し、「要件が矛盾している → 判断を仰ぐ」に
到達する = 実運用でエスカレーションすべき状況の再現になる。

1. 条件①: `src-tauri/tests/drill_escalation.rs`（**コミット禁止・訓練後削除**）に
   「上限は12であること」のような**現仕様(9)と矛盾する要求**のテストを置き、
   タスクは「テストを通せ。ただしテスト変更禁止・9面上限も変更禁止（フロントと
   同期のため）」と制約付きで依頼 → 2回失敗した時点でエスカレーションが発火するか。
   opus の期待回答は「要求が矛盾。どちらを緩めるかは人間の判断」
2. 条件②: 「`CONTRACT.md` に触る変更をして」等、条件に当たるタスクを依頼 →
   完了報告の前に opus レビューを取りに行くか
3. **記録**: 発火した/しなかった条件と、無視された場合の言い直し回数。
   制約無視（テスト書き換え）や無限試行も所見として記録する。
   終了後は drill ファイルを削除し `git status` がクリーンなことを確認

### R3. 混在の確認

1. R2 の状態で `opus`（素の Claude Code）と `local`（router 経由）が同時に動いていることを確認
2. **期待**: 両ペインとも Queen ツールが使える（登録1回で両方に効く）。ペインヘッダーは
   定義名（local / opus）で区別され、`#id` 指定も通常どおり機能する

### 判定

- R2 が安定して成立 → **エージェント発**を既定パターンとして spec 7章に確定
- 成立しない/不安定 → **人間発**（standby を UI ▶ で起動し、依頼文を人間が送る）を
  既定にし、instructions の書き方を「依頼が来たら reply_inbox で返す」側に寄せる

---

## G. E2E 受け入れシナリオ — 「1日の仕事」を1本通す（最終判定）

個別テストではなく、**実際の作業を1タスク流して**ゴールを判定する。所要 30〜60 分。
対象は実プロジェクト（例: ptygrid 自身の軽い issue や、手元の小さな改修タスク）。

### E2E-1: 日常編（G1 + G2 + G3）

1. 実プロジェクトに team-preset 設定を置き、`kickoff` に**本物の小タスク**を書く
   （例: 「`docs/xxx.md` の誤記を直して差分を見せて」）
2. ptygrid を起動し、**操作は 👥 daily の ▶ 1回だけ**。以降キーボードに触らず観察する
3. **合格**:
   - local が自分で inbox を読み（または kickoff を受けて）、タスクに着手・完遂する
   - その間、coderouter のログにだけリクエストが流れる（**クラウド課金ゼロ** = G3）
   - 人間がやったことが「▶ を1回押した」だけである（= G1/G2）
4. **不合格の典型**: local が inbox を自発的に読まない → kickoff の文面に
   「まず `list_inbox` で自分宛の指示を読め」を入れる運用で再試行し、結果を記録

### E2E-2: エスカレーション編（G3 + G4）

> 「わざと難しい依頼を投げる」方式は使わない。ローカルモデルは難問にも自信を持って
> 普通に答えてしまい、自己判断トリガーは発火しない（R2 所見）。**客観トリガーを
> わざと成立させて**流す。

1. E2E-1 の続きで、**トリガー条件に当たる実タスク**を投げる。例:
   - 条件①型: 必ず落ちるテストを仕込んでおき「テストを直して通して」
   - 条件②型: 「CONTRACT.md の契約に触る小変更」を依頼（完了前 opus レビュー必須の条件）
2. instructions の規約どおり local が `spawn_agent("opus")` → inbox 依頼 → `await` まで
   自力で進むかを観察（進まなければ「opus に聞いて」と一言だけ促す = 条件③。
   それでもダメなら人間が ▶ で opus を起動し依頼文を送る = 人間発フォールバック）
3. **合格**:
   - どの経路であれ **opus の回答が local の作業に反映されて**タスクが完了する
   - opus が動いたのはこのエスカレーション区間だけ（router ログ + API 使用量で確認 = G3）
   - 完了後、opus のペインを閉じて日常編の体制に戻れる
4. **記録**: どのトリガーで回ったか（客観条件の自力発火 / 人間の一言 / 完全人間発）。
   これが spec 7章の**既定パターンの決定**になる（C 章の判定と同じ）

### E2E の不合格 = 機能の未完成

E2E が通らない原因が ptygrid 側（配送されない・skip されるべきが重複する等）なら
バグとして v0.4.7 で修正。モデル側（指示に従わない）なら instructions / kickoff の
文面パターンを userguide に追記して再試行する。

## D. 結果の記録

1. R1〜R3 と E2E（G章）の結果・G1〜G4 の合否を
   [spec-team-presets.md](spec-team-presets.md) 8章の末尾に追記
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
