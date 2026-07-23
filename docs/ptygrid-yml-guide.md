# ptygrid.yml 執筆マニュアル — agents / workflows / team_presets 編

対象読者: ptygrid.yml を書く人間、および ptygrid.yml を読んで自分やチームメイトを spawn する
エージェント自身(特に統合担当 integrator)。

**位置づけ**: [userguide.md](userguide.md) の「ptygrid.yml リファレンス」節が `agents:` /
`processes:` / `queen:` の基本を、「チームプリセット(team_presets)」節が `team_presets:` を
既にカバーしている。本マニュアルはそれを前提に、(1) userguide.md にまだ無い `workflows:`
ブロックの書き方、(2) **「書けるが動かないフィールド」を含む実装状況の正確な線引き**、
(3) 実運用で実際に踏んだ落とし穴、の3点に特化する。フィールドの網羅表は本書、日常操作の
手順は userguide.md、を使い分けてほしい。

検証時点: 2026-07-23、`v0.5.6` 時点のソース(`src-tauri/src/config.rs` /
`orchestrator.rs` / `queen.rs`)を直接確認して書いている。

---

## 1. 最初に: 実装状況マトリクス(最重要)

`ptygrid.yml` の `workflows:` は**未知フィールドを黙って無視する**設計(forward compat)。
これは「将来のバージョンの config を今のバージョンでも読める」という利点の裏返しとして、
**「書いても何も起きないフィールド」がエラーにならず気づけない**という罠になる。
実装するかどうか決める前に、必ずこの表で「パースされるか」と「実際に動くか」を分けて確認すること。

| 機能 | YAML キー | パース(バリデーション) | 実行時に効くか | 備考 |
|---|---|---|---|---|
| pipeline パターン | `pattern: pipeline` | ✅ | ✅ | 実装済み(5.0.0 MVO) |
| fan-out パターン | `pattern: fan-out` | ✅ | ✅ | 実装済み(5.0.0 MVO) |
| supervisor パターン | `pattern: supervisor` | ✅(パースは通る) | ❌ | `spawn_workflow` 実行時に `"pattern Supervisor not implemented in MVO (lands in Phase 5.0.4)"` で **明示エラー**。5.0.4 まで使えない |
| handoff パターン | `pattern: handoff` | ✅(パースは通る) | ❌ | 同上 |
| `dependsOn` | 各 step | ✅ | ✅ | 実装済み。循環検出・未知 id 参照はロード時エラー |
| `fanOut` | 各 step | ✅ | ✅ | 実装済み。`>= 2` 必須、fan-out パターン以外では宣言不可 |
| `joinOn: all` / `any` / 数値 `N` | 各 step | ✅ | ✅ | 実装済み |
| `joinOn: reply` | 各 step | ✅(パースは通る) | ❌ | 「kickoff への reply で完了」は**未実装**。exit 0 でも `AgentStatus::Done` でも完了扱いにならず、**step が永遠に RUNNING のまま止まる可能性がある**。現状は使わないこと |
| `timeoutMs` | 各 step | ✅(パースは通る) | ❌ | 超過してもタイムアウトしない。**何も起きない** — 書いても安全装置にならないので注意 |
| `retry:` | 各 step | — | — | **フィールド自体が存在しない**。`config.rs` の `WorkflowStep` に `retry` は無い。書いても構文エラーにはならず(未知キーは無視)、単に何も効かない |
| `onFailure: fail-fast` / `continue` | workflow 直下 | ✅ | ✅ | 実装済み。既定 `fail-fast` |
| `kickoff` | 各 step | ✅ | ✅ | 実装済み。spawn 直後に inbox へ配送 |
| `arena: true` | workflow 直下 | ✅(パースは通る) | ❌ | Arena UI 自体が未実装(S7 前半、別 Phase)。書いても何も開かない |
| `autoClose`(workflow 単位) | workflow 直下 | ✅ | ✅ | 実装済み(5.0.0 追補)。詳細は §4 |
| `close_on_exit`(agent 単位) | agent 定義 | ✅ | ✅ | 実装済み(5.0.0 追補)。詳細は §4 |
| workflow 再開(アプリ再起動後の Y/N) | (config には書かない) | — | ✅ | 実装済み(5.0.1)。詳細は §5 |
| escalation(retry 枯渇時の外部通知) | (config には書かない) | — | ❌ | retry 自体が無いので発火しようがない |

