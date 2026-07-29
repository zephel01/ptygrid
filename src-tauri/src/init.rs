//! Phase 5.0.2 `ptygrid init` — detection + `ptygrid.yml` generation.
//!
//! Backend half of spec-init-5.0.2.md (§3 design / §4 generated file / §5.2
//! wire). Three moving parts, deliberately small:
//!
//! 1. **Detection** ([`scan`]): agent CLIs on `PATH`, project kind, git repo,
//!    a live local LLM router, and any pre-existing config file. Every probe is
//!    best-effort — a failing probe never fails the scan (spec §3.2).
//! 2. **Generation** ([`render_config`]): a string template, NOT
//!    `serde_norway::to_string`. Serializing a `Config` drops every comment and
//!    emits `env` maps in nondeterministic order, which would break both the
//!    value of the output and its byte-for-byte idempotency (spec §3.1).
//! 3. **Self-check + write** ([`preview`] / [`write`]): the generated text must
//!    pass [`crate::config::parse_config`] before anything is written, and the
//!    check is re-run on the (user-editable) content handed to [`write`]
//!    (spec §3.5). Writes go through temp + rename, never a bare `fs::write`.
//!
//! Two rules this module never bends:
//!
//! - **It never rewrites an existing config.** If a `ptygrid.yml` already sits
//!   in the destination directory the output goes to the sidecar
//!   `ptygrid.init.yml` instead, and the existing file is read as text only,
//!   for diffing (spec §3.4).
//! - **It never touches trust.** Generated definitions are all
//!   `autostart: false` and `trust::add_trusted` is not called — whether a
//!   folder is trusted stays an explicit user decision (spec §3.3 / §3.7).

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{
    launch_dir, parse_config, resolve_config_path_pure, ConfigOrigin, CONFIG_FILE_NAME,
    GLOBAL_CONFIG_DIR, LEGACY_CONFIG_FILE_NAME,
};

/// Output filename used when a config file already exists in the destination
/// directory. Deliberately NOT one of the names config resolution searches
/// (`ptygrid.yml` / `mterm.yml`), so it neither shadows the user's config nor
/// trips the watcher's filename filter (spec §3.4).
pub const SIDECAR_FILE_NAME: &str = "ptygrid.init.yml";

/// Local LLM router port probed by D4. The default used by
/// `router.settings.json` and `example/team-preset`.
const ROUTER_PORT: u16 = 3456;

/// Connect timeout for the D4 probe. Loopback only; kept short so a dead port
/// costs nothing noticeable and a filtered one cannot stall the caller.
const ROUTER_PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// Comment wrap column for the generated header (character count, not display
/// width — good enough for a comment and keeps the output deterministic).
const COMMENT_WIDTH: usize = 72;

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Project-kind markers, probed (and reported) in this fixed order.
const PROJECT_MARKERS: &[(&str, &str)] = &[
    ("cargo", "Cargo.toml"),
    ("npm", "package.json"),
    ("python", "pyproject.toml"),
    ("go", "go.mod"),
];

// ---------------------------------------------------------------------------
// wire types (spec §5.2) — camelCase, matching the existing command surface
// ---------------------------------------------------------------------------

/// Where the generated file goes. `project` = `<work>/ptygrid.yml` (the
/// default), `global` = `~/.ptygrid/ptygrid.yml`.
///
/// The choice is a security decision, not a convenience one: `Global` is
/// unconditionally trusted by [`crate::trust`], so autostart there runs without
/// a prompt. `Project` is the default for exactly that reason (spec §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InitTarget {
    #[default]
    Project,
    Global,
}

/// The config file found by the existing search order, if any.
/// `legacy` marks the old `mterm.yml` name.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingConfig {
    pub path: String,
    pub origin: ConfigOrigin,
    pub legacy: bool,
}

/// Result of detection (spec §3.2). Every field degrades to "not found" rather
/// than to an error.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitScanReport {
    /// Scanned working folder (absolute).
    pub dir: String,
    /// Agent CLI names found on `PATH`, in `KNOWN_AGENTS` declaration order.
    pub agents: Vec<String>,
    /// `"cargo" | "npm" | "python" | "go"`, in [`PROJECT_MARKERS`] order.
    pub project_kinds: Vec<String>,
    pub git_repo: bool,
    /// Port of the local LLM router that answered, or `None`.
    pub router_port: Option<u16>,
    pub existing: Option<ExistingConfig>,
}

/// Generated content + destination + self-check result. Nothing is written.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitPreview {
    pub content: String,
    pub path: String,
    pub target: InitTarget,
    pub sidecar: bool,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_content: Option<String>,
    pub scan: InitScanReport,
}

/// Outcome of a successful write.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitWriteResult {
    pub path: String,
    pub bytes: usize,
    pub sidecar: bool,
    /// True when a `project`-origin config carrying an `autostart: true`
    /// definition was written, i.e. the next load is expected to raise the
    /// trust prompt. init generates only `autostart: false`, so this is false
    /// unless the user edited the preview. Note that sidecar output is never
    /// loaded, so the prompt cannot actually fire until the file is renamed.
    pub trust_prompt_expected: bool,
}

// ---------------------------------------------------------------------------
// D1 — agent CLIs on PATH
// ---------------------------------------------------------------------------

/// Executable names to look for, derived from `pty::KNOWN_AGENTS`.
/// Path-anchored tokens (`sourcegraph/amp`) contribute their last segment,
/// which is both the launcher name and the agent name ptygrid displays.
/// Order is preserved and duplicates dropped, so generated `agents[].name`
/// values are unique by construction (spec §3.5).
fn agent_candidates() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for name in crate::pty::KNOWN_AGENTS {
        let bin = name.rsplit('/').next().unwrap_or(name);
        if !out.contains(&bin) {
            out.push(bin);
        }
    }
    out
}

/// Pure PATH scan: for each candidate, the first `dir/name{ext}` accepted by
/// `is_exec` counts as a hit. `exts` holds the extensions to try (just `""` on
/// Unix; `PATHEXT` plus `""` on Windows).
fn find_agents_pure(
    path_dirs: &[PathBuf],
    exts: &[String],
    is_exec: &dyn Fn(&Path) -> bool,
) -> Vec<String> {
    let mut found = Vec::new();
    for bin in agent_candidates() {
        let hit = path_dirs.iter().any(|dir| {
            exts.iter()
                .any(|ext| is_exec(&dir.join(format!("{bin}{ext}"))))
        });
        if hit {
            found.push(bin.to_string());
        }
    }
    found
}

