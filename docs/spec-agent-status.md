# ptygrid 仕様: エージェント意味的状態の可視化と blocked 通知

作成日: 2026-07-17 / 状態: ドラフト / 対象: Phase 4.4（未実装・仕様のみ）

関連: [herdr-research.md](research/herdr-research.md)（検出方式の参考元）/
[design.md](design.md)（アーキテクチャ原則）/ [competitive-landscape.md](competitive-landscape.md)
（「通知リング / 要承認ハイライト」バックログ）/ [plan.md](plan.md)（バージョニング）/
[../CONTRACT.md](../CONTRACT.md)（IPC/MCP 契約）。

---

## 1. 目的と背景

ptygrid は複数の AI CLI を PTY ペインで並行実行するが、Phase 4.3 時点で各ペインが
「いま動いているのか／承認待ちで止まっているのか／終わって手空きなのか」を人間が
区別する手段は、ペインを覗いて出力を読むことだけである。ペインが 4〜9 枚になると、
どのペインが自分の操作を待っているか（＝承認プロンプトで停止しているか）を見落とし、
エージェントを遊ばせる時間が生まれる。

herdr はこの問題を「**端末出力ヒューリスティック（TOML manifest の正規表現）でエージェント
状態を Blocked/Working/Done/Idle に自動分類し、色分け表示する**」ことで解決し、これが最大の
差別化点になっている（[herdr-research.md](research/herdr-research.md) 3章）。競合調査でも
「通知リング / 要承認ハイライト」は cmux / Architect が強い UX 領域として、ptygrid が次に
取るべき機能に挙がっている（[competitive-landscape.md](competitive-landscape.md)）。

本仕様は、この価値を ptygrid に翻案する。ただし herdr と同じ TOML リモート更新ではなく、
ptygrid の設計思想（config-as-code / ptygrid.yml）に沿って **内蔵既定パターン + ptygrid.yml
上書き**方式を採る。

### プロセス生死とは別レイヤの「意味的状態」

ptygrid には既に `SessionState = starting|running|exited|restarting`（[CONTRACT.md](../CONTRACT.md)
Phase 1）がある。これは **PTY プロセスの生死**を表すランタイム状態で、backend が生成する権威的な
事実である。本仕様が導入する `working|blocked|done|idle`（+ `unknown`）は、それとは**別レイヤ**の
**意味的状態（semantic status）**であり、生きている PTY（`running`）の**上に重ねて**推定する
ヒューリスティックである。両者を混同しない（3.1、2章参照）。

---

## 2. 状態モデル

### 2.1 状態の定義

意味的状態 `AgentStatus` は次の5値。既存 `SessionState` を置き換えず、別フィールド／別イベントで
運ぶ。

| 状態 | 色 | 定義 | 典型トリガ |
|---|---|---|---|
| `blocked` | 🔴 赤 | エージェントがユーザーの承認・入力を待って停止している。**既知の承認/選択/権限 UI にマッチしたときだけ** | `Do you want to proceed?` + 選択肢、`Allow this tool?`、`[y/N]` プロンプト |
| `working` | 🟡 黄 | エージェントがタスクを実行中（思考・ツール実行・生成） | `esc to interrupt`、スピナー行、`Thinking…`、`Running…` |
| `done` | 🔵 青 | 直前まで working だったが、割り込み可能表示が消えプロンプトへ復帰した（＝直近の作業が完了） | working から入力プロンプト復帰への遷移、`✓ Done`、完了マーカー |
| `idle` | 🟢 緑 | 生きているが特筆すべき作業も承認待ちもない待機。**既定のフォールバック** | どのパターンにもマッチしない running PTY |
| `unknown` | ⚪ 白（バッジ非表示） | 状態を推定するルールセットが無い／評価前 | ルールセット未割当のプロセス、評価前の起動直後 |

### 2.2 既存 running/exited との関係

- **意味的状態は `SessionState == running` の PTY セッションにのみ付与する。**
- `exited` / `restarting` / `starting` のセッションは意味的状態の**対象外**。`exited` になった瞬間、
  そのセッションの `AgentStatus` は破棄する（UI は既存の「終了」タグ／状態ドットで表現）。
- `transcript`（PTY なし・observe）セッションは、tail の整形テキスト末尾を対象に**同じ検出経路**を
  適用できる（3.4）。ただし transcript は read-only なので blocked 通知の第一目的（＝ユーザーの
  操作待ち解消）とは意味が薄い。既定では transcript の blocked 通知は**抑制**し、状態バッジ表示のみ
  行う（6.4）。

意味的状態は「意見（推定）」であり、`SessionState` のような「事実」ではない。この非対称性を UI・
契約・実装のすべてで保つ（8章）。

### 2.3 blocked 保守主義（herdr 同原則）

**blocked は誤検出のコストが最も高い**（「止まっている」と誤表示するとユーザーが不要な介入をする、
または通知が空振りする）。したがって herdr と同じく**保守的**に判定する:

- blocked は「**既知の可視な承認/質問/権限 UI にマッチした時だけ**」立てる。
- 未知のプロンプト・未知の停止は blocked にせず、`idle`（またはマッチが何も無ければ `idle`）扱い。
- 「working パターンが消えた」だけでは blocked にしない（それは `done`→`idle` の経路）。