> [!WARNING]
> `docs/inside/spec-phase5-0.md` はこの機能セットの**設計ドキュメント**であり、`retry` /
> `timeoutMs` 実行 / `joinOn: reply` / supervisor / handoff を実装込みで説明している箇所がある。
> spec は「最終形の設計」を書いたもので、「今何が動くか」は上の表と `CONTRACT.md` の
> 「Phase 5.0 追加契約」節(状態: 5.0.0 実装済み、5.0.1 以降は設計のみ、と明記)を信じること。
> spec と実装の間にギャップがあるのは普通で、悪いことではない — 混同しなければ問題ない。

---

## 2. workflows: の書き方

### 2.1 全体構造

```yaml
workflows:
  <workflow 名>:
    pattern: pipeline | fan-out          # 必須。supervisor/handoff は5.0.4まで実行不可(§1)
    onFailure: fail-fast | continue      # 任意、既定 fail-fast
    arena: true                          # 任意、現状 UI 未配線(§1)なので実質意味なし
    autoClose: never | success | always  # 任意、既定 never(§4)
    steps:
      - id: <step 名>                    # 必須。workflow内で一意
        agent: <agents: の定義名>         # 必須。processes: は不可
        dependsOn: [<先行 step id>, ...] # 任意。pipeline は最大1件、省略でルート
        fanOut: <N>                      # fan-out パターンのみ、>= 2
        joinOn: all | any | <N>          # fan-out の集約規則。既定 all。reply は§1参照
        timeoutMs: <ミリ秒>              # パースのみ、実行時未enforced(§1)
        kickoff: "<inbox に投函する初回メッセージ>"  # 任意だが強く推奨(§3.3)
```

`agent`(step)、`agents:`(定義参照)、`agent:`(team_presets メンバー)と紛らわしい名前が
並ぶが、**workflow の step から起動できるのは `agents:` に定義された名前だけ**
(許可リスト方式、`team_presets` と同じ思想)。`processes:` はワークフローの step にはできない。

### 2.2 バリデーションルール(ロード時に失敗する条件)

`workflows:` は `parse_config` の中で検証され、**1つでも違反があると config 全体の
読み込みが失敗する**(`team_presets:` と同じ厳格さ)。エラーメッセージは
`workflows.<名前>: ...` の形で該当箇所を名指しする。

- workflow 名が空文字列
- `steps` が空
- step `id` が空、または同一 workflow 内で重複
- `agent` が `agents:` に存在しない名前(`processes:` の名前も不可)
- `dependsOn` が同一 workflow 内に存在しない step id を指す
- `dependsOn` に自分自身を指定(自己依存)
- `dependsOn` チェーンに循環がある(DFS 3色塗り分けで検出)
- `pattern: pipeline` なのに、いずれかの step が `dependsOn` を2件以上持つ
  (pipeline は線形 DAG。分岐/合流したいなら fan-out か将来の supervisor を使う)
- `pattern: pipeline` なのに `fanOut` を宣言している step がある
  (`pattern: fan-out` を使うこと)
- `pattern: fan-out` なのに、どの step も `fanOut` を宣言していない
- `fanOut` が 2 未満

**未知フィールドは無視される**(forward compat)。ただし `pattern` / `joinOn` の名前付き値
(`all`/`any`/`reply`)のように**閉じた列挙**は綴り間違いで即エラーになる —
`joinOn: al` のようなタイポは検出されるが、フィールド名自体のタイポ(`fanOutt:` など)は
検出されない。新しいフィールドを書いたら、必ず §1 の表と照らして「本当に効くか」を確認する
習慣をつけること。

### 2.3 実例: 4-step パイプライン(実運用中の track-b-mcp-otel)

以下は実際に日次開発で使っている workflow をそのまま抜粋したもの。
`opus-planner`(設計)→ `sonnet-coder`(実装)→ `opus-reviewer`(レビュー)→
`sonnet-docs`(文書化)という「1 patch のライフサイクル」を pipeline 1本で表現している。

