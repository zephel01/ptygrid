# ptygrid: 秘密(APIキー)の書き方 — 3系統マニュアル

作成日: 2026-07-20 / 対象: 複数の AI CLI(aider / opencode / claude / pi 等)を1つの ptygrid.yml で束ねるときの、APIキーの置き場所。

関連: [../ptygrid.example.yml](../ptygrid.example.yml)(注釈付き設定例) /
[userguide.md](userguide.md)(`ptygrid.yml` の全体像) /
[troubleshooting.md](troubleshooting.md)(GUI 起動で env が空になる件)。

---

## 1. 何を解決するのか

複数のエージェント CLI を並べたい。GLM / Qwen などは `aider` / `opencode` / `pi` のような
OpenAI 互換 CLI で各ペインに置ける(GLM は `claude` 自身を Anthropic 互換で向ける手もある。
→ 5章)。いずれにせよ詰まるのが、**APIキーをどこに、何回書くか**。

- 各 CLI ごとに `~/.aider.conf.yml`・`opencode auth login`・`pi` の設定… と**書き場所が散る**
- ptygrid.yml に直書きすると **git 事故**が怖い(`.gitignore` の隙間や、バックアップ複製が漏れる)
- macOS の GUI 起動(Finder / Dock / ptygrid のアプリアイコン)では **`.zshrc` が読まれず** `${VAR}` が空になる

ptygrid は次の2つでこれを畳む:

1. **トップレベル `env:`** — プロジェクト共通の env をひとつ書けば、全ペインに配られる
2. **`${...}` の3系統展開** — 参照だけを書き、実体は環境変数 / ファイル / OS キーチェーンから読む

秘密の**参照**は1回だけ、実体は好きな保管先から。

---

## 2. 3つの書き方 — 早見表

| 記法 | 実体の在処 | GUI起動で効く | 平文がディスクに残る | いつ選ぶ |
|---|---|---|---|---|
| `${VAR}` | シェルの環境変数(`export VAR=...`) | ✗ (シェルから起動なら○) | 起動時のシェル state 依存 | ターミナルから起動・CI |
| `${export:NAME}` | `~/.ptygrid/secrets.env`(`chmod 600`) | ○ | ○(1ファイルに集約・git外) | 手元 Mac・シンプルさ優先 |
| `${keychain:SVC/ACCT}` | OS キーチェーン(macOS Keychain 等) | ○ | ✗ | 最も安全・共有マシン |

同じ ptygrid.yml の中で**3つを混在させて構わない**。よく使うのは「日常キーは export、
本番キーだけ keychain」のような**併用**。

---

## 3. 3系統それぞれの詳しい使い方

### 3.1 `${VAR}` — 既存の環境変数を読む

書き方(ptygrid.yml):

```yaml
env:
  OPENAI_API_KEY: "${GLM_API_KEY}"
  ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}"
```

事前に、ptygrid を起動するシェルで export しておく:

```bash
# ~/.zshrc など
export GLM_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
```

その上でターミナルから `npm run tauri dev` 等で ptygrid を起動する。

**注意点(Mac):** Finder/Dock/アプリアイコンから起動した ptygrid は `.zshrc` を読まない。
その場合 `${GLM_API_KEY}` は**空文字**に展開されて渡り、CLI 側で「認証エラー」になる。
この時は 3.2 か 3.3 に切り替える。

**向いている用途:** CI(既に env が整っている)・ターミナル起動のみで完結する開発。

### 3.2 `${export:NAME}` — secrets ファイルから読む

書き方(ptygrid.yml):

```yaml
env:
  GLM_API_KEY:  "${export:GLM_API_KEY}"
  QWEN_API_KEY: "${export:QWEN_API_KEY}"
```

`~/.ptygrid/secrets.env` を作る(パーミッションは必ず `600`):

```bash
mkdir -p ~/.ptygrid
cat > ~/.ptygrid/secrets.env <<'EOF'
export GLM_API_KEY=sk-glm-...
export QWEN_API_KEY=sk-qwen-...
# コメント行と空行はスキップされる
# 値のクォート(""や'')は自動で剥がされる
EOF
chmod 600 ~/.ptygrid/secrets.env
```

ptygrid **自身**がこのファイルを読むので、GUI 起動でも Finder 起動でも同じように効く。