優先順位（3.3）でも blocked を最優先にするが、**発火条件は最も厳しく**する。この非対称性
（優先度は高いが条件は厳格）が保守主義の実装形。

---

## 3. 検出方式

### 3.1 対象とルールセットの選択

全 PTY ペイン（`kind: pty`, `state: running`）と observe transcript を対象とする。各セッションに
**1つのルールセット**（blocked/working/done 正規表現の束）を割り当てる。割り当ては次の順で決める:

1. **agent 定義名** — セッションが ptygrid.yml の agent 定義由来（`spec.name = Some("claude")` 等）で、
   その名前に対応するルールセット（内蔵既定 or `agent_status.patterns` の同名キー）があればそれ。
2. **フォアグラウンドプロセス名** — 上記が無い／adhoc シェル（`spec.name = None`）の場合、その PTY の
   フォアグラウンドプロセス名（既存 `foreground_pid` → `process_name`、[session.rs](../src-tauri/src/session.rs)）が
   ルールセットのキーに一致すればそれ。手打ちで起動した `claude` / `codex` を拾うため。
3. **どちらも無ければ `unknown`** — ルールセットを割り当てず、状態は `unknown`（バッジ非表示）。
   generic なフォールバックルールは**既定では割り当てない**（誤検出を避ける保守方針。generic を
   使いたいユーザーは `agent_status.patterns` に `"*"` キーを定義して opt-in できる、4.2）。

フォアグラウンドプロセス名は変動するため、ルールセット選択は**評価のたびに**行う（`list_sessions`
と同じく遅延解決。spawn 時に固定しない。[session.rs](../src-tauri/src/session.rs) の既存注記に従う）。

### 3.2 検出入力の作り方（ANSI 再構成の再利用）

生の output ring 末尾をそのまま正規表現に掛けると、TUI のスピナー残骸・カーソル移動・alternate
screen で壊れる。そこで **Queen `read_output` と同じ経路**を使う:

1. `SessionManager::output_snapshot(id)` で ring 全体（最大 256 KiB）と現在の cols/rows を取得
   （[session.rs](../src-tauri/src/session.rs) の既存 API）。
2. `ansi::render_terminal(&text, rows, cols)`（[ansi.rs](../src-tauri/src/ansi.rs)）で**現在の端末画面を
   再構成**したテキストを得る（スピナーは最終状態に畳まれ、alternate screen は現在アクティブな面のみ
   残る）。
3. その末尾 **N 行**（既定 `N = 24`、`agent_status.tail_lines` で 4..=200 に clamp）を検出対象文字列とする。

この畳み込みにより「⠙ Thinking」のような上書き行が最終状態でマッチでき、TUI の再描画残骸で
誤検出しない。**検出は必ずこの再構成テキストに対して行い、生バイト列には掛けない。**

### 3.3 マッチと優先順位（決定順）

再構成末尾テキストに対し、ルールセットの各カテゴリの正規表現配列を評価する。**決定順は
blocked > working > done > idle**:

1. **blocked** パターンのいずれかにマッチ → `blocked`（保守条件を満たす明示マッチのみ、2.3）。
2. でなければ **working** パターンのいずれかにマッチ → `working`。
3. でなければ **done** パターンのいずれかにマッチ → `done`（3.5 の遷移規則も併用）。
4. どれにもマッチしなければ → `idle`。

`unknown` はルールセット未割当のときだけ（3.1-3）。マッチ結果は「最初にマッチしたルールの id」を
`matchedRule` として保持し、デバッグ（`explain`、10章）に使う。

正規表現は**行アンカー無しの部分一致**を既定とし、末尾テキスト全体（複数行）に対して評価する
（`(?m)` マルチライン。パターン側で `^`/`$` を使える）。大文字小文字は既定で区別しない
（`(?i)` 相当。パターン個別に上書き可能）。

### 3.4 done→idle の遷移と done の扱い

`done` は本質的に**遷移状態**であり、放置すれば `idle` と区別が曖昧になる（11章の既知課題）。
本仕様では次の具体規則で決め切る:

- `done` パターンにマッチ、**または** 直前状態が `working` で今回 working パターンが消えプロンプト
  復帰（idle 相当）と判定された場合、状態を `done` にする（「作業が終わった直後」を表す）。
- `done` は **`done_linger` 秒間（既定 6 秒、`agent_status.done_linger_ms` で 0..=60000 に clamp）** 保持し、
  その間に blocked/working へ遷移しなければ `idle` へ落とす。`done_linger_ms: 0` で done を使わず
  即 idle。
- linger 中に新たな出力で working/blocked にマッチしたら即座にそちらへ遷移（done を破棄）。

これにより「完了 → 数秒だけ青 → 緑（手空き）」という自然な減衰になり、`done` が永続して idle と
混ざる問題を避ける。

### 3.5 transcript（observe）の検出

observe transcript は PTY を持たず、tail の整形済みテキスト（`role: text` 連結、[transcript.rs](../src-tauri/src/transcript.rs)）を
保持する。この場合:

- ANSI 再構成は行わず（既に整形済み）、tail テキストの末尾 N 行をそのまま検出対象にする。
- ルールセットは親 lead と同じ agent 名（例 `claude`）を用いる。
- `SubagentStop`（transcript が `exited`）で意味的状態を破棄する。