```yaml
workflows:
  track-b-mcp-otel:
    pattern: pipeline
    steps:
      - id: design
        agent: opus-planner
        kickoff: >-
          Phase 5.5.0「MCP 2026-07-28 RC 両立ルータ」の設計です。
          docs/inside/spec-phase5-5.md の §3.1(M1)と §5(Contract)を読み、
          設計方針を pins キー「design-5.5.0」に書き出してください。
          設計のみでコードは書かない(ワーキングツリーを汚さない)。
      - id: implement
        agent: sonnet-coder
        dependsOn: [design]
        kickoff: >-
          最初に必ず git switch -c track/b-mcp-5.5.0 で作業ブランチを
          切ること(main 直接編集は禁止)。pins キー「design-5.5.0」の
          設計方針に従い実装してください。cargo test / clippy 通過を
          確認してから commit すること。ビルドが通らない状態で
          作業を中断・終了しない。
      - id: verify
        agent: opus-reviewer
        dependsOn: [implement]
        kickoff: >-
          git switch track/b-mcp-5.5.0 に移動し、main との diff を
          敵対的にレビューしてください。差し戻しは具体的な修正指示を、
          approve なら根拠を reply_inbox で。main へはマージしない
          (マージは人間が行う)。
      - id: docs
        agent: sonnet-docs
        dependsOn: [verify]
        kickoff: >-
          approve 済みの変更について CONTRACT.md と docs/userguide.md を
          追記のみで更新してください。既存契約は破壊しない。
```

この実例が体現している運用ルールは3つ:

1. **kickoff にブランチ切り替え手順を埋め込む**。`workflows:` の YAML にブランチ名の
   フィールドは無い(worktree 分離は `agents[].worktree` で別に持つ機構)ので、
   「main を汚さない」を守らせるには kickoff 本文で明示的に指示するしかない。
2. **各 kickoff が「何を読み」「何をし」「どこに結果を書き」「完了条件は何か」を
   1つずつ明示している**。曖昧な指示(「よしなにやって」)は書きかけ停止を誘発する(§3.3)。
3. **verify(レビュー)step は自分でマージしない**、と明記している。DAG 上は
   `verify` の後に `docs` が続くだけで、マージという操作自体はどの step にも
   属していない — マージは常に人間が握る(自主運用ガイドの原則)。

### 2.4 実例: fan-out(N並列生成 + 集約)

```yaml
workflows:
  triple-review:
    pattern: fan-out
    steps:
      - id: candidate
        agent: claude-local
        fanOut: 3            # 同じ agent 定義から3並列 spawn
        joinOn: all          # 既定。3本とも成功するまで待つ("any"なら最初の1本で確定)
        kickoff: "この feature の設計案を1つ書いてくれ。"
```

`fanOut` は同じ `agents:` 定義名を複数 session 立ち上げる。**copies(並列数)が 2 以上のときは
必ず新規 spawn**(同名の既存 live session があっても再利用しない)。これは `spawn_agent` /
pipeline の通常挙動(同名の生きているセッションがあれば再利用してスキップ)とは
**意図的に異なる**動きなので、「fan-out で立てたはずが1本しかペインが増えない」と
思ったら、まず copies 数と既存セッションの有無を疑うのではなく、この仕様どおり
新規3本立っているか(idの末尾に `#run-<runId>-<i>` のような論理タグが付く)を確認すること。

`joinOn: any` にすると最初の1本が成功した時点で親が進み、**残りの fan-out step は
自動的に CANCELLED になる**(kill_pty される)。「まだ見たいのに消えた」を避けたいなら
`joinOn: all` を使う。

---

## 3. agents: / processes: の書き方(userguide.md の補完)

userguide.md の「フィールド一覧」がベースフィールド(`name` / `cmd` / `cwd` / `env` /
`autostart` / `autorestart` / `resume` / `worktree.*`)を網羅している。本書では
そこに無い、または workflow 運用と関わりが深いフィールドだけ補う。

