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
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// LM Studio's default server port. Only used to pick the documented
/// `ANTHROPIC_AUTH_TOKEN` placeholder for that endpoint — see
/// [`auth_token_for`].
const LM_STUDIO_PORT: u16 = 1234;

/// Connect timeout for the D4 probe. Loopback only; kept short so a dead port
/// costs nothing noticeable and a filtered one cannot stall the caller.
const ROUTER_PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// Comment wrap column for the generated header (character count, not display
/// width — good enough for a comment and keeps the output deterministic).
const COMMENT_WIDTH: usize = 72;

// --- local LLM probe (5.0.2 追補) -----------------------------------------
//
// Deliberately NOT part of [`scan`]: the probe issues real HTTP requests and
// costs up to [`PROBE_TOTAL_BUDGET`], while `scan` is contractually
// best-effort and non-blocking. It runs only when the user asks for it.

/// Ports probed by default: Ollama / LM Studio / coderouter. No range scan —
/// these three plus whatever the user typed, and nothing else.
const DEFAULT_PROBE_PORTS: &[u16] = &[11434, 1234, 3456];

/// Per-port connect + read timeout.
const PROBE_PORT_TIMEOUT: Duration = Duration::from_secs(1);

/// Whole-probe budget. Ports run concurrently, so this bounds the command even
/// when several of them hang.
const PROBE_TOTAL_BUDGET: Duration = Duration::from_secs(3);

/// Response read cap. A local server is not trusted to be small.
const PROBE_MAX_BYTES: usize = 64 * 1024;

/// Model-name cap per endpoint (the generated comment stays readable).
const PROBE_MAX_MODELS: usize = 20;

/// How many hand-typed ports may be added on top of the defaults.
const MAX_EXTRA_PORTS: usize = 4;

/// First Ollama version whose `/v1/messages` speaks the Anthropic Messages API.
const OLLAMA_MIN_ANTHROPIC: (u32, u32, u32) = (0, 14, 0);

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

/// One local endpoint that answered `GET /v1/models`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalLlmEndpoint {
    /// The port that answered.
    pub port: u16,
    /// `data[].id` from `GET /v1/models`, capped at [`PROBE_MAX_MODELS`].
    pub models: Vec<String>,
    /// Whether Anthropic Messages API compatibility could be *confirmed*.
    /// `Some(true)` = confirmed (today: Ollama v0.14.0+ only)
    /// `Some(false)` = confirmed unsupported
    /// `None` = unknown (only an OpenAI-compatible answer was seen)
    pub anthropic: Option<bool>,
    /// Display label. Carries a product name only when the server identified
    /// itself, e.g. `"Ollama 0.14.3"` / `"127.0.0.1:1234 (OpenAI 互換の応答)"`.
    pub label: String,
}

/// Return value of `init_probe_llm`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitProbeReport {
    /// Ports actually probed (input deduplicated, ascending).
    pub probed_ports: Vec<u16>,
    /// Only the ones that answered. Ascending by port.
    pub endpoints: Vec<LocalLlmEndpoint>,
    /// True when the whole-probe budget was spent before every port reported.
    pub timed_out: bool,
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
// local LLM probe (5.0.2 追補)
// ---------------------------------------------------------------------------
//
// Why a *second* request decides `anthropic`, and why only `/api/version` can
// set it to `Some(true)`:
//
// `GET /v1/models` answering proves exactly one thing — something on that port
// speaks the OpenAI-compatible surface. It says nothing about `/v1/messages`,
// which is what `claude` actually talks to via `ANTHROPIC_BASE_URL`. An
// OpenAI-only server (LM Studio, vLLM, llama.cpp, LiteLLM without the
// Anthropic route) and an Ollama older than v0.14.0 return the *same* 200 with
// the *same* shape as an Ollama that does support the Messages API. Probing
// `/v1/messages` directly is no better: it is a POST that would spend tokens
// and load a model just to find out, and a 404 there is indistinguishable from
// a proxy that 404s everything it does not recognize.
//
// So confirmation is only ever taken from `/api/version` — the Ollama-specific
// endpoint that lets the server name itself *and* state a version we can
// compare against [`OLLAMA_MIN_ANTHROPIC`]. Everything else stays `None`
// ("unknown"), which generation renders as commented-out lines rather than as
// a live definition.

/// Normalize the port list: the three defaults plus the caller's extras,
/// deduplicated and sorted. `bad_port:` is the only error this can produce
/// (contract §Tauri command) — *no answer is not an error*.
fn probe_ports(extra: &[u16]) -> Result<Vec<u16>, String> {
    if extra.contains(&0) {
        return Err("bad_port: 0 はポート番号として使えません".to_string());
    }
    if extra.len() > MAX_EXTRA_PORTS {
        return Err(format!(
            "bad_port: 追加ポートは最大 {MAX_EXTRA_PORTS} 本までです ({} 本受け取りました)",
            extra.len()
        ));
    }
    let mut ports: Vec<u16> = DEFAULT_PROBE_PORTS.to_vec();
    ports.extend_from_slice(extra);
    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}

/// `data[].id` out of a `/v1/models` body, capped at [`PROBE_MAX_MODELS`].
/// Anything unexpected (no `data`, not an array, a non-string `id`, truncated
/// JSON) is dropped silently — an empty result means "did not answer usefully".
fn parse_models(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(entries) = value.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()))
        .filter(|id| !id.is_empty())
        .take(PROBE_MAX_MODELS)
        .map(|id| id.to_string())
        .collect()
}

/// A version string as a numeric triple. Compared numerically, never
/// lexically: `"0.9.0" > "0.14.0"` as text but is *older* as a version. A
/// pre-release suffix (`0.14.0-rc1`, `0.14.0+build`) is dropped; missing minor
/// or patch components read as 0. `None` = not a version we can compare.
fn parse_version(raw: &str) -> Option<(u32, u32, u32)> {
    let core = raw
        .trim()
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .trim();
    if core.is_empty() {
        return None;
    }
    let mut parts = core.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = match parts.next() {
        Some(p) => p.parse().ok()?,
        None => 0,
    };
    let patch: u32 = match parts.next() {
        Some(p) => p.parse().ok()?,
        None => 0,
    };
    // A fourth component is more than a semver; refuse rather than guess.
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// `version` out of an `/api/version` body, kept as the *original* string for
/// the label plus its parsed triple. `None` when the field is missing or is
/// not something [`parse_version`] can compare.
fn parse_api_version(body: &str) -> Option<(String, (u32, u32, u32))> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let raw = value.get("version")?.as_str()?;
    let parsed = parse_version(raw)?;
    Some((raw.to_string(), parsed))
}