### 3.6 評価頻度・デバウンス・コスト（hot path に regex を置かない）

**session hot path（reader thread → `pty-output` emit）に正規表現を絶対に置かない。** 検出は次の
分離した経路で行う（7章の設計と一致）:

- reader thread は出力受信時に該当セッション id を「dirty」としてマーク（`AtomicBool` の set、または
  `mpsc`/`watch` へ id を送るだけ。ロックも regex も無し）。
- 別の **単一の評価タスク**（`tauri::async_runtime` の 1 タスク）が **デバウンス間隔ごと**に起き、
  dirty なセッションだけを `output_snapshot` → `render_terminal` → 末尾 N 行 → regex 評価する。
- デバウンス間隔は既定 **250ms**（`agent_status.debounce_ms`、100..=2000 に clamp）。バースト出力
  でも評価は 4 回/秒に上限される。
- 状態が**変化したときだけ** `agent-status` イベントを emit（7.2）。変化なしなら何もしない。
- コスト上限: 1 セッションの評価は「末尾 N 行（〜数 KiB）× ルールセットの正規表現本数」。regex は
  **起動時に一度コンパイルしてキャッシュ**（`OnceLock`/`Lazy` + config reload 時に再構築）。hot path
  では一切コンパイルしない。

`render_terminal` は ring 全体（最大 256 KiB）を走査するため、評価対象セッション数 × 4 回/秒の
コストになる。9 面フルでも 36 回/秒程度で、既存 `read_output` と同オーダー。過大なら将来
「末尾ウィンドウのみ再構成」の最適化余地を残す（11章）。

---

## 4. ptygrid.yml スキーマ拡張

### 4.1 YAML 例

```yaml
# グローバル agent_status ブロック（すべて任意。ブロック省略で既定値）
agent_status:
  enabled: true          # default true。false で検出・イベント・通知をすべて停止
  notify: true           # default true。blocked 通知（ネイティブ通知）の可否
  notify_sound: true     # default true。ネイティブ通知に音を付ける
  tail_lines: 24         # default 24。検出に使う再構成末尾行数（4..=200）
  debounce_ms: 250       # default 250。評価デバウンス（100..=2000）
  done_linger_ms: 6000   # default 6000。done を保持してから idle へ落とす（0..=60000、0で無効）
  renotify_ms: 0         # default 0。blocked 継続中の再通知抑制の追加クールダウン（0=状態変化まで再通知しない）

  # ルールセット定義。キー = agent 定義名 or フォアグラウンドプロセス名。
  # 内蔵既定（claude/codex/grok…）へ「マージ（追記）」するのが既定。
  # replace: true でそのキーの内蔵既定を捨てて完全置換。
  patterns:
    claude:
      # merge（既定）: 内蔵 claude ルールに以下を追記する
      blocked:
        - 'Do you want to proceed\?'
        - '❯\s*\d+\.\s*Yes'
      working:
        - 'esc to interrupt'
      done:
        - '✓\s'

    codex:
      replace: true          # codex の内蔵既定を捨て、以下だけを使う
      blocked:
        - 'Allow command\?'
        - '\[y/N\]'
      working:
        - 'Working\b'
        - 'Thinking\b'
      done: []

    # 自作 CLI 用の新規ルールセット（内蔵に無いキーは常に新規追加）
    my-agent:
      blocked:
        - 'Confirm\?\s*\(yes/no\)'
      working:
        - 'Processing'

    # opt-in の generic フォールバック（未割当セッションにも適用したい場合のみ）
    "*":
      blocked:
        - '\[y/N\]\s*$'
```

### 4.2 マージ／置換セマンティクス

- `agent_status.patterns.<key>` は、**同名の内蔵既定ルールセットがあれば既定でマージ**する:
  各カテゴリ（blocked/working/done）ごとに **内蔵配列 + ユーザー配列** を連結する（重複除去はしない、
  順序は内蔵→ユーザー）。
- そのキーに `replace: true` を付けると、内蔵既定を**破棄して**ユーザー定義だけを使う。
- 内蔵に無いキー（例 `my-agent`）は常に新規ルールセットとして追加。
- カテゴリを省略した場合はそのカテゴリの内蔵配列をそのまま使う（merge 時）／空扱い（replace 時）。
- `"*"` キーは特別扱い: 3.1 でルールセットが割り当たらなかった running PTY に対して**opt-in で**
  適用する generic ルールセット（既定では存在しない）。
- 正規表現が不正な場合、その**1本のパターンだけをスキップ**して backend ログに警告を出し、
  残りは有効化する（設定全体を失敗させない。config reload の非破壊性を保つ）。

### 4.3 TypeScript 相当の型（frontend / 契約）

```ts
export type AgentStatus = "working" | "blocked" | "done" | "idle" | "unknown";

export type AgentStatusPatternSet = {
  replace?: boolean;      // default false（merge）
  blocked?: string[];
  working?: string[];
  done?: string[];
};

export type AgentStatusConfig = {
  enabled?: boolean;              // default true
  notify?: boolean;              // default true
  notifySound?: boolean;         // default true
  tailLines?: number;            // default 24, clamp 4..200
  debounceMs?: number;           // default 250, clamp 100..2000
  doneLingerMs?: number;         // default 6000, clamp 0..60000
  renotifyMs?: number;           // default 0
  patterns?: Record<string, AgentStatusPatternSet>;
};

// Config へ additive に追加
// export type Config = { project?; agents; processes; agentStatus?: AgentStatusConfig };
```