fn path_dirs() -> Vec<PathBuf> {
    match std::env::var_os("PATH") {
        Some(raw) => std::env::split_paths(&raw).collect(),
        None => Vec::new(),
    }
}

#[cfg(windows)]
fn path_exts() -> Vec<String> {
    // Bare name first (a real .exe reachable without an extension), then the
    // PATHEXT list — npm-installed CLIs are `.cmd` shims on Windows.
    let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut out = vec![String::new()];
    for ext in raw.split(';').filter(|s| !s.is_empty()) {
        out.push(ext.to_ascii_lowercase());
    }
    out
}

#[cfg(not(windows))]
fn path_exts() -> Vec<String> {
    vec![String::new()]
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

// ---------------------------------------------------------------------------
// scan (D1..D5)
// ---------------------------------------------------------------------------

/// Injection points for [`scan_pure`], so detection is testable without a
/// specific host (same technique as `config::resolve_config_path_pure`).
pub(crate) struct ScanEnv<'a> {
    pub dir: &'a Path,
    pub launch: Option<&'a Path>,
    pub home: Option<&'a Path>,
    pub is_file: &'a dyn Fn(&Path) -> bool,
    pub path_dirs: &'a [PathBuf],
    pub exts: &'a [String],
    pub is_exec: &'a dyn Fn(&Path) -> bool,
    pub is_git_repo: &'a dyn Fn(&Path) -> bool,
    pub router_alive: &'a dyn Fn(u16) -> bool,
}

pub(crate) fn scan_pure(env: &ScanEnv) -> InitScanReport {
    let agents = find_agents_pure(env.path_dirs, env.exts, env.is_exec);

    let project_kinds = PROJECT_MARKERS
        .iter()
        .filter(|(_, marker)| (env.is_file)(&env.dir.join(marker)))
        .map(|(kind, _)| (*kind).to_string())
        .collect();

    let existing = resolve_config_path_pure(env.dir, env.launch, env.home, env.is_file)
        .ok()
        .map(|(path, origin)| ExistingConfig {
            legacy: path
                .file_name()
                .is_some_and(|n| n == LEGACY_CONFIG_FILE_NAME),
            path: path.display().to_string(),
            origin,
        });

    InitScanReport {
        dir: env.dir.display().to_string(),
        agents,
        project_kinds,
        git_repo: (env.is_git_repo)(env.dir),
        router_port: (env.router_alive)(ROUTER_PORT).then_some(ROUTER_PORT),
        existing,
    }
}

/// Make a caller-supplied working folder absolute and lexically clean, so
/// `InitScanReport.dir` (and every path derived from it, including the
/// destination and the generated `project:` name) really is the absolute path
/// the wire contract promises.
///
/// `.` and `..` are folded away lexically rather than via `canonicalize`:
/// resolving symlinks here would silently retarget the write, and the
/// symlinked-working-folder question is still open (spec §9). A relative input
/// is joined onto the process cwd; if that cannot be read the input is returned
/// unchanged, since detection is best-effort and must not fail.
pub(crate) fn absolute_dir(dir: &Path) -> PathBuf {
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(dir),
            Err(_) => return dir.to_path_buf(),
        }
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // `..` above the root is the root itself; keep a literal `..`
                // only when there is no root to stay at.
                if !out.pop() && out.as_os_str().is_empty() {
                    out.push(component.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        absolute
    } else {
        out
    }
}

/// D4: is something listening on the loopback router port? A refused or timed
/// out connection simply means "no router" (spec §3.2 best-effort). Runs on the
/// calling thread with a short timeout, so no blocking I/O reaches the PTY
/// reader or the async runtime.
fn router_alive(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, ROUTER_PROBE_TIMEOUT).is_ok()
}

/// D3: `git rev-parse --show-toplevel` via the existing git service. A missing
/// `git` executable is just "not a repo" here.
fn is_git_repo(dir: &Path) -> bool {
    crate::git_service::repository_root(dir).is_ok()
}

/// Run every detection against the real host. Never fails. `dir` is made
/// absolute first, so the report (and anything derived from it) matches the
/// absolute-path wire contract.
pub fn scan(dir: &Path) -> InitScanReport {
    let dir = absolute_dir(dir);
    let home = crate::pty::home_dir().map(PathBuf::from);
    let launch = launch_dir();
    let dirs = path_dirs();
    let exts = path_exts();
    scan_pure(&ScanEnv {
        dir: &dir,
        launch: launch.as_deref(),
        home: home.as_deref(),
        is_file: &|p| p.is_file(),
        path_dirs: &dirs,
        exts: &exts,
        is_exec: &is_executable,
        is_git_repo: &is_git_repo,
        router_alive: &router_alive,
    })
}

// ---------------------------------------------------------------------------
// generation (spec §4)
// ---------------------------------------------------------------------------

/// Terminal columns a string occupies, counting the CJK/full-width ranges as
/// two. Only used to line comments up, so an approximation is fine — but it has
/// to be a deterministic one, since the generated bytes must not vary.
fn display_width(text: &str) -> usize {
    text.chars()
        .map(|c| {
            let cp = c as u32;
            let wide = (0x1100..=0x115F).contains(&cp)
                || (0x2E80..=0x303E).contains(&cp)
                || (0x3041..=0x33FF).contains(&cp)
                || (0x3400..=0x4DBF).contains(&cp)
                || (0x4E00..=0x9FFF).contains(&cp)
                || (0xA000..=0xA4CF).contains(&cp)
                || (0xAC00..=0xD7A3).contains(&cp)
                || (0xF900..=0xFAFF).contains(&cp)
                || (0xFE30..=0xFE6F).contains(&cp)
                || (0xFF00..=0xFF60).contains(&cp)
                || (0xFFE0..=0xFFE6).contains(&cp)
                || (0x20000..=0x3FFFD).contains(&cp);
            if wide {
                2
            } else {
                1
            }
        })
        .sum()
}

