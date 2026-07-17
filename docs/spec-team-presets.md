# 仕様書: Queen team preset（Phase 4.3・方式C）

作成日: 2026-07-17 / ステータス: **実装済み（2026-07-17）— 実機偵察（8章）は継続項目**

> 実装は本書の計画どおり CONTRACT.md「Phase 4.3 追加契約」→ backend
> （`config.rs` 検証 + `team_presets.rs` + Queen tool `spawn_team` 19本目 +
> Tauri command）→ frontend（👥 チップ + 起動レポート）→ docs/example の順で完了。
> ユーザー判断により偵察ゲートは「エスカレーション既定パターンの決定」のみに縮小し、
> 実装を先行した。実装時に確定した設計差分は 10 章を参照。

関連: [spec-claude-teams-panes.md](spec-claude-teams-panes.md) 2章（方式Cの位置づけ）・11章（未解決事項「方式Cの具体像」）、[plan.md](plan.md)、[competitive-landscape.md](competitive-landscape.md)

---

## 1. 目的と背景

### 1.1 何を作るか

`ptygrid.yml` に **team preset**（名前付きのチーム構成）を宣言し、複数エージェントを
1操作で一括起動できるようにする。既存の `spawn_agent` / `inbox` / `await` を土台にした
ergonomics 拡張であり、Claude Code の内部仕様（tmux シム・hooks・実験フラグ）には
一切依存しない。方式A（host）/ 方式B（observe）とは独立した機能として実装する。

### 1.2 なぜ作るか（投資対効果の判断）

機能的には `spawn_agent` の逐次呼び出しで代替可能（spec-claude-teams-panes 11章の指摘）
だが、preset には逐次呼び出しにない価値がある:

- **再現性** — 毎回同じ構成（メンバー・役割・キックオフ）を1操作で再現できる
- **宣言性** — 役割指示（instructions）を構成とセットで config-as-code として管理できる。
  これは ptygrid の設計思想（config-as-code）の自然な延長である
- **差別化** — 方式A/Bと異なり Claude Code 非依存で成立する「Teams 風」であり、
  Queen 型協調という ptygrid の勝ち筋を最も直接的に強化する

投資対効果は「軽量に実装できる範囲に限定する」ことで成立させる。実体は
**既存 spawn 経路の薄いラッパー + YAML スキーマ拡張**であり、重い新機構
（スケジューラ・専用プロトコル・自動エスカレーション判定など）は作らない。

### 1.3 主想定ユースケース: ローカルLLM混在のコスト階層型チーム

運用の第一想定は「**普段はローカルLLM、難しい問題だけクラウドモデル
（Claude Opus / Grok / GPT 等）**」のコスト階層型チームである。

- CLI は Claude Code のまま、**coderouter（claude-code-router）経由で
  llama.cpp / ollama にルーティング**する構成を第一級の想定とする
- ルーティングは**プロセス単位の env**（`ANTHROPIC_BASE_URL` 等）で決まるため、
  ptygrid 側は既存の per-agent `env`（`${VAR}` 展開付き）で対応済み。新機構は不要
- CLI が同一 `claude` バイナリでも、ptygrid はペインを**定義名**（`local` / `opus` 等）で
  区別するため宛先曖昧性は生じない。Queen の MCP 登録（`-s user`）も1回で全ペインに効く
- **複数ローカルモデルの使い分けは coderouter 以降（ルーター側）で切り替える。**
  ここはユーザー側で実装を積む予定であり、ptygrid から見えるローカル系統は
  1エンドポイントである。モデル切替・ルーティングルールは本仕様の**非スコープ**

---

## 2. スコープ / 非スコープ

### 2.1 スコープ

- `ptygrid.yml` の `team_presets:` スキーマ追加（パース・検証・Reload 追随）
- 一括起動の backend 実装（既存 spawn/allowlist 経路の再利用、起動結果レポート）
- 起動経路2つ（実体は同一 backend 関数）:
  - **ツールバー UI** — preset 選択 → チーム起動
  - **Queen tool `spawn_team`** — エージェント自身がチームを組める
- メンバー `instructions` の inbox 配送、`kickoff` メッセージの lead 配送
- userguide / CONTRACT.md / `example/` の同時更新

### 2.2 非スコープ

- **エスカレーションの自動判定機構** — 「難しさ」の判定はモデル側と instructions 規約に
  委ねる。ptygrid は判定に関与しない（7章の運用パターンとして文書化のみ）
- **coderouter 側のモデル切替** — 複数ローカルモデルのルーティングはルーター側
  （ユーザー実装）の責務。ptygrid は env を渡すだけ
- **standby メンバーの自動終了** — 当面は手動 close で運用。idle 自動終了は
  運用データを見てから将来検討（10章）
- **方式A/B（teammates hooks / host）との統合** — team preset は独立機能。
  preset メンバーが個々に `teams:` 設定を持つことは妨げないが、相互作用は設計しない

---

## 3. `ptygrid.yml` スキーマ拡張