Rust 側は既存 `TeammatesConfig` と同じ `#[serde(skip_serializing_if = "Option::is_none")]` +
`effective_*()` アクセサ方式で既定補完する（[config.rs](../src-tauri/src/config.rs) パターン）。

### 4.4 内蔵既定パターン（バイナリ同梱）

内蔵既定は Rust バイナリに**コンパイル時同梱**する。実装は `src-tauri/src/agent_status_defaults.yml`
を `include_str!` で埋め込み、起動時に一度パースしてキャッシュする（外部ファイル依存なし・オフライン
動作）。ptygrid.yml の `agent_status.patterns` は 4.2 の規則でこれへマージ／置換される。

初期同梱キー（**代表例・陳腐化しうる**。実 CLI の UI は頻繁に変わるため、下記は 2026-07 時点の
観測に基づく初期値で、保守対象＝11章）:

| key | blocked（承認/入力待ち） | working（実行中） | done（完了/復帰） |
|---|---|---|---|
| `claude` | `Do you want to proceed\?` / `❯\s*\d+\.\s*Yes` / `Allow this (tool|command)` | `esc to interrupt` / `(?i)\bThinking\b` / `(?i)\bWorking\b` | `(?i)^\s*✓` |
| `codex` | `Allow command\?` / `\[y/N\]` / `Approve this` | `(?i)\b(Working|Thinking|Running)\b` / `esc to interrupt` | `(?i)\bdone\b` |
| `grok` | `\[y/N\]` / `Confirm\?` | `(?i)\b(Thinking|Generating)\b` | — |
| `aider` | `\(Y\)es/\(N\)o` / `Add .+ to the chat\?` | `(?i)\bThinking\b` | — |

> 注記（必読）: 上表は**具体化のための初期例であり、各エージェント CLI の UI 変更で容易に陳腐化する**。
> herdr はリモート manifest 自動更新でこれを保守するが、ptygrid はオフライン・config-as-code 方針の
> ため、**内蔵既定はリリースごとに更新 + ユーザーが ptygrid.yml で即時上書き**、を保守手段とする
> （11章・9章）。シェルプロンプト復帰（`\$\s*$` / `❯\s*$` / `%\s*$`）は誤検出が多いため**内蔵 done
> には入れない**（generic `"*"` opt-in か個別 CLI 定義でのみ扱う）。

---

## 5. UI 仕様

### 5.1 ペインヘッダーの意味的状態バッジ

既存のペインヘッダー（[App.svelte](../src/App.svelte) の `.pane-header`、状態ドット `.dot.state-*`）は
**PTY 生死**を表す。意味的状態は**それに重ねる別バッジ**として、状態ドットの直後に小さな
色付きバッジ（丸 or ラベル）を追加する:

- `blocked` 🔴 / `working` 🟡 / `done` 🔵 / `idle` 🟢。`unknown` はバッジ非表示。
- バッジ色は既存 `.dot.state-*` とは別クラス `.astatus-{blocked|working|done|idle}` を新設
  （既存 CSS を壊さない）。tooltip に状態名 + `matchedRule`（あれば）を表示。
- `SessionState != running` のペインではバッジを出さない（生死ドットのみ）。
- frontend は `agent-status` イベントを購読して `ui.agentStatus: Record<number, AgentStatus>` を更新し、
  ヘッダーはそれを参照する（`session-state` とは独立。`exited`／close 時にエントリを削除）。

### 5.2 blocked の要承認ハイライト（通知リング）

blocked は最重要状態なので、ヘッダーバッジに加えて**ペイン枠に通知リング**を出す:

- `ui.agentStatus[id] === "blocked"` のペインに CSS クラス `.pane-blocked` を付与し、
  枠に赤系のリング（例 `box-shadow: inset 0 0 0 2px #e0574a;` + 低頻度パルスアニメーション）を出す。
- 既存の teammate-focus リング（`.pane-focused`、青 `inset 0 0 0 2px #4a9be0`）と**排他ではなく重畳**
  してよいが、blocked リングを優先色にする。パルスは `prefers-reduced-motion` を尊重して静的リングへ
  縮退する。
- リングは状態が blocked でなくなった時点で除去（イベント駆動、ポーリングしない）。

### 5.3 ステータスサイドバー（左・格納式）

複数ペインの意味的状態を一望するため、グリッドの左に**格納式サイドバー**を設ける。herdr・cmux が
縦リストで採る定番 UX で、ペインが 4〜9 枚に増えたとき「いまどれが承認待ちか」を1か所で俯瞰できる。
ペインヘッダーの個別バッジ（5.1）が「各ペイン内の詳細」、サイドバーが「全体の俯瞰＋ナビ」を担う。

- **配置**: グリッド（`Splitpanes`）の左に固定幅（既定 200px、ドラッグでリサイズ可）のサイドバー。
  上端に格納トグル（`‹` / `›`）を置き、開閉する。
