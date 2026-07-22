<script lang="ts">
  import { onMount } from "svelte";
  import { Splitpanes, Pane } from "svelte-splitpanes";
  import Terminal from "./lib/Terminal.svelte";
  import TranscriptPane from "./lib/TranscriptPane.svelte";
  import GitPanel from "./lib/GitPanel.svelte";
  import StatusSidebar from "./lib/StatusSidebar.svelte";
  import type { StatusRow } from "./lib/StatusSidebar.svelte";
  import WorkflowPanel from "./lib/WorkflowPanel.svelte";
  import {
    ui,
    MAX_PANES,
    initGlobalListeners,
    paneTitle,
    addNotice,
    dismissNotice,
    refreshQueenStatus,
    refreshTeammateHooks,
    refreshTeamsHostStatus,
    markPaneClosed,
    focusPane,
    clearAgentStatus,
    type LayoutMode,
  } from "./lib/stores.svelte";
  import { disposeTermHandle, writeToTerm } from "./lib/terminals";
  import { msg, i18n, setLocaleSetting, type LocaleSetting } from "./lib/i18n.svelte";
  import { invokeCmd, isTauri } from "./lib/tauri";
  import { buildCdCommand, selectCdTargets } from "./lib/broadcast";
  import type {
    AgentStatus,
    ConfigInfo,
    HostLeadStatus,
    LogicalSession,
    ProjectState,
    SessionInfo,
    TeamPreset,
    TeamStartReport,
    WorkflowDef,
    WorkflowRun,
    WorktreeInfo,
  } from "./lib/types";

  const DEFAULT_COLS = 80;
  const DEFAULT_ROWS = 24;

  // Current UI dictionary (reactive: re-evaluates when the locale changes).
  let m = $derived(msg());

  let configDirInput = $state("");
  let loadingConfig = $state(false);
  let bulkOpening = $state(false);
  let persistenceReady = $state(false);
  let stateSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let demoNextId = 1;

  // ---- left dock: tabbed status list + Git (Phase 4.4.1 + Git 移設) ----
  // Open state, active tab, and per-tab width are UI-only, project-independent
  // settings persisted to localStorage (keeps this backend-free, no new command).
  // Git needs more room (file list + diff) than the status list, so each tab
  // remembers its own width and the dock resizes when you switch tabs.
  const SIDEBAR_OPEN_KEY = "ptygrid.statusSidebar.open";
  const SIDEBAR_WIDTH_KEY = "ptygrid.statusSidebar.width";
  const DOCK_TAB_KEY = "ptygrid.dock.tab";
  const DOCK_GIT_WIDTH_KEY = "ptygrid.dock.gitWidth";

  const STATUS_MIN_W = 150;
  const STATUS_MAX_W = 480;
  const GIT_MIN_W = 320;
  const GIT_MAX_W = 760;

  function loadDockOpen(): boolean {
    try {
      return localStorage.getItem(SIDEBAR_OPEN_KEY) !== "0";
    } catch {
      return true;
    }
  }
  function loadStatusWidth(): number {
    try {
      const raw = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY));
      return Number.isFinite(raw) && raw >= STATUS_MIN_W && raw <= STATUS_MAX_W
        ? raw
        : 200;
    } catch {
      return 200;
    }
  }
  function loadGitWidth(): number {
    try {
      const raw = Number(localStorage.getItem(DOCK_GIT_WIDTH_KEY));
      return Number.isFinite(raw) && raw >= GIT_MIN_W && raw <= GIT_MAX_W
        ? raw
        : 380;
    } catch {
      return 380;
    }
  }
  function loadDockTab(): "status" | "git" | "workflow" {
    try {
      const saved = localStorage.getItem(DOCK_TAB_KEY);
      return saved === "git" || saved === "workflow" ? saved : "status";
    } catch {
      return "status";
    }
  }

  let statusSidebarOpen = $state(loadDockOpen());
  let statusSidebarWidth = $state(loadStatusWidth());
  let gitDockWidth = $state(loadGitWidth());
  let dockTab = $state<"status" | "git" | "workflow">(loadDockTab());

  // The dock renders the active tab's remembered width.
  let activeDockWidth = $derived(
    dockTab === "git" ? gitDockWidth : statusSidebarWidth,
  );

  // Persist dock UI state whenever it changes (best-effort; ignore quota errors).
  $effect(() => {
    const open = statusSidebarOpen;
    const sw = statusSidebarWidth;
    const gw = gitDockWidth;
    const tab = dockTab;
    try {
      localStorage.setItem(SIDEBAR_OPEN_KEY, open ? "1" : "0");
      localStorage.setItem(SIDEBAR_WIDTH_KEY, String(Math.round(sw)));
      localStorage.setItem(DOCK_GIT_WIDTH_KEY, String(Math.round(gw)));
      localStorage.setItem(DOCK_TAB_KEY, tab);
    } catch {
      // localStorage unavailable/full: dock still works, just not persisted.
    }
  });

  // Dock resize handle: drag adjusts the active tab's width, clamped per tab.
  function startDockResize(e: PointerEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const isGit = dockTab === "git";
    const startW = isGit ? gitDockWidth : statusSidebarWidth;
    const min = isGit ? GIT_MIN_W : STATUS_MIN_W;
    const max = isGit ? GIT_MAX_W : STATUS_MAX_W;
    const move = (ev: PointerEvent) => {
      const next = Math.min(max, Math.max(min, startW + (ev.clientX - startX)));
      if (isGit) gitDockWidth = next;
      else statusSidebarWidth = next;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  let LAYOUT_MODES = $derived<
    { value: LayoutMode; label: string; hint: string }[]
  >([
    { value: "auto", label: m.layoutAuto, hint: m.layoutAutoHint },
    { value: 1, label: m.layout1, hint: m.layout1Hint },
    { value: 2, label: m.layout2, hint: m.layout2Hint },
    { value: 3, label: m.layout3, hint: m.layout3Hint },
  ]);

  let SHELL_PRESETS = $derived<
    { count: number; label: string; hint: string }[]
  >([
    { count: 1, label: "＋1", hint: m.shellAddHint(1) },
    { count: 4, label: "＋4", hint: m.shellAddHint(4) },
    { count: 9, label: "＋9", hint: m.shellAddHint(9) },
  ]);

  // ---- grid shape ----
  // "auto": 1 / 1x2 / 2x2 / 2x3 / 3x3 heuristic; otherwise fixed column
  // count, wrapping downward (1col = every pane stacked full-width).
  let cols = $derived.by(() => {
    if (ui.layoutMode !== "auto") return ui.layoutMode;
    const n = ui.panes.length;
    return n <= 1 ? 1 : n <= 4 ? 2 : 3;
  });
  let rowChunks = $derived.by(() => {
    const chunks: number[][] = [];
    for (let i = 0; i < ui.panes.length; i += cols) {
      chunks.push(ui.panes.slice(i, i + cols));
    }
    return chunks;
  });

  let paneCount = $derived(ui.panes.length);
  let canAddPane = $derived(paneCount < MAX_PANES);
  let agentDefs = $derived(ui.configInfo?.config.agents ?? []);
  let processDefs = $derived(ui.configInfo?.config.processes ?? []);
  /** Phase 4.3: [preset名, preset] の一覧（無ければ空）。 */
  let teamPresets = $derived(
    Object.entries(ui.configInfo?.config.team_presets ?? {}),
  );
  /** Phase 5.0.0.f: [workflow名, def] の一覧（無ければ空）。 */
  let workflows = $derived(
    Object.entries(ui.configInfo?.config.workflows ?? {}),
  );
  let activeWorktrees = $derived.by(() => {
    const byPath = new Map<string, WorktreeInfo>();
    for (const session of Object.values(ui.sessions)) {
      if (session.worktree) byPath.set(session.worktree.path, session.worktree);
    }
    return [...byPath.values()];
  });
  let totalResources = $derived.by(() => {
    let cpuPercent = 0;
    let memoryBytes = 0;
    let processCount = 0;
    let sessionCount = 0;
    for (const usage of Object.values(ui.resources)) {
      cpuPercent += usage.cpuPercent;
      memoryBytes += usage.memoryBytes;
      processCount += usage.processCount;
      sessionCount += 1;
    }
    return { cpuPercent, memoryBytes, processCount, sessionCount };
  });

  // ---- Queen status badge ----
  // green = running, red = enabled but stopped/errored, gray = disabled
  // (or no Tauri runtime / not yet fetched).
  let queenClass = $derived.by(() => {
    if (!isTauri()) return "queen-off";
    const q = ui.queenStatus;
    if (!q || !q.enabled) return "queen-off";
    return q.running ? "queen-running" : "queen-error";
  });
  let queenLabel = $derived.by(() => {
    if (!isTauri()) return "Queen —";
    const q = ui.queenStatus;
    return q?.port ? `Queen :${q.port}` : "Queen —";
  });
  let queenTooltip = $derived.by(() => {
    if (!isTauri()) return m.queenTooltipNoTauri;
    const q = ui.queenStatus;
    if (!q) return m.queenTooltipUnknown;
    if (!q.enabled) return m.queenTooltipDisabled;
    const lines: string[] = [];
    if (q.url) lines.push(q.url);
    else if (q.port) lines.push(`http://127.0.0.1:${q.port}/mcp`);
    if (q.error) lines.push(m.queenTooltipError(q.error));
    if (!q.running) lines.push(m.queenTooltipStopped);
    // トークンは app-data に永続化され再起動後も有効。初回のみ登録が必要。
    lines.push(m.queenTooltipClickHint);
    return lines.join("\n") || m.queenTooltipFallback;
  });

  /** Token-carrying register URL (falls back to the token-free URL if the
   *  token is unavailable — e.g. server not yet running). */
  function queenRegisterUrl(q: NonNullable<typeof ui.queenStatus>): string {
    const base = q.url ?? `http://127.0.0.1:${q.port}/mcp`;
    return q.token ? `${base}?token=${q.token}` : base;
  }

  async function copyQueenCommand(): Promise<void> {
    const q = ui.queenStatus;
    if (!isTauri() || !q || (!q.url && !q.port)) return;
    const url = queenRegisterUrl(q);
    // -s user: デフォルトの local スコープは「実行したディレクトリ限定」のため、
    // ペインの cwd と登録時の cwd が違うと接続できない。user スコープで全体登録する。
    //
    // 冪等化: `claude mcp add` は既存登録があると "already exists" で弾き、上書き
    // しない。そのため古いトークンの登録が残ると 401 になる。先に remove して
    // から add することで、再クリックしても常に現在のトークンで登録し直せる
    // (remove は未登録でも無害。存在しない場合の非0終了は `|| true` で無視)。
    // URL は必ずクォートする: `?token=...` の `?` を zsh が glob して
    // 「no matches found」になるのを防ぐ。
    const cmd =
      `claude mcp remove queen -s user 2>/dev/null || true; ` +
      `claude mcp add -s user --transport http queen "${url}"`;
    try {
      await navigator.clipboard.writeText(cmd);
      // 認証トークンは永続化され再起動後も有効。再クリックでも安全に再登録できる。
      addNotice(m.queenCmdCopied, cmd);
    } catch (err) {
      ui.errorBanner = m.clipboardCopyFailed(err);
    }
  }

  /** Token-free base URL (`.../mcp`, no `?token=`). codex/grok authenticate via
   *  the injected QUEEN_TOKEN env instead of embedding the token in the URL. */
  function queenBaseUrl(q: NonNullable<typeof ui.queenStatus>): string {
    return q.url ?? `http://127.0.0.1:${q.port}/mcp`;
  }

  /** codex: ~/.codex/config.toml has no first-class `mcp add` for HTTP servers,
   *  so we copy the TOML table. `bearer_token_env_var = "QUEEN_TOKEN"` reads the
   *  injected env at runtime → stale-proof across token regeneration (no 401). */
  async function copyCodexSnippet(): Promise<void> {
    const q = ui.queenStatus;
    if (!isTauri() || !q || (!q.url && !q.port)) return;
    const base = queenBaseUrl(q);
    const snippet =
      `[mcp_servers.queen]\n` +
      `url = "${base}"\n` +
      `bearer_token_env_var = "QUEEN_TOKEN"\n`;
    try {
      await navigator.clipboard.writeText(snippet);
      addNotice(m.codexSnippetCopied, snippet);
    } catch (err) {
      ui.errorBanner = m.clipboardCopyFailed(err);
    }
  }

  /** grok: a Codex-style CLI that reads MCP servers from ~/.grok/config.toml
   *  with the same table shape as codex (verified on a real machine). We copy
   *  the TOML block; `bearer_token_env_var = "QUEEN_TOKEN"` reads the injected
   *  env at runtime → stale-proof across token regeneration (no 401). */
  async function copyGrokSnippet(): Promise<void> {
    const q = ui.queenStatus;
    if (!isTauri() || !q || (!q.url && !q.port)) return;
    const base = queenBaseUrl(q);
    const snippet =
      `[mcp_servers.queen]\n` +
      `url = "${base}"\n` +
      `bearer_token_env_var = "QUEEN_TOKEN"\n`;
    try {
      await navigator.clipboard.writeText(snippet);
      addNotice(m.grokSnippetCopied, snippet);
    } catch (err) {
      ui.errorBanner = m.clipboardCopyFailed(err);
    }
  }

  // ---- 汎用コピー（claude/codex/grok 以外の新しいエージェント CLI 向け） ----
  // 専用ボタンを持たないツールでも登録できるよう、どこにでも貼れる形を用意する。
  // 認証の渡し方は結局2つ: (1) URL クエリ `?token=`（ヘッダ/env 不要で最も汎用。
  // ただし再生成で貼り直しが要る）、(2) env `QUEEN_TOKEN`→Bearer（stale-proof、
  // codex/grok の TOML が使用）。JSON の headers に `${QUEEN_TOKEN}` を書く env
  // 展開方式はツール差で壊れる（claude-code で未展開の不具合報告あり）ため採らず、
  // JSON はトークンを URL に埋める。

  /** 汎用: トークン込み URL。HTTP エンドポイント URL を受け付ける任意の MCP
   *  クライアントに貼れる、最も互換性の高い原始プリミティブ。ヘッダも env も不要。 */
  async function copyUniversalUrl(): Promise<void> {
    const q = ui.queenStatus;
    if (!isTauri() || !q || (!q.url && !q.port)) return;
    const url = queenRegisterUrl(q);
    try {
      await navigator.clipboard.writeText(url);
      addNotice(m.universalUrlCopied, url);
    } catch (err) {
      ui.errorBanner = m.clipboardCopyFailed(err);
    }
  }

  /** 汎用: 標準 mcpServers JSON（type: http）。Cursor / Cline / VS Code /
   *  Gemini CLI など JSON 設定を読むツール向け。env 展開に依存させず URL に
   *  トークンを埋める（stale-proof ではないが最も確実）。 */
  async function copyUniversalJson(): Promise<void> {
    const q = ui.queenStatus;
    if (!isTauri() || !q || (!q.url && !q.port)) return;
    const url = queenRegisterUrl(q);
    const snippet = JSON.stringify(
      { mcpServers: { queen: { type: "http", url } } },
      null,
      2,
    );
    try {
      await navigator.clipboard.writeText(snippet);
      addNotice(m.universalJsonCopied, snippet);
    } catch (err) {
      ui.errorBanner = m.clipboardCopyFailed(err);
    }
  }

  /** 汎用: 手貼り用の生の値。未対応形式の新ツールでも、この素材から手で組める。
   *  エンドポイント URL / トークン / env 変数名 / トークン込み URL。 */
  async function copyRawValues(): Promise<void> {
    const q = ui.queenStatus;
    if (!isTauri() || !q || (!q.url && !q.port)) return;
    const base = queenBaseUrl(q);
    const snippet = [
      `endpoint_url = ${base}`,
      `token        = ${q.token ?? "(not available)"}`,
      `token_env    = QUEEN_TOKEN`,
      `token_url    = ${queenRegisterUrl(q)}`,
    ].join("\n");
    try {
      await navigator.clipboard.writeText(snippet);
      addNotice(m.rawValuesCopied, snippet);
    } catch (err) {
      ui.errorBanner = m.clipboardCopyFailed(err);
    }
  }

  // ---- Teammates badge (Phase 4.0 hooks) ----
  let teammatesPanelOpen = $state(false);
  // Anchor rect for the Teammates panel. The panel is rendered position:fixed
  // (not absolute) because the toolbar uses overflow-x:auto, which would clip
  // an absolutely-positioned dropdown. We compute its viewport coordinates
  // from the badge element when opening.
  let teammatesBadgeEl = $state<HTMLButtonElement | null>(null);
  // Toolbar element ref: the badge scrolls with the toolbar's overflow-x, so
  // the open panel must re-anchor on toolbar scroll, not just window resize.
  let toolbarEl = $state<HTMLDivElement | null>(null);
  let teammatesPanelPos = $state<{ bottom: number; right: number }>({ bottom: 0, right: 0 });
  let registering = $state(false);
  let regenerating = $state(false);

  // ---- Queen registration panel (claude / codex / grok) ----
  // The Queen badge opens a small panel (same fixed-position pattern as the
  // Teammates panel) offering per-CLI register commands, because the three CLIs
  // register MCP servers differently (claude/grok have CLIs, codex is TOML).
  let queenPanelOpen = $state(false);
  let queenBadgeEl = $state<HTMLButtonElement | null>(null);
  let queenPanelPos = $state<{ bottom: number; right: number }>({ bottom: 0, right: 0 });

  // Phase 4.2: any host lead that fell back to observe (host unavailable).
  let hostFallbackActive = $derived.by(
    () => ui.teamsHost?.leads.some((l) => l.fallback) ?? false,
  );

  let teammatesClass = $derived.by(() => {
    if (!isTauri()) return "queen-off";
    if (hostFallbackActive) return "queen-error";
    return ui.teammateHooks?.enabled ? "queen-running" : "queen-off";
  });
  let teammatesTooltip = $derived.by(() => {
    if (!isTauri()) return m.queenTooltipNoTauri;
    if (hostFallbackActive) return m.tmTooltipFallback;
    const t = ui.teammateHooks;
    if (!t) return m.tmTooltipUnknown;
    return t.enabled ? m.tmTooltipEnabled : m.tmTooltipDisabled;
  });

  // The hooks JSON snippet (token embedded) users paste into settings.json.
  let hooksSnippet = $derived.by(() => {
    const t = ui.teammateHooks;
    if (!t) return "";
    const events: [string, string][] = [
      ["SubagentStart", "subagent-start"],
      ["SubagentStop", "subagent-stop"],
      ["TeammateIdle", "teammate-idle"],
      ["TaskCreated", "task-created"],
      ["TaskCompleted", "task-completed"],
    ];
    const hooks: Record<string, unknown> = {};
    for (const [event, suffix] of events) {
      hooks[event] = [
        {
          hooks: [
            {
              type: "http",
              url: `http://127.0.0.1:${t.port}/hooks/v1/${suffix}`,
              headers: { Authorization: `Bearer ${t.token}` },
            },
          ],
        },
      ];
    }
    return JSON.stringify({ hooks }, null, 2);
  });

  async function copyHooksSnippet(): Promise<void> {
    if (!hooksSnippet) return;
    try {
      await navigator.clipboard.writeText(hooksSnippet);
      addNotice(m.hooksSnippetCopied);
    } catch (err) {
      ui.errorBanner = m.clipboardCopyFailed(err);
    }
  }

  async function registerHooks(): Promise<void> {
    if (!isTauri() || registering) return;
    registering = true;
    try {
      const result = await invokeCmd<{ written: boolean; path: string }>(
        "register_teammate_hooks",
        { scope: "user" },
      );
      addNotice(
        result.written ? m.hooksRegistered : m.hooksAlreadyCurrent,
        result.path,
      );
    } catch (err) {
      ui.errorBanner = m.hooksRegisterFailed(err);
    } finally {
      registering = false;
    }
  }

  // Rotate the persisted auth token(s) for leak recovery. The backend updates
  // the live /mcp + hook auth layers in place (no server restart), but the
  // already-registered settings.json / MCP URL now carry the old token, so we
  // refresh both statuses and prompt the user to re-register.
  async function regenerateTokens(which: "hook" | "queen"): Promise<void> {
    if (!isTauri() || regenerating) return;
    regenerating = true;
    try {
      const result = await invokeCmd<{ regenerated: string[] }>(
        "regenerate_auth_tokens",
        { which },
      );
      // Reflect the new token values in the snippet / register URL.
      await refreshTeammateHooks();
      await refreshQueenStatus();
      const labels = result.regenerated
        .map((r) => (r === "hook" ? "hook" : "Queen"))
        .join(" / ");
      addNotice(m.tokensRegenerated(labels));
    } catch (err) {
      ui.errorBanner = m.tokenRegenFailed(err);
    } finally {
      regenerating = false;
    }
  }

  function teammateEventLabel(ev: { kind: string; agentType?: string; agentId?: string; taskName?: string; taskId?: string; sessionId?: string }): string {
    const who =
      ev.agentType ?? ev.agentId ?? ev.taskName ?? ev.taskId ?? ev.sessionId ?? "teammate";
    return `${who} · ${m.teammateKindText[ev.kind] ?? ev.kind}`;
  }

  // ---- projects root suggestions for the working-folder input ----
  // The working-folder input offers a <datalist> of `<projectsRoot>/<name>`
  // entries to prevent typos. `projectsRoot` is a persisted app setting
  // (get/set_projects_root) and its child folders come from list_project_dirs.
  // Best-effort: any failure or an unset root simply yields no suggestions.
  let projectsRoot = $state<string | null>(null);
  let projectDirs = $state<string[]>([]);

  // `<root>/<name>` for each listed folder. The root keeps its verbatim form
  // (a leading `~` is preserved) so a home-relative root stays home-relative.
  let dirSuggestions = $derived.by(() => {
    const root = projectsRoot;
    if (!root) return [];
    const base = root.replace(/\/+$/, "");
    return projectDirs.map((name) => `${base}/${name}`);
  });

  // Ordered SessionInfo for the currently open panes (skips ids with no entry).
  let cdPaneSessions = $derived(
    ui.panes
      .map((id) => ui.sessions[id])
      .filter((s): s is SessionInfo => Boolean(s)),
  );

  // list_sessions is the only source of foreground process names (session-state
  // events omit them); refresh before a bulk cd so the shell-only default is
  // accurate.
  async function refreshForegroundInfo(): Promise<void> {
    if (!isTauri()) return;
    try {
      const list = await invokeCmd<SessionInfo[]>("list_sessions");
      for (const s of list) {
        const existing = ui.sessions[s.id];
        if (existing) existing.foreground = s.foreground;
      }
    } catch {
      // Best-effort: without foreground info, panes are treated as shells.
    }
  }

  // Fetch the persisted projects root and its child folders for the suggestion
  // list. Runs at startup and when the working-folder input gains focus so a
  // freshly-set root (e.g. after a manual load) is reflected. Errors and an
  // unset root both clear the suggestions silently (no UI noise).
  async function loadDirSuggestions(): Promise<void> {
    if (!isTauri()) return;
    try {
      const res = await invokeCmd<{
        root: string;
        dirs: string[];
        truncated: boolean;
      }>("list_project_dirs");
      projectsRoot = res.root;
      projectDirs = res.dirs;
    } catch {
      projectsRoot = null;
      projectDirs = [];
    }
  }

  // Parent of an absolute path, or null when it has no usable parent. A
  // trailing slash is ignored; `/a/b` -> `/a`, `/a` -> `/`, a bare name -> null.
  function parentDir(path: string): string | null {
    const norm = path.replace(/\/+$/, "");
    const idx = norm.lastIndexOf("/");
    if (idx < 0) return null;
    return idx === 0 ? "/" : norm.slice(0, idx);
  }

  // After a successful manual load, remember the loaded working folder's PARENT
  // as the projects root so its siblings become working-folder suggestions.
  // Skipped when the parent is `/` or the home directory itself (too broad to
  // be a useful project container). Best-effort: failures are ignored, no toast.
  async function rememberProjectsRoot(dir: string): Promise<void> {
    if (!isTauri()) return;
    const parent = parentDir(dir);
    if (parent === null || parent === "/") return;
    try {
      const { homeDir } = await import("@tauri-apps/api/path");
      if (parent === (await homeDir()).replace(/\/+$/, "")) return;
    } catch {
      // If home can't be determined, fall through and still try to save.
    }
    try {
      await invokeCmd<{ root: string | null }>("set_projects_root", {
        root: parent,
      });
      await loadDirSuggestions();
    } catch {
      // Best-effort helper: never block the load or surface an error.
    }
  }

  // ---- actions ----

  function addPane(id: number): void {
    if (!ui.panes.includes(id)) ui.panes.push(id);
  }

  function savedLayoutMode(): ProjectState["layoutMode"] {
    return String(ui.layoutMode) as ProjectState["layoutMode"];
  }

  function restoreLayoutMode(mode: ProjectState["layoutMode"]): LayoutMode {
    return mode === "auto" ? "auto" : (Number(mode) as 1 | 2 | 3);
  }

  // Phase 4.1/4.2: teammate panes (observe transcript AND host PTY) are
  // ephemeral and excluded from persistence — the exclusion keys off the
  // teammate meta, not the kind (CONTRACT: ProjectState 除外). They are
  // re-created by the lead on resume.
  let persistablePanes = $derived(
    ui.panes.filter((id) => {
      const s = ui.sessions[id];
      return !s?.teammate && s?.kind !== "transcript";
    }),
  );

  function logicalSession(id: number): LogicalSession {
    const session = ui.sessions[id];
    if (session?.name) {
      return {
        kind: "definition",
        name: session.name,
        ...(session.worktree ? { worktree: session.worktree } : {}),
      };
    }
    return { kind: "shell" };
  }

  function currentProjectState(): ProjectState | null {
    const info = ui.configInfo;
    if (!info) return null;
    const maximizedIndex =
      ui.maximizedId === null
        ? undefined
        : persistablePanes.indexOf(ui.maximizedId);
    return {
      version: 1,
      configDir: info.dir,
      layoutMode: savedLayoutMode(),
      sessions: persistablePanes.map(logicalSession),
      ...(maximizedIndex !== undefined && maximizedIndex >= 0
        ? { maximizedIndex }
        : {}),
    };
  }

  // Debounced snapshots keep pane/layout changes durable without putting
  // commands, terminal output, or expanded environment values on disk.
  $effect(() => {
    const state = currentProjectState();
    if (!persistenceReady || !isTauri() || !state) return;
    const snapshot = JSON.stringify(state);
    if (stateSaveTimer !== null) clearTimeout(stateSaveTimer);
    stateSaveTimer = setTimeout(() => {
      stateSaveTimer = null;
      invokeCmd<void>("save_project_state", {
        state: JSON.parse(snapshot) as ProjectState,
      }).catch((err) => {
        ui.errorBanner = m.saveStateFailed(err);
      });
    }, 250);
    return () => {
      if (stateSaveTimer !== null) {
        clearTimeout(stateSaveTimer);
        stateSaveTimer = null;
      }
    };
  });

  async function newShell(): Promise<void> {
    if (!canAddPane) return;
    if (!isTauri()) {
      const id = demoNextId++;
      ui.sessions[id] = { id, cmd: "local-echo-demo", state: "running" };
      addPane(id);
      return;
    }
    try {
      const id = await invokeCmd<number>("spawn_shell", {
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
      });
      if (!ui.sessions[id]) {
        ui.sessions[id] = { id, cmd: "shell", state: "starting" };
      }
      addPane(id);
    } catch (err) {
      ui.errorBanner = m.spawnShellFailed(err);
    }
  }

  async function openShells(count: number): Promise<void> {
    if (bulkOpening) return;
    const remaining = MAX_PANES - ui.panes.length;
    if (remaining <= 0) {
      ui.errorBanner = m.paneLimitReached(MAX_PANES);
      return;
    }
    let n = count;
    if (n > remaining) {
      ui.errorBanner = m.openShellsCapped(remaining, n);
      n = remaining;
    }
    bulkOpening = true;
    try {
      // Sequential awaits so session ids stay in spawn order; panes are
      // added one by one as each spawn resolves.
      for (let i = 0; i < n; i++) {
        await newShell();
      }
    } finally {
      bulkOpening = false;
    }
  }

  async function spawnAgent(name: string): Promise<void> {
    if (!canAddPane) return;
    if (!isTauri()) return;
    try {
      const id = await invokeCmd<number>("spawn_agent", {
        name,
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
      });
      if (!ui.sessions[id]) {
        ui.sessions[id] = { id, name, cmd: "", state: "starting" };
      }
      addPane(id);
    } catch (err) {
      ui.errorBanner = m.spawnAgentFailed(name, err);
    }
  }

  // Phase 4.3: launch a named team preset through the spawn_team command
  // (same backend function as the Queen tool). Individual member failures
  // come back inside the report, not as a reject.
  let launchingTeam = $state(false);
  let launchingWorkflow = $state(false);

  function teamPresetTitle(name: string, preset: TeamPreset): string {
    const members = preset.members
      .map((member) => (member.standby ? `${member.agent} (standby)` : member.agent))
      .join(", ");
    return m.teamChipTitle(name, members);
  }

  async function spawnTeam(name: string): Promise<void> {
    if (launchingTeam) return;
    if (!isTauri()) return;
    launchingTeam = true;
    try {
      const report = await invokeCmd<TeamStartReport>("spawn_team", {
        preset: name,
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
      });
      // Mirror spawnAgent: track + pane the members started by this call
      // (skipped members already have panes; the session-state fallback
      // covers races just like Queen spawns).
      for (const m of report.members) {
        if (m.status === "started" && m.id !== undefined) {
          if (!ui.sessions[m.id]) {
            ui.sessions[m.id] = {
              id: m.id,
              name: m.agent,
              cmd: "",
              state: "starting",
            };
          }
          addPane(m.id);
        }
      }
      const started = report.members.filter((m) => m.status === "started");
      const skipped = report.members.filter((m) => m.status === "skipped");
      const failed = report.members.filter((m) => m.status === "failed");
      const standby = report.members.filter((m) => m.status === "standby");
      const counts = [
        `${m.lblStarted} ${started.length}`,
        skipped.length > 0 ? `${m.lblSkippedExisting} ${skipped.length}` : null,
        failed.length > 0 ? `${m.lblFailed} ${failed.length}` : null,
        standby.length > 0 ? `${m.lblStandby} ${standby.length}` : null,
      ]
        .filter((s) => s !== null)
        .join(" / ");
      const kickoff = report.kickoffDelivered ? m.kickoffSent : "";
      addNotice(m.teamNoticeTitle(name), `${counts}${kickoff}`);
      if (failed.length > 0) {
        ui.errorBanner = m.teamMembersFailed(
          name,
          failed
            .map((member) => `${member.agent} (${member.error ?? m.unknownError})`)
            .join(", "),
        );
      }
    } catch (err) {
      ui.errorBanner = m.spawnTeamFailed(name, err);
    } finally {
      launchingTeam = false;
    }
  }

  function workflowChipTitle(name: string, def: WorkflowDef): string {
    const n = def.steps.length;
    return `${name} [${def.pattern}] — ${n} step${n === 1 ? "" : "s"}`;
  }

  // Phase 5.0.0.f: launch a declared workflow (spawn_workflow). Same idiom as
  // spawnTeam: invokeCmd, then optimistically track + pane any step that
  // came back already Running so the grid updates without waiting for the
  // session-state event round-trip. The workflow-state event (see
  // stores.svelte.ts) keeps ui.workflowRuns current afterward regardless.
  async function launchWorkflow(name: string): Promise<void> {
    if (launchingWorkflow) return;
    if (!isTauri()) return;
    launchingWorkflow = true;
    try {
      const run = await invokeCmd<WorkflowRun>("spawn_workflow", {
        name,
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
      });
      ui.workflowRuns[run.runId] = run;
      for (const step of run.steps) {
        if (step.state === "running" && step.sessionId !== undefined) {
          if (!ui.sessions[step.sessionId]) {
            ui.sessions[step.sessionId] = {
              id: step.sessionId,
              name: step.agent,
              cmd: "",
              state: "starting",
            };
          }
          addPane(step.sessionId);
        }
      }
      addNotice(`Workflow "${name}" launched`, `run ${run.runId} · ${run.state}`);
    } catch (err) {
      ui.errorBanner = `Failed to launch workflow "${name}": ${err}`;
    } finally {
      launchingWorkflow = false;
    }
  }

  /** Cancel a running workflow run (cancel_workflow). Idempotent on the
   * backend; a terminal run is simply returned unchanged. */
  async function cancelWorkflow(runId: string): Promise<void> {
    if (!isTauri()) return;
    try {
      const run = await invokeCmd<WorkflowRun>("cancel_workflow", { runId });
      ui.workflowRuns[run.runId] = run;
    } catch (err) {
      ui.errorBanner = `Failed to cancel workflow run: ${err}`;
    }
  }

  /** One-shot fetch of every in-memory workflow run (list_workflow_runs),
   * called once from onMount to seed ui.workflowRuns before the first
   * workflow-state event (if any) arrives. */
  async function refreshWorkflowRuns(): Promise<void> {
    if (!isTauri()) return;
    try {
      const runs = await invokeCmd<WorkflowRun[]>("list_workflow_runs");
      for (const run of runs) {
        ui.workflowRuns[run.runId] = run;
      }
    } catch {
      // Best-effort: the workflow-state event keeps the store current
      // regardless of whether this initial fetch succeeds.
    }
  }

  // Spawn every `autostart: true` agent/process of a loaded config, in order,
  // up to the pane cap. Callers must have already cleared the trust gate.
  async function runAutostart(info: ConfigInfo): Promise<void> {
    const autostarts = [...info.config.agents, ...info.config.processes].filter(
      (d) => d.autostart,
    );
    for (const def of autostarts) {
      if (ui.panes.length >= MAX_PANES) break;
      await spawnAgent(def.name);
    }
  }

  // Finding S2 trust gate: a project/launch config from an untrusted folder may
  // define attacker-controlled `cmd`/`worktree.setup` (and leak host env via
  // `${VAR}`), so its autostart commands are NOT run automatically. Instead we
  // surface a one-time "trust this folder?" prompt; global/default configs are
  // trusted implicitly and autostart immediately. Manual agent-chip launches and
  // viewing the config are never blocked — only this automatic loop is gated.
  async function maybeAutostart(info: ConfigInfo): Promise<void> {
    if (info.trusted) {
      await runAutostart(info);
      return;
    }
    const hasAutostart = [...info.config.agents, ...info.config.processes].some(
      (d) => d.autostart,
    );
    // Only bother the user when there is actually something that would autostart.
    if (hasAutostart) ui.trustPrompt = info;
  }

  // User accepted the trust prompt: persist the folder as trusted, then run the
  // autostart loop that was withheld. Subsequent loads of this folder report
  // `trusted: true` and skip the prompt.
  async function onTrustFolder(): Promise<void> {
    const info = ui.trustPrompt;
    if (!info) return;
    if (!isTauri()) {
      ui.trustPrompt = null;
      return;
    }
    try {
      await invokeCmd<{ trusted: boolean }>("trust_working_folder", {
        dir: info.dir,
      });
      ui.trustPrompt = null;
      if (ui.configInfo && ui.configInfo.dir === info.dir) {
        ui.configInfo = { ...ui.configInfo, trusted: true };
      }
      await runAutostart(info);
    } catch (err) {
      ui.errorBanner = m.trustFailed(err);
    }
  }

  // 設定ファイルの由来を短いラベルにする（origin バッジ・成功トースト用）。
  function originLabel(origin: ConfigInfo["origin"]): string {
    switch (origin) {
      case "project":
        return m.originProject;
      case "launch":
        return m.originLaunch;
      case "global":
        return m.originGlobal;
      case "default":
        return m.originDefault;
    }
  }

  // `allowDefault` opts into the built-in default config when no ptygrid.yml is
  // found anywhere (manual load only — the startup auto-load omits it so its
  // `not_found:` fallback is preserved).
  async function loadConfig(
    dir?: string,
    allowDefault = false,
  ): Promise<ConfigInfo> {
    loadingConfig = true;
    try {
      const info = await invokeCmd<ConfigInfo>("load_config", {
        ...(dir ? { dir } : {}),
        ...(allowDefault ? { allowDefault: true } : {}),
      });
      ui.configInfo = info;
      ui.configChangedPath = null;
      // Drop any stale trust prompt; maybeAutostart re-raises it if still needed.
      ui.trustPrompt = null;
      // Queen may have been restarted if the port changed in ptygrid.yml.
      void refreshQueenStatus();
      // teammates.enabled / hook_notifications may have changed too.
      void refreshTeammateHooks();
      // host leads may have started/stopped after a config change.
      void refreshTeamsHostStatus();
      return info;
    } finally {
      loadingConfig = false;
    }
  }

  // Send `cd '<dir>'` + Enter to the default (shell-only) target panes, reusing
  // the bulk-cd logic. `includeNonShell` is false so panes running a CLI are
  // skipped. Returns how many panes received it; 0 targets is not an error.
  async function sendCdToShells(dir: string): Promise<number> {
    if (dir.trim() === "") return 0;
    // list_sessions is the only source of foreground names — refresh so the
    // shell-only filter skips CLI panes accurately (best-effort).
    await refreshForegroundInfo();
    const targets = selectCdTargets(cdPaneSessions, false);
    if (targets.length === 0) return 0;
    const data = `${buildCdCommand(dir)}\r`; // Enter is a carriage return
    let sent = 0;
    for (const target of targets) {
      try {
        if (isTauri()) {
          await invokeCmd<void>("write_pty", { id: target.id, data });
        } else {
          writeToTerm(target.id, data); // demo mode: local echo only
        }
        sent += 1;
      } catch (err) {
        ui.errorBanner = m.cdBroadcastFailed(target.id, err);
      }
    }
    return sent;
  }

  async function onLoadClick(): Promise<void> {
    if (!isTauri()) {
      ui.errorBanner = m.loadConfigNeedsTauri;
      return;
    }
    const rawInput = configDirInput.trim();
    try {
      // Manual load opts into the built-in default so a missing config file
      // still succeeds (and can cd); the startup auto-load does not.
      const info = await loadConfig(rawInput || undefined, true);
      ui.errorBanner = null;
      // 読み込み成功 = 作業フォルダ確定。その親ディレクトリをプロジェクトルート
      // として自動記憶し、作業フォルダ入力欄のサジェスト元にする（best-effort）。
      void rememberProjectsRoot(info.dir);
      // 読み込み成功 = 作業フォルダ確定。cd と同じ動きで、開いているシェルの
      // ペインをその作業フォルダへ移動させる。
      const sent = await sendCdToShells(info.dir);
      const shownDir = rawInput || info.dir;
      const cdPart = sent > 0 ? m.cdSent(sent) : m.cdNoTargets;
      addNotice(
        m.loadedNotice(shownDir, originLabel(info.origin), cdPart),
        info.path,
      );
    } catch (err) {
      ui.errorBanner = String(err);
    }
  }

  async function onReloadConfig(): Promise<void> {
    // Reload the watched config file; running sessions are NOT respawned.
    try {
      await loadConfig();
    } catch (err) {
      ui.errorBanner = String(err);
      ui.configChangedPath = null;
    }
  }

  async function restartSession(id: number): Promise<void> {
    if (!isTauri()) {
      writeToTerm(id, "\r\n\x1b[2m— restarted —\x1b[0m\r\n");
      return;
    }
    try {
      await invokeCmd<void>("restart_session", { id });
      writeToTerm(id, "\r\n\x1b[2m— restarted —\x1b[0m\r\n");
    } catch (err) {
      ui.errorBanner = m.restartFailed(err);
    }
  }

  function closePane(id: number): void {
    // Tombstone the id first so a late session-state event racing the kill
    // can't resurrect this pane as a zombie (BUG-2).
    markPaneClosed(id);
    if (isTauri()) {
      // pane close is authoritative on the frontend, but a real kill failure
      // (orphaned process) must be surfaced, not silently swallowed (BUG-5).
      invokeCmd<void>("kill_pty", { id }).catch((err) => {
        ui.errorBanner = m.killPaneFailed(id, err);
      });
    }
    ui.panes = ui.panes.filter((p) => p !== id);
    if (ui.maximizedId === id) ui.maximizedId = null;
    disposeTermHandle(id);
    delete ui.resources[id];
    delete ui.sessions[id];
    delete ui.transcripts[id];
    clearAgentStatus(id);
  }

  function toggleMaximize(id: number): void {
    ui.maximizedId = ui.maximizedId === id ? null : id;
  }

  // ---- status sidebar derived view (Phase 4.4.1, spec 5.3) ----
  // Display name for a sidebar row: definition name / foreground, with teammate
  // (role + lead) and read-only transcript markers.
  function statusRowName(s: SessionInfo): string {
    if (s.kind === "transcript") {
      const role = s.teammate?.role;
      return `${s.name ?? "sub"}${role ? ` ▸${role}` : ""} 📖RO`;
    }
    if (s.teammate?.mode === "host") {
      const role = s.teammate.role;
      return `${s.name ?? "team"}${role ? ` ▸${role}` : ""} ↳#${s.teammate.leadId}`;
    }
    if (s.name) return s.name;
    // Phase 4.4.3: append the destination detail (`ssh user@host`) so the
    // sidebar shows where each ssh pane is connected.
    if (s.foreground) {
      const detail = ui.foregroundDetail[s.id];
      return detail ? `${s.foreground} ${detail}` : s.foreground;
    }
    return "shell";
  }

  // Pure derived view over ui.panes / ui.sessions / ui.agentStatus: every
  // running pane (incl. observe transcript + host teammate). No backend.
  let statusRows = $derived.by<StatusRow[]>(() =>
    ui.panes
      .map((id) => ui.sessions[id])
      .filter(
        (s): s is SessionInfo => Boolean(s) && s!.state === "running",
      )
      .map((s) => ({
        id: s.id,
        status: (ui.agentStatus[s.id] ?? "unknown") as AgentStatus,
        matchedRule: ui.agentStatusRule[s.id],
        name: statusRowName(s),
        alive: s.state === "running",
      })),
  );

  // Blocked (承認待ち) pane count, shown on the footer toggle so an alert is
  // visible whether the sidebar is open or collapsed.
  let statusBlockedCount = $derived(
    statusRows.filter((r) => r.status === "blocked").length,
  );

  // ---- sidebar row actions ----
  function sidebarFocus(id: number): void {
    focusPane(id);
  }

  function sidebarClose(id: number): void {
    // Closing a host teammate kills a real process → route through the existing
    // inline confirm shown in that pane's header (spec: 破壊的確認は既存フロー踏襲).
    if (ui.sessions[id]?.teammate?.mode === "host") {
      requestCloseHostTeammate(id);
    } else {
      closePane(id);
    }
  }

  // ---- header semantic-status badge (spec 5.1) ----
  let ASTATUS_LABEL = $derived<Record<AgentStatus, string>>({
    blocked: m.astatusBlocked,
    working: m.astatusWorking,
    done: m.astatusDone,
    idle: m.astatusIdle,
    unknown: m.astatusUnknown,
  });

  function astatusTooltip(id: number): string {
    const st = ui.agentStatus[id] ?? "unknown";
    const rule = ui.agentStatusRule[id];
    return rule ? `${ASTATUS_LABEL[st]} · ${rule}` : ASTATUS_LABEL[st];
  }

  // Whether to render the semantic badge: running + a known (non-unknown) state.
  function showAstatus(id: number): boolean {
    const s = ui.sessions[id];
    if (!s || s.state !== "running") return false;
    const st = ui.agentStatus[id];
    return st !== undefined && st !== "unknown";
  }

  // A teammate pane (observe transcript or host PTY) that has finished, i.e.
  // its subagent/teammate reported Stop and the session is `exited`.
  function isFinishedTeammatePane(id: number): boolean {
    const s = ui.sessions[id];
    if (!s) return false;
    const isTeammate = s.kind === "transcript" || s.teammate?.mode === "host";
    return isTeammate && s.state === "exited";
  }

  // Panes eligible for the bulk "close finished" action.
  let finishedTeammatePaneIds = $derived(
    ui.panes.filter((id) => isFinishedTeammatePane(id)),
  );

  // Close every finished teammate pane at once. read-only transcripts stop
  // their tail; finished host teammates are already dead, so no process is
  // killed that wasn't already gone.
  function closeFinishedTeammatePanes(): void {
    for (const id of [...finishedTeammatePaneIds]) closePane(id);
  }

  // Phase 4.1 transcript pane header, e.g. `claude·sub #7 ▸reviewer 📖RO`.
  function transcriptTitle(id: number): string {
    const role = ui.sessions[id]?.teammate?.role;
    return `claude·sub #${id}${role ? ` ▸${role}` : ""} 📖RO`;
  }

  // Phase 4.2 host teammate pane header, e.g. `claude·team #7 ▸reviewer`.
  // The literal `claude·team` mirrors the observe transcript header; the role
  // and lead id (`↳#<leadId>`) are omitted when unavailable.
  function hostTeammateTitle(id: number): string {
    const role = ui.sessions[id]?.teammate?.role;
    return `claude·team #${id}${role ? ` ▸${role}` : ""}`;
  }

  // Phase 4.2: closing a host teammate kills a real process, so it is gated
  // behind a small inline confirm (no confirm() precedent in the app).
  let killConfirmId = $state<number | null>(null);

  function requestCloseHostTeammate(id: number): void {
    killConfirmId = id;
  }
  function confirmCloseHostTeammate(id: number): void {
    killConfirmId = null;
    closePane(id);
  }
  function cancelCloseHostTeammate(): void {
    killConfirmId = null;
  }

  // Phase 4.2: promote a paneless teammate onto the grid (reuses addPane; the
  // existing session-state path already tracks it in ui.sessions). Guarded by
  // the 9-pane cap so we never exceed the grid limit.
  function showTeammatePane(id: number): void {
    if (ui.panes.includes(id)) return;
    if (ui.panes.length >= MAX_PANES) {
      ui.errorBanner = m.paneLimitReachedClose(MAX_PANES);
      return;
    }
    addPane(id);
  }

  async function stopOrphanTeammate(id: number): Promise<void> {
    if (isTauri()) {
      try {
        await invokeCmd<void>("kill_pty", { id });
      } catch (err) {
        ui.errorBanner = m.stopTeammateFailed(id, err);
        return;
      }
    }
    if (ui.panes.includes(id)) closePane(id);
    else delete ui.sessions[id];
    void refreshTeamsHostStatus();
  }

  // Every live host-teammate PTY session (kind:"pty" + teammate.mode:"host").
  let hostTeammateSessions = $derived.by(() =>
    Object.values(ui.sessions).filter(
      (s) => s.teammate?.mode === "host",
    ),
  );

  // Lead ids that teams_host_status currently reports as active host leads.
  let liveHostLeadIds = $derived.by(
    () => new Set((ui.teamsHost?.leads ?? []).map((l) => l.id)),
  );

  // Orphaned host teammates: their lead is no longer an active host lead
  // (lead exited / torn down) yet the teammate PTY is still alive.
  let orphanTeammates = $derived.by(() =>
    hostTeammateSessions.filter((s) => {
      const leadId = s.teammate!.leadId;
      const lead = ui.sessions[leadId];
      const leadAlive =
        liveHostLeadIds.has(leadId) ||
        (lead !== undefined &&
          (lead.state === "running" ||
            lead.state === "starting" ||
            lead.state === "restarting"));
      return !leadAlive;
    }),
  );

  // Role/state for a teammate id shown in the Teammates panel list.
  function teammateRow(id: number): {
    id: number;
    role?: string;
    paneless: boolean;
    state?: string;
  } {
    const s = ui.sessions[id];
    return {
      id,
      role: s?.teammate?.role,
      paneless: !ui.panes.includes(id),
      state: s?.state,
    };
  }

  function positionTeammatesPanel(): void {
    if (!teammatesBadgeEl) return;
    const rect = teammatesBadgeEl.getBoundingClientRect();
    // Right-align the panel to the badge, but keep a small margin from the
    // viewport edge so it never gets clipped off-screen.
    const right = Math.max(6, window.innerWidth - rect.right);
    teammatesPanelPos = { bottom: window.innerHeight - rect.top + 6, right };
  }

  async function openTeammatesPanel(): Promise<void> {
    teammatesPanelOpen = !teammatesPanelOpen;
    if (teammatesPanelOpen) {
      positionTeammatesPanel();
      void refreshTeamsHostStatus();
    }
  }

  function positionQueenPanel(): void {
    if (!queenBadgeEl) return;
    const rect = queenBadgeEl.getBoundingClientRect();
    const right = Math.max(6, window.innerWidth - rect.right);
    queenPanelPos = { bottom: window.innerHeight - rect.top + 6, right };
  }

  function openQueenPanel(): void {
    queenPanelOpen = !queenPanelOpen;
    if (queenPanelOpen) positionQueenPanel();
  }

  // ---- settings (⚙) panel: minimal app preferences (UI language only for
  // now; structured so more per-user, machine-local settings can join later).
  // Project config stays in ptygrid.yml by design (config-as-code).
  let settingsPanelOpen = $state(false);
  let settingsBadgeEl = $state<HTMLButtonElement | null>(null);
  let settingsPanelPos = $state<{ bottom: number; right: number }>({
    bottom: 0,
    right: 0,
  });

  function positionSettingsPanel(): void {
    if (!settingsBadgeEl) return;
    const rect = settingsBadgeEl.getBoundingClientRect();
    const right = Math.max(6, window.innerWidth - rect.right);
    settingsPanelPos = { bottom: window.innerHeight - rect.top + 6, right };
  }

  function openSettingsPanel(): void {
    settingsPanelOpen = !settingsPanelOpen;
    if (settingsPanelOpen) positionSettingsPanel();
  }

  let LOCALE_OPTIONS = $derived<{ value: LocaleSetting; label: string }[]>([
    { value: "auto", label: m.langAuto },
    { value: "en", label: m.langEn },
    { value: "ja", label: m.langJa },
  ]);

  function formatCpu(percent: number): string {
    return `${percent.toFixed(1)}%`;
  }

  function formatMemory(bytes: number): string {
    const mib = bytes / (1024 * 1024);
    if (mib < 1024) return `${mib < 10 ? mib.toFixed(1) : mib.toFixed(0)} MiB`;
    return `${(mib / 1024).toFixed(1)} GiB`;
  }

  async function restoreProjectState(): Promise<boolean> {
    const saved = await invokeCmd<ProjectState | null>("load_project_state");
    if (!saved) return false;

    await loadConfig(saved.configDir);
    ui.layoutMode = restoreLayoutMode(saved.layoutMode);
    const resumedByIndex: Array<number | undefined> = [];
    const errors: string[] = [];
    let skippedForCap = 0;
    for (const [index, session] of saved.sessions.entries()) {
      if (ui.panes.length >= MAX_PANES) {
        // Remaining saved sessions can't be restored past the pane cap; report
        // the count instead of dropping them silently (BUG-9).
        skippedForCap = saved.sessions.length - index;
        break;
      }
      try {
        const id = await invokeCmd<number>("resume_logical_session", {
          session,
          cols: DEFAULT_COLS,
          rows: DEFAULT_ROWS,
        });
        resumedByIndex[index] = id;
        if (!ui.sessions[id]) {
          ui.sessions[id] = {
            id,
            ...(session.kind === "definition" ? { name: session.name } : {}),
            cmd: "",
            state: "starting",
            ...(session.kind === "definition" && session.worktree
              ? { worktree: session.worktree }
              : {}),
          };
        }
        addPane(id);
      } catch (err) {
        const label =
          session.kind === "definition" ? session.name : `shell ${index + 1}`;
        errors.push(`${label}: ${err}`);
      }
    }
    if (saved.maximizedIndex !== undefined) {
      ui.maximizedId = resumedByIndex[saved.maximizedIndex] ?? null;
    }
    if (skippedForCap > 0) {
      errors.push(m.restoreSkippedForCap(MAX_PANES, skippedForCap));
    }
    if (errors.length > 0) {
      ui.errorBanner = m.restoreSomeFailed(errors.join(" / "));
    }
    return true;
  }

  // ---- startup flow per contract ----
  onMount(() => {
    // Keep the fixed-position Teammates panel anchored to its badge when the
    // window is resized while the panel is open.
    const onReposition = () => {
      if (teammatesPanelOpen) positionTeammatesPanel();
      if (queenPanelOpen) positionQueenPanel();
      if (settingsPanelOpen) positionSettingsPanel();
    };
    window.addEventListener("resize", onReposition);
    // Toolbar overflow-x:auto scrolls the anchor badge; keep the fixed panel
    // aligned to it while it is open (BUG-7).
    const toolbar = toolbarEl;
    toolbar?.addEventListener("scroll", onReposition);

    (async () => {
      if (!isTauri()) {
        // Plain-browser fallback: fake two panes with local echo.
        await newShell();
        await newShell();
        return;
      }
      await initGlobalListeners();
      void refreshQueenStatus();
      void refreshTeammateHooks();
      void refreshTeamsHostStatus();
      void refreshWorkflowRuns();
      void loadDirSuggestions();
      let restored = false;
      try {
        restored = await restoreProjectState();
      } catch (err) {
        ui.errorBanner = m.restorePrevFailed(err);
      }
      try {
        if (!restored) {
          const info = await loadConfig();
          await maybeAutostart(info);
        }
      } catch (err) {
        const msg = String(err);
        if (msg.startsWith("not_found")) {
          await newShell(); // Phase 0-like: one adhoc shell
        } else if (!ui.errorBanner) {
          ui.errorBanner = msg;
        }
      } finally {
        persistenceReady = true;
      }
    })();

    return () => {
      window.removeEventListener("resize", onReposition);
      toolbar?.removeEventListener("scroll", onReposition);
    };
  });
</script>

<main>
  {#snippet astatusBadge(id: number)}
    {#if showAstatus(id)}
      <span
        class="astatus astatus-{ui.agentStatus[id]}"
        title={astatusTooltip(id)}
        aria-label={astatusTooltip(id)}
      ></span>
    {/if}
  {/snippet}

  <div class="toolbar" bind:this={toolbarEl}>
    <span class="title">ptygrid</span>

    <div class="tb-group">
      <span class="tb-caption">{m.tbTerminal}</span>
      <span class="tb-controls" role="group" aria-label={m.ariaAddShells}>
        {#each SHELL_PRESETS as preset (preset.count)}
          <button
            class="btn"
            onclick={() => openShells(preset.count)}
            disabled={!canAddPane || bulkOpening}
            title={preset.hint}
          >
            {preset.label}
          </button>
        {/each}
      </span>
    </div>

    <div class="tb-group">
      <span class="tb-caption">{m.tbLayout}</span>
      <span class="tb-controls layout-group" role="group" aria-label={m.ariaColumns}>
        {#each LAYOUT_MODES as mode (mode.value)}
          <button
            class="seg-btn"
            class:seg-active={ui.layoutMode === mode.value}
            onclick={() => (ui.layoutMode = mode.value)}
            title={mode.hint}
          >
            {mode.label}
          </button>
        {/each}
      </span>
    </div>

    <div class="tb-group">
      <span class="tb-caption">{m.tbWorkingFolder}</span>
      <span class="tb-controls">
        <input
          class="dir-input"
          type="text"
          list="dir-suggestions"
          placeholder={m.dirPlaceholder}
          bind:value={configDirInput}
          title={m.dirInputTitle}
          onfocus={loadDirSuggestions}
          onkeydown={(e) => {
            if (e.key === "Enter") onLoadClick();
          }}
        />
        <datalist id="dir-suggestions">
          {#each dirSuggestions as dir (dir)}
            <option value={dir}></option>
          {/each}
        </datalist>
        <button class="btn" onclick={onLoadClick} disabled={loadingConfig}>
          {loadingConfig ? m.btnLoading : m.btnLoad}
        </button>

        {#if ui.configInfo}
          <span
            class="origin-badge"
            title={ui.configInfo.origin === "default"
              ? m.originBadgeTitleDefault(ui.configInfo.path, ui.configInfo.dir)
              : m.originBadgeTitle(ui.configInfo.path, ui.configInfo.dir)}
          >
            {m.configBadge(originLabel(ui.configInfo.origin))}
          </span>
          <span class="project-name" title={ui.configInfo.path}>
            {ui.configInfo.config.project ?? m.unnamedProject}
          </span>
          <span class="chips">
            {#each agentDefs as def (def.name)}
              <span class="chip chip-agent" title={def.cmd}>
                {def.name}
                <button
                  class="chip-run"
                  onclick={() => spawnAgent(def.name)}
                  disabled={!canAddPane}
                  title={m.runAgentTitle(def.name)}
                >
                  ▶
                </button>
              </span>
            {/each}
            {#each processDefs as def (def.name)}
              <span class="chip chip-process" title={def.cmd}>
                {def.name}
                <button
                  class="chip-run"
                  onclick={() => spawnAgent(def.name)}
                  disabled={!canAddPane}
                  title={m.runProcessTitle(def.name)}
                >
                  ▶
                </button>
              </span>
            {/each}
            {#each teamPresets as [name, preset] (name)}
              <span class="chip chip-team" title={teamPresetTitle(name, preset)}>
                👥 {name}
                <button
                  class="chip-run"
                  onclick={() => spawnTeam(name)}
                  disabled={launchingTeam}
                  title={teamPresetTitle(name, preset)}
                >
                  ▶
                </button>
              </span>
            {/each}
            {#each workflows as [name, def] (name)}
              <span class="chip chip-workflow" title={workflowChipTitle(name, def)}>
                🔀 {name}
                <button
                  class="chip-run"
                  onclick={() => launchWorkflow(name)}
                  disabled={launchingWorkflow}
                  title={workflowChipTitle(name, def)}
                >
                  ▶
                </button>
              </span>
            {/each}
          </span>
        {/if}
      </span>
    </div>

  </div>

  {#if ui.errorBanner}
    <div class="banner banner-error" role="alert">
      <span class="banner-text">{ui.errorBanner}</span>
      <button
        class="btn btn-small"
        onclick={() => (ui.errorBanner = null)}
        title={m.btnClose}
      >
        ✕
      </button>
    </div>
  {/if}

  <div class="workspace">
  {#if statusSidebarOpen}
    <aside
      class="dock"
      style="width: {activeDockWidth}px;"
      aria-label={m.dockAria}
    >
      <div class="dock-head">
        <button
          class="dock-collapse"
          onclick={() => (statusSidebarOpen = false)}
          title={m.dockCollapse}
          aria-label={m.dockCollapse}
        >
          ‹
        </button>
        <div class="dock-tabs" role="tablist">
          <button
            class="dock-tab"
            class:active={dockTab === "status"}
            role="tab"
            aria-selected={dockTab === "status"}
            onclick={() => (dockTab = "status")}
          >
            {m.tabStatus}
            {#if statusBlockedCount > 0}
              <span class="dock-tab-badge">🔴 {statusBlockedCount}</span>
            {/if}
          </button>
          <button
            class="dock-tab"
            class:active={dockTab === "git"}
            role="tab"
            aria-selected={dockTab === "git"}
            onclick={() => (dockTab = "git")}
          >
            {m.tabGit}
          </button>
          <button
            class="dock-tab"
            class:active={dockTab === "workflow"}
            role="tab"
            aria-selected={dockTab === "workflow"}
            onclick={() => (dockTab = "workflow")}
          >
            Workflows
          </button>
        </div>
      </div>
      <div class="dock-body">
        {#if dockTab === "status"}
          <StatusSidebar
            rows={statusRows}
            onFocus={sidebarFocus}
            onToggleMax={toggleMaximize}
            onClose={sidebarClose}
          />
        {:else if dockTab === "workflow"}
          <WorkflowPanel />
        {:else}
          {#key ui.configInfo?.path}
            <GitPanel embedded worktrees={activeWorktrees} />
          {/key}
        {/if}
      </div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="dock-resizer"
        role="separator"
        aria-orientation="vertical"
        aria-label={m.dockResizeAria}
        onpointerdown={startDockResize}
      ></div>
    </aside>
  {/if}
  <div class="grid" class:has-max={ui.maximizedId !== null}>
    {#if paneCount === 0}
      <div class="empty-hint">
        {m.emptyHint}
      </div>
    {:else}
      <Splitpanes horizontal theme="mterm-theme">
        {#each rowChunks as row, rowIndex (rowIndex)}
          <Pane minSize={5}>
            <Splitpanes theme="mterm-theme">
              {#each row as id (id)}
                <Pane minSize={5}>
                  {@const session = ui.sessions[id]}
                  {@const resources = ui.resources[id]}
                  <section
                    class="pane"
                    class:is-max={ui.maximizedId === id}
                    class:pane-focused={ui.focusedTeammates[id]}
                  >
                    {#if session?.kind === "transcript"}
                      {@const stopped = session.state === "exited"}
                      <header class="pane-header">
                        <span
                          class="dot tstate-{stopped ? 'stopped' : 'active'}"
                          title={stopped ? "stopped" : "active"}
                        ></span>
                        {@render astatusBadge(id)}
                        <span
                          class="pane-title"
                          title={m.transcriptROTitle}
                        >
                          {transcriptTitle(id)}
                        </span>
                        {#if session.teammate}
                          <span class="lead-ref" title={m.leadRefTitle}>
                            ↳#{session.teammate.leadId}
                          </span>
                        {/if}
                        {#if stopped}
                          <span
                            class="finished-tag"
                            title={m.finishedTagSubTitle}
                          >
                            {m.finishedTag}
                          </span>
                        {/if}
                        <span class="spacer"></span>
                        <button
                          class="pane-btn"
                          title={ui.maximizedId === id ? m.titleUnmaximize : m.titleMaximize}
                          onclick={() => toggleMaximize(id)}
                        >
                          ⤢
                        </button>
                        <button
                          class="pane-btn pane-btn-close"
                          title={m.titleCloseTranscript}
                          onclick={() => closePane(id)}
                        >
                          ✕
                        </button>
                      </header>
                      <div class="pane-body">
                        <TranscriptPane sessionId={id} {stopped} />
                      </div>
                    {:else if session?.teammate?.mode === "host"}
                      <header class="pane-header">
                        <span
                          class="dot state-{session?.state ?? 'starting'}"
                          title={session?.state ?? "starting"}
                        ></span>
                        {@render astatusBadge(id)}
                        <span
                          class="pane-title"
                          title={m.hostPTYTitle}
                        >
                          {hostTeammateTitle(id)}
                        </span>
                        <span class="lead-ref" title={m.leadRefTitle}>
                          ↳#{session.teammate.leadId}
                        </span>
                        {#if session?.state === "exited"}
                          <span
                            class="finished-tag"
                            title={m.finishedTagTeammateTitle}
                          >
                            {m.finishedTag}
                          </span>
                          <span class="exit-code">
                            exit {session.code ?? "?"}
                          </span>
                        {/if}
                        {#if resources && session?.state === "running"}
                          <span
                            class="resource-usage"
                            title={`${resources.processCount} processes · ${resources.memoryBytes.toLocaleString()} bytes`}
                          >
                            CPU {formatCpu(resources.cpuPercent)} · {formatMemory(resources.memoryBytes)}
                          </span>
                        {/if}
                        <span class="spacer"></span>
                        {#if killConfirmId === id}
                          <span class="kill-confirm" role="alertdialog">
                            <span class="kill-confirm-text">{m.killConfirmText}</span>
                            <button
                              class="btn btn-small tm-stop"
                              onclick={() => confirmCloseHostTeammate(id)}
                            >
                              {m.btnStop}
                            </button>
                            <button
                              class="btn btn-small"
                              onclick={cancelCloseHostTeammate}
                            >
                              {m.btnCancel}
                            </button>
                          </span>
                        {:else}
                          <button
                            class="pane-btn"
                            title={m.titleRestart}
                            onclick={() => restartSession(id)}
                          >
                            ⟳
                          </button>
                          <button
                            class="pane-btn"
                            title={ui.maximizedId === id ? m.titleUnmaximize : m.titleMaximize}
                            onclick={() => toggleMaximize(id)}
                          >
                            ⤢
                          </button>
                          <button
                            class="pane-btn pane-btn-close"
                            title={m.titleCloseTeammate}
                            onclick={() => requestCloseHostTeammate(id)}
                          >
                            ✕
                          </button>
                        {/if}
                      </header>
                      <div class="pane-body">
                        <Terminal sessionId={id} title={hostTeammateTitle(id)} />
                      </div>
                    {:else}
                      <header class="pane-header">
                        <span
                          class="dot state-{session?.state ?? 'starting'}"
                          title={session?.state ?? "starting"}
                        ></span>
                        {@render astatusBadge(id)}
                        <span class="pane-title">{paneTitle(id)}</span>
                        {#if session?.worktree}
                          <span
                            class="worktree-badge"
                            title={`worktree: ${session.worktree.path}`}
                          >
                            ⑂ {session.worktree.branch}
                          </span>
                        {/if}
                        {#if session?.state === "exited"}
                          <span class="exit-code">
                            exit {session.code ?? "?"}
                          </span>
                        {/if}
                        {#if resources && session?.state === "running"}
                          <span
                            class="resource-usage"
                            title={`${resources.processCount} processes · ${resources.memoryBytes.toLocaleString()} bytes`}
                          >
                            CPU {formatCpu(resources.cpuPercent)} · {formatMemory(resources.memoryBytes)}
                          </span>
                        {/if}
                        <span class="spacer"></span>
                        <button
                          class="pane-btn"
                          title={m.titleRestart}
                          onclick={() => restartSession(id)}
                        >
                          ⟳
                        </button>
                        <button
                          class="pane-btn"
                          title={ui.maximizedId === id ? m.titleUnmaximize : m.titleMaximize}
                          onclick={() => toggleMaximize(id)}
                        >
                          ⤢
                        </button>
                        <button
                          class="pane-btn pane-btn-close"
                          title={m.titleClosePane}
                          onclick={() => closePane(id)}
                        >
                          ✕
                        </button>
                      </header>
                      <div class="pane-body">
                        <Terminal sessionId={id} title={paneTitle(id)} />
                      </div>
                    {/if}
                  </section>
                </Pane>
              {/each}
            </Splitpanes>
          </Pane>
        {/each}
      </Splitpanes>
    {/if}
  </div>
  </div>

  <!-- Bottom status bar. The sidebar open/close toggle lives at its left end,
       aligned under the sidebar column — the conventional (VS Code-style) home
       for it, always visible whether the sidebar is open or collapsed. -->
  <footer class="statusbar">
    <button
      class="sb-toggle"
      class:active={statusSidebarOpen}
      onclick={() => (statusSidebarOpen = !statusSidebarOpen)}
      title={statusSidebarOpen ? m.sbCollapse : m.sbOpen}
      aria-label={statusSidebarOpen ? m.sbCollapse : m.sbOpen}
      aria-pressed={statusSidebarOpen}
    >
      <span class="sb-toggle-icon" aria-hidden="true">◧</span>
      <span>{m.sbLabel}</span>
      {#if statusBlockedCount > 0}
        <span
          class="sb-blocked"
          title={m.sbBlockedTitle(statusBlockedCount)}
        >
          🔴 {statusBlockedCount}
        </span>
      {/if}
    </button>
    <span class="sb-spacer"></span>
    <div class="teammates-wrap">
      <button
        bind:this={queenBadgeEl}
        class="queen-badge {queenClass}"
        onclick={openQueenPanel}
        title={queenTooltip}
        aria-label={m.queenAria}
      >
        <span class="queen-dot"></span>
        {queenLabel}
      </button>
      {#if queenPanelOpen}
        <div
          class="teammates-panel"
          role="dialog"
          aria-label={m.queenPanelTitle}
          style="bottom: {queenPanelPos.bottom}px; right: {queenPanelPos.right}px;"
        >
          <div class="tm-head">
            <span class="tm-title">{m.queenPanelTitle}</span>
            <button
              class="btn btn-small"
              onclick={() => (queenPanelOpen = false)}
              title={m.btnClose}
            >
              ✕
            </button>
          </div>
          {#if !isTauri()}
            <p class="tm-note">{m.tauriOnly}</p>
          {:else if !ui.queenStatus?.enabled}
            <p class="tm-note">
              {m.queenTooltipDisabled}
            </p>
          {:else}
            <p class="tm-note">
              {m.queenPanelIntro}
            </p>
            <div class="tm-actions">
              <button
                class="btn btn-small"
                onclick={copyQueenCommand}
                title={m.titleClaudeCmd}
              >
                {m.btnClaudeCmd}
              </button>
              <button
                class="btn btn-small"
                onclick={copyCodexSnippet}
                title={m.titleCodexSnippet}
              >
                {m.btnCodexSnippet}
              </button>
              <button
                class="btn btn-small"
                onclick={copyGrokSnippet}
                title={m.titleGrokSnippet}
              >
                {m.btnGrokSnippet}
              </button>
            </div>
            <p class="tm-note">
              {m.queenPanelFootnote}
            </p>
            <p class="tm-note">{m.queenPanelUniversalLabel}</p>
            <div class="tm-actions">
              <button
                class="btn btn-small"
                onclick={copyUniversalUrl}
                title={m.titleUniversalUrl}
              >
                {m.btnUniversalUrl}
              </button>
              <button
                class="btn btn-small"
                onclick={copyUniversalJson}
                title={m.titleUniversalJson}
              >
                {m.btnUniversalJson}
              </button>
              <button
                class="btn btn-small"
                onclick={copyRawValues}
                title={m.titleRawValues}
              >
                {m.btnRawValues}
              </button>
            </div>
            <p class="tm-note">
              {m.queenPanelUniversalNote}
            </p>
          {/if}
        </div>
      {/if}
    </div>
    <div class="teammates-wrap">
      <button
        bind:this={teammatesBadgeEl}
        class="queen-badge {teammatesClass}"
        onclick={openTeammatesPanel}
        title={teammatesTooltip}
        aria-label={m.teammatesAria}
      >
        <span class="queen-dot"></span>
        Teammates{hostFallbackActive ? " ⚠" : ""}
      </button>
      {#if teammatesPanelOpen}
        <div
          class="teammates-panel"
          role="dialog"
          aria-label={m.tmPanelTitle}
          style="bottom: {teammatesPanelPos.bottom}px; right: {teammatesPanelPos.right}px;"
        >
          <div class="tm-head">
            <span class="tm-title">{m.tmPanelTitle}</span>
            <button
              class="btn btn-small"
              onclick={() => (teammatesPanelOpen = false)}
              title={m.btnClose}
            >
              ✕
            </button>
          </div>
          {#if !isTauri()}
            <p class="tm-note">{m.tauriOnly}</p>
          {:else if !ui.teammateHooks}
            <p class="tm-note">{m.tmFetching}</p>
          {:else}
            <p class="tm-note">
              {m.tmStatusLine(
                ui.teammateHooks.enabled,
                ui.teammateHooks.hookNotifications,
                ui.teammateHooks.port,
              )}
            </p>
            <div class="tm-actions">
              <button class="btn btn-small" onclick={copyHooksSnippet}>
                {m.btnCopySnippet}
              </button>
              <button
                class="btn btn-small"
                onclick={registerHooks}
                disabled={registering}
                title={m.titleRegisterSettings}
              >
                {registering ? m.btnRegistering : m.btnRegisterSettings}
              </button>
            </div>
            <p class="tm-note">
              {m.tmTokenNote}
            </p>
            <div class="tm-actions">
              <button
                class="btn btn-small"
                onclick={() => regenerateTokens("hook")}
                disabled={regenerating}
                title={m.titleRegenHook}
              >
                {m.btnRegenHook}
              </button>
              <button
                class="btn btn-small"
                onclick={() => regenerateTokens("queen")}
                disabled={regenerating}
                title={m.titleRegenQueen}
              >
                {m.btnRegenQueen}
              </button>
            </div>
            {#if finishedTeammatePaneIds.length > 0}
              <div class="tm-actions">
                <button
                  class="btn btn-small"
                  onclick={closeFinishedTeammatePanes}
                  title={m.titleCloseFinished}
                >
                  {m.btnCloseFinished(finishedTeammatePaneIds.length)}
                </button>
              </div>
            {/if}
            <div class="tm-events">
              <div class="tm-subhead">{m.tmHostHead}</div>
              {#if (ui.teamsHost?.leads.length ?? 0) === 0 && orphanTeammates.length === 0}
                <div class="tm-empty">
                  {m.tmNoHostLeads}
                </div>
              {:else}
                {#each ui.teamsHost?.leads ?? [] as lead (lead.id)}
                  <div class="tm-lead">
                    <div class="tm-lead-head">
                      <span class="tm-lead-id">lead #{lead.id}</span>
                      <span
                        class="tm-lead-badge {lead.fallback
                          ? 'tm-badge-warn'
                          : 'tm-badge-ok'}"
                      >
                        {lead.fallback ? m.tmLeadBadgeFallback : m.tmLeadBadgeHost}
                      </span>
                    </div>
                    {#if lead.teammates.length === 0}
                      <div class="tm-empty">{m.tmNoTeammates}</div>
                    {:else}
                      {#each lead.teammates.map(teammateRow) as tm (tm.id)}
                        <div class="tm-teammate">
                          <span class="tm-teammate-label">
                            #{tm.id}{tm.role ? ` ▸${tm.role}` : ""}
                            {tm.paneless ? m.tmPaneless : ""}
                          </span>
                          {#if tm.paneless}
                            <button
                              class="btn btn-small"
                              onclick={() => showTeammatePane(tm.id)}
                              disabled={!canAddPane}
                              title={m.titleShowOnGrid}
                            >
                              {m.btnShowOnGrid}
                            </button>
                          {/if}
                        </div>
                      {/each}
                    {/if}
                  </div>
                {/each}
                {#if orphanTeammates.length > 0}
                  <div class="tm-lead">
                    <div class="tm-lead-head">
                      <span class="tm-lead-id">{m.tmOrphanHead}</span>
                    </div>
                    {#each orphanTeammates as s (s.id)}
                      <div class="tm-teammate">
                        <span class="tm-teammate-label">
                          #{s.id}{s.teammate?.role ? ` ▸${s.teammate.role}` : ""}
                          ↳#{s.teammate?.leadId}
                        </span>
                        <button
                          class="btn btn-small tm-stop"
                          onclick={() => stopOrphanTeammate(s.id)}
                          title={m.titleStopOrphan}
                        >
                          {m.btnStop}
                        </button>
                      </div>
                    {/each}
                  </div>
                {/if}
              {/if}
            </div>
            <div class="tm-events">
              <div class="tm-subhead">{m.tmEventsHead}</div>
              {#if ui.teammateEvents.length === 0}
                <div class="tm-empty">{m.tmNoEvents}</div>
              {:else}
                {#each ui.teammateEvents as ev (ev.key)}
                  <div class="tm-event">{teammateEventLabel(ev)}</div>
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>
    {#if totalResources.sessionCount > 0}
      <span
        class="total-resources"
        title={`${totalResources.sessionCount} sessions · ${totalResources.processCount} processes · ${totalResources.memoryBytes.toLocaleString()} bytes`}
      >
        Σ CPU {formatCpu(totalResources.cpuPercent)} · {formatMemory(totalResources.memoryBytes)}
      </span>
    {/if}
    <span class="pane-count">{m.paneCount(paneCount, MAX_PANES)}</span>
    <div class="teammates-wrap">
      <button
        bind:this={settingsBadgeEl}
        class="settings-badge"
        onclick={openSettingsPanel}
        title={m.settingsTitle}
        aria-label={m.settingsAria}
      >
        ⚙
      </button>
      {#if settingsPanelOpen}
        <div
          class="teammates-panel"
          role="dialog"
          aria-label={m.settingsTitle}
          style="bottom: {settingsPanelPos.bottom}px; right: {settingsPanelPos.right}px;"
        >
          <div class="tm-head">
            <span class="tm-title">{m.settingsTitle}</span>
            <button
              class="btn btn-small"
              onclick={() => (settingsPanelOpen = false)}
              title={m.btnClose}
            >
              ✕
            </button>
          </div>
          <!-- Each setting is a labeled row; add future app-level (per-user,
               machine-local) settings as additional .settings-row entries.
               Project-level config intentionally stays in ptygrid.yml. -->
          <div class="settings-row">
            <span class="settings-label">{m.settingsLanguage}</span>
            <span class="tb-controls layout-group" role="group" aria-label={m.settingsLanguage}>
              {#each LOCALE_OPTIONS as opt (opt.value)}
                <button
                  class="seg-btn"
                  class:seg-active={i18n.setting === opt.value}
                  onclick={() => setLocaleSetting(opt.value)}
                >
                  {opt.label}
                </button>
              {/each}
            </span>
          </div>
        </div>
      {/if}
    </div>
  </footer>

  {#if ui.notices.length > 0}
    <div class="notices" aria-live="polite">
      {#each ui.notices as notice (notice.key)}
        <div class="notice" role="status">
          <div class="notice-body">
            <div class="notice-title">{notice.title}</div>
            {#if notice.message}
              <div class="notice-message">{notice.message}</div>
            {/if}
          </div>
          <button
            class="btn btn-small"
            onclick={() => dismissNotice(notice.key)}
            title={m.btnClose}
          >
            ✕
          </button>
        </div>
      {/each}
    </div>
  {/if}

  {#if ui.configChangedPath}
    <div class="toast" role="status">
      <span class="toast-text">{m.configChanged}</span>
      <button class="btn btn-small" onclick={onReloadConfig}>{m.btnReload}</button>
      <button
        class="btn btn-small"
        onclick={() => (ui.configChangedPath = null)}
        title={m.btnClose}
      >
        ✕
      </button>
    </div>
  {/if}

  {#if ui.trustPrompt}
    <div class="toast trust-toast" role="alertdialog" aria-label={m.trustAria}>
      <span class="toast-text">
        {m.trustText(ui.trustPrompt.dir)}
      </span>
      <button
        class="btn btn-small"
        onclick={onTrustFolder}
        title={m.titleTrust}
      >
        {m.btnTrust}
      </button>
      <button
        class="btn btn-small"
        onclick={() => (ui.trustPrompt = null)}
        title={m.titleLater}
      >
        {m.btnLater}
      </button>
    </div>
  {/if}
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    background: #1e1e1e;
    color: #cccccc;
  }

  /* ---- toolbar ---- */

  .toolbar {
    flex: 0 0 auto;
    display: flex;
    align-items: stretch;
    gap: 14px;
    padding: 4px 10px 5px;
    background: #252526;
    border-bottom: 1px solid #333;
    font-size: 12px;
    -webkit-user-select: none;
    user-select: none;
    overflow-x: auto;
    white-space: nowrap;
  }

  .title {
    font-weight: 600;
    letter-spacing: 0.02em;
    align-self: center;
  }

  /* fieldset-like group: 10px muted caption above the controls */
  .tb-group {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 2px;
  }

  .tb-caption {
    font-size: 10px;
    line-height: 1;
    color: #6f6f6f;
    letter-spacing: 0.06em;
    padding-left: 1px;
  }

  .tb-controls {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .spacer {
    flex: 1 1 auto;
    min-width: 4px;
  }

  .btn {
    background: #333;
    color: #ddd;
    border: 1px solid #444;
    border-radius: 4px;
    padding: 3px 8px;
    font-size: 12px;
    cursor: pointer;
  }

  .btn:hover:not(:disabled) {
    background: #3d3d3d;
  }

  .btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .btn-small {
    padding: 1px 6px;
    font-size: 11px;
  }

  .layout-group {
    display: inline-flex;
    align-items: center;
    gap: 0; /* segmented control: buttons sit flush */
    border: 1px solid #444;
    border-radius: 4px;
    overflow: hidden;
  }

  .seg-btn {
    background: #2a2a2a;
    color: #aaa;
    border: none;
    border-right: 1px solid #444;
    padding: 3px 8px;
    font-size: 11px;
    cursor: pointer;
  }

  .seg-btn:last-child {
    border-right: none;
  }

  .seg-btn:hover:not(.seg-active) {
    background: #353535;
    color: #ddd;
  }

  .seg-active {
    background: #3b5b7a;
    color: #fff;
  }

  .dir-input {
    background: #1b1b1b;
    color: #ddd;
    border: 1px solid #444;
    border-radius: 4px;
    padding: 3px 6px;
    font-size: 12px;
    width: 220px;
  }

  .dir-input::placeholder {
    color: #777;
  }

  .origin-badge {
    align-self: center;
    color: #b5b5b5;
    background: #2a2a2a;
    border: 1px solid #444;
    border-radius: 10px;
    padding: 2px 8px;
    font-size: 10px;
    white-space: nowrap;
  }

  .project-name {
    color: #9cdcfe;
    font-weight: 600;
  }

  .chips {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border-radius: 10px;
    padding: 2px 4px 2px 8px;
    font-size: 11px;
    border: 1px solid #444;
    background: #2d2d2d;
  }

  .chip-agent {
    border-color: #3b5b7a;
  }

  .chip-process {
    border-color: #4d6b3c;
  }
  .chip-team {
    border-color: #7a5b8f;
  }

  .chip-workflow {
    border-color: #3c6b6b;
  }

  .chip-run {
    background: transparent;
    border: none;
    color: #7cc27e;
    cursor: pointer;
    font-size: 10px;
    padding: 0 3px;
  }

  .chip-run:hover:not(:disabled) {
    color: #a5e0a7;
  }

  .chip-run:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .pane-count {
    color: #888;
    font-variant-numeric: tabular-nums;
    align-self: center;
  }

  .total-resources {
    align-self: center;
    color: #b8c2cc;
    font-family: Menlo, monospace;
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .queen-badge {
    align-self: center;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: #2a2a2a;
    color: #bbb;
    border: 1px solid #444;
    border-radius: 10px;
    padding: 3px 9px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    margin-right: 8px;
  }

  .queen-badge:hover {
    background: #353535;
    color: #ddd;
  }

  .queen-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #777;
  }

  .queen-running .queen-dot {
    background: #4caf50;
  }

  .queen-error .queen-dot {
    background: #e06c75;
  }

  .queen-off .queen-dot {
    background: #777;
  }

  .queen-off {
    opacity: 0.7;
  }

  /* ---- teammates panel (popover under the badge) ---- */

  .teammates-wrap {
    position: relative;
    align-self: center;
    margin-right: 8px;
  }

  .teammates-wrap .queen-badge {
    margin-right: 0;
  }

  /* ⚙ app-settings button (footer). Same chrome family as the queen badge. */
  .settings-badge {
    align-self: center;
    background: transparent;
    color: #bbb;
    border: none;
    border-radius: 4px;
    padding: 1px 6px;
    font-size: 13px;
    line-height: 1;
    cursor: pointer;
  }

  .settings-badge:hover {
    background: #3a3a3a;
    color: #eee;
  }

  .settings-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 0;
  }

  .settings-label {
    flex: 0 0 auto;
    color: #bbb;
    font-size: 11px;
  }

  .teammates-panel {
    position: fixed;
    z-index: 120;
    width: 340px;
    max-width: calc(100vw - 12px);
    max-height: calc(100vh - 60px);
    overflow-y: auto;
    background: #2d2d30;
    border: 1px solid #4a4a4a;
    border-radius: 6px;
    padding: 10px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.55);
    white-space: normal;
  }

  .tm-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }

  .tm-title {
    font-weight: 700;
    color: #e8e8e8;
  }

  .tm-note {
    margin: 0 0 8px;
    color: #b5b5b5;
    font-size: 11px;
  }

  .tm-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 8px;
  }

  .tm-events {
    border-top: 1px solid #3d3d3d;
    padding-top: 6px;
  }

  .tm-subhead {
    color: #8a8a8a;
    font-size: 10px;
    letter-spacing: 0.06em;
    margin-bottom: 4px;
  }

  .tm-empty {
    color: #6f6f6f;
    font-size: 11px;
  }

  .tm-event {
    color: #cfcfcf;
    font-family: Menlo, monospace;
    font-size: 11px;
    padding: 2px 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* host mode: per-lead status + teammate list */
  .tm-lead {
    margin-bottom: 6px;
    padding-bottom: 4px;
    border-bottom: 1px solid #333;
  }

  .tm-lead:last-child {
    border-bottom: none;
  }

  .tm-lead-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 2px;
  }

  .tm-lead-id {
    color: #d0d0d0;
    font-family: Menlo, monospace;
    font-size: 11px;
    font-weight: 600;
  }

  .tm-lead-badge {
    font-size: 10px;
    border-radius: 8px;
    padding: 0 6px;
  }

  .tm-badge-ok {
    color: #7cc27e;
    border: 1px solid #3f6b3f;
    background: #23301f;
  }

  .tm-badge-warn {
    color: #f1b0b0;
    border: 1px solid #6b2b2b;
    background: #301f1f;
  }

  .tm-teammate {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 1px 0 1px 10px;
  }

  .tm-teammate-label {
    color: #cfcfcf;
    font-family: Menlo, monospace;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tm-stop {
    color: #f1b0b0;
    border-color: #6b2b2b;
  }

  .tm-stop:hover:not(:disabled) {
    background: #4b1e1e;
  }

  /* ---- banners / toast ---- */

  .banner {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    font-size: 12px;
  }

  .banner-error {
    background: #4b1e1e;
    color: #f1b0b0;
    border-bottom: 1px solid #6b2b2b;
  }

  .banner-text {
    flex: 1 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* stacked auto-dismiss toasts (queen-notify etc.), top-right over grid */
  .notices {
    position: fixed;
    top: 52px;
    right: 14px;
    z-index: 110;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 360px;
  }

  .notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    background: #2d2d30;
    border: 1px solid #4a4a4a;
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 12px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }

  .notice-body {
    flex: 1 1 auto;
    min-width: 0;
  }

  .notice-title {
    font-weight: 700;
    color: #e8e8e8;
  }

  .notice-message {
    margin-top: 2px;
    color: #b5b5b5;
    white-space: pre-wrap;
    word-break: break-all;
  }

  .toast {
    position: fixed;
    right: 14px;
    bottom: 14px;
    z-index: 100;
    display: flex;
    align-items: center;
    gap: 8px;
    background: #2d2d30;
    border: 1px solid #4a4a4a;
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 12px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  }

  .toast-text {
    color: #e5c07b;
  }

  /* Trust prompt: warn-tinted variant, wider to fit the folder path + question. */
  .trust-toast {
    border-color: #b5893a;
    max-width: 520px;
  }
  .trust-toast .toast-text {
    color: #e0a94f;
  }

  /* ---- workspace (status sidebar + grid) ---- */

  .workspace {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    align-items: stretch;
    overflow: hidden;
  }

  /* ---- grid ---- */

  .grid {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    position: relative;
    overflow: hidden;
  }

  /* ---- left dock (tabbed: status list + Git) ---- */

  .dock {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: #232324;
    border-right: 1px solid #333;
    -webkit-user-select: none;
    user-select: none;
  }

  .dock-head {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 4px;
    background: #252526;
    border-bottom: 1px solid #333;
  }

  .dock-collapse {
    flex: 0 0 auto;
    background: transparent;
    border: 1px solid #444;
    border-radius: 4px;
    color: #bbb;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
    padding: 2px 7px;
  }
  .dock-collapse:hover {
    background: #353535;
    color: #eee;
  }

  .dock-tabs {
    display: flex;
    gap: 3px;
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
  }

  .dock-tab {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 10px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: #9a9a9a;
    cursor: pointer;
    font-size: 11px;
    line-height: 1.6;
    white-space: nowrap;
  }
  .dock-tab:hover {
    background: #2d2d2e;
    color: #ddd;
  }
  .dock-tab.active {
    background: #2f3d51;
    border-color: #3d5573;
    color: #dbe7f5;
  }
  .dock-tab-badge {
    color: #f0b8b8;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .dock-body {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .dock-resizer {
    position: absolute;
    top: 0;
    right: -2px;
    width: 5px;
    height: 100%;
    cursor: ew-resize;
    z-index: 5;
  }
  .dock-resizer:hover {
    background: #3d5573;
  }

  /* ---- bottom status bar (footer) ---- */

  .statusbar {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    padding: 0 8px;
    background: #252526;
    border-top: 1px solid #333;
    font-size: 11px;
    -webkit-user-select: none;
    user-select: none;
  }

  .sb-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 18px;
    padding: 0 8px;
    background: transparent;
    border: 1px solid #444;
    border-radius: 4px;
    color: #bbb;
    cursor: pointer;
    font-size: 11px;
    line-height: 1;
  }

  .sb-toggle:hover {
    background: #353535;
    color: #eee;
  }

  .sb-toggle.active {
    background: #2f3d51;
    border-color: #3d5573;
    color: #dbe7f5;
  }

  .sb-toggle-icon {
    font-size: 12px;
    line-height: 1;
  }

  .sb-blocked {
    color: #f0b8b8;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .sb-spacer {
    flex: 1 1 auto;
  }

  /* ---- semantic status badge (spec 5.1) + toolbar aggregate ---- */

  /* Small colored dot rendered right after the liveness dot; distinct from the
     existing .dot.state-* (that one shows PTY liveness). */
  .astatus {
    flex: 0 0 auto;
    width: 9px;
    height: 9px;
    border-radius: 2px; /* rounded square to differentiate from the round liveness dot */
    background: #666;
  }

  .astatus-blocked {
    background: #e0574a;
  }
  .astatus-working {
    background: #e5c07b;
  }
  .astatus-done {
    background: #4a9be0;
  }
  .astatus-idle {
    background: #4caf50;
  }

  .empty-hint {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #666;
    font-size: 13px;
  }

  /* splitpanes custom dark theme */
  .grid :global(.splitpanes.mterm-theme .splitpanes__pane) {
    background: #1e1e1e;
    overflow: hidden;
  }

  .grid :global(.splitpanes.mterm-theme > .splitpanes__splitter) {
    background: #2b2b2b;
    border: none;
  }

  .grid :global(.splitpanes.mterm-theme > .splitpanes__splitter:hover) {
    background: #4a6b9a;
  }

  .grid :global(.splitpanes--vertical.mterm-theme > .splitpanes__splitter) {
    width: 5px;
    cursor: col-resize;
  }

  .grid :global(.splitpanes--horizontal.mterm-theme > .splitpanes__splitter) {
    height: 5px;
    cursor: row-resize;
  }

  /* maximize: the maximized pane fills the grid; other panes get zero size
     but stay mounted (terminals + scrollback preserved, not destroyed). */
  .grid.has-max :global(.splitpanes__splitter) {
    display: none !important;
  }

  .grid.has-max :global(.splitpanes--horizontal > .splitpanes__pane) {
    height: 0 !important;
  }

  .grid.has-max
    :global(.splitpanes--horizontal > .splitpanes__pane:has(.is-max)) {
    height: 100% !important;
  }

  .grid.has-max :global(.splitpanes--vertical > .splitpanes__pane) {
    width: 0 !important;
  }

  .grid.has-max
    :global(.splitpanes--vertical > .splitpanes__pane:has(.is-max)) {
    width: 100% !important;
  }

  /* ---- pane ---- */

  .pane {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: #1e1e1e;
  }

  .pane-header {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px;
    background: #252526;
    border-bottom: 1px solid #333;
    font-size: 11px;
    -webkit-user-select: none;
    user-select: none;
  }

  .pane-title {
    font-weight: 600;
    color: #d0d0d0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .worktree-badge {
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #9cdcfe;
    font-family: Menlo, monospace;
    font-size: 10px;
  }

  .resource-usage {
    flex: 0 0 auto;
    color: #a8b3bd;
    font-family: Menlo, monospace;
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .exit-code {
    color: #e06c75;
    font-variant-numeric: tabular-nums;
  }

  .pane-btn {
    background: transparent;
    border: none;
    color: #999;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
    padding: 2px 4px;
    border-radius: 3px;
  }

  .pane-btn:hover {
    background: #3a3a3a;
    color: #eee;
  }

  .pane-btn-close:hover {
    background: #6b2b2b;
    color: #fff;
  }

  .dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #888;
  }

  .dot.state-starting {
    background: #e5c07b;
  }

  .dot.state-running {
    background: #4caf50;
  }

  .dot.state-restarting {
    background: #ff9800;
  }

  .dot.state-exited {
    background: #e06c75;
  }

  /* transcript (observe) logical states */
  .dot.tstate-active {
    background: #4caf50;
  }

  .dot.tstate-stopped {
    background: #888;
  }

  .lead-ref {
    flex: 0 0 auto;
    color: #8a9aa8;
    font-family: Menlo, monospace;
    font-size: 10px;
  }

  /* Explicit "finished" tag so a stopped teammate/transcript pane is legible
     without relying on the small state dot's color alone. */
  .finished-tag {
    flex: 0 0 auto;
    padding: 0 5px;
    border-radius: 3px;
    background: #5a2d2d;
    color: #f0b8b8;
    font-size: 10px;
    font-weight: 700;
  }

  /* teammate-focus pulse: a short accent ring around the focused pane */
  .pane.pane-focused {
    box-shadow: inset 0 0 0 2px #4a9be0;
    transition: box-shadow 0.2s ease;
  }

  /* inline confirm for the destructive host-teammate close */
  .kill-confirm {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .kill-confirm-text {
    color: #e5c07b;
    font-size: 10px;
    white-space: nowrap;
  }

  .pane-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }
</style>
