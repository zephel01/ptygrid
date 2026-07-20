# ptygrid 単体で秘密を「1回・安全に」定義する — 3系統(env / export / keychain)

CodeRouter に依存せず、ptygrid.yml のトップレベル `env:`(集約)と `${...}` の取得元拡張
(env / export ファイル / OS キーチェーン)だけで完結させる実装差分。

対象コミット基準:
- `src-tauri/src/config.rs` の `Config`(L23)/`expand_vars`(L607)/`resolve_def`(L821)
- `src-tauri/src/session.rs` の `launch_agent`(L475, `config::expanded_env(def)`) は**無改造**
  (集約は `resolve_def` 側で def にマージ、取得元は `expand_vars` 内で解決するため)

---

## 1. `Config` にトップレベル `env:` を追加 (集約)

`src-tauri/src/config.rs` の `pub struct Config { ... }`(L23〜)に1フィールド追加:

```rust
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default)]
    pub agents: Vec<AgentDef>,
    #[serde(default)]
    pub processes: Vec<AgentDef>,

    /// プロジェクト共通の env。各 agent/process の `env:` の“下”に敷かれ、
    /// キー衝突時は agent 側が勝つ(= agent で個別上書き可能)。
    /// 秘密の“参照”をここに1回だけ書き、全ペインへ配るための場所。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub queen: Option<QueenConfig>,
    // ...(以下そのまま)
}
```

マージ用ヘルパー(同ファイル内、`expanded_env` の近くに):

```rust
/// プロジェクト共通 `env:` を定義側 `env:` の下に敷いてマージした def を返す。
/// 定義側キーが優先。展開は行わない(値は `${...}` 参照のまま)。
pub fn with_shared_env(
    def: &AgentDef,
    shared: Option<&std::collections::HashMap<String, String>>,
) -> AgentDef {
    let mut merged: std::collections::HashMap<String, String> =
        shared.cloned().unwrap_or_default();
    if let Some(own) = &def.env {
        for (k, v) in own {
            merged.insert(k.clone(), v.clone()); // 定義側で上書き
        }
    }
    let mut out = def.clone();
    out.env = if merged.is_empty() { None } else { Some(merged) };
    out
}
```

`resolve_def`(L821)で、返す直前に共通 env を畳み込む:

```rust
pub fn resolve_def(&self, name: &str) -> Result<(AgentDef, PathBuf), String> {
    let inner = self.lock();
    let config = inner
        .config
        .as_ref()
        .ok_or_else(|| "no config loaded (call load_config first)".to_string())?;
    let def = config
        .agents
        .iter()
        .chain(config.processes.iter())
        .find(|d| d.name == name)
        .cloned()
        .ok_or_else(|| format!("agent or process '{name}' not found in config"))?;

    // ★ 追加: プロジェクト共通 env をマージ(定義側優先)
    let def = with_shared_env(&def, config.env.as_ref());

    let dir = inner
        .dir
        .clone()
        .ok_or_else(|| "config dir missing".to_string())?;
    Ok((def, dir))
}
```

> 注意: Queen の `spawn_agent` 等、`resolve_def` を通さずに `config.current()` から
> def を取り出して起動する経路がある場合は、そこでも同様に
> `config::with_shared_env(&def, cfg.env.as_ref())` を挟むこと。
> `launch_agent`(session.rs) 自体は `def.env` しか見ないので、
> 「def にマージ済みで渡す」方針なら session.rs は無改造で済む。

---

## 2. `expand_vars` を「取得元つき」に拡張 (3系統)

`src-tauri/src/config.rs` の `expand_vars`(L607-629)を置換。`${VAR}` の既存挙動は不変、
`keychain:` / `export:` 接頭辞だけ分岐を足す。