- **内容**: `running` な全 PTY ペイン（observe transcript / host teammate 含む）を縦リストで表示。
  各行 = 意味的状態ドット（🔴🟡🔵🟢、unknown は無彩色）＋ `#id` ＋ 表示名（定義名 or foreground、
  teammate は `▸role ↳#lead`、transcript は `📖RO` 印）＋ 生死の小ドット。
- **ソート**: `blocked` を最上部に集約し、次いで working → done → idle → unknown。同状態内は `#id` 昇順。
  → blocked のペインが常にリスト先頭に来るので見落とさない。
- **操作**: 行クリックで該当ペインをフォーカス（既存の focus 強調 `.pane-focused` を流用）、
  ダブルクリック（または行内ボタン）で最大化トグル。行の `×` でクローズ（既存 `closePane`、
  host teammate など破壊的なものは既存の確認フローを踏襲）。
- **ヘッダー**: 「🔴 N」（blocked ペイン数）を上部に集約表示。0 のときは控えめ表示。
- **格納時**: サイドバーを畳んだ状態では、ツールバーに集約バッジ「🔴 N」を出す（クリックで最初の
  blocked ペインへフォーカス／サイドバーを一時展開）。畳んでいても状態は裏で更新し続ける。
- **永続化**: 開閉状態と幅は app 設定（[app_settings](../src-tauri/src/app_settings.rs) 相当）に保存し、
  project 非依存で復元する。
- **実装**: `ui.sessions` / `ui.agentStatus` / `ui.panes` を参照する**純粋な派生ビュー**。状態は
  `agent-status` イベントで既に届くため、サイドバー用の新規 backend / IPC は不要（frontend のみ）。
  Teammates バッジ・host lead 行への更なる集約は任意（将来、11章）。

---

## 6. 通知仕様

### 6.1 ネイティブ通知の経路

blocked 検出時に **macOS ネイティブ通知**を出す。実装は Tauri v2 の
**`tauri-plugin-notification`** を採用する（現状 [Cargo.toml](../src-tauri/Cargo.toml) 未導入・
[capabilities/default.json](../src-tauri/capabilities/default.json) に権限未追加のため、依存追加 +
capability 追加 + `Builder::plugin(tauri_plugin_notification::init())` 登録が前提。実装フェーズで
プラグイン API を [mcp-server-check 方式のスタンドアロン検証]相当で一度実証してから本体へ入れる）。

- 通知本文: タイトル `ptygrid — 承認待ち`、本文 `<pane 表示名> #<id> が承認を待っています`。
- 通知クリックでアプリ window を前面化し、該当ペインへスクロール／最大化する（`teammate-focus`
  相当の frontend ハイライトを流用）。deep-link 実装が重い場合、初期は「window を前面化するだけ」で可。
- 音は `agent_status.notify_sound`（既定 on）。

### 6.2 フォアグラウンド + 可視時の抑制

「アプリがフォアグラウンド **かつ** 該当ペインが可視」なら通知を**抑制**する（画面を見ている人に
ネイティブ通知は不要）。判定素材:

- **アプリのフォアグラウンド**: Tauri の window focus を backend で追跡する（`WindowEvent::Focused(bool)`
  を購読し `AtomicBool` に保持）。現状 window focus 追跡は未実装のため新設する。
- **ペインの可視性**: frontend が「現在描画中で可視なペイン id 集合」を backend へ通知する新コマンド
  `set_visible_panes { ids: number[] }`（最大化中は最大化ペインのみ可視、通常グリッドは全ペイン可視、
  最小化/隠しは除外）。backend は最新集合を保持する。
- 抑制条件 = `window_focused == true && visible_panes.contains(id)`。どちらかが偽なら通知する。

抑制時も**ヘッダーバッジと通知リングは必ず出す**（抑制するのはネイティブ通知だけ）。

### 6.3 連発抑制（re-notify 規律）

- 同一ペインが blocked に**入った瞬間に1回だけ**通知する（`idle/working/done/unknown → blocked` の
  エッジ）。
- blocked が**継続している間は再通知しない**（`blocked → blocked` は状態変化イベントを出さないので
  自然に1回）。
- blocked から**別状態へ抜けて再び blocked**になったら**再通知する**（新しいエッジ）。
- `agent_status.renotify_ms > 0` の場合のみ、追加のクールダウンとして「前回通知から renotify_ms 未満の
  再エッジは通知しない」を適用する（既定 0 = クールダウンなし・エッジ即通知）。
- backend は per-session に `last_notified_status` と `last_notified_at` を保持して上記を判定する。

### 6.4 対象の限定

- 通知は **PTY セッション（`kind: pty`）の blocked** のみ。observe transcript（read-only）の blocked は
  ネイティブ通知しない（ユーザーが操作できないため）。バッジ／リングは出す。
- `agent_status.enabled: false` または `agent_status.notify: false` で通知を止める（検出自体は
  `enabled` で止まる）。

### 6.5 通知権限

- macOS はネイティブ通知に OS 権限が要る。初回 blocked 通知の前に権限状態を確認し、未許可なら
  `tauri-plugin-notification` の権限リクエスト API を呼ぶ。拒否された場合は**ネイティブ通知を諦め、
  ヘッダーバッジ + 通知リング + アプリ内トースト（既存 notices 経路）へフォールバック**する
  （通知が全く出ない状態を作らない）。権限拒否は1度だけ backend ログに残し、再リクエストで
  ユーザーを煩わせない。

