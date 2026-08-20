# 次のリリース v0.5.10 の整理

作成日: 2026-08-20 / 実装基準: `main` = `d31a43a`（PR #20 マージ後）+
`fix/cross-run-pane-adoption` = `ccbdf9b`
前段: [next-release-v0.5.9.md](next-release-v0.5.9.md)（v0.5.9 の整理。§3 の実機検証計画と
§4 の「並行 run 間でペインが横取りされうる」がそのまま本リリースの入力になっている）
方針: **v0.5.9 が返せなかった負債＝実機検証を、今度は本当に返す**

---

## 0. 結論（先に）

v0.5.10 は **2 つのコード変更 + 実機検証の消化**である。新機能は入れない。

1. **pane connection context**（Phase 4.4.4）— `feat/pane-connection-context` として
   v0.5.9 のあとに `main` へマージ済み（PR #20）。ペインのヘッダとステータスバーに
   作業ディレクトリ / git ブランチ / AWS プロファイル / LLM エンドポイントを出す。
   **タグが無いだけで実装は入っている**ので、v0.5.10 の主たる中身はこれ。
2. **並行 run 間のペイン横取りの修正**（`ccbdf9b`）— v0.5.9 の整理 §4 が指摘して
   「実機再現は未実施」のまま残していたもの。実機セッションの前に塞いだ。
   → plan.md §6.26 / CONTRACT.md 続報24。
3. **実機検証**: U10 は**消化済み**（2026-08-20）。U4 / U16 / U17 / U18 / U19 は
   検証キットを用意した状態で未実施。

**スコープに入れないもの**（意識して外した 2 件、いずれも別枠）:

- **5.5.0 の完成（F-1 / F-2 / F-3）** — U10 で見つかった RC ルートの 3 件。
  設計判断（RC の `initialize` を rmcp に通すか / RC 応答に session id を返すか）を
  含むので、v0.5.10 に混ぜない。→ plan.md §1 の「5.5.0 の完成」行、§3 P6。
- **ordinary step の mailbox を run スコープにする** — 同梱サンプル 3 本が
  agent 名を直書きしているため、移行方針とセットでないと入れられない。
  → plan.md §3 バックログ。U4 で実機の姿を見てから決める。

---

## 1. コード変更 2 件

### 1.1 pane connection context（PR #20、`1e6bcc5` + `8d59100`）

v0.5.9 タグ後に `main` へ入った。手順書は `docs/guide/pane-context.md`、
実機の手動テスト項目は `docs/note/pane-context-test-and-merge.md` §2 にある
（個人メモなので git 管理外）。**このリリースで初めてタグに載る。**

`ptygrid.yml` に `pane_context:` ブロックが増えている（`enabled` / `interval_ms`）。
既定は有効。`ptygrid.example.yml` に説明あり。

### 1.2 並行 run 間のペイン横取り（`ccbdf9b`）

詳細は plan.md §6.26 / CONTRACT.md 続報24。要点だけ:

- `agent_claimed_by_other_step` は自分の run しか見えないのに、同名ペインの探索は
  `PtyManager` 全体を舐める → 2 本目の run が 1 本目のペインを adopt する。
- 1 つの `session_id` を 2 つの run が持つので、**1 回の exit で両方の step が完了**。
  `slots_needed` も 0 と数えるので 9 面上限を抜ける。
- `WorkflowRegistry::panes_claimed_by_other_runs` で他の**生きている** run のペインを
  adopt 候補から外した。終端 run のペインは従来どおり再利用可。
- 再利用判定を `adoptable_session` 1 か所に集約（`slots_needed` と `spawn_step` が
  手で足並みを揃える必要が消えた）。
- **wire 契約は無変更**。unit test 2 本。Linux で lib 567 + 統合 14 passed。

---

## 2. 実機検証

### 2.1 U10 — 消化済み（2026-08-20）

GUI 不要の枠。probe 30 本 + 追加 8 本を `/mcp` に当て、**PASS 29 / FAIL 0**。
結果と生ログは `_OUTPUTS/u10-verify/`（`results.md` / `raw-output.txt` /
`followup-output.txt`）。→ plan.md §2 U10、CONTRACT.md 続報25。

**消えた不安**: 本番の層順（`mcp_auth` → compat → rmcp）が実機で裏づけられた。
legacy ルートは完動（22 tools・`tools/call` まで）。

**出てきた不安**: RC ルートは実 rmcp 相手に実用にならない（F-1 / F-2 / F-3）。
3 件とも「middleware 単体では正しく、実 rmcp と組んだときだけ壊れる」種類で、
偽 downstream の前で middleware を回す現在のテスト設計では原理的に検出できない。
**U10 をやらずに 5.5.1 へ進んでいたら、載せた OTel が動かないところまで行っていた。**

### 2.2 バンドル1（U4 / U16 / U17）— 未実施

`_OUTPUTS/bundle1-verify/`（1 config・1 起動、約 70 分）。
**修正版バイナリで流すこと** — §1.2 が入っていないと U4 は必ず「横取りあり」になる。

U4 の観測点は 3 つ: (a) `lead` のペインが 2 枚立つか（横取りが直っているか）、
(b) 2 本の run の `session_id` が違うか、(c) 各ペインの返信に書かせた
**「await が何件返したか」が 1 か 2 か**。(c) が 2 なら「ordinary step の mailbox が
並行 run で共有される」が実機で確定する（→ §0 の別枠）。

### 2.3 バンドル2（U18 / U19）— 未実施

`_OUTPUTS/bundle2-verify/`（エージェント不要、約 60 分）。
`exit 1` と `sleep` だけで回る。U19 は `seed-workflow-runs.sh` で
ダミー終端 run を仕込む（`project_dir` は本物の run 行から読む — 手で書くと
`prune` の `WHERE` に一致せず偽陰性を引く）。

**U19 で期待が割れている点が 1 つある**: v0.5.9 で `QueenStore::open` からの
起動時一括掃除が入ったので、`next-release-v0.5.9.md` §3.2 が書いた
「起動時には何も消えない」は**その追加より前の手順**である。
どちらが実際かを観測して決める（縮んでいれば起動時掃除が効いたということで、FAIL ではない）。

### 2.4 v0.5.10 に含めない実機検証

`next-release-v0.5.9.md` §3.4 のとおり。U12 / U13 / U14（バンドル3）、U20（schedule、
実時間に縛られる独立枠）、U6 / U7 / U8（1.0.0 条件の枠）。

---

## 3. タグを打つ前の条件

v0.5.9 は「計画書が自ら定義した完了条件を満たさないままタグを打った」
（→ plan.md §6.25）。同じことを 3 度やらないために、**打つ前に満たすものを先に書く**。

1. macOS 実機で `npm run check` / `npm run build` / `cargo test` / `cargo clippy` が通る
   （§1.2 はコンテナでしか回していない）。
2. U4 / U16 / U17 / U18 / U19 のうち、**少なくとも U4 は消えている**こと。
   §1.2 の修正が実機で効いていることの確認そのものなので、これだけは外せない。
3. `scripts/bump-version.sh 0.5.10` で 4 ファイルを揃える。
4. CI（macos-14 / ubuntu-22.04）が当該コミットで success。

**満たせないまま打つ場合は、何を満たしていないかを plan.md §6 に明記して打つ。**
黙って打たない、が v0.5.8 / v0.5.9 から引き継ぐ唯一の教訓である。