```rust
/// `${...}` を展開する。3つの取得元を接頭辞で切り替える:
///   ${VAR}                       -> ホスト環境変数(未定義は空)              [env]
///   ${keychain:SERVICE}          -> OS キーチェーン(account は $USER 既定)  [keychain]
///   ${keychain:SERVICE/ACCOUNT}  -> OS キーチェーン(account 明示)           [keychain]
///   ${export:NAME}               -> secrets ファイルの `export NAME=...`    [export]
/// 未終端の "${" はそのまま(既存仕様)。どの取得元も失敗時は空文字。
pub fn expand_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find('}') {
            Some(end) => {
                let token = &rest[start + 2..start + 2 + end];
                out.push_str(&resolve_ref(token));
                rest = &rest[start + 2 + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]); // 未終端はそのまま
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// 単一 `${...}` トークンを取得元に応じて解決する。
fn resolve_ref(token: &str) -> String {
    if let Some(spec) = token.strip_prefix("keychain:") {
        return keychain_lookup(spec).unwrap_or_default();
    }
    if let Some(name) = token.strip_prefix("export:") {
        return export_file_lookup(name).unwrap_or_default();
    }
    // 既定: ホスト環境変数(従来どおり)
    std::env::var(token).unwrap_or_default()
}

/// OS キーチェーンから読む。`SERVICE` または `SERVICE/ACCOUNT`。
/// ACCOUNT 省略時は $USER。取得失敗(未登録/権限)は None。
fn keychain_lookup(spec: &str) -> Option<String> {
    let (service, account) = match spec.split_once('/') {
        Some((s, a)) => (s.to_string(), a.to_string()),
        None => (spec.to_string(), std::env::var("USER").unwrap_or_default()),
    };
    keyring::Entry::new(&service, &account)
        .ok()?
        .get_password()
        .ok()
}

/// secrets ファイル(`export KEY=VALUE` 形式)から NAME の値を読む。
/// パスは env `PTYGRID_ENV_FILE`、無ければ `~/.ptygrid/secrets.env`。
fn export_file_lookup(name: &str) -> Option<String> {
    let path = secrets_file_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == name {
                return Some(unquote(v.trim()).to_string());
            }
        }
    }
    None
}

fn secrets_file_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("PTYGRID_ENV_FILE") {
        if !p.is_empty() {
            return Some(expand_home(&p));
        }
    }
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())?;
    Some(std::path::Path::new(&home).join(".ptygrid").join("secrets.env"))
}

fn expand_home(p: &str) -> std::path::PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::Path::new(&home).join(rest);
        }
    }
    std::path::PathBuf::from(p)
}

/// 値前後の対クォート("..." / '...')を1組だけ剥がす。
fn unquote(v: &str) -> &str {
    let b = v.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"')
            || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        &v[1..v.len() - 1]
    } else {
        v
    }
}
```

`expanded_env`(L648)は無改造(内部で新しい `expand_vars` を呼ぶだけ)。

---

## 3. `Cargo.toml` に keyring を追加 (プラットフォーム別)

Ubuntu CI を壊さないよう、Linux は secret-service、mac/win はネイティブに分ける。
`src-tauri/Cargo.toml` の `[dependencies]` の後に追記:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
keyring = { version = "3", features = ["apple-native"] }

[target.'cfg(target_os = "windows")'.dependencies]
keyring = { version = "3", features = ["windows-native"] }

[target.'cfg(target_os = "linux")'.dependencies]
# secret-service は D-Bus 依存。CI(Ubuntu)で keychain 実アクセスするテストは
# 走らせない(下記ユニットテストは env/export のみ)。
keyring = { version = "3", features = ["sync-secret-service", "crypto-rust"] }
```

> Linux で D-Bus を持ち込みたくない場合は、Linux では keychain 取得元を無効化し
> (`keychain_lookup` を `#[cfg(not(target_os = "linux"))]` で切り替え、Linux 版は
> `None` を返す)、Linux は **export ファイル**を正とする運用でも良い。

---

## 4. ユニットテスト (env/export はstdlibのみ・keychainは実機依存なので対象外)

`config.rs` の `#[cfg(test)]` に追加。既存 `expands_vars_from_host_env` と同じ流儀:

```rust
#[test]
fn expand_export_source_reads_secrets_file() {
    let dir = std::env::temp_dir().join(format!("ptygrid-sec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("secrets.env");
    std::fs::write(
        &f,
        "# comment\nexport GLM_KEY=sk-abc\nQWEN_KEY=\"sk-xyz\"\n",
    )
    .unwrap();
    // SAFETY: single-threaded test manipulating process env.
    unsafe { std::env::set_var("PTYGRID_ENV_FILE", &f) };

    assert_eq!(expand_vars("${export:GLM_KEY}"), "sk-abc");
    assert_eq!(expand_vars("k=${export:QWEN_KEY}"), "k=sk-xyz"); // クォート除去
    assert_eq!(expand_vars("${export:NOPE}"), "");               // 無ければ空

    unsafe { std::env::remove_var("PTYGRID_ENV_FILE") };
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shared_env_merges_under_definition() {
    use std::collections::HashMap;
    let mut shared = HashMap::new();
    shared.insert("A".into(), "shared-a".into());
    shared.insert("B".into(), "shared-b".into());
    let mut own = HashMap::new();
    own.insert("B".into(), "own-b".into()); // 定義側が勝つ
    let def = AgentDef {
        name: "x".into(),
        cmd: "true".into(),
        cwd: None,
        env: Some(own),
        autostart: None,
        autorestart: None,
        instructions: None,
        resume: None,
        worktree: None,
        teams: None,
    };
    let merged = with_shared_env(&def, Some(&shared)).env.unwrap();
    assert_eq!(merged.get("A").unwrap(), "shared-a");
    assert_eq!(merged.get("B").unwrap(), "own-b");
}
```

(`AgentDef` のフィールドが増減している場合はテスト内リテラルを合わせること。)

---

## 5. `ptygrid.example.yml` への追記(ドキュメント)

```yaml
# ── secrets: 3つの書き方 ────────────────────────────────────────────
# プロジェクト共通 env。ここに“参照”を1回書けば全ペインに配られる。
# 各 agent の env: が同キーを持つとそちらが勝つ(個別上書き)。
#
#   ① env      : ${VAR}                 既に export 済みのホスト環境変数
#   ② export   : ${export:NAME}         ~/.ptygrid/secrets.env の export 行
#                                        (PTYGRID_ENV_FILE で差し替え可・chmod600)
#   ③ keychain : ${keychain:SVC/ACCT}   OS キーチェーン(account 省略時 $USER)
#
# ①はターミナル/CI 起動向き(Finder 起動だと空になる)。②③は GUI 起動でも効く。
# env:
#   GLM_API_KEY:  "${keychain:ptygrid/glm}"
#   QWEN_API_KEY: "${export:QWEN_API_KEY}"
#   ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}"

agents:
  - name: glm-aider
    cmd: "aider --model openai/glm-4.6"
    env:
      OPENAI_API_BASE: "https://api.z.ai/api/paas/v4"  # 秘密でない=直書き
      OPENAI_API_KEY:  "${GLM_API_KEY}"                # 共通 env を参照
  - name: qwen-opencode
    cmd: "opencode"
    env:
      OPENAI_API_BASE: "https://openrouter.ai/api/v1"
      OPENAI_API_KEY:  "${QWEN_API_KEY}"
```

secrets ファイルの例(`~/.ptygrid/secrets.env`, `chmod 600`):

```
export GLM_API_KEY=sk-...
export QWEN_API_KEY=sk-...
```

keychain 登録(macOS):

```
security add-generic-password -s ptygrid -a glm  -w   # プロンプトで入力=履歴に残らない
security add-generic-password -s ptygrid -a qwen -w
```

---

## 6. 運用の詰め(漏らさない)

- `ptygrid.yml` は gitignore 済みだが、`ptygrid.yml.0717` と `router.settings.json` は
  **無視対象外**。`.gitignore` に追加するか、平文バックアップを置かない。
- `~/.ptygrid/secrets.env` は `chmod 600`。`~/.ptygrid/` はグローバル設定領域なので
  リポジトリ外=git に乗らない。
- 論理セッション永続(session.rs)では**展開後の値をディスクに書かない**
  (生の def を保存し、spawn 時に展開)。token_store の 0600 方針と一貫。
- 余力があれば、CodeRouter の `doctor --check-env` に倣い
  「ptygrid.yml に `${...}` で包まれていない生キーらしき値があれば警告」する
  軽い lint を config ロード時に足すと、事故を構造的に防げる。

---

## まとめ

- **集約**: トップレベル `env:` で秘密の参照は1回だけ。全ペインが継承。
- **3系統**: `${VAR}`(env) / `${export:NAME}`(ファイル) / `${keychain:SVC/ACCT}`(OS)。
  ②③は macOS の GUI 起動でも空にならない。
- **無改造で済む範囲**: session.rs の launch 経路は触らず、`resolve_def` でのマージと
  `expand_vars` の取得元拡張だけ。CodeRouter 非依存で ptygrid 単体完結。