---

## 7. アーキテクチャ

### 7.1 検出ロジックの置き場所

新モジュール **`src-tauri/src/agent_status.rs`** に隔離する（session hot path・lib.rs に regex を
置かない原則、[design.md](design.md) §11）。責務:

- ルールセットのコンパイル・キャッシュ（内蔵既定 + config マージ、config reload で再構築）。
- **純関数 `classify(text: &str, rules: &CompiledRuleSet) -> (AgentStatus, Option<RuleId>)`**
  （decision order 適用、テスト可能・副作用なし、3.3）。
- 状態機械 `AgentStatusTracker`: per-session の `current`・`done_since`・`last_notified_*` を保持し、
  分類結果 + `done_linger` から遷移と emit 要否・通知要否を決める（3.4 / 6.3）。
- デバウンス評価ループ（3.6）: dirty 集合を受け、`output_snapshot`（session.rs）→ `render_terminal`
  （ansi.rs）→ `tail_lines` → `classify` を回す。

session.rs 側の追加は最小化: reader thread が出力時に「dirty 通知」を送るフック（id を `watch`/`mpsc`
に送るだけ、ロック・regex なし）と、`agent_status.rs` が使う既存 API（`output_snapshot` /
`running_foreground_sessions` / `session_kind`）の再利用のみ。

### 7.2 新イベント `agent-status`

```ts
// Tauri Event: backend → frontend
export type AgentStatusPayload = {
  id: number;
  status: AgentStatus;             // working | blocked | done | idle | unknown
  matchedRule?: string;            // マッチしたルール id（デバッグ・tooltip 用、任意）
  ruleSet?: string;                // 適用したルールセットキー（claude / codex / "*" 等、任意）
};
```

- 状態が**変化したときだけ** emit（`blocked→blocked` は出さない）。
- `agent_status.enabled: false` の間は一切 emit しない。
- `exited` 遷移時、backend は `agent_status.rs` の tracker からそのセッションを除去する（frontend も
  `session-state: exited` で `ui.agentStatus[id]` を削除するので、専用のクリアイベントは不要）。

### 7.3 既存経路の再利用

- 出力再構成は `ansi::render_terminal`（[ansi.rs](../src-tauri/src/ansi.rs)）、末尾抽出は queen.rs の
  `tail_lines` 相当（共通化して `agent_status.rs` からも使えるよう `pub(crate)` 化）。
- foreground 解決は `pty::process_name` / `foreground_pid`（既存）。
- 通知トーストのフォールバックは既存 `queen-notify`/notices 経路（[stores.svelte.ts](../src/lib/stores.svelte.ts)）。

### 7.4 Queen 連携（任意・将来）

- `list_agents` の `sessions[]` や `read_output` に意味的状態を**含めるかは任意・将来**。4.4 の必須
  範囲外。含める場合は additive に `SessionInfo.agentStatus?: AgentStatus`（`skip_serializing_if`）を
  追加し、CONTRACT に追記する（8章）。初期は**イベント（`agent-status`）のみ**で UI 表示は成立するため、
  MCP 面は手を付けない。
- 将来 Queen に `agent_status_explain { agent }`（どのルールセット・どのルールが発火したか）を足すと
  herdr の `agent explain` 相当のデバッグができる（11章）。

### 7.5 window focus / visible panes 追跡（通知抑制用）

- backend に `AppFocusState { window_focused: AtomicBool, visible_panes: Mutex<HashSet<u32>> }` を新設。
- `WindowEvent::Focused(b)` で `window_focused` を更新。
- 新コマンド `set_visible_panes { ids }` で `visible_panes` を更新（frontend がグリッド再配置・最大化
  トグル・ペイン開閉のたびに呼ぶ）。
- 6.2 の抑制判定にのみ使う。検出・状態遷移には影響しない。

---

## 8. 既存設計原則との整合 / CONTRACT 追記項目

### 8.1 原則整合

- **config-as-code**: 検出パターンは ptygrid.yml `agent_status` で完全に制御可能（内蔵既定は出発点）。
  リモート自動更新は行わない（オフライン・再現性重視）。
- **推測で状態を誤判定しない**: blocked 保守主義（2.3）、未割当は `unknown`、generic は opt-in。
  意味的状態は「意見」であり `SessionState`（事実）を上書きしない。
- **project 境界**: 検出は現在ロード中の ptygrid.yml のルールで動く。ルールセットは project 単位の
  config に属し、グローバル（`~/.ptygrid`）にも書ける（既存 config 探索順を踏襲）。
- **hot path 分離**: regex・render_terminal は専用タスクのみ（3.6 / 7.1）。session reader は dirty
  マークだけ。

### 8.2 CONTRACT.md 追記が必要な項目（Phase 4.4 契約）

1. **ptygrid.yml スキーマ**: グローバル `agent_status:` ブロックと `patterns` の merge/replace
   セマンティクス、各既定値と clamp 範囲（4章）。
2. **新イベント `agent-status`**: payload（`{ id, status, matchedRule?, ruleSet? }`）、emit 条件
   （状態変化時のみ・`enabled` 時のみ）、`exited` 時のクリア規約（7.2）。