| フィールド | 必須 | デフォルト | 説明 |
|---|---|---|---|
| `.close_on_exit` | - | `never` | `never` / `success` / `always`。セッションが exited になったとき、`success` なら exit code 0 のときだけ、`always` なら常に、**3秒後**にペインを自動クローズ。詳細は §4 |
| `.instructions` | - | - | エージェント固有の役割指示。**workflow の kickoff とは別物**(§3.3参照) — `instructions` は概念上の役割説明、`kickoff` は「今この瞬間やるべき具体タスク」 |
| `.teams.*` | - | - | Claude Code teammate/subagent 検出(Phase 4.1/4.2)。workflow とは独立した機能なので、workflow 内の step agent に `teams:` を付けても干渉しない |

### 3.1 定義済み CLI プロンプトへの初期プロンプト埋め込み

実運用では、対話 CLI 自体の起動引数に長い日本語プロンプトを埋め込むパターンを多用している
(下記は `opus-planner` の実際の定義):

```yaml
agents:
  - name: opus-planner
    cmd: >-
      claude --model claude-opus-4-7
      "Queen MCP の inbox ツールで mailbox「opus-planner」宛のメッセージを読んで。
      あなたは設計担当。届いた指示に従い、該当 spec を読んで実装方針を
      pins に短く書き出す。実装コードは書かない。終わったら元メッセージに
      reply_inbox で要約を返して作業終了。"
    cwd: "."
    autostart: false
    instructions: >-
      あなたは Track の設計担当(Opus)。docs/inside/spec-phase5-5.md の該当節を読み、
      実装方針を pins に短く書き出す。実装コードは書かない。
```

`cmd` の引数として渡した文字列は CLI 起動直後の最初のプロンプトになる。YAML の `>-`
(折り返しを空白に変換、末尾改行なし)で複数行を1つの引用符付き文字列にまとめている。
`instructions` フィールドはこの CLI 引数プロンプトと**内容が重複している**ように見えるが、
役割が違う: `cmd` の埋め込みプロンプトは**そのエージェント固有の CLI に渡す起動時引数**、
`instructions` は ptygrid 側が保持する「このエージェントの役割」のメタ情報で、
`team_presets` や将来のツールが参照する。どちらか一方だけ書いても動作はするが、
両方書いておくと「なぜこのエージェントがこう動くのか」が ptygrid.yml を読むだけで分かる。

### 3.2 通し番号の付け方(integrator を含む役割セット)

実運用では「設計(Opus)→実装(Sonnet)→レビュー(Opus)→文書化(Sonnet)」の4役 +
「統合担当(integrator)」を `agents:` に常設し、`workflows:` からはこの許可リストの
名前だけを参照する形にしている。integrator の定義と使い方は
[autonomous-operation-guide.md](autonomous-operation-guide.md) §6 を参照。

### 3.3 kickoff を書かない step は危険(joinOn: reply が未実装であることの帰結)

§1 の表のとおり `joinOn: reply` は現状使えない。つまり **step の完了判定は「PTY が
exit code 0 で終了した」か「`AgentStatus` が `done` になり `done_linger_ms` 経過した」の
どちらか**しかない。対話 CLI(Claude Code 等)はタスクが終わっても自然終了しないので、
実質的に判定は「意味的 done」頼みになる。

ここで **kickoff を書かない(または空の)step** を作ると、起動直後にエージェントが
「inbox は空です」で応答して即座に `AgentStatus::Done` を検出されてしまい、
**実質何もしていないのに step が SUCCEEDED 扱いになって次の step へ進む**という事故が起きる
(「空振り done」問題、実際に発生した)。`joinOn: reply` が実装されるまでの回避策は:

- **すべての step に具体的な kickoff を書く**(§2.3 の実例のように、何を読み・何をし・
  完了条件は何かまで明示する)
- 「早々に諦めた」と「やり遂げた」を意味的状態だけでは区別できないという前提を忘れず、
  重要な workflow は**人間または integrator が完了直後に成果物を確認する**運用でカバーする

---

## 4. autoClose / close_on_exit(終了ペインの自動クローズ)

デフォルトでは、セッションが終了(exit)しても ptygrid はペインを残す(exit code や最終出力を
見落とさないための意図的な仕様)。これを自動化したいときのフィールドが2つある。**別の階層に
別の綴り規則で存在する**ので混同注意:

| 階層 | フィールド名(綴り) | 型 | 対象 |
|---|---|---|---|
| `agents[]` 単位 | `close_on_exit`(snake_case) | `never`(既定) / `success` / `always` | そのエージェント定義から起動された、workflow に属さないセッション全般 |
| `workflows.<name>` 単位 | `autoClose`(camelCase) | `never`(既定) / `success` / `always` | その workflow run の各 step のセッション |

```yaml
agents:
  - name: t1
    cmd: "sh -c '...'"
    close_on_exit: success   # exit code 0 のときだけ、3秒後に自動クローズ

workflows:
  smoke:
    pattern: pipeline
    autoClose: success       # workflow 由来のセッションはこちらが優先
    steps:
      - id: a
        agent: t1
      - id: b
        agent: t2
        dependsOn: [a]
```

**優先順位**: あるセッションが「現在実行中のいずれかの workflow run の、いずれかの step の
sessionId」と一致する場合、**その workflow の `autoClose` だけが評価され、agent 定義側の
`close_on_exit` は一切参照されない**。workflow に属さない通常起動(手動 ▶、`spawn_agent`、
`team_presets`)は `close_on_exit` が評価される。

**共通の意味論**:

- `success`: step(または agent セッション)が `succeeded` になったときだけクローズ対象。
  `failed` / `cancelled` は**絶対にクローズしない**(デバッグに必要なため)。
- `always`: `failed` / `cancelled` も含めてクローズする。
- クローズは**判定成立から3秒後**に実行される(即座には消えない)。
- 発火直前に「まだ終了状態のままか」「maximized 中でないか」を再チェックする。
  **maximized(全画面表示)中のペインは自動クローズされない。**
- 判定・実行ロジックは**すべて frontend 側**(`stores.svelte.ts` の `shouldAutoClose` /
  `scheduleAutoClose`)。Rust 側は enum のパースのみを担当する。

---

## 5. workflow の再開(アプリ再起動をまたぐ Resume/Abandon)

アプリが落ちる・再起動すると、in-memory の workflow registry は消えるが、`queen.sqlite3`
の `workflow_runs` テーブル(`PRAGMA user_version` 3)に write-through で永続化されているため
実行中だった run を検出できる(5.0.1、実装済み)。

- `load_config` 成功時、その project で `state='running'` のまま残っている run があれば、
  frontend に「前回の run '<name>' が途中で中断されています。再開しますか?」という
  Y/N バナーが出る(複数 run があれば複数バナー)。
- **再開**を選ぶと `resume_workflow` が呼ばれ、`succeeded` / `failed` / `skipped` だった
  step はそのまま保持し、**`running` だった step だけ `pending` に戻して再 spawn**する
  (PTY プロセス自体は死んでいるので新しいペインとして立ち上がるが、エージェントは
  inbox / pins から前回の文脈を回収できる想定)。同じ `run_id` のまま既存の driver が
  続きを進める。
- **破棄**を選ぶと `abandon_workflow` が呼ばれ、DB 上で `state='cancelled'` +
  `error="abandoned after restart"` に更新される(以後同じ run で再プロンプトされない)。
- config 側で書く設定項目は無い(挙動は常時有効)。ただし resume 前に `ptygrid.yml` を
  編集してその workflow 定義自体を消してしまうと、「定義なし」エラーになりバナー表示のみで
  再開できない — workflow 定義を変更中は、実行中の run が無いことを確認してから編集する。

---

## 6. team_presets との使い分け

`team_presets`(Phase 4.3)は「複数エージェントを依存関係なしで一括起動する」薄いラッパー、
`workflows`(Phase 5.0)は「依存関係(DAG)を持つ実行」。**両方使える場面では workflows の方が
表現力が高いが、team_presets の方が単純で読みやすい**。「ただ全員起動して pins を見て
好きに動いてもらう」ような緩い並列(実運用の `daily` preset がこれ)には team_presets、
「A が終わってから B、B が3並列で終わったら C」のような**順序・集約が必須**の作業には
workflows を選ぶとよい。書き方は userguide.md 「チームプリセット(team_presets)」節を参照。