**別の場所に置きたい場合:** `PTYGRID_ENV_FILE` で上書きできる。

```bash
export PTYGRID_ENV_FILE=~/works/secrets/ptygrid.env
```

**書式の受理範囲:**

- `export NAME=VALUE` / `NAME=VALUE` どちらも可
- 値の前後の `"..."` / `'...'` は1組だけ剥がす
- `#` から始まる行と空行は無視

**セキュリティのコツ:**

- `~/.ptygrid/` はグローバル設定領域なのでプロジェクト git には入らない
- `PTYGRID_ENV_FILE` を**プロジェクト内**に置くのは非推奨(消し忘れの `git add .` で漏れる)。
  どうしてもプロジェクト内に置くなら、そのファイルを必ず `.gitignore` に書く
- 定期バックアップに含まれる場所(iCloud Drive / Dropbox の同期対象)は避ける

**向いている用途:** 手元 Mac の日常使い・ptygrid.example.yml をチームで共有しながらキーだけ個人持ちしたい場合。

### 3.3 `${keychain:SVC/ACCT}` — OS キーチェーンから読む

書き方(ptygrid.yml):

```yaml
env:
  GLM_API_KEY:  "${keychain:ptygrid/glm}"      # service=ptygrid, account=glm
  QWEN_API_KEY: "${keychain:ptygrid/qwen}"
  ANTHROPIC_API_KEY: "${keychain:ptygrid}"     # account 省略 = $USER
```

**保存(macOS Keychain):**

```bash
# -w に続けて値を書かない = プロンプト入力 = シェル履歴に残らない
security add-generic-password -s ptygrid -a glm  -w
security add-generic-password -s ptygrid -a qwen -w
```

登録済みの確認・削除:

```bash
security find-generic-password -s ptygrid -a glm -w   # 値の表示(Keychain 承認ダイアログが出る)
security delete-generic-password -s ptygrid -a glm    # 削除
```

**書式:** `${keychain:SERVICE/ACCOUNT}` または `${keychain:SERVICE}`
(後者は account を `$USER` として引く)。

**プラットフォーム別:**

- macOS: システムの Keychain(推奨)
- Windows: Credential Manager
- Linux: Secret Service(GNOME Keyring / KWallet)。CI 等 D-Bus が無い環境では動かない。
  Linux 常用の場合は 3.2(export ファイル)を主にする運用でよい

**取得できなかったとき:** 空文字が返る(未登録・権限拒否 等)。CLI 側で「認証エラー」が出たら
`security find-generic-password` で登録を確認する。

**向いている用途:** 平文をどこにも残したくない・共有マシン・チームのポリシー上要求される場合。

---

## 4. 集約: トップレベル `env:` と各 agent の関係

ptygrid.yml の**トップレベル `env:`** はプロジェクト共通の env。各 agent の `env:` の
「下敷き」として敷かれ、**同じキーがあれば agent 側が勝つ**(= 個別上書き可能)。

```yaml
project: my-project

env:                                       # ← ここに“参照”を1回書く
  GLM_API_KEY:  "${keychain:ptygrid/glm}"
  QWEN_API_KEY: "${export:QWEN_API_KEY}"
  ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}"

agents:
  - name: glm-aider
    cmd: "aider --model openai/glm-4.6"
    env:                                   # 秘密でない値だけ、ペインごとに個別化
      OPENAI_API_BASE: "https://api.z.ai/api/paas/v4"
      OPENAI_API_KEY:  "${GLM_API_KEY}"    # ← 共通envを参照

  - name: qwen-opencode
    cmd: "opencode"
    env:
      OPENAI_API_BASE: "https://openrouter.ai/api/v1"
      OPENAI_API_KEY:  "${QWEN_API_KEY}"

  - name: claude
    cmd: "claude"                          # ANTHROPIC_API_KEY は共通envから自動継承
```

**設計方針:**

- 「秘密の**参照**」はトップレベル `env:` に集約
- 「秘密でない**接続先**(base_url・モデル名)」は各 agent の `env:` に置く
- 秘密の**実体**は 3.1/3.2/3.3 のいずれか(または混在)

これで、キーを追加するときに触るのは「トップレベル `env:` を1行」+「取得元に保存」の2箇所だけ。

---

## 5. Claude Code(`claude`)を GLM 5.2 につなぐ — coding plan(Anthropic 互換)