3. **`AgentStatus` 型**（`working|blocked|done|idle|unknown`）と、それが `SessionState` とは別レイヤで
   ある旨の明文化（2章）。
4. **新コマンド `set_visible_panes { ids: number[] } -> void`**（通知抑制用の可視ペイン集合更新、7.5）。
5. **通知の契約**: blocked エッジで1回・継続中は再通知しない・フォアグラウンド+可視で抑制・権限拒否時
   フォールバック（6章）。ネイティブ通知経路（`tauri-plugin-notification`）と capability 追加。
6. **（任意・実装時のみ）** `SessionInfo.agentStatus?` / `list_agents`・`read_output` への状態同梱を
   行う場合は additive 追記（7.4）。4.4 では**行わない**方針を明記。
7. **非回帰の明記**: 既存 `session-state`／`SessionState`／`pty-output`／`read_output`／`list_agents` は
   不変（本仕様はすべて additive）。

### 8.3 既存 session-state との非回帰

`session-state` イベント・`SessionState` enum・`SessionInfo` の既存フィールドは**一切変更しない**。
`agent-status` は完全に別イベント。frontend も `ui.sessions`（生死）と `ui.agentStatus`（意味的状態）を
別 map で持ち、既存のペイン⇄id マッピングやリソース表示に干渉しない。

---

## 9. Phase リリース計画

[plan.md](plan.md) の流儀（y=Phase 番号、z=Phase 内連番、CONTRACT 先行追記、両プラットフォーム CI）に
従い、**Phase 4.4** として3段階に分ける。

### Phase 4.4.0 — 検出基盤 + ヘッダー表示（通知なし）

- `agent_status.rs`: 内蔵既定同梱、ルールセット選択、`classify` 純関数、デバウンス評価ループ、
  done_linger 遷移。
- ptygrid.yml `agent_status` パース + merge/replace + 既定補完（config.rs）。
- 新イベント `agent-status`（emit は状態変化時のみ）。
- frontend: ヘッダー意味的状態バッジ（🔴🟡🔵🟢）+ `ui.agentStatus` map + `exited` クリア。
- **completion gate**: `classify` の純関数テスト・merge/replace テスト・デバウンステスト通過、
  `cargo test`/`clippy`/`svelte-check`/build 通過、両プラットフォーム CI、`agent-status` を CONTRACT へ
  先行追記済み、既存 `session-state` 非回帰確認、userguide に「状態バッジ」節追加。

### Phase 4.4.1 — ステータスサイドバー（左・格納式）

- frontend のみ（5.3）。`ui.sessions`/`ui.agentStatus`/`ui.panes` の派生ビューとして左サイドバーを実装:
  running な全 PTY ペインの縦リスト、blocked 最上部ソート、状態ドット + `#id` + 表示名、行クリックで
  フォーカス・最大化トグル・クローズ、ヘッダーに blocked 集約「🔴 N」。
- 開閉トグルと幅を app 設定に永続化（格納時はツールバーに集約バッジ）。
- **completion gate**: `svelte-check`/build 通過、既存グリッド操作（focus/最大化/close/splitpanes リサイズ）
  非回帰、開閉・幅の永続化が復元される、backend 無変更（`agent-status` イベントのみ参照）。
  userguide に「ステータスサイドバー」節。

### Phase 4.4.2 — blocked 通知 + 要承認ハイライト

- `tauri-plugin-notification` 導入 + capability + 権限ハンドリング（6.5）。
- window focus 追跡 + `set_visible_panes` + 抑制ロジック（6.2 / 7.5）。
- 連発抑制（6.3）。
- frontend: 通知リング `.pane-blocked`（reduced-motion 対応）、通知クリック→前面化+該当ペイン強調、
  権限拒否時のトーストフォールバック、（任意）ツールバー blocked 集約バッジ。
- **completion gate**: 抑制ロジックの単体テスト（focus×可視×状態の組合せ）、連発抑制テスト、
  実機手動検証（macOS 必須／Linux ベストエフォート、10.3）、CONTRACT に通知契約 + `set_visible_panes`
  追記、userguide に「承認待ち通知」節。

> バージョン: 4.4.0 → `v0.4.z`、4.4.1（サイドバー）→ `v0.4.(z+1)`、4.4.2（通知）→ `v0.4.(z+2)`
> （plan.md の 1 リリース = 1 patch 規約）。サイドバーは backend 無変更で frontend のみなので、
> 4.4.0 と同一リリースにまとめても可（実装時に判断）。

---

## 10. テスト計画

### 10.1 純関数・ロジックテスト（backend, `cargo test`）

- **`classify` パターン→状態**: 代表的な claude/codex 出力断片（承認プロンプト・`esc to interrupt`・
  完了行）を入力に、期待 `AgentStatus` と `matchedRule` を assert。decision order（blocked>working>done>idle）
  の優先が効くことを、複数カテゴリ同時マッチ入力で確認。
- **blocked 保守性**: 未知のプロンプト・空プロンプト・シェル復帰（`$ `）が blocked に**ならない**こと
  （idle になる）を明示 assert。
- **内蔵既定の妥当性**: 同梱 YAML が全パターン正規表現としてコンパイル可能・キーが重複しないことの
  テスト（回帰で壊れないよう固定）。