/// `# <label>: a / b / c`, wrapped with a `/`-terminated continuation like the
/// spec's example header. Pure and deterministic.
fn wrap_comment(label: &str, items: &[String]) -> String {
    let head = format!("# {label}: ");
    let cont = format!("#{}", " ".repeat(display_width(&head).saturating_sub(1)));
    if items.is_empty() {
        return format!("{head}なし");
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = head;
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            if display_width(&cur) + 3 + display_width(item) > COMMENT_WIDTH {
                cur.push_str(" /");
                lines.push(std::mem::replace(&mut cur, cont.clone()));
            } else {
                cur.push_str(" / ");
            }
        }
        cur.push_str(item);
    }
    lines.push(cur);
    lines.join("\n")
}

/// Quote a scalar only when it needs it, so the common case reads like the
/// hand-written examples (`project: my-app`).
fn yaml_scalar(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if plain {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn project_name(dir: &str) -> String {
    Path::new(dir)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "project".to_string())
}

fn marker_for(kind: &str) -> &'static str {
    PROJECT_MARKERS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, m)| *m)
        .unwrap_or("?")
}

/// What the header records as found (spec §3.2: "何を見て何を出したか").
fn detected_items(scan: &InitScanReport) -> Vec<String> {
    let mut items = Vec::new();
    if !scan.agents.is_empty() {
        items.push(format!("{} (PATH)", scan.agents.join(" / ")));
    }
    for kind in &scan.project_kinds {
        items.push(marker_for(kind).to_string());
    }
    if scan.git_repo {
        items.push("git リポジトリ".to_string());
    }
    if let Some(port) = scan.router_port {
        items.push(format!("ローカル LLM ルータ 127.0.0.1:{port} (応答あり)"));
    }
    items
}

/// The other half of the record: what was looked for and NOT found. This is
/// what explains an absent block ("no git" = "that is why there is no worktree
/// note"), mirroring the preview UI requirement in spec §6.
fn missing_items(scan: &InitScanReport) -> Vec<String> {
    let mut items = Vec::new();
    if scan.agents.is_empty() {
        items.push("PATH 上の既知の CLI".to_string());
    }
    if scan.project_kinds.is_empty() {
        items.push(
            "プロジェクト種別 (Cargo.toml / package.json / pyproject.toml / go.mod)".to_string(),
        );
    }
    if !scan.git_repo {
        items.push("git リポジトリ".to_string());
    }
    if scan.router_port.is_none() {
        items.push(format!("ローカル LLM ルータ (127.0.0.1:{ROUTER_PORT})"));
    }
    items
}

/// Build the `ptygrid.yml` text for a scan result. Pure: same inputs (including
/// `today`) always produce the same bytes — the property `serde_norway` output
/// could not give us (spec §3.1).
pub(crate) fn render_config(scan: &InitScanReport, today: &str) -> String {
    let mut out = String::new();

    // ---- header: provenance + what detection saw ----
    out.push_str(&format!(
        "# {CONFIG_FILE_NAME} — ptygrid init が生成しました ({today})\n"
    ));
    out.push_str(&wrap_comment("検出", &detected_items(scan)));
    out.push('\n');
    let missing = missing_items(scan);
    if !missing.is_empty() {
        out.push_str(&wrap_comment("未検出", &missing));
        out.push('\n');
    }
    out.push_str(
        "# 中身はすべて手で編集できます。全ブロックの注釈つき例は ptygrid.example.yml、\n",
    );
    out.push_str("# 用途別の見本は example/ を参照してください。\n\n");

    // ---- project ----
    let project = format!("project: {}", yaml_scalar(&project_name(&scan.dir)));
    let pad = " ".repeat(25usize.saturating_sub(display_width(&project)).max(1));
    out.push_str(&format!(
        "{project}{pad}# 作業フォルダ名から。ヘッダーに出る表示名\n"
    ));

    // ---- queen (only worth mentioning once there is more than one agent) ----
    if scan.agents.len() >= 2 {
        out.push_str(
            "\n# queen: ペイン間の読み書き・メッセージ・spawn を仲介する内蔵 MCP サーバー。\n",
        );
        out.push_str("# 各 CLI への登録コマンドはツールバーの Queen バッジからコピーできます。\n");
        out.push_str("queen:\n  enabled: true\n");
        out.push_str(&format!("  port: {}\n", crate::queen::DEFAULT_PORT));
    }

    // ---- agents ----
    if scan.agents.is_empty() {
        // Key line by key line (spec §3.5): leaving a bare `agents:` behind
        // would parse as null and fail the load.
        out.push_str("\n# PATH 上に既知の CLI が見つかりませんでした。CLI を入れたら、\n");
        out.push_str("# 次のブロックの各行の先頭 # を外してください。\n");
        out.push_str("# agents:\n");
        out.push_str("#   - name: claude\n");
        out.push_str("#     cmd: \"claude\"\n");
        out.push_str("#     cwd: \".\"\n");
        out.push_str("#     autostart: false\n");
    } else {
        out.push_str("\nagents:\n");
        for (i, name) in scan.agents.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!("  - name: {}\n", yaml_scalar(name)));
            out.push_str(&format!("    cmd: \"{name}\"\n"));
            out.push_str("    cwd: \".\"\n");
            if i == 0 {
                out.push_str(
                    "    autostart: false     # 読み込みと同時に起動するなら true（初回は手動 ▶ 起動）\n",
                );
            } else {
                out.push_str("    autostart: false\n");
            }
        }
    }

    // ---- local LLM router (always commented out: needs router.settings.json) ----
    if let Some(port) = scan.router_port {
        let ind = if scan.agents.is_empty() { "" } else { "  " };
        out.push_str(&format!(
            "\n{ind}# ローカル LLM ルータ (127.0.0.1:{port}) が応答しました。使うならコメントを外し、\n"
        ));
        out.push_str(&format!(
            "{ind}# router.settings.json を用意してください（env だけに頼らず --settings を渡すのが\n"
        ));
        out.push_str(&format!(
            "{ind}# 確実な理由は example/team-preset/ptygrid.yml を参照）。\n"
        ));
        out.push_str(&format!("{ind}# - name: local\n"));
        out.push_str(&format!(
            "{ind}#   cmd: \"claude --settings router.settings.json\"\n"
        ));
        out.push_str(&format!("{ind}#   cwd: \".\"\n"));
        out.push_str(&format!("{ind}#   env:\n"));
        out.push_str(&format!(
            "{ind}#     ANTHROPIC_BASE_URL: \"${{CODEROUTER_URL}}\"\n"
        ));
        out.push_str(&format!("{ind}#   autostart: false\n"));
    }

    // ---- processes (guidance only; never uncommented by init) ----
    if !scan.project_kinds.is_empty() {
        let markers = scan
            .project_kinds
            .iter()
            .map(|k| marker_for(k))
            .collect::<Vec<_>>()
            .join(" / ");
        let cmd = if scan.project_kinds.iter().any(|k| k == "npm") {
            "npm run dev"
        } else {
            "<常駐させたいコマンド>"
        };
        out.push_str(&format!(
            "\n# {markers} を検出しました。dev サーバーやテスト watch を常駐させるなら\n"
        ));
        out.push_str("# 次のブロックの各行の先頭 # を外してください（agents と同じフィールドを持ちます）。\n");
        out.push_str("# processes:\n");
        out.push_str("#   - name: dev\n");
        out.push_str(&format!("#     cmd: \"{cmd}\"\n"));
        out.push_str("#     cwd: \".\"\n");
        out.push_str("#     autostart: false\n");
        out.push_str("#     autorestart: on-failure   # 異常終了時のみ再起動\n");
    }

    // ---- pointers for the blocks init deliberately does not generate (§4.3) ----
    if scan.git_repo {
        out.push_str("\n# git リポジトリを検出しました。ペインごとに linked worktree を切るなら\n");
        out.push_str(
            "# example/worktree を参照してください（init は worktree: を生成しません）。\n",
        );
    }
    out.push_str("\n# チーム一括起動 (team_presets:) は example/team-preset、\n");
    out.push_str(
        "# DAG オーケストレーション (workflows:) は example/adaptive-orchestration を参照。\n",
    );

    out
}