ここまでの GLM 例(3.x / 4章)は `aider` 経由、つまり **OpenAI 互換**エンドポイント
(`https://api.z.ai/api/paas/v4` + `OPENAI_API_KEY`)だった。

Z.ai の GLM coding plan は、これとは別に **Anthropic 互換**エンドポイントも公開している。
そちらを使うと **`claude`(Claude Code)そのもの**を GLM 5.2 で走らせられる。
`ANTHROPIC_BASE_URL` を差し替えるだけで CLI は `claude` のまま、という点で
`local`(coderouter 経由)ペインと同じ畳み方になる。

**なぜ ptygrid でこれが効くのか:** queen / teams(spawn_agent・inbox・observe)の連携は
`claude` バイナリのペインにしか効かない。GLM を `aider` で置くとその輪に入れないが、
Anthropic 互換で **GLM を `claude` ペインとして**置けば、安いモデルをそのまま
team のメンバー(一次担当・セカンドオピニオン等)に組み込める。

### 5.1 2つのエンドポイントの違い(混同注意)

| 使う CLI | 互換方式 | Base URL | 認証の env |
|---|---|---|---|
| `aider` / `opencode` / `pi` | OpenAI 互換 | `https://api.z.ai/api/paas/v4` | `OPENAI_API_KEY` |
| `claude`(Claude Code) | Anthropic 互換 | `https://api.z.ai/api/anthropic` | **`ANTHROPIC_AUTH_TOKEN`** |

**最頻の落とし穴:** Anthropic 互換側では `ANTHROPIC_API_KEY` ではなく
**`ANTHROPIC_AUTH_TOKEN`** を使う。前者は `x-api-key` ヘッダ、後者は
`Authorization: Bearer` ヘッダで送られ、ゲートウェイ/プロキシは Bearer を期待するため、
`ANTHROPIC_API_KEY` を渡すと **401(認証エラー)**になる。

### 5.2 env の中身

`claude` を GLM 5.2 に向けるために各ペインへ渡す env:

| env | 値 | 役割 |
|---|---|---|
| `ANTHROPIC_BASE_URL` | `https://api.z.ai/api/anthropic` | 接続先(Anthropic 互換) |
| `ANTHROPIC_AUTH_TOKEN` | Z.ai の coding plan キー | 認証(=秘密。3系統で持つ) |
| `ANTHROPIC_API_KEY` | `""`(空で上書き) | 本物 Anthropic への誤送信を防ぐ |
| `ANTHROPIC_DEFAULT_HAIKU_MODEL` | `glm-4.7` | 軽量ティアの割当 |
| `ANTHROPIC_DEFAULT_SONNET_MODEL` | `glm-5.2` | 標準ティアの割当 |
| `ANTHROPIC_DEFAULT_OPUS_MODEL` | `glm-5.2` | 最上位ティアの割当 |
| `API_TIMEOUT_MS` | `3000000`(任意) | 長い生成のタイムアウト緩和 |

`claude` は内部の haiku/sonnet/opus 3ティアそれぞれに送るモデル名を上の3つの env で
差し替える。GLM 5.2 の 1M コンテキストを使う場合はモデル名を `glm-5.2[1m]` とし、
併せて `CLAUDE_CODE_AUTO_COMPACT_WINDOW: "1000000"` を置く(Claude Code 限定機能)。

> `ANTHROPIC_API_KEY: ""` の空上書きが要点。ptygrid のトップレベル `env:` に本物の
> `ANTHROPIC_API_KEY` を置いていると、GLM ペインもそれを継承してしまう。両方が
> セットされた状態はツール/エンドポイント次第でどちらが勝つか不定になり 401 の温床。
> **GLM ペインでは空文字で明示的に潰す**(agent 側 env がトップレベルに勝つ仕様)。

### 5.3 ptygrid.yml の書き方

秘密の**実体**はこれまで同様 3系統(`${VAR}` / `${export:NAME}` / `${keychain:SVC/ACCT}`)
から選ぶ。**平文でキーを直書きしない**こと。