- **merge/replace**: 同名キーの merge が内蔵+ユーザー連結、`replace: true` が置換、不正 regex が
  1本スキップ + 残り有効、を assert。
- **done_linger 遷移**: working→done→（linger 経過）→idle、linger 中の working 再検出で done 破棄、を
  疑似時刻で assert。
- **デバウンス**: バースト dirty で評価回数が上限（4/秒相当）に収まること、状態変化時のみ emit される
  こと（emit カウンタで確認）。
- **抑制ロジック**: `should_notify(status_edge, window_focused, pane_visible, last_notified)` の真理値表を
  網羅（フォアグラウンド+可視で抑制、片方偽で通知、blocked 継続で再通知なし、抜けて再 blocked で通知、
  renotify_ms クールダウン）。
- **ANSI 再構成連携**: スピナー残骸を含む生バイト列 → `render_terminal` → `classify` で working に
  なること（生バイト直マッチだと壊れるケースとの差分）。

### 10.2 frontend（`svelte-check` + 単体可能なら）

- `agent-status` 受信で `ui.agentStatus` 更新、`session-state: exited` でエントリ削除。
- ヘッダーバッジのクラス割当、`.pane-blocked` リングの付与/除去、unknown/非running でバッジ非表示。

### 10.3 実機手動検証（macOS 必須 / Linux ベストエフォート）

1. claude ペインで承認を要するツールを走らせ、承認プロンプトで🔴+通知リング+ネイティブ通知が出る。
2. 承認して working（🟡 `esc to interrupt`）→完了で🔵→数秒後🟢へ減衰。
3. アプリを前面 + 該当ペイン可視のときネイティブ通知が**出ない**、背面 or 別ペイン最大化時は**出る**。
4. blocked 継続中に再通知が来ない、別状態へ抜けて再 blocked で再通知が来る。
5. codex を手打ち起動（adhoc シェル）してフォアグラウンド名でルールセットが当たる。
6. `agent_status.enabled: false` / `notify: false` の各停止が効く。
7. 通知権限を拒否した状態でトーストフォールバックに落ちる。
8. config reload で `patterns` 変更が再コンパイルされ即反映される。

---

## 11. リスクと未解決事項

- **誤検出（false positive/negative）**: 正規表現ヒューリスティックは本質的に不完全。blocked 保守主義で
  false positive を抑えるが、working/done は取りこぼしうる。緩和: `matchedRule` を tooltip 表示し、
  ユーザーが ptygrid.yml で即補正できる導線を用意。将来 `agent_status_explain`（7.4）を検討。
- **パターン陳腐化**: エージェント CLI の UI 変更で内蔵既定が古くなる。herdr はリモート manifest 自動
  更新で解決するが、ptygrid はオフライン・config-as-code 方針。**未決**: (a) 内蔵既定をリリースごとに
  更新（現方針）、(b) 将来 opt-in の「ルールセット更新チャンネル」（署名付き・手動取得）を足すか。
  当面は (a) + ユーザー上書きで運用し、リモート自動更新は導入しない。
- **多言語 UI**: エージェントを英語以外の locale で使うと英語前提パターンが外れる。内蔵既定は英語のみ。
  **未決**: 非英語パターンの内蔵可否／locale 別ルールセットキー。当面はユーザー上書きに委ねる。
- **done→idle の曖昧さ**: 3.4 の done_linger で「作業直後の数秒」に限定して緩和したが、「本当に完了」か
  「一時停止」かの区別は出力だけからは原理的に困難。lifecycle hook（Phase 4.0 の SubagentStop 等）が
  取れる範囲では hook を優先する将来余地（herdr の多層方式に相当）を残す。
- **通知権限**: macOS で拒否されると価値が半減する。6.5 でトーストフォールバックするが、初回導線の
  UX（いつ権限を求めるか）は実機で要調整。
- **render_terminal のコスト**: ring 全体（256 KiB）再構成 × 評価頻度。9 面フルで問題化するなら
  「末尾ウィンドウのみ再構成」最適化が必要（3.6 に余地明記）。**未決**: 最適化の要否は実測後判断。
- **ロールアップ範囲**: tab/workspace が無いため herdr 型の階層ロールアップは対象外。ツールバー集約
  バッジ（5.3）どまり。teammate/host lead への集約は将来。
- **host teammate PTY**: host モードの実 PTY teammate（Phase 4.2）も通常 PTY として検出対象になる。
  親 lead と teammate が別ルールセットになる場合の見え方は実機確認が必要（初期は同一 `claude`
  ルールで足りる想定）。

---

## 12. 参考

- [docs/research/herdr-research.md](research/herdr-research.md) — herdr の状態検出方式（TOML manifest、
  blocked 保守判定、pane→tab→workspace ロールアップ、リモート自動更新）。本仕様の翻案元。
- [docs/competitive-landscape.md](competitive-landscape.md) — 「通知リング / 要承認ハイライト」を
  次に取る UX として位置づけ（cmux / Architect 対抗）。
- [docs/design.md](design.md) — hot path 分離・config-as-code・推測回避の原則。
- [docs/plan.md](plan.md) — バージョニング（Phase=minor）とリリース規律。
- [CONTRACT.md](../CONTRACT.md) — 既存 `session-state` / `SessionInfo` / Phase 4.x イベント・
  teammate 契約。本仕様の追記先。