// ---------------------------------------------------------------------------
// destination + preview
// ---------------------------------------------------------------------------

/// Directory the generated file goes into. `Global` resolves under `$HOME`.
fn target_dir(dir: &Path, target: InitTarget, home: Option<&Path>) -> Result<PathBuf, String> {
    match target {
        InitTarget::Project => Ok(dir.to_path_buf()),
        InitTarget::Global => home
            .map(|h| h.join(GLOBAL_CONFIG_DIR))
            .ok_or_else(|| "no_home: cannot determine home directory".to_string()),
    }
}

/// True when `ptygrid.yml` already exists in the destination directory, in
/// which case the output goes to the sidecar rather than overwriting it
/// (spec §3.4).
///
/// A lone legacy `mterm.yml` deliberately does NOT force the sidecar: the
/// natural destination stays `<dir>/ptygrid.yml`, which is exactly the case
/// [`write`] refuses, because that file would silently win the search order
/// over the legacy one. Once a real `ptygrid.yml` is present the legacy file is
/// already shadowed by it, so a sidecar write changes nothing and is allowed.
fn sidecar_needed(target_dir: &Path, is_file: &dyn Fn(&Path) -> bool) -> bool {
    is_file(&target_dir.join(CONFIG_FILE_NAME))
}

/// Destination path plus whether it is the sidecar.
fn destination(
    dir: &Path,
    target: InitTarget,
    home: Option<&Path>,
    is_file: &dyn Fn(&Path) -> bool,
) -> Result<(PathBuf, bool), String> {
    let base = target_dir(dir, target, home)?;
    let sidecar = sidecar_needed(&base, is_file);
    let name = if sidecar {
        SIDECAR_FILE_NAME
    } else {
        CONFIG_FILE_NAME
    };
    Ok((base.join(name), sidecar))
}

/// The existing file the sidecar output would sit next to, for the two-pane
/// diff. Read as text only — never parsed, never written back (spec §3.4).
fn existing_text(target_dir: &Path) -> Option<String> {
    std::fs::read_to_string(target_dir.join(CONFIG_FILE_NAME)).ok()
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Generate + self-check for an already-computed scan. Split out so tests can
/// pin the detection result (and the date) instead of depending on the host.
pub(crate) fn preview_from_scan(
    scan: InitScanReport,
    dir: &Path,
    target: InitTarget,
    home: Option<&Path>,
    today: &str,
) -> Result<InitPreview, String> {
    let content = render_config(&scan, today);
    let (path, sidecar) = destination(dir, target, home, &|p| p.is_file())?;
    // Self-check (spec §3.5). A failure here is a ptygrid bug, not user error,
    // so it is surfaced rather than silently swallowed — and `init_write`
    // refuses the same content anyway.
    let (valid, error) = match parse_config(&content) {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e)),
    };
    let existing_content = if sidecar {
        path.parent().and_then(existing_text)
    } else {
        None
    };
    Ok(InitPreview {
        content,
        path: path.display().to_string(),
        target,
        sidecar,
        valid,
        error,
        existing_content,
        scan,
    })
}

/// Scan the host, generate, self-check. Writes nothing.
pub fn preview(dir: &Path, target: InitTarget) -> Result<InitPreview, String> {
    let dir = absolute_dir(dir);
    let home = crate::pty::home_dir().map(PathBuf::from);
    preview_from_scan(scan(&dir), &dir, target, home.as_deref(), &today())
}

// ---------------------------------------------------------------------------
// write
// ---------------------------------------------------------------------------

/// temp + rename, skipping the write entirely when the file already holds the
/// exact bytes (spec §9 "同値なら書かない"). Returns `(bytes, written)`.
fn write_atomic_if_changed(path: &Path, content: &str) -> Result<(usize, bool), String> {
    if let Ok(existing) = std::fs::read(path) {
        if existing == content.as_bytes() {
            return Ok((existing.len(), false));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| "init destination has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("cannot create config directory: {e}"))?;
    let suffix = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".ptygrid-init-{}-{suffix}.tmp", std::process::id()));
    std::fs::write(&temp, content).map_err(|e| format!("cannot write generated config: {e}"))?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace generated config: {e}")
    })?;
    Ok((content.len(), true))
}