```yaml
env:                                          # 秘密の“参照”はここに1回
  GLM_API_KEY: "${keychain:ptygrid/glm}"      # coding plan キー(実体は keychain 等)
  ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}"   # 本物 Claude 用(別ペインで使う)

agents:
  # GLM 5.2 を “claude ペイン” として走らせる(queen/teams に混ぜられる)
  - name: glm
    cmd: "claude"
    env:
      ANTHROPIC_BASE_URL: "https://api.z.ai/api/anthropic"
      ANTHROPIC_AUTH_TOKEN: "${GLM_API_KEY}"  # ← 共通envの参照を再利用
      ANTHROPIC_API_KEY: ""                   # ← 本物キーの継承を空で打ち消す
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "glm-4.7"
      ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.2"
      ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-5.2"
      API_TIMEOUT_MS: "3000000"
    autostart: false

  # 比較: 本物 Claude はトップレベルの ANTHROPIC_API_KEY をそのまま継承
  - name: claude
    cmd: "claude"
    autostart: false
```

これで `glm` ペインは中身 GLM 5.2、外から見れば普通の `claude`。queen を登録済みなら
`spawn_agent {name: "glm"}` で team に呼べるし、`daily` プリセットの standby メンバーにも置ける。

### 5.4 自動セットアップ(手元の `~/.claude` 向け)

ptygrid を介さず、手元の `claude` を丸ごと GLM に向けたいだけなら Z.ai 公式ヘルパーが早い:

```bash
npx @z_ai/coding-helper
```

これは `~/.claude/settings.json` に上記 env を書き込む。ただし ptygrid で**ペインごとに
モデルを分けたい**(GLM ペインと本物 Claude ペインを同居)場合は、この全体設定ではなく
5.3 の **agent 単位 env** で分けるほうが事故らない(`~/.claude` はプロセス全体に効くため)。

### 5.5 動作確認

- GLM ペイン内で `echo $ANTHROPIC_BASE_URL` → `https://api.z.ai/api/anthropic` が出るか
- `echo $ANTHROPIC_API_KEY` が**空**か(本物キーを潰せているか)
- `claude` 起動後、応答が返れば疎通 OK。401 が出たら 9章のトラブルシュート、特に
  「`ANTHROPIC_AUTH_TOKEN` を使えているか / `ANTHROPIC_API_KEY` を空にできているか」を確認(9章)
- env の変更は spawn 時に展開されるため、**ペイン(または ptygrid)の再起動**が必要

---

## 6. どれを選ぶか — 決定木

```
Q1. GUI(Finder/Dock/アプリアイコン)から ptygrid を起動する?
    ├─ いいえ(必ずターミナルから起動) → ${VAR} でOK
    └─ はい                            → Q2 へ
Q2. 平文ファイルが1つあることは許容できる?(chmod 600, git外)
    ├─ はい(手元 Mac、自分専用マシン)  → ${export:NAME}
    └─ いいえ(共有マシン、社ポリシー)  → ${keychain:SVC/ACCT}
```

迷ったら **`${export:NAME}`** から始めるのが導入が最も軽い(ファイル1つ作って `chmod 600`)。
運用が固まったら keychain へ順次移行、が現実的。

---

## 7. シナリオ例 — 4ペインで GLM/Qwen/Claude/local を並べる

```yaml
project: multi-agent

env:
  # 秘密の“参照”はここに1回だけ
  GLM_API_KEY:      "${keychain:ptygrid/glm}"   # 最重要は keychain
  QWEN_API_KEY:     "${export:QWEN_API_KEY}"    # 日常キーは export
  ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}"     # 既に export 済みなら env

queen:
  enabled: true

agents:
  - name: glm
    cmd: "aider --model openai/glm-4.6 --no-auto-commits"
    env:
      OPENAI_API_BASE: "https://api.z.ai/api/paas/v4"
      OPENAI_API_KEY:  "${GLM_API_KEY}"
    autostart: false

  - name: qwen
    cmd: "opencode"
    env:
      OPENAI_API_BASE: "https://openrouter.ai/api/v1"
      OPENAI_API_KEY:  "${QWEN_API_KEY}"
    autostart: false

  - name: claude
    cmd: "claude"           # ANTHROPIC_API_KEY は共通envから継承
    autostart: false

  - name: local
    cmd: "claude"           # ローカルLLM は既存の coderouter パターンでも可
    env:
      ANTHROPIC_BASE_URL:  "http://127.0.0.1:8088"
      ANTHROPIC_AUTH_TOKEN: "dummy"
    autostart: false
```