---

## 7. 実際に踏んだ落とし穴(実運用ログより)

### 7.1 `agents:` からエージェントを消すと workflows/team_presets ごと config が壊れる

`workflows.<name>.steps[].agent` と `team_presets.<name>.members[].agent` は
どちらも `agents:` への参照で、**参照先が消えると config 全体の読み込みが `parse_config`
の時点で失敗する**。1つの workflow のために追加した agent 定義を「もう使わないから」と
消すと、**その workflow を触っていなくても他の agent チップまで全部消える**
(config 読み込み自体が失敗するため)。エージェント定義を消す前に
`grep -n "agent: <名前>" ptygrid.yml` で参照元(workflows/team_presets 双方)を確認すること。

### 7.2 複数個所に ptygrid.yml があると「意図しない方」が読まれる

探索順序は「作業フォルダ内 → アプリ起動フォルダ → `~/.ptygrid/ptygrid.yml`」
(userguide.md 参照)。実運用で、`workflows:` を追記したつもりが実際には
プロジェクト外の古い `~/.ptygrid/ptygrid.yml`(workflows だけあって agent 定義が無い版)が
先に読まれてしまい、**バリデーションエラーで agent チップが全部消える**事故があった。
「設定を直したのにチップが出ない/減った」ときは、まずツールバーの設定バッジ
(`プロジェクト内` / `起動フォルダ` / `~/.ptygrid` / `既定`)で**どこを読んだか**を確認する。

### 7.3 fan-out のつもりが1本しかペインが増えない

§2.4 に既述: fan-out の `copies >= 2` は常に新規 spawn だが、pipeline の通常 step
(fan-out していない step)は**同名 live session があれば再利用してスキップする**
冪等ロジックが効く。「fan-out で3本のはずが1本しか出ない」場合、fanOut の値そのものが
`1` になっていないか、pattern を `fan-out` にし忘れて `pipeline` のままになっていないかを
先に疑うこと(pipeline では `fanOut` 宣言自体がバリデーションエラーで弾かれるので、
この事故は「エラーにならず黙って1本になる」パターンではなく気づきやすいが、念のため)。

### 7.4 `retry:` / `timeoutMs` を書いても保険にならない

§1 の表のとおり、`retry:` は存在しないフィールド、`timeoutMs` はパースされるだけで
実行時に効かない。**「timeoutMs: 600000 と書いたから10分でタイムアウトするはず」という
前提で運用しない**こと。現状、詰まった step を止めたいときは人間または integrator が
`kill_pty` 相当の操作(ペインを手動で閉じる、または integrator に該当セッションの停止を
依頼)を行う必要がある。

---

## 8. 検証コマンド

変更を保存する前に、可能であれば以下を実行して壊れていないことを確認する
(自主運用ガイドの「ビルドが通らない状態で中断しない」原則そのもの):

```bash
cd src-tauri && cargo test && cargo clippy -- -D warnings
npm run check   # svelte-check
```

`ptygrid.yml` 自体の構文だけを確かめたい場合は、アプリの「読み込み」ボタンを押すのが
最速(`parse_config` が即座にエラーメッセージを返す)。ファイル監視が効いているので、
保存すると自動で「Reload」トーストが出る。

---

## 9. 関連ドキュメント

- [userguide.md](userguide.md) — 基本フィールド一覧、信頼確認、team_presets、Queen セットアップ
- [troubleshooting.md](troubleshooting.md) — Queen MCP 接続・Inbox・pins conflict 等の実運用トラブル集
- [autonomous-operation-guide.md](autonomous-operation-guide.md) — kickoff の書き方の鉄則、integrator の運用、障害分類
- [inside/spec-phase5-0.md](inside/spec-phase5-0.md) — workflows の設計ドキュメント(§1 の警告を踏まえて読むこと)
- [CONTRACT.md](../CONTRACT.md) 「Phase 5.0 追加契約」節 — 実装状況の一次情報
- [ptygrid.example.yml](../ptygrid.example.yml) — 注釈付きの基本設定例(workflows は未収録、追記の余地あり)