/// Re-check `content` and write it (spec §3.5).
///
/// Refuses in two cases, without touching the disk:
/// - the write would create `<dir>/ptygrid.yml` while the working folder still
///   uses the legacy `mterm.yml` name — the new file would silently win the
///   search order, so the rename has to happen first (spec §9). A sidecar write
///   (or a `global` one) does not take that shadowing risk and is allowed;
/// - `content` does not pass [`parse_config`].
pub fn write(dir: &Path, target: InitTarget, content: &str) -> Result<InitWriteResult, String> {
    let dir = absolute_dir(dir);
    let home = crate::pty::home_dir().map(PathBuf::from);
    let (path, sidecar) = destination(&dir, target, home.as_deref(), &|p| p.is_file())?;
    let legacy = dir.join(LEGACY_CONFIG_FILE_NAME);
    if path == dir.join(CONFIG_FILE_NAME) && legacy.is_file() {
        return Err(format!(
            "legacy_config: {} が残っています。init は書き込みません — \
             先に {LEGACY_CONFIG_FILE_NAME} を {CONFIG_FILE_NAME} にリネームしてください",
            legacy.display()
        ));
    }
    let config = parse_config(content).map_err(|e| format!("invalid_config: {e}"))?;
    let (bytes, _written) = write_atomic_if_changed(&path, content)?;
    let has_autostart = config
        .agents
        .iter()
        .chain(config.processes.iter())
        .any(|d| d.autostart == Some(true));
    Ok(InitWriteResult {
        path: path.display().to_string(),
        bytes,
        sidecar,
        trust_prompt_expected: target == InitTarget::Project && has_autostart,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- helpers -------------------------------------------------------

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ptygrid-init-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `is_file` / `is_exec` predicate over a fixed set of paths.
    fn present(paths: &[PathBuf]) -> impl Fn(&Path) -> bool + '_ {
        move |p: &Path| paths.iter().any(|x| x == p)
    }

    const NO_PATHS: &[PathBuf] = &[];

    fn exts() -> Vec<String> {
        vec![String::new()]
    }

    /// A scan report with nothing detected.
    fn empty_scan(dir: &str) -> InitScanReport {
        InitScanReport {
            dir: dir.to_string(),
            agents: Vec::new(),
            project_kinds: Vec::new(),
            git_repo: false,
            router_port: None,
            existing: None,
        }
    }

    fn scan_with(dir: &str, agents: &[&str]) -> InitScanReport {
        InitScanReport {
            agents: agents.iter().map(|a| a.to_string()).collect(),
            ..empty_scan(dir)
        }
    }

    // ---- detection -----------------------------------------------------

    #[test]
    fn finds_agent_clis_across_path_dirs_in_known_agents_order() {
        let bin = PathBuf::from("/usr/local/bin");
        let opt = PathBuf::from("/opt/tools");
        let existing = vec![
            bin.join("codex"),
            opt.join("claude"),
            opt.join("claude"), // same name twice: reported once
        ];
        let found = find_agents_pure(&[bin.clone(), opt.clone()], &exts(), &present(&existing));
        // KNOWN_AGENTS declaration order puts claude before codex.
        assert_eq!(found, vec!["claude".to_string(), "codex".to_string()]);

        // Present but not executable (predicate says no) -> not found.
        assert!(find_agents_pure(&[bin, opt], &exts(), &|_| false).is_empty());
    }

    #[test]
    fn agent_candidates_use_last_path_segment_and_are_unique() {
        let cands = agent_candidates();
        assert!(cands.contains(&"amp"), "sourcegraph/amp -> amp");
        assert!(!cands.iter().any(|c| c.contains('/')));
        let mut sorted = cands.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), cands.len(), "candidate names must be unique");
    }

    #[test]
    fn detects_project_kinds_git_router_and_existing_config() {
        let dir = PathBuf::from("/work");
        let existing = vec![dir.join("Cargo.toml"), dir.join("package.json")];
        let report = scan_pure(&ScanEnv {
            dir: &dir,
            launch: None,
            home: None,
            is_file: &present(&existing),
            path_dirs: NO_PATHS,
            exts: &exts(),
            is_exec: &|_| false,
            is_git_repo: &|_| true,
            router_alive: &|p| p == ROUTER_PORT,
        });
        assert_eq!(report.project_kinds, vec!["cargo", "npm"]);
        assert!(report.git_repo);
        assert_eq!(report.router_port, Some(ROUTER_PORT));
        assert!(report.existing.is_none());
        assert!(report.agents.is_empty());
    }

    #[test]
    fn reports_legacy_mterm_yml_as_existing_config() {
        let dir = PathBuf::from("/work");
        let existing = vec![dir.join(LEGACY_CONFIG_FILE_NAME)];
        let report = scan_pure(&ScanEnv {
            dir: &dir,
            launch: None,
            home: None,
            is_file: &present(&existing),
            path_dirs: NO_PATHS,
            exts: &exts(),
            is_exec: &|_| false,
            is_git_repo: &|_| false,
            router_alive: &|_| false,
        });
        let found = report.existing.expect("legacy config detected");
        assert!(found.legacy);
        assert_eq!(found.origin, ConfigOrigin::Project);
        assert!(found.path.ends_with(LEGACY_CONFIG_FILE_NAME));

        // The preferred name is not legacy.
        let preferred = vec![dir.join(CONFIG_FILE_NAME)];
        let report = scan_pure(&ScanEnv {
            dir: &dir,
            launch: None,
            home: None,
            is_file: &present(&preferred),
            path_dirs: NO_PATHS,
            exts: &exts(),
            is_exec: &|_| false,
            is_git_repo: &|_| false,
            router_alive: &|_| false,
        });
        assert!(!report.existing.unwrap().legacy);
    }

    #[test]
    fn real_scan_succeeds_on_an_empty_directory() {
        let dir = temp_dir("real-scan");
        let report = scan(&dir);
        assert_eq!(report.dir, dir.display().to_string());
        assert!(report.project_kinds.is_empty());
        assert!(report.existing.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- generation + self-check --------------------------------------

    #[test]
    fn minimal_generated_config_parses_with_one_agent() {
        let yaml = render_config(&scan_with("/tmp/my-app", &["claude"]), "2026-07-29");
        let config = parse_config(&yaml).expect("generated config must parse");
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].name, "claude");
        assert_eq!(config.agents[0].cmd, "claude");
        assert_eq!(config.project.as_deref(), Some("my-app"));
        // A single CLI does not need the queen explainer.
        assert!(!yaml.contains("queen:"));
    }

    #[test]
    fn multi_cli_generated_config_parses_with_all_agents() {
        let scan = InitScanReport {
            project_kinds: vec!["npm".to_string()],
            git_repo: true,
            router_port: Some(ROUTER_PORT),
            ..scan_with("/tmp/my-app", &["claude", "codex", "grok"])
        };
        let yaml = render_config(&scan, "2026-07-29");
        let config = parse_config(&yaml).expect("generated config must parse");
        assert_eq!(config.agents.len(), 3);
        let names: Vec<_> = config.agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["claude", "codex", "grok"]);
        // queen block is emitted once there is more than one CLI.
        assert_eq!(
            config.queen.and_then(|q| q.port),
            Some(crate::queen::DEFAULT_PORT)
        );
        // The router / processes blocks stay commented out: nothing generated.
        assert!(config.processes.is_empty());
        assert!(yaml.contains("#   cmd: \"claude --settings router.settings.json\""));
        assert!(yaml.contains("#     cmd: \"npm run dev\""));
    }

    #[test]
    fn every_generated_definition_is_autostart_false() {
        for scan in [
            scan_with("/tmp/my-app", &["claude"]),
            InitScanReport {
                project_kinds: vec!["cargo".to_string(), "npm".to_string()],
                git_repo: true,
                router_port: Some(ROUTER_PORT),
                ..scan_with("/tmp/my-app", &["claude", "codex", "grok"])
            },
            empty_scan("/tmp/my-app"),
        ] {
            let yaml = render_config(&scan, "2026-07-29");
            let config = parse_config(&yaml).unwrap();
            for def in config.agents.iter().chain(config.processes.iter()) {
                assert_eq!(
                    def.autostart,
                    Some(false),
                    "generated definition {} must not autostart",
                    def.name
                );
            }
            assert!(!yaml.contains("autostart: true"));
        }
    }

    #[test]
    fn generation_is_deterministic_for_the_same_scan() {
        let scan = InitScanReport {
            project_kinds: vec!["cargo".to_string()],
            git_repo: true,
            router_port: Some(ROUTER_PORT),
            ..scan_with("/tmp/my-app", &["claude", "codex"])
        };
        let a = render_config(&scan, "2026-07-29");
        let b = render_config(&scan, "2026-07-29");
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn header_records_what_detection_saw_and_missed() {
        let scan = InitScanReport {
            project_kinds: vec!["cargo".to_string()],
            git_repo: true,
            ..scan_with("/tmp/my-app", &["claude", "codex"])
        };
        let yaml = render_config(&scan, "2026-07-29");
        let header: String = yaml
            .lines()
            .take_while(|l| l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(header.contains("ptygrid init が生成しました (2026-07-29)"));
        assert!(header.contains("検出:"));
        assert!(header.contains("claude / codex (PATH)"));
        assert!(header.contains("Cargo.toml"));
        assert!(header.contains("git リポジトリ"));
        // Not found is recorded too — it explains the absent router block.
        assert!(header.contains("未検出:"));
        assert!(header.contains("ローカル LLM ルータ (127.0.0.1:3456)"));
        // Wrapped continuation lines stay comments and line up under the label.
        for line in header.lines() {
            assert!(line.starts_with('#'));
        }
        // A long "未検出" list wraps onto an aligned continuation line.
        let minimal = render_config(&scan_with("/tmp/my-app", &["claude"]), "2026-07-29");
        assert!(minimal.contains("\n#         git リポジトリ"), "{minimal}");
    }

    #[test]
    fn display_width_counts_full_width_characters_as_two() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("検出"), 4);
        assert_eq!(display_width("# 検出: "), 8);
        // Long lists wrap; every produced line is still a comment.
        let items: Vec<String> = (0..12).map(|i| format!("項目{i}")).collect();
        let wrapped = wrap_comment("検出", &items);
        assert!(wrapped.lines().count() > 1);
        assert!(wrapped.lines().all(|l| l.starts_with('#')));
        assert!(wrapped
            .lines()
            .all(|l| display_width(l) <= COMMENT_WIDTH + 2));
        assert_eq!(wrap_comment("検出", &[]), "# 検出: なし");
    }

    #[test]
    fn nothing_detected_still_yields_a_valid_commented_out_skeleton() {
        let dir = temp_dir("empty-detect");
        let scan = empty_scan(&dir.display().to_string());
        let preview =
            preview_from_scan(scan, &dir, InitTarget::Project, None, "2026-07-29").unwrap();
        assert!(preview.valid, "self-check failed: {:?}", preview.error);
        assert!(!preview.sidecar);
        let config = parse_config(&preview.content).unwrap();
        assert!(config.agents.is_empty());
        assert!(config.processes.is_empty());
        assert!(preview.content.contains("# agents:"));
        assert!(preview.content.contains("#     autostart: false"));
        assert!(preview.content.contains("# 検出: なし"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partially_commented_out_block_is_caught_by_the_self_check() {
        // Spec §3.5 worries that commenting a block body while leaving the key
        // line behind yields a null value that fails to parse. Measured with
        // serde_norway 0.9 that is only half true, and the halves matter:
        //
        //   `processes:` with no value node  -> the field reads as MISSING, so
        //                                       `#[serde(default)]` supplies the
        //                                       empty Vec and the config parses;
        //   `processes: null` / `processes: ~` -> a real unit value, which is a
        //                                       type error against `Vec`.
        let empty_block = parse_config("project: x\nprocesses:\nagents:\n").unwrap();
        assert!(empty_block.agents.is_empty() && empty_block.processes.is_empty());
        for explicit_null in ["project: x\nprocesses: null\n", "project: x\nagents: ~\n"] {
            assert!(
                parse_config(explicit_null).is_err(),
                "an explicit null must not pass: {explicit_null:?}"
            );
        }
        // The shape a half-commented block actually leaves behind — a list entry
        // stripped of its `cmd` — is what the self-check has to catch before
        // anything is written.
        assert!(parse_config("project: x\nagents:\n  - name: a\n").is_err());
        // Every generated variant passes the same check.
        assert!(parse_config(&render_config(&empty_scan("/tmp/x"), "2026-07-29")).is_ok());
    }

    // ---- destination + write ------------------------------------------

    #[test]
    fn destination_is_the_sidecar_only_when_a_config_already_exists() {
        let dir = PathBuf::from("/work");
        let (path, sidecar) = destination(&dir, InitTarget::Project, None, &|_| false).unwrap();
        assert_eq!(path, dir.join(CONFIG_FILE_NAME));
        assert!(!sidecar);

        // An existing ptygrid.yml diverts the output to the sidecar.
        let files = vec![dir.join(CONFIG_FILE_NAME)];
        let (path, sidecar) =
            destination(&dir, InitTarget::Project, None, &present(&files)).unwrap();
        assert_eq!(path, dir.join(SIDECAR_FILE_NAME));
        assert!(sidecar);

        // A lone legacy mterm.yml does NOT: the destination stays
        // <dir>/ptygrid.yml, which is precisely the shadowing case `write`
        // refuses (see legacy_mterm_yml_only_blocks_the_shadowing_write).
        let files = vec![dir.join(LEGACY_CONFIG_FILE_NAME)];
        let (path, sidecar) =
            destination(&dir, InitTarget::Project, None, &present(&files)).unwrap();
        assert_eq!(path, dir.join(CONFIG_FILE_NAME));
        assert!(!sidecar);

        // Global resolves under ~/.ptygrid, and needs a home to do so.
        let home = PathBuf::from("/home/u");
        let (path, _) = destination(&dir, InitTarget::Global, Some(&home), &|_| false).unwrap();
        assert_eq!(path, home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME));
        assert!(destination(&dir, InitTarget::Global, None, &|_| false)
            .unwrap_err()
            .starts_with("no_home:"));
    }

    #[test]
    fn existing_config_is_left_untouched_and_output_goes_to_the_sidecar() {
        let dir = temp_dir("sidecar");
        let body = "# hand written\nproject: mine\nagents: []\n";
        let main = dir.join(CONFIG_FILE_NAME);
        std::fs::write(&main, body).unwrap();

        let preview = preview_from_scan(
            scan_with(&dir.display().to_string(), &["claude"]),
            &dir,
            InitTarget::Project,
            None,
            "2026-07-29",
        )
        .unwrap();
        assert!(preview.sidecar);
        assert_eq!(
            preview.path,
            dir.join(SIDECAR_FILE_NAME).display().to_string()
        );
        assert_eq!(preview.existing_content.as_deref(), Some(body));

        let result = write(&dir, InitTarget::Project, &preview.content).unwrap();
        assert!(result.sidecar);
        assert_eq!(
            result.path,
            dir.join(SIDECAR_FILE_NAME).display().to_string()
        );
        assert!(!result.trust_prompt_expected);

        // The user's file is byte-for-byte unchanged.
        assert_eq!(std::fs::read_to_string(&main).unwrap(), body);
        assert_eq!(
            std::fs::read_to_string(dir.join(SIDECAR_FILE_NAME)).unwrap(),
            preview.content
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identical_sidecar_content_is_not_rewritten() {
        let dir = temp_dir("idempotent");
        std::fs::write(dir.join(CONFIG_FILE_NAME), "agents: []\n").unwrap();
        let content = render_config(
            &scan_with(&dir.display().to_string(), &["claude"]),
            "2026-07-29",
        );

        let first = write(&dir, InitTarget::Project, &content).unwrap();
        let path = PathBuf::from(&first.path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        // The second run agrees byte-for-byte, so nothing is written at all.
        assert!(!write_atomic_if_changed(&path, &content).unwrap().1);
        let second = write(&dir, InitTarget::Project, &content).unwrap();
        assert_eq!(second.path, first.path);
        assert_eq!(second.bytes, content.len());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        // Differing content does get written.
        assert!(write_atomic_if_changed(&path, "agents: []\n").unwrap().1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_mterm_yml_only_blocks_the_shadowing_write_not_the_preview() {
        let dir = temp_dir("legacy");
        std::fs::write(dir.join(LEGACY_CONFIG_FILE_NAME), "agents: []\n").unwrap();

        // Preview still works (spec §9: show it, just do not write).
        let preview = preview_from_scan(
            scan_with(&dir.display().to_string(), &["claude"]),
            &dir,
            InitTarget::Project,
            None,
            "2026-07-29",
        )
        .unwrap();
        assert!(preview.valid);
        // The destination is <dir>/ptygrid.yml — the file that would silently
        // win the search order over mterm.yml. That is the refused case, and
        // the frontend can predict it from `sidecar: false` + `existing.legacy`.
        assert!(!preview.sidecar);
        assert_eq!(
            preview.path,
            dir.join(CONFIG_FILE_NAME).display().to_string()
        );

        let err = write(&dir, InitTarget::Project, &preview.content).unwrap_err();
        assert!(err.starts_with("legacy_config:"), "unexpected error: {err}");
        assert!(err.contains(LEGACY_CONFIG_FILE_NAME));
        assert!(!dir.join(CONFIG_FILE_NAME).exists());
        assert!(!dir.join(SIDECAR_FILE_NAME).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_mterm_yml_beside_a_real_config_still_allows_the_sidecar_write() {
        // Regression: the refusal is about shadowing, not about the mere
        // presence of the legacy name. With a real ptygrid.yml already there,
        // mterm.yml is ALREADY shadowed by it, the output goes to the sidecar,
        // and nothing about the search order changes — so the write must
        // succeed (scan reports existing.legacy = false here, so a refusal
        // would also be unpredictable for the frontend).
        let dir = temp_dir("legacy-coexist");
        let main_body = "# hand written\nproject: mine\nagents: []\n";
        let legacy_body = "project: old\nagents: []\n";
        std::fs::write(dir.join(CONFIG_FILE_NAME), main_body).unwrap();
        std::fs::write(dir.join(LEGACY_CONFIG_FILE_NAME), legacy_body).unwrap();

        let report = scan(&dir);
        let existing = report.existing.clone().expect("a config is found");
        assert!(!existing.legacy, "ptygrid.yml wins the search order");

        let preview =
            preview_from_scan(report, &dir, InitTarget::Project, None, "2026-07-29").unwrap();
        assert!(preview.sidecar);
        assert_eq!(preview.existing_content.as_deref(), Some(main_body));

        let result = write(&dir, InitTarget::Project, &preview.content).unwrap();
        assert!(result.sidecar);
        assert_eq!(
            result.path,
            dir.join(SIDECAR_FILE_NAME).display().to_string()
        );
        // Neither existing file is touched.
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG_FILE_NAME)).unwrap(),
            main_body
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(LEGACY_CONFIG_FILE_NAME)).unwrap(),
            legacy_body
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(SIDECAR_FILE_NAME)).unwrap(),
            preview.content
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Spec §3.5 / §7.2 require temp + rename, never a bare
    /// `fs::write(path, content)`. The two differ observably at a symlinked
    /// destination: `fs::write` follows the link and writes THROUGH it (so a
    /// dangling link fails outright), while `rename` replaces the link itself.
    /// Degrading [`write_atomic_if_changed`] to `fs::write` therefore turns
    /// this test red — which is the point of having it.
    #[cfg(unix)]
    #[test]
    fn write_replaces_the_destination_by_rename_not_by_writing_through_it() {
        let dir = temp_dir("atomic");
        let path = dir.join(CONFIG_FILE_NAME);
        let dangling = dir.join("no-such-dir").join("target.yml");
        std::os::unix::fs::symlink(&dangling, &path).unwrap();
        assert!(
            std::fs::symlink_metadata(&path).unwrap().is_symlink(),
            "the destination must start out as a symlink"
        );
        // Precondition of the discriminator: writing through the link cannot
        // work, so a passing write proves rename was used.
        assert!(std::fs::write(&path, "x").is_err());

        let content = "project: x\nagents: []\n";
        let (bytes, written) = write_atomic_if_changed(&path, content).unwrap();
        assert!(written);
        assert_eq!(bytes, content.len());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        assert!(
            !std::fs::symlink_metadata(&path).unwrap().is_symlink(),
            "rename must replace the link itself, not follow it"
        );
        assert!(
            !dangling.exists(),
            "nothing may be written through the link"
        );

        // The rename consumed the temp file; none is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_failing_the_self_check_is_never_written() {
        let dir = temp_dir("invalid");
        let err = write(&dir, InitTarget::Project, "agents:\n  - name: x\n").unwrap_err();
        assert!(
            err.starts_with("invalid_config:"),
            "unexpected error: {err}"
        );
        assert!(!dir.join(CONFIG_FILE_NAME).exists());
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_reports_trust_prompt_when_the_user_edits_autostart_on() {
        let dir = temp_dir("trust-flag");
        let content = "project: x\nagents:\n  - name: a\n    cmd: \"true\"\n    autostart: true\n";
        let result = write(&dir, InitTarget::Project, content).unwrap();
        assert!(result.trust_prompt_expected);
        assert!(!result.sidecar);
        assert_eq!(result.bytes, content.len());
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG_FILE_NAME)).unwrap(),
            content
        );
        // No temp files left behind by the rename.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn written_config_is_resolved_as_a_project_config() {
        let dir = temp_dir("resolve");
        let content = render_config(
            &scan_with(&dir.display().to_string(), &["claude"]),
            "2026-07-29",
        );
        let result = write(&dir, InitTarget::Project, &content).unwrap();
        assert!(!result.sidecar);
        let report = scan(&dir);
        let existing = report.existing.expect("the written file is now discovered");
        assert_eq!(existing.origin, ConfigOrigin::Project);
        assert!(!existing.legacy);
        assert_eq!(existing.path, result.path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn working_folders_are_made_absolute_and_lexically_clean() {
        assert_eq!(absolute_dir(Path::new("/tmp/a/..")), PathBuf::from("/tmp"));
        assert_eq!(
            absolute_dir(Path::new("/tmp/a/../b/./c")),
            PathBuf::from("/tmp/b/c")
        );
        assert_eq!(absolute_dir(Path::new("/..")), PathBuf::from("/"));
        assert_eq!(absolute_dir(Path::new("/")), PathBuf::from("/"));
        // A relative input is anchored on the process cwd.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(absolute_dir(Path::new("sub/dir")), cwd.join("sub/dir"));
        assert!(absolute_dir(Path::new(".")).is_absolute());

        // The normalized value is what the report (and the generated project
        // name derived from it) carries — spec §5.2 promises an absolute path.
        let dir = temp_dir("absolute");
        let messy = dir.join("nested").join("..");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        let report = scan(&messy);
        assert_eq!(report.dir, dir.display().to_string());
        assert!(Path::new(&report.dir).is_absolute());
        assert!(!report.dir.contains(".."));
        assert_eq!(
            project_name(&report.dir),
            dir.file_name().unwrap().to_string_lossy()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_normalizes_the_working_folder_before_choosing_a_destination() {
        let dir = temp_dir("absolute-write");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        let messy = dir.join("nested").join("..");
        let content = "project: x\nagents: []\n";
        let result = write(&messy, InitTarget::Project, content).unwrap();
        assert_eq!(
            result.path,
            dir.join(CONFIG_FILE_NAME).display().to_string()
        );
        assert!(!result.path.contains(".."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_names_needing_quotes_are_quoted() {
        assert_eq!(yaml_scalar("my-app"), "my-app");
        assert_eq!(yaml_scalar("my app"), "\"my app\"");
        assert_eq!(yaml_scalar(""), "\"\"");
        assert_eq!(project_name("/tmp/my app"), "my app");
        assert_eq!(project_name("/"), "project");
        let yaml = render_config(&scan_with("/tmp/my app", &["claude"]), "2026-07-29");
        assert_eq!(
            parse_config(&yaml).unwrap().project.as_deref(),
            Some("my app")
        );
    }
}