ペインを増やすときも、キーは既に共通 env に居るので、agent を1つ足すだけ。

---

## 8. 事故防止チェックリスト

**リポジトリ側:**

- `.gitignore` に **`ptygrid.yml` が入っている**か(既定で入っているが確認)
- `ptygrid.yml.0717` のような**バックアップ複製**が gitignore から漏れていないか
  → `.gitignore` に `/ptygrid.yml*` のようにパターンで塞ぐと安全
- `router.settings.json` 等、平文キーを書きうる**周辺ファイル**を gitignore に足す

**ファイルパーミッション:**

- `~/.ptygrid/secrets.env` は必ず `chmod 600`
- `stat -f "%Sp" ~/.ptygrid/secrets.env` で `-rw-------` を確認

**同期対象から外す:**

- iCloud Drive / Dropbox / OneDrive 直下に `~/.ptygrid/` を置かない
  (macOS デフォルトは `$HOME` 直下=同期対象外)
- Time Machine の除外は個人ポリシー次第

**キーローテーション:**

- `${keychain:...}` → `security delete-generic-password` して再登録
- `${export:...}` → `~/.ptygrid/secrets.env` を編集して保存
- `${VAR}` → シェルの export を書き換えて ptygrid を再起動

ptygrid の**再起動は必要**(spawn 時に展開するため、既に走っているペインには反映されない)。

---

## 9. トラブルシューティング

**「認証エラー(401 等)が CLI から返る」**

- `${VAR}` を使っていて GUI 起動 → 3.2/3.3 に切り替える
- `${export:...}` → ファイルパスを確認: `echo "${PTYGRID_ENV_FILE:-$HOME/.ptygrid/secrets.env}"`
  ファイルの中身を `cat` で確認(`export NAME=VALUE` の形式か、NAME が一致しているか)
- `${keychain:...}` → `security find-generic-password -s <SVC> -a <ACCT> -w` で取れるか確認

**「GLM を `claude` ペインで動かすと 401(5章の Anthropic 互換)」**

- `ANTHROPIC_API_KEY` ではなく **`ANTHROPIC_AUTH_TOKEN`** を使っているか(前者は `x-api-key`
  ヘッダになりゲートウェイが弾く)
- トップレベル `env:` の本物 `ANTHROPIC_API_KEY` を継承したままになっていないか。
  GLM ペインの `env:` に `ANTHROPIC_API_KEY: ""` を置いて空で潰す
- `ANTHROPIC_BASE_URL` が `https://api.z.ai/api/anthropic`(OpenAI 互換の
  `/api/paas/v4` ではない)になっているか

**「空文字が渡っているように見える」**

- 展開結果は log に**残さない設計**なので、直接見る方法は限られる。切り分けとして
  一時的に `env:` に `DEBUG_ECHO: "${...}"` を置いて、ペイン内で `echo $DEBUG_ECHO` する
  (確認後は必ず消す)

**「Linux で keychain が動かない」**

- D-Bus と Secret Service デーモン(gnome-keyring 等)が要る。ヘッドレス Linux や WSL では
  期待どおり動かないことがある。その場合は 3.2(export ファイル)を主にする

**「同じキー名を agent 側でも書いてしまった」**

- 定義側(agent の `env:`)が勝つ仕様。上書きしたつもりが逆になっていないか、
  agent の `env:` に同名キーが残っていないか確認

---

## 10. まとめ

- 秘密の**参照**はトップレベル `env:` に集約 → 全ペインが自動継承
- 秘密の**実体**は 3系統(`${VAR}` / `${export:NAME}` / `${keychain:SVC/ACCT}`)から選ぶ
- 迷ったら `${export:NAME}` から始める、共有マシンや厳しめの場面は `${keychain:...}`
- GLM 5.2 を **queen/teams に混ぜたい**なら、`claude` を Anthropic 互換
  (`ANTHROPIC_BASE_URL=https://api.z.ai/api/anthropic` + `ANTHROPIC_AUTH_TOKEN`)で
  向ける(5章)。`ANTHROPIC_API_KEY` は空で潰す
- ptygrid.yml と、そのバックアップ複製が gitignore 済みかを必ず確認。
  **キーの平文直書きは 3系統いずれかへ**(下記の注意も参照)