```yaml
# 既存の agents: を参照する名前付きチーム構成（任意・ブロックごと省略可）
team_presets:
  daily:                        # preset 名（一意）
    lead: local                 # 任意: kickoff の宛先。省略時は最初の非 standby メンバー
    members:
      - agent: local            # 必須: agents: で定義済みの名前のみ（allowlist 整合）
        instructions: >-        # 任意: 起動後に inbox へ配送される役割指示
          実装とレビューの一次担当。自力で難しいと判断した問題は
          spawn_agent で opus を起動し、inbox で依頼して await で回答を待つ。
      - agent: opus
        standby: true           # 任意(default false): チーム起動時には立ち上げない。
        instructions: >-        #   定義参照のみ。必要時に spawn_agent / UI で起動される
          難問のみ担当。アーキテクチャ判断と難しいデバッグ。
      - agent: grok
        standby: true
    kickoff: >-                 # 任意: 全非 standby メンバー起動後、lead の inbox へ投函
      今日のタスク: ...
```

### 3.1 検証ルール（config ロード時）

- `members[].agent` は `agents:` の定義名への参照のみ。未定義名は **config エラー**
  （`processes:` は参照不可。チームは対話エージェントの構成である）
- preset 名は一意。`members` は空でないこと。**非 standby メンバーが1名以上**あること
- `lead` は非 standby メンバーであること（standby を lead にはできない）
- `instructions` / `kickoff` / `standby` / `lead` はすべて任意
- 既存ブロック（`teammates:` / `agents[].teams`）とはキー名レベルで独立。
  `team_presets` という名前は両者との衝突・混同を避けるために選ぶ

---

## 4. 起動セマンティクス

### 4.1 起動順序と standby

- 非 standby メンバーを**宣言順に逐次起動**する（既存の spawn 経路をそのまま使用。
  autostart と同じ信頼確認の枠内）
- `standby: true` のメンバーは起動しない。preset 上の役割定義（instructions）だけを保持し、
  後から `spawn_agent`（エージェント発）または UI（人間発）で起動されたときに
  instructions を同様に配送する

### 4.2 冪等性（既に起動中のメンバー）

同じ定義名の実行中セッションが既にある場合は**重複起動せず skip** し、
起動結果レポートに `skipped` として含める。「チーム起動」ボタンの連打や
`spawn_team` の再呼び出しが安全であること（冪等）を保証する。

### 4.3 9ペイン上限との衝突

起動途中で上限に達した場合は**部分起動 + 明示レポート**とする
（起動できたメンバー / できなかったメンバーを列挙）。事前チェックによる
全部拒否はしない。既存の「9面上限バナー」の思想（作業は止めない・状況は隠さない）に合わせる。

### 4.4 instructions / kickoff の配送

- **配送は Queen の永続 inbox に統一**する。CLI 引数への埋め込みは行わない
  （CLI ごとの引数仕様差異を持ち込まないため。inbox は全 MCP 対応 CLI で同一に働く）
- 各メンバーの `instructions` は、そのメンバーの起動完了後に inbox へ投函する
- `kickoff` は全非 standby メンバーの起動完了後、lead の inbox へ投函する
- 宛先解決は**この preset 起動で生成されたセッション `#id`** に限定する。
  同名 CLI が他に実行中でも誤配しない（既存の「推測拒否」原則を維持）
- 送信者表記は暫定 `queen:preset/<preset名>`（10章の未解決事項。CONTRACT 追記時に確定）

### 4.5 起動結果レポート

UI（toast + Teammates/ステータス面）と `spawn_team` の戻り値の両方で、
`started` / `skipped` / `failed` を `#id` 付きで返す。部分起動時の
「起動できなかった理由」（上限 / spawn 失敗）を含む。

---

## 5. 起動経路

| 経路 | 対象 | 内容 |
|---|---|---|
| ツールバー UI | 人間 | 読み込み済み config に `team_presets` があるとき preset 選択 UI を表示 → 1クリックでチーム起動。standby メンバーも一覧に出し、個別起動ボタンを添える |
| Queen tool `spawn_team` | エージェント | `{ preset: "daily" }` を受け、同一の backend 関数を呼ぶ。戻り値は 4.5 のレポート |

- 両経路とも実体は**同一の backend 関数**。挙動差を作らない
- `spawn_team` は allowlist 整合そのもの（preset = config 定義の集合）であり、
  新しい信頼境界を導入しない。呼び出し権限は当面**全エージェント可**とする
  （起動対象が allowlist 内に閉じているため被害が限定される。10章参照）

---

## 6. 既存設計原則との整合 / CONTRACT.md 追記項目

- **許可リスト spawn**: メンバーは `agents:` 定義の参照のみ。spawn 経路も既存を再利用。
  方式比較（spec-claude-teams-panes 2.2）で方式Cが「完全整合」とされた性質を保つ
- **project 境界**: preset は config 単位 = project 単位。他 project への越境はない
- **宛先の曖昧さ拒否**: instructions/kickoff は preset 起動が生成した `#id` に限定（4.4）
- **信頼確認**: preset 起動は autostart と同じ信頼済みフォルダの枠内でのみ実行される