/// Probe one port, given an injected `GET` that returns the body of a 200 and
/// `None` for everything else (connection refused, timeout, non-200). Pure
/// with respect to the network, so the decision table is testable without a
/// server (same technique as [`scan_pure`]).
fn probe_endpoint_pure(
    port: u16,
    get: &dyn Fn(u16, &str) -> Option<String>,
) -> Option<LocalLlmEndpoint> {
    let models = parse_models(&get(port, "/v1/models")?);
    if models.is_empty() {
        return None;
    }
    // See the module note above: only `/api/version` can confirm anything.
    let (anthropic, label) = match get(port, "/api/version")
        .as_deref()
        .and_then(parse_api_version)
    {
        Some((raw, parsed)) => (
            Some(parsed >= OLLAMA_MIN_ANTHROPIC),
            format!("Ollama {raw}"),
        ),
        None => (None, format!("127.0.0.1:{port} (OpenAI 互換の応答)")),
    };
    Some(LocalLlmEndpoint {
        port,
        models,
        anthropic,
        label,
    })
}

/// Run `probe_one` for every port on its own short-lived thread and collect
/// whatever reports within `budget`. Threads are detached on purpose: a hung
/// port must not extend the command past the budget, and each thread is
/// already bounded by [`PROBE_PORT_TIMEOUT`] anyway. Returns the endpoints
/// sorted by port plus whether the budget ran out.
fn probe_concurrent(
    ports: &[u16],
    budget: Duration,
    probe_one: Arc<dyn Fn(u16) -> Option<LocalLlmEndpoint> + Send + Sync>,
) -> (Vec<LocalLlmEndpoint>, bool) {
    use std::sync::mpsc::{channel, RecvTimeoutError};

    let deadline = Instant::now() + budget;
    let (tx, rx) = channel();
    for &port in ports {
        let tx = tx.clone();
        let probe_one = Arc::clone(&probe_one);
        std::thread::spawn(move || {
            // The receiver may already be gone (budget spent); that send
            // failing is the normal, expected outcome, not an error.
            let _ = tx.send(probe_one(port));
        });
    }
    drop(tx);

    let mut endpoints = Vec::new();
    let mut timed_out = false;
    for _ in 0..ports.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(Some(endpoint)) => endpoints.push(endpoint),
            Ok(None) => {}
            Err(RecvTimeoutError::Timeout) => {
                timed_out = true;
                break;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    endpoints.sort_by_key(|e| e.port);
    (endpoints, timed_out)
}

/// One real `GET` against loopback. 127.0.0.1 only, never a hostname and never
/// another host. Non-200, a connection error and a timeout are all just
/// "no answer" — the body is read through a [`PROBE_MAX_BYTES`] cap so a local
/// server cannot stream us out of memory.
fn probe_get(port: u16, path: &str) -> Option<String> {
    use std::io::Read;

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(PROBE_PORT_TIMEOUT)
        .timeout(PROBE_PORT_TIMEOUT)
        .build();
    let response = agent
        .get(&format!("http://127.0.0.1:{port}{path}"))
        .call()
        .ok()?;
    if response.status() != 200 {
        return None;
    }
    let mut body = Vec::new();
    response
        .into_reader()
        .take(PROBE_MAX_BYTES as u64)
        .read_to_end(&mut body)
        .ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// Probe the default ports plus `extra` for a local LLM endpoint. Touches no
/// disk and no host other than 127.0.0.1. Ports that do not answer are simply
/// absent from `endpoints`.
pub fn probe_llm(extra: Option<&[u16]>) -> Result<InitProbeReport, String> {
    let probed_ports = probe_ports(extra.unwrap_or(&[]))?;
    let (endpoints, timed_out) = probe_concurrent(
        &probed_ports,
        PROBE_TOTAL_BUDGET,
        Arc::new(|port| probe_endpoint_pure(port, &probe_get)),
    );
    Ok(InitProbeReport {
        probed_ports,
        endpoints,
        timed_out,
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

/// Flatten a probe-supplied string onto one line. Model ids and version
/// strings come straight off a local server we do not control: a newline in
/// one of them would end the generated comment (or the quoted scalar) and let
/// the rest of the value land in the file as YAML.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
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

/// One header line for an endpoint that answered the probe.
///
/// The address is ours (the port we connected to), never the server's word for
/// itself. `label` *is* the server's word for itself, so it goes through
/// [`one_line`] before it reaches the file — the same rule the generated
/// definitions follow.
///
/// The three shapes differ because the reader has to be able to tell a
/// confirmed endpoint from a merely-responding one from the header alone; that
/// distinction is exactly what decides whether the definition below is live or
/// commented out.
fn detected_llm_item(endpoint: &LocalLlmEndpoint) -> String {
    let label = one_line(&endpoint.label);
    let label = label.trim();
    let detail = match endpoint.anthropic {
        // Confirmed: the server named itself via `/api/version`, so the label
        // carries a product and a version ("Ollama 0.32.1").
        Some(true) if !label.is_empty() => label.to_string(),
        Some(true) => "応答あり".to_string(),
        // Also named itself, but the version predates the Messages API. The
        // separator is `・`, not ` / `, which is what [`wrap_comment`] puts
        // *between* items — one item must not look like two.
        Some(false) if !label.is_empty() => format!("{label}・Messages API 非対応"),
        Some(false) => "Messages API 非対応".to_string(),
        // Only an OpenAI-compatible answer was seen. The label carries no
        // product name in this case (it is built from the port we already
        // print), so stating what is *missing* is the useful half.
        None => "OpenAI 互換・Messages API 未確認".to_string(),
    };
    format!("ローカル LLM 127.0.0.1:{} ({detail})", endpoint.port)
}

/// What the header records as found (spec §3.2: "何を見て何を出したか").
///
/// `llm` is the (already ordered and deduplicated) probe result. An empty
/// slice reproduces the pre-probe header byte for byte.
fn detected_items(scan: &InitScanReport, llm: &[&LocalLlmEndpoint]) -> Vec<String> {
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
    // D4 is only "something accepted a TCP connect on 3456". A probe result for
    // the same port knows strictly more, so it supersedes the D4 line here for
    // the same reason it suppresses the router block further down — otherwise
    // one port would be reported twice, in two different vocabularies.
    let router_probed = llm.iter().any(|e| Some(e.port) == scan.router_port);
    if let Some(port) = scan.router_port.filter(|_| !router_probed) {
        items.push(format!("ローカル LLM ルータ 127.0.0.1:{port} (応答あり)"));
    }
    for endpoint in llm {
        items.push(detected_llm_item(endpoint));
    }
    items
}

/// The other half of the record: what was looked for and NOT found. This is
/// what explains an absent block ("no git" = "that is why there is no worktree
/// note"), mirroring the preview UI requirement in spec §6.
fn missing_items(scan: &InitScanReport, llm: &[&LocalLlmEndpoint]) -> Vec<String> {
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
    // A port that answered must never also be listed as missing — that is the
    // misreading this whole pair of functions exists to prevent.
    //
    // The router line is kept when 3456 itself stayed silent, even if another
    // port (11434) answered: "3456 did not answer" is a fact about 3456, and it
    // is still what explains the absent router block below. With the answering
    // endpoints now listed on the 検出 side, the two lines read as the separate
    // statements they are rather than as "no local LLM was found".
    let router_answered = scan.router_port.is_some() || llm.iter().any(|e| e.port == ROUTER_PORT);
    if !router_answered {
        items.push(format!("ローカル LLM ルータ (127.0.0.1:{ROUTER_PORT})"));
    }
    items
}

/// The `ANTHROPIC_AUTH_TOKEN` placeholder for a probed endpoint.
///
/// The CLI rejects an empty token before it ever makes a request, so there is
/// always *some* value; which one is a documentation question, not a security
/// one — none of these are secrets, and none of them authenticate anything:
///
/// - `"ollama"` — Ollama documents the header as required but ignored.
/// - `"lmstudio"` — the value LM Studio's own docs use in their examples. It is
///   arbitrary while the server has authentication disabled (the default), and
///   is meant to be replaced by a real key once it is not.
/// - `"local"` — anything else that answered: a neutral placeholder rather than
///   the name of a product we have no evidence is running there.
///
/// `label` comes off a local server we do not control, so the match on it is
/// deliberately naive — lowercase plus substring, no parsing — and the matched
/// text never reaches the generated file: this returns one of three fixed
/// literals and nothing derived from its input.
fn auth_token_for(endpoint: &LocalLlmEndpoint) -> &'static str {
    let names_ollama = endpoint.label.to_ascii_lowercase().contains("ollama");
    // `anthropic == Some(true)` can only come from Ollama's `/api/version`
    // (see the probe module note), so it identifies an Ollama just as well as
    // the label does — and covers a label that never named the product.
    if names_ollama || endpoint.anthropic == Some(true) {
        "ollama"
    } else if endpoint.port == LM_STUDIO_PORT {
        "lmstudio"
    } else {
        "local"
    }
}

/// One generated definition for a probed endpoint (contract §生成).
///
/// `commented` renders the exact same shape behind `# `, so uncommenting the
/// block yields the live form character for character. The `他に` line keeps
/// its own `#` in that case for the same reason.
fn push_llm_definition(out: &mut String, endpoint: &LocalLlmEndpoint, ind: &str, commented: bool) {
    let label = one_line(&endpoint.label);
    let port = endpoint.port;
    if commented {
        // Why this is not a live definition: `/v1/models` answered, but
        // nothing proved `/v1/messages` exists (see the probe module note).
        out.push_str(&format!(
            "{ind}# {label} — /v1/messages が応答するかは未確認です。\n"
        ));
        out.push_str(&format!(
            "{ind}# 使うなら次の各行の先頭 # を外してください。\n"
        ));
    } else {
        out.push_str(&format!("{ind}# {label}\n"));
    }
    let head = if commented {
        format!("{ind}# ")
    } else {
        ind.to_string()
    };
    let models: Vec<String> = endpoint.models.iter().map(|m| one_line(m)).collect();
    let model = models.first().map(String::as_str).unwrap_or_default();
    out.push_str(&format!("{head}- name: local-{port}\n"));
    out.push_str(&format!(
        "{head}  cmd: {}\n",
        yaml_scalar(&format!("claude --model {model}"))
    ));
    if models.len() > 1 {
        out.push_str(&format!("{head}  # 他に: {}\n", models[1..].join(" / ")));
    }
    out.push_str(&format!("{head}  env:\n"));
    out.push_str(&format!(
        "{head}    ANTHROPIC_BASE_URL: \"http://127.0.0.1:{port}\"\n"
    ));
    // Which placeholder this is, and why it is never empty, is [`auth_token_for`].
    out.push_str(&format!(
        "{head}    ANTHROPIC_AUTH_TOKEN: \"{}\"\n",
        auth_token_for(endpoint)
    ));
    out.push_str(&format!("{head}  autostart: false\n"));
}

/// Probe results in generation order: one entry per port, ascending. The probe
/// already guarantees both, but generation must not depend on its caller
/// having done so — the same input has to yield the same bytes (spec §3.1).
fn ordered_endpoints(llm: &[LocalLlmEndpoint]) -> Vec<&LocalLlmEndpoint> {
    let mut out: Vec<&LocalLlmEndpoint> = Vec::new();
    for endpoint in llm {
        if !out.iter().any(|e| e.port == endpoint.port) {
            out.push(endpoint);
        }
    }
    out.sort_by_key(|e| e.port);
    out
}

/// Build the `ptygrid.yml` text for a scan result. Pure: same inputs (including
/// `today`) always produce the same bytes — the property `serde_norway` output
/// could not give us (spec §3.1).
///
/// `llm` is the (optional) result of [`probe_llm`]. An empty slice reproduces
/// the pre-probe output byte for byte.
pub(crate) fn render_config(
    scan: &InitScanReport,
    llm: &[LocalLlmEndpoint],
    today: &str,
) -> String {
    let mut out = String::new();

    // Only a confirmed endpoint becomes a live definition; everything else is
    // rendered commented out (contract §生成).
    let endpoints = ordered_endpoints(llm);
    let (confirmed, unconfirmed): (Vec<&LocalLlmEndpoint>, Vec<&LocalLlmEndpoint>) =
        endpoints.iter().partition(|e| e.anthropic == Some(true));
    // Whether the output carries a real `agents:` key, which decides the
    // indent of every commented-out definition that follows it.
    let agents_key = !scan.agents.is_empty() || !confirmed.is_empty();
    let ind = if agents_key { "  " } else { "" };

    // ---- header: provenance + what detection saw ----
    out.push_str(&format!(
        "# {CONFIG_FILE_NAME} — ptygrid init が生成しました ({today})\n"
    ));
    out.push_str(&wrap_comment("検出", &detected_items(scan, &endpoints)));
    out.push('\n');
    let missing = missing_items(scan, &endpoints);
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
    if scan.agents.is_empty() && confirmed.is_empty() {
        // Key line by key line (spec §3.5): leaving a bare `agents:` behind
        // would parse as null and fail the load.
        out.push_str("\n# PATH 上に既知の CLI が見つかりませんでした。CLI を入れたら、\n");
        out.push_str("# 次のブロックの各行の先頭 # を外してください。\n");
        out.push_str("# agents:\n");
        out.push_str("#   - name: claude\n");
        out.push_str("#     cmd: \"claude\"\n");
        out.push_str("#     cwd: \".\"\n");
        out.push_str("#     autostart: false\n");
    } else if scan.agents.is_empty() {
        // No CLI on PATH, but a probed endpoint earned a live definition — so
        // the key really is there and the skeleton has to become a commented
        // *entry* under it, not a second `agents:` key.
        out.push_str("\n# PATH 上に既知の CLI が見つかりませんでした。CLI を入れたら、\n");
        out.push_str("# 次のブロックの各行の先頭 # を外してください。\n");
        out.push_str("agents:\n");
        out.push_str("  # - name: claude\n");
        out.push_str("  #   cmd: \"claude\"\n");
        out.push_str("  #   cwd: \".\"\n");
        out.push_str("  #   autostart: false\n");
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

    // ---- probed local LLM endpoints (5.0.2 追補 §生成) ----
    for &endpoint in &confirmed {
        out.push('\n');
        push_llm_definition(&mut out, endpoint, ind, false);
    }
    for &endpoint in &unconfirmed {
        out.push('\n');
        push_llm_definition(&mut out, endpoint, ind, true);
    }

    // ---- local LLM router (always commented out: needs router.settings.json) ----
    //
    // D4 is only a weak hint ("something answered a TCP connect"), so a probe
    // result for the same port supersedes it and this block is dropped. A
    // probe that did not cover the router port leaves it exactly as before.
    let router_probed = endpoints.iter().any(|e| Some(e.port) == scan.router_port);
    if let Some(port) = scan.router_port.filter(|_| !router_probed) {
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
    llm: &[LocalLlmEndpoint],
    dir: &Path,
    target: InitTarget,
    home: Option<&Path>,
    today: &str,
) -> Result<InitPreview, String> {
    let content = render_config(&scan, llm, today);
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
///
/// `llm` carries an earlier [`probe_llm`] result, if the user ran one; an
/// empty slice generates exactly what the pre-probe version did. The probe is
/// never run from here — it costs seconds and stays an explicit user action.
pub fn preview(
    dir: &Path,
    target: InitTarget,
    llm: &[LocalLlmEndpoint],
) -> Result<InitPreview, String> {
    let dir = absolute_dir(dir);
    let home = crate::pty::home_dir().map(PathBuf::from);
    preview_from_scan(scan(&dir), llm, &dir, target, home.as_deref(), &today())
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

    /// "The user never ran the probe" — the pre-5.0.2-追補 generation input.
    const NO_LLM: &[LocalLlmEndpoint] = &[];

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
        let yaml = render_config(&scan_with("/tmp/my-app", &["claude"]), NO_LLM, "2026-07-29");
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
        let yaml = render_config(&scan, NO_LLM, "2026-07-29");
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
            let yaml = render_config(&scan, NO_LLM, "2026-07-29");
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
        let a = render_config(&scan, NO_LLM, "2026-07-29");
        let b = render_config(&scan, NO_LLM, "2026-07-29");
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn header_records_what_detection_saw_and_missed() {
        let scan = InitScanReport {
            project_kinds: vec!["cargo".to_string()],
            git_repo: true,
            ..scan_with("/tmp/my-app", &["claude", "codex"])
        };
        let yaml = render_config(&scan, NO_LLM, "2026-07-29");
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
        let minimal = render_config(&scan_with("/tmp/my-app", &["claude"]), NO_LLM, "2026-07-29");
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
            preview_from_scan(scan, NO_LLM, &dir, InitTarget::Project, None, "2026-07-29").unwrap();
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
        assert!(parse_config(&render_config(&empty_scan("/tmp/x"), NO_LLM, "2026-07-29")).is_ok());
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
            NO_LLM,
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
            NO_LLM,
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
            NO_LLM,
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

        let preview = preview_from_scan(
            report,
            NO_LLM,
            &dir,
            InitTarget::Project,
            None,
            "2026-07-29",
        )
        .unwrap();
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
            NO_LLM,
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
        let yaml = render_config(&scan_with("/tmp/my app", &["claude"]), NO_LLM, "2026-07-29");
        assert_eq!(
            parse_config(&yaml).unwrap().project.as_deref(),
            Some("my app")
        );
    }

    // ---- local LLM probe (5.0.2 追補) ----------------------------------

    fn endpoint(port: u16, models: &[&str], anthropic: Option<bool>) -> LocalLlmEndpoint {
        LocalLlmEndpoint {
            port,
            models: models.iter().map(|m| m.to_string()).collect(),
            anthropic,
            label: match anthropic {
                Some(true) => "Ollama 0.14.3".to_string(),
                _ => format!("127.0.0.1:{port} (OpenAI 互換の応答)"),
            },
        }
    }

    /// A loopback HTTP server answering `handler(path)` with 200 + that body,
    /// or 404. One request per connection (`Connection: close`), which is all
    /// the probe needs and keeps the test free of keep-alive timing.
    fn test_server(handler: impl Fn(&str) -> Option<String> + Send + 'static) -> u16 {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Ok(peek) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(peek);
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                loop {
                    let mut header = String::new();
                    match reader.read_line(&mut header) {
                        Ok(0) => break,
                        Ok(_) if header.trim().is_empty() => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let response = match handler(&path) {
                    Some(body) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                    }
                };
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        port
    }

    #[test]
    fn parses_model_ids_out_of_a_v1_models_body() {
        assert_eq!(
            parse_models(r#"{"object":"list","data":[{"id":"a"},{"id":"b"}]}"#),
            vec!["a".to_string(), "b".to_string()]
        );
        // Nothing usable -> "did not answer usefully", never an error.
        for body in [
            "{}",                                  // no data
            r#"{"data":{}}"#,                      // data not an array
            r#"{"data":[{"id":42},{"id":null}]}"#, // id not a string
            r#"{"data":[{"name":"a"}]}"#,          // no id at all
            r#"{"data":[{"id":""}]}"#,             // empty id
            "not json at all",
            "",
        ] {
            assert!(parse_models(body).is_empty(), "unexpected models: {body}");
        }
        // Mixed: the usable ids survive, the rest are dropped.
        assert_eq!(
            parse_models(r#"{"data":[{"id":1},{"id":"ok"},{"x":2}]}"#),
            vec!["ok".to_string()]
        );
        // Order is the server's, and the count is capped.
        let many: Vec<String> = (0..PROBE_MAX_MODELS + 7)
            .map(|i| format!(r#"{{"id":"m{i}"}}"#))
            .collect();
        let models = parse_models(&format!(r#"{{"data":[{}]}}"#, many.join(",")));
        assert_eq!(models.len(), PROBE_MAX_MODELS);
        assert_eq!(models[0], "m0");
        assert_eq!(
            models[PROBE_MAX_MODELS - 1],
            format!("m{}", PROBE_MAX_MODELS - 1)
        );
        // A body cut off at the byte cap is simply invalid JSON.
        let huge = format!(r#"{{"data":[{{"id":"{}"}}]}}"#, "a".repeat(PROBE_MAX_BYTES));
        assert!(parse_models(&huge[..PROBE_MAX_BYTES]).is_empty());
    }

    #[test]
    fn versions_are_compared_numerically_not_lexically() {
        assert_eq!(parse_version("0.14.0"), Some((0, 14, 0)));
        assert_eq!(parse_version("0.14.3"), Some((0, 14, 3)));
        assert_eq!(parse_version("0.9.0"), Some((0, 9, 0)));
        // Pre-release / build suffixes are dropped, missing parts read as 0.
        assert_eq!(parse_version("0.14.0-rc1"), Some((0, 14, 0)));
        assert_eq!(parse_version("0.14.0+build.7"), Some((0, 14, 0)));
        assert_eq!(parse_version(" 1.0 "), Some((1, 0, 0)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
        // Not comparable -> unknown, never a guess.
        for broken in ["", "  ", "abc", "v0.14.0", "0.x.1", "1.2.3.4", "-", "0..1"] {
            assert_eq!(parse_version(broken), None, "must not parse: {broken:?}");
        }
        // The point of the triple: as text "0.9.0" sorts ABOVE "0.14.0".
        assert!("0.9.0" > "0.14.0");
        assert!(parse_version("0.9.0").unwrap() < OLLAMA_MIN_ANTHROPIC);
        assert!(parse_version("0.14.0").unwrap() >= OLLAMA_MIN_ANTHROPIC);
        assert!(parse_version("0.14.3").unwrap() >= OLLAMA_MIN_ANTHROPIC);
        assert!(parse_version("1.0.0").unwrap() >= OLLAMA_MIN_ANTHROPIC);
        assert!(parse_version("0.13.9").unwrap() < OLLAMA_MIN_ANTHROPIC);

        // The same rules through the /api/version body.
        assert_eq!(
            parse_api_version(r#"{"version":"0.14.3"}"#),
            Some(("0.14.3".to_string(), (0, 14, 3)))
        );
        for body in [
            "{}",
            r#"{"version":42}"#,
            r#"{"version":"nightly"}"#,
            "nope",
        ] {
            assert_eq!(parse_api_version(body), None, "must not confirm: {body}");
        }
    }

    #[test]
    fn only_api_version_can_confirm_anthropic_support() {
        let models = r#"{"data":[{"id":"gpt-oss:20b"},{"id":"qwen3"}]}"#.to_string();
        // OpenAI-compatible only: an answer, but no confirmation.
        let openai_only = probe_endpoint_pure(1234, &|_, path| {
            (path == "/v1/models").then(|| models.clone())
        })
        .expect("an endpoint that answers /v1/models is reported");
        assert_eq!(openai_only.anthropic, None);
        assert_eq!(openai_only.label, "127.0.0.1:1234 (OpenAI 互換の応答)");
        assert_eq!(openai_only.models, vec!["gpt-oss:20b", "qwen3"]);

        // Ollama new enough: confirmed, and it gets to name itself.
        let ollama = probe_endpoint_pure(11434, &|_, path| match path {
            "/v1/models" => Some(models.clone()),
            "/api/version" => Some(r#"{"version":"0.14.3"}"#.to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(ollama.anthropic, Some(true));
        assert_eq!(ollama.label, "Ollama 0.14.3");

        // Ollama too old: confirmed UNsupported, not merely unknown.
        let old = probe_endpoint_pure(11434, &|_, path| match path {
            "/v1/models" => Some(models.clone()),
            "/api/version" => Some(r#"{"version":"0.9.0"}"#.to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(old.anthropic, Some(false));
        assert_eq!(old.label, "Ollama 0.9.0");

        // No usable /v1/models -> not an endpoint at all, whatever else answers.
        assert!(probe_endpoint_pure(1234, &|_, _| None).is_none());
        assert!(probe_endpoint_pure(1234, &|_, path| {
            (path == "/v1/models").then(|| r#"{"data":[]}"#.to_string())
        })
        .is_none());
        assert!(probe_endpoint_pure(1234, &|_, path| match path {
            "/v1/models" => Some("<html>not an api</html>".to_string()),
            "/api/version" => Some(r#"{"version":"9.9.9"}"#.to_string()),
            _ => None,
        })
        .is_none());
    }

    #[test]
    fn probe_talks_to_a_real_loopback_server_within_the_byte_cap() {
        // Full path through ureq: request, 200, JSON, second request.
        let models = r#"{"data":[{"id":"gpt-oss:20b"}]}"#;
        let port = test_server(move |path| match path {
            "/v1/models" => Some(models.to_string()),
            "/api/version" => Some(r#"{"version":"0.14.3"}"#.to_string()),
            _ => None,
        });
        let found = probe_endpoint_pure(port, &probe_get).expect("the server answers");
        assert_eq!(found.port, port);
        assert_eq!(found.models, vec!["gpt-oss:20b"]);
        assert_eq!(found.anthropic, Some(true));

        // A body past the cap is cut off mid-JSON, so nothing is reported —
        // the read never grows with the response.
        let big = test_server(|path| {
            (path == "/v1/models").then(|| {
                format!(
                    r#"{{"data":[{{"id":"{}"}}]}}"#,
                    "a".repeat(PROBE_MAX_BYTES * 2)
                )
            })
        });
        let body = probe_get(big, "/v1/models").expect("200 with a huge body");
        assert_eq!(body.len(), PROBE_MAX_BYTES);
        assert!(probe_endpoint_pure(big, &probe_get).is_none());

        // 404 / a port with nothing on it are both just "no answer".
        assert!(probe_get(big, "/api/version").is_none());
        let dead = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let dead_port = dead.local_addr().unwrap().port();
        drop(dead);
        assert!(probe_endpoint_pure(dead_port, &probe_get).is_none());
    }

    #[test]
    fn probe_ports_are_the_defaults_plus_deduplicated_sorted_extras() {
        assert_eq!(probe_ports(&[]).unwrap(), vec![1234, 3456, 11434]);
        assert_eq!(
            probe_ports(&[8080, 1234, 8080]).unwrap(),
            vec![1234, 3456, 8080, 11434],
            "extras join the defaults once each, ascending"
        );
        assert_eq!(probe_ports(&[1, 2, 3, 4]).unwrap().len(), 7);
    }

    #[test]
    fn bad_port_is_the_only_rejection() {
        for extra in [vec![0u16], vec![8080, 0], vec![0, 0]] {
            let err = probe_ports(&extra).unwrap_err();
            assert!(err.starts_with("bad_port:"), "unexpected error: {err}");
        }
        // One extra too many.
        let too_many: Vec<u16> = (0..MAX_EXTRA_PORTS as u16 + 1).map(|i| 9000 + i).collect();
        let err = probe_ports(&too_many).unwrap_err();
        assert!(err.starts_with("bad_port:"), "unexpected error: {err}");
        // Exactly the limit is fine.
        let at_limit: Vec<u16> = (0..MAX_EXTRA_PORTS as u16).map(|i| 9000 + i).collect();
        assert!(probe_ports(&at_limit).is_ok());
        // A port that simply does not answer is NOT an error.
        let report = probe_llm(Some(&[9])).unwrap();
        assert!(report.probed_ports.contains(&9));
        assert!(!report.timed_out);
    }

    #[test]
    fn probe_returns_whatever_finished_inside_the_budget() {
        // A port that hangs past the budget must not extend the command; the
        // ports that did answer are still reported, sorted, with timed_out.
        let slow = Arc::new(|port: u16| {
            if port == 11434 {
                std::thread::sleep(Duration::from_secs(30));
            }
            Some(endpoint(port, &["m"], None))
        });
        let started = Instant::now();
        let (endpoints, timed_out) =
            probe_concurrent(&[1234, 3456, 11434], Duration::from_millis(300), slow);
        assert!(timed_out);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "budget not honored"
        );
        assert_eq!(
            endpoints.iter().map(|e| e.port).collect::<Vec<_>>(),
            vec![1234, 3456]
        );

        // Everything answering in time: no timeout flag, ascending, and ports
        // with nothing on them are absent rather than an error.
        let quick = Arc::new(|port: u16| (port != 3456).then(|| endpoint(port, &["m"], None)));
        let (endpoints, timed_out) =
            probe_concurrent(&[11434, 3456, 1234], PROBE_TOTAL_BUDGET, quick);
        assert!(!timed_out);
        assert_eq!(
            endpoints.iter().map(|e| e.port).collect::<Vec<_>>(),
            vec![1234, 11434]
        );
    }

    // ---- generation with probe results ---------------------------------

    /// The exact bytes generation produced before the probe existed. `llm`
    /// empty must reproduce them character for character (contract §生成:
    /// 決定性は維持 / init_preview の引数追加は additive).
    const GOLDEN_NO_LLM: &str = "\
# ptygrid.yml — ptygrid init が生成しました (2026-07-29)
# 検出: claude / codex (PATH) / Cargo.toml / package.json /
#       git リポジトリ / ローカル LLM ルータ 127.0.0.1:3456 (応答あり)
# 中身はすべて手で編集できます。全ブロックの注釈つき例は ptygrid.example.yml、
# 用途別の見本は example/ を参照してください。

project: my-app          # 作業フォルダ名から。ヘッダーに出る表示名

# queen: ペイン間の読み書き・メッセージ・spawn を仲介する内蔵 MCP サーバー。
# 各 CLI への登録コマンドはツールバーの Queen バッジからコピーできます。
queen:
  enabled: true
  port: 39237

agents:
  - name: claude
    cmd: \"claude\"
    cwd: \".\"
    autostart: false     # 読み込みと同時に起動するなら true（初回は手動 ▶ 起動）

  - name: codex
    cmd: \"codex\"
    cwd: \".\"
    autostart: false

  # ローカル LLM ルータ (127.0.0.1:3456) が応答しました。使うならコメントを外し、
  # router.settings.json を用意してください（env だけに頼らず --settings を渡すのが
  # 確実な理由は example/team-preset/ptygrid.yml を参照）。
  # - name: local
  #   cmd: \"claude --settings router.settings.json\"
  #   cwd: \".\"
  #   env:
  #     ANTHROPIC_BASE_URL: \"${CODEROUTER_URL}\"
  #   autostart: false

# Cargo.toml / package.json を検出しました。dev サーバーやテスト watch を常駐させるなら
# 次のブロックの各行の先頭 # を外してください（agents と同じフィールドを持ちます）。
# processes:
#   - name: dev
#     cmd: \"npm run dev\"
#     cwd: \".\"
#     autostart: false
#     autorestart: on-failure   # 異常終了時のみ再起動

# git リポジトリを検出しました。ペインごとに linked worktree を切るなら
# example/worktree を参照してください（init は worktree: を生成しません）。

# チーム一括起動 (team_presets:) は example/team-preset、
# DAG オーケストレーション (workflows:) は example/adaptive-orchestration を参照。
";

    fn golden_scan() -> InitScanReport {
        InitScanReport {
            project_kinds: vec!["cargo".to_string(), "npm".to_string()],
            git_repo: true,
            router_port: Some(ROUTER_PORT),
            ..scan_with("/tmp/my-app", &["claude", "codex"])
        }
    }

    #[test]
    fn generation_without_probe_results_is_byte_for_byte_the_old_output() {
        assert_eq!(
            render_config(&golden_scan(), NO_LLM, "2026-07-29").as_bytes(),
            GOLDEN_NO_LLM.as_bytes()
        );
        // An empty list and "the user never probed" are the same thing.
        assert_eq!(
            render_config(&golden_scan(), &[], "2026-07-29"),
            render_config(&golden_scan(), NO_LLM, "2026-07-29")
        );
        // ...including for the nothing-detected skeleton.
        let bare = empty_scan("/tmp/my-app");
        assert_eq!(
            render_config(&bare, &[], "2026-07-29"),
            render_config(&bare, NO_LLM, "2026-07-29")
        );
    }

    #[test]
    fn only_a_confirmed_endpoint_becomes_a_live_definition() {
        let llm = vec![
            endpoint(1234, &["local-model"], None),
            endpoint(11434, &["gpt-oss:20b", "qwen3", "llama3"], Some(true)),
            endpoint(8080, &["old"], Some(false)),
        ];
        let yaml = render_config(&golden_scan(), &llm, "2026-07-29");
        let config = parse_config(&yaml).expect("generated config must parse");

        // Confirmed -> a real definition, named with its port.
        let names: Vec<_> = config.agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["claude", "codex", "local-11434"]);
        let local = config
            .agents
            .iter()
            .find(|a| a.name == "local-11434")
            .unwrap();
        assert_eq!(local.cmd, "claude --model gpt-oss:20b");
        assert_eq!(local.autostart, Some(false));
        let env = local.env.clone().expect("env is generated");
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("http://127.0.0.1:11434")
        );
        assert_eq!(
            env.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str),
            Some("ollama")
        );
        // The models it did not pick are recorded as a comment, in order.
        assert!(yaml.contains("    # 他に: qwen3 / llama3"), "{yaml}");

        // Unknown and confirmed-unsupported are commented out, with the reason.
        for port in [1234, 8080] {
            assert!(
                !yaml
                    .lines()
                    .any(|l| l.trim_start().starts_with(&format!("- name: local-{port}"))),
                "local-{port} must not be a live definition:\n{yaml}"
            );
        }
        assert!(yaml.contains("  # - name: local-1234"), "{yaml}");
        assert!(yaml.contains("  # - name: local-8080"), "{yaml}");
        assert!(yaml.contains("/v1/messages が応答するかは未確認です。"));
        assert!(yaml.contains("  # 127.0.0.1:1234 (OpenAI 互換の応答) —"));

        // Nothing generated ever autostarts (contract §禁止事項).
        assert!(!yaml.contains("autostart: true"));
        for def in config.agents.iter().chain(config.processes.iter()) {
            assert_eq!(def.autostart, Some(false), "{} autostarts", def.name);
        }
    }

    /// An endpoint with a label of its own, which the `endpoint` helper does
    /// not produce (it only ever builds the two labels the probe builds).
    fn labelled(port: u16, anthropic: Option<bool>, label: &str) -> LocalLlmEndpoint {
        LocalLlmEndpoint {
            port,
            models: vec!["m".to_string()],
            anthropic,
            label: label.to_string(),
        }
    }

    /// The 検出 / 未検出 halves of the header, wrapped continuation lines
    /// folded back in. A continuation is `#` + padding, which is what tells it
    /// apart from the prose comment lines that follow.
    fn header_sections(yaml: &str) -> (String, String) {
        let (mut found, mut missing) = (String::new(), String::new());
        let (mut into_found, mut active) = (false, false);
        for line in yaml.lines().take_while(|l| l.starts_with('#')) {
            if let Some(rest) = line.strip_prefix("# 検出: ") {
                (into_found, active) = (true, true);
                found.push_str(rest);
            } else if let Some(rest) = line.strip_prefix("# 未検出: ") {
                (into_found, active) = (false, true);
                missing.push_str(rest);
            } else if active && line.starts_with("#  ") {
                let target = if into_found { &mut found } else { &mut missing };
                target.push(' ');
                target.push_str(line.trim_start_matches(['#', ' ']));
            } else {
                active = false;
            }
        }
        (found, missing)
    }

    #[test]
    fn an_endpoint_that_answered_is_recorded_as_detected_never_as_missing() {
        // The case seen on a real machine: 3456 stayed silent while 11434
        // answered as an Ollama, and the header still read "not found".
        let scan = InitScanReport {
            router_port: None,
            ..golden_scan()
        };
        let llm = vec![
            labelled(11434, Some(true), "Ollama 0.32.1"),
            endpoint(1234, &["local-model"], None),
        ];
        let (found, missing) = header_sections(&render_config(&scan, &llm, "2026-07-29"));

        // Both answering ports are on the 検出 side, the confirmed one naming
        // the product, the unconfirmed one saying what is still unknown.
        assert!(
            found.contains("ローカル LLM 127.0.0.1:11434 (Ollama 0.32.1)"),
            "{found}"
        );
        assert!(
            found.contains("ローカル LLM 127.0.0.1:1234 (OpenAI 互換・Messages API 未確認)"),
            "{found}"
        );
        // ...and on neither is repeated as missing.
        assert!(!missing.contains("11434"), "{missing}");
        assert!(!missing.contains("1234"), "{missing}");
        // 3456 really did stay silent, so that line is still the truth — and
        // is no longer readable as "no local LLM was found".
        assert!(
            missing.contains("ローカル LLM ルータ (127.0.0.1:3456)"),
            "{missing}"
        );

        // A confirmed-unsupported endpoint says so rather than going quiet.
        let (found, _) = header_sections(&render_config(
            &scan,
            &[labelled(11434, Some(false), "Ollama 0.13.0")],
            "2026-07-29",
        ));
        assert!(
            found.contains("ローカル LLM 127.0.0.1:11434 (Ollama 0.13.0・Messages API 非対応)"),
            "{found}"
        );
    }

    #[test]
    fn the_router_port_answering_the_probe_replaces_the_d4_header_line() {
        // Probed 3456 while D4 saw nothing: it belongs on the 検出 side, and
        // the missing line it would otherwise explain has to go with it.
        let silent = InitScanReport {
            router_port: None,
            ..golden_scan()
        };
        let llm = vec![labelled(
            ROUTER_PORT,
            None,
            "127.0.0.1:3456 (OpenAI 互換の応答)",
        )];
        let (found, missing) = header_sections(&render_config(&silent, &llm, "2026-07-29"));
        assert!(found.contains("ローカル LLM 127.0.0.1:3456"), "{found}");
        assert!(!missing.contains("3456"), "{missing}");

        // D4 and the probe both saw it: one line, the one that knows more.
        let (found, missing) = header_sections(&render_config(&golden_scan(), &llm, "2026-07-29"));
        assert_eq!(found.matches("3456").count(), 1, "{found}");
        assert!(
            !found.contains("ルータ 127.0.0.1:3456 (応答あり)"),
            "{found}"
        );
        assert!(!missing.contains("3456"), "{missing}");

        // Without a probe result D4's own line is untouched.
        let (found, _) = header_sections(&render_config(&golden_scan(), NO_LLM, "2026-07-29"));
        assert!(
            found.contains("ローカル LLM ルータ 127.0.0.1:3456 (応答あり)"),
            "{found}"
        );
    }

    #[test]
    fn the_auth_token_placeholder_follows_the_endpoint() {
        // 1. Ollama — by confirmation, by name, or by both.
        assert_eq!(
            auth_token_for(&labelled(11434, Some(true), "Ollama 0.32.1")),
            "ollama"
        );
        // Named itself but is too old for the Messages API: still an Ollama.
        assert_eq!(
            auth_token_for(&labelled(11434, Some(false), "Ollama 0.13.0")),
            "ollama"
        );
        // Confirmed without ever naming the product.
        assert_eq!(auth_token_for(&labelled(11434, Some(true), "")), "ollama");
        // Matching is naive on purpose: case and surrounding text do not matter.
        assert_eq!(
            auth_token_for(&labelled(9999, None, "custom OLLAMA build")),
            "ollama"
        );

        // 2. LM Studio's port, with nothing claiming to be something else.
        assert_eq!(
            auth_token_for(&endpoint(LM_STUDIO_PORT, &["m"], None)),
            "lmstudio"
        );
        // An Ollama on that port is still an Ollama; the name beats the port.
        assert_eq!(
            auth_token_for(&labelled(LM_STUDIO_PORT, None, "Ollama 0.32.1")),
            "ollama"
        );

        // 3. Everything else gets a neutral placeholder.
        assert_eq!(auth_token_for(&endpoint(8080, &["m"], None)), "local");
        assert_eq!(
            auth_token_for(&labelled(ROUTER_PORT, None, "LM Studio")),
            "local"
        );

        // ...and that is what lands in the generated file, per endpoint.
        let llm = vec![
            labelled(11434, Some(true), "Ollama 0.32.1"),
            endpoint(LM_STUDIO_PORT, &["local-model"], None),
            endpoint(8080, &["m"], None),
        ];
        let yaml = render_config(&golden_scan(), &llm, "2026-07-29");
        let token_lines: Vec<&str> = yaml
            .lines()
            .filter(|l| l.contains("ANTHROPIC_AUTH_TOKEN"))
            .map(|l| l.trim_start_matches(['#', ' ']))
            .collect();
        assert_eq!(
            token_lines,
            vec![
                "ANTHROPIC_AUTH_TOKEN: \"ollama\"",
                "ANTHROPIC_AUTH_TOKEN: \"lmstudio\"",
                "ANTHROPIC_AUTH_TOKEN: \"local\"",
            ],
            "{yaml}"
        );
        let config = parse_config(&yaml).expect("generated config must parse");
        let live = config
            .agents
            .iter()
            .find(|a| a.name == "local-11434")
            .unwrap();
        assert_eq!(
            live.env
                .as_ref()
                .and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN"))
                .map(String::as_str),
            Some("ollama")
        );

        // The string the decision was made on is never what gets written: a
        // lowercased match must not turn into a lowercased label.
        let yaml = render_config(
            &golden_scan(),
            &[labelled(9999, None, "MyOllama Server 1.0")],
            "2026-07-29",
        );
        assert!(yaml.contains("MyOllama Server 1.0"), "{yaml}");
        assert!(yaml.contains("ANTHROPIC_AUTH_TOKEN: \"ollama\""), "{yaml}");
        assert!(!yaml.contains("myollama"), "{yaml}");
    }

    #[test]
    fn a_confirmed_endpoint_carries_the_agents_key_when_no_cli_is_on_path() {
        let scan = empty_scan("/tmp/my-app");
        let llm = vec![endpoint(11434, &["gpt-oss:20b"], Some(true))];
        let yaml = render_config(&scan, &llm, "2026-07-29");
        let config = parse_config(&yaml).expect("generated config must parse");
        // Exactly one `agents:` key: the skeleton became a commented ENTRY, so
        // uncommenting it cannot produce a duplicate key.
        assert_eq!(yaml.matches("\nagents:\n").count(), 1);
        assert!(!yaml.contains("# agents:"));
        assert!(yaml.contains("  # - name: claude"));
        let names: Vec<_> = config.agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["local-11434"]);

        // With nothing confirmed the skeleton stays exactly as it was.
        let unknown = vec![endpoint(1234, &["m"], None)];
        let yaml = render_config(&scan, &unknown, "2026-07-29");
        assert!(yaml.contains("# agents:\n"));
        assert!(parse_config(&yaml).unwrap().agents.is_empty());
    }

    #[test]
    fn a_probed_router_port_suppresses_the_router_comment_block() {
        let scan = golden_scan();
        assert_eq!(scan.router_port, Some(ROUTER_PORT));
        let block = "# ローカル LLM ルータ (127.0.0.1:3456) が応答しました";

        // Same port in the probe result: the probe wins, D4's weak hint goes.
        for probed in [
            endpoint(ROUTER_PORT, &["m"], Some(true)),
            endpoint(ROUTER_PORT, &["m"], None),
            endpoint(ROUTER_PORT, &["m"], Some(false)),
        ] {
            let yaml = render_config(&scan, &[probed], "2026-07-29");
            assert!(!yaml.contains(block), "router block not suppressed: {yaml}");
            assert!(yaml.contains("local-3456"));
        }

        // A probe that did not cover the router port leaves it alone.
        let elsewhere = vec![endpoint(11434, &["m"], Some(true))];
        let yaml = render_config(&scan, &elsewhere, "2026-07-29");
        assert!(yaml.contains(block), "{yaml}");
        // ...and so does one where 3456 did not answer.
        assert!(render_config(&scan, NO_LLM, "2026-07-29").contains(block));
    }

    #[test]
    fn generation_is_deterministic_for_the_same_probe_result() {
        let scan = golden_scan();
        // Ports out of order and duplicated: the output must not depend on it.
        let llm = vec![
            endpoint(11434, &["a", "b"], Some(true)),
            endpoint(1234, &["c"], None),
            endpoint(11434, &["a", "b"], Some(true)),
        ];
        let a = render_config(&scan, &llm, "2026-07-29");
        let b = render_config(&scan, &llm, "2026-07-29");
        assert_eq!(a.as_bytes(), b.as_bytes());
        // One definition per port, ascending, whatever order they arrived in.
        let sorted = vec![llm[1].clone(), llm[0].clone()];
        assert_eq!(render_config(&scan, &sorted, "2026-07-29"), a);
        assert_eq!(a.matches("- name: local-11434").count(), 1);
        assert!(a.find("local-1234").unwrap() > a.find("local-11434").unwrap());
        assert!(parse_config(&a).is_ok());
    }

    #[test]
    fn probe_supplied_strings_cannot_break_out_of_the_generated_file() {
        // A local server is not trusted: a newline in a model id or a version
        // suffix would otherwise end the comment and inject YAML.
        let hostile = LocalLlmEndpoint {
            port: 11434,
            models: vec!["m\nagents: []".to_string(), "other\n# nope".to_string()],
            anthropic: Some(true),
            label: "Ollama 0.14.0-\nqueen:\n  port: 1".to_string(),
        };
        let yaml = render_config(&golden_scan(), &[hostile], "2026-07-29");
        let config = parse_config(&yaml).expect("hostile input still parses");
        assert_eq!(config.agents.len(), 3);
        assert_eq!(
            config.queen.and_then(|q| q.port),
            Some(crate::queen::DEFAULT_PORT)
        );
        assert_eq!(one_line("a\nb\tc"), "a b c");
    }

    #[test]
    fn preview_passes_probe_results_through_to_the_generated_content() {
        let dir = temp_dir("probe-preview");
        let llm = vec![endpoint(11434, &["gpt-oss:20b"], Some(true))];
        let preview = preview_from_scan(
            scan_with(&dir.display().to_string(), &["claude"]),
            &llm,
            &dir,
            InitTarget::Project,
            None,
            "2026-07-29",
        )
        .unwrap();
        assert!(preview.valid, "self-check failed: {:?}", preview.error);
        assert!(preview.content.contains("- name: local-11434"));
        assert_eq!(
            preview.content,
            render_config(
                &scan_with(&dir.display().to_string(), &["claude"]),
                &llm,
                "2026-07-29"
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