CONTRACT.md への追記（実装前に先行追記）:

1. `team_presets` スキーマ（検証ルール含む）
2. `spawn_team` tool の引数・戻り値（起動レポートの形）
3. 起動レポートの frontend イベント（UI 表示用）
4. inbox 配送の送信者表記規約

---

## 7. エスカレーション運用パターン（ドキュメント化のみ・機構は作らない）

「普段ローカル、難問だけクラウド」は次の**instructions 規約**として userguide に載せる:

> 一次担当（local）への指示例: 「自力で詰まった（2往復以上進展がない / 設計判断が必要）
> と判断したら、`spawn_agent` で `opus` を起動し、inbox で問題の要約と試したことを送り、
> `await` で回答を待って作業に反映する。完了したら opus のペインは人間が閉じる。」

ローカルモデルの tool 呼び出し品質が不足する場合のフォールバックは
「**人間が UI から standby メンバーを起動する**」運用。preset の価値
（宣言・一括起動・役割配送）はこのフォールバックでも保たれる。

---

## 8. 実装前の偵察（ゲート）

実装着手の前に、想定構成での実機確認を行い、結果を本書に追記する:

1. **coderouter + llama.cpp / ollama 経由の Claude Code** が Queen に MCP 接続でき、
   `read_output` / `send_message` / inbox 系を安定して呼べるか
2. 同構成で **`spawn_agent` → inbox 依頼 → `await` のエスカレーション一連**が
   ローカルモデル発で成立するか（成立しなければ 7章のフォールバック運用を既定にする）
3. 素の Claude Code（クラウド）ペインとの**混在動作**（同一 `claude` バイナリ・
   env 差のみで両ペインが共存し、Queen 登録が両方に効くこと）

偵察の結果は「エスカレーションの既定パターン（エージェント発 / 人間発）」の決定として
7章に反映してから実装に入る。

---

## 9. リリース計画とテスト

- リリースは **v0.4.4（Phase 4.3）** の1本を想定（plan.md のバージョニング規約に従う）
- [phase3.md](phase3.md) のリリース規律を踏襲: CONTRACT 先行追記、`lib.rs`/hot path に
  新ロジックを置かない、該当挙動のみ userguide 更新

テスト:

- **cargo test（ユニット）**: `team_presets` パース / 検証ルール（未定義参照・standby lead・
  非 standby ゼロ等のエラー系を含む）
- **cargo test（結合）**: 起動セマンティクス — 冪等 skip、部分起動（上限衝突）、
  instructions/kickoff の inbox 配送と `#id` 限定、起動レポートの内容
- **frontend**: preset 選択 UI・起動レポート表示の `svelte-check` / `npm run build`
- CI は macOS / Ubuntu 両方の既存ワークフローを通す

---

## 10. 実装時に確定した設計差分（2026-07-17）

実装調査で判明した既存契約との整合により、4章の記述から次を変更して確定した
（wire の正は CONTRACT.md「Phase 4.3 追加契約」）:

- **inbox 宛先は `#id` ではなく定義名 mailbox** — Phase 3.7 契約で inbox の mailbox は
  安定した論理名であり `#id` を禁止している。「推測拒否」の懸念は live PTY への
  `send_message` の問題であり、durable inbox（本人が自分の mailbox 名で読む）では
  誤配が構造的に起きないため、定義名宛てで整合する。
- **配送は「チームが実際に起動した呼び出し」のみ** — `started` が 1 件以上のときに限り、
  started メンバーの instructions → **standby メンバーの instructions**（durable なので
  後から起動しても読める）→ kickoff（effective lead 宛）の順で配送。全 skip の冪等
  no-op 呼び出しは何も再送しない。
- **ペイン上限はセッション総数 9 の backend 近似** — 9 面グリッドは frontend 状態のため、
  backend はセッション総数（状態・種別不問）で近似し、超過分を spawn せず
  `failed ("pane limit")` にする（既存 Queen spawn の「paneless で走り続ける」挙動を
  チーム一括起動で量産しないため）。
- **冪等 skip の判定は「生存」セッション**（starting / running / restarting）。
- テスト: config 検証 8 件 + 起動セマンティクス 5 件を追加し、既存 196 → 210 全通過。

## 11. 未解決事項

- **standby の寿命管理** — idle 自動終了の要否。当面は手動 close とし、運用データで判断
- **`spawn_team` の呼び出し権限** — 当面は全エージェント可。preset の lead のみに
  制限すべき事例が出たら再検討
- **inbox 送信者表記** — `queen:preset/<preset名>` を暫定案とし CONTRACT 追記時に確定
- **preset の入れ子・継承** — 必要性が実証されるまで作らない（YAGNI）
- **standby メンバーの UI 表現** — 「定義済み・未起動」をペイングリッド外で
  どう見せるか（Teammates パネル拡張 or ステータス面）。実装時に決定
