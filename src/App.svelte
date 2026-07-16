<script lang="ts">
  import { onMount } from "svelte";
  import { Splitpanes, Pane } from "svelte-splitpanes";
  import Terminal from "./lib/Terminal.svelte";
  import TranscriptPane from "./lib/TranscriptPane.svelte";
  import GitPanel from "./lib/GitPanel.svelte";
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
    type LayoutMode,
  } from "./lib/stores.svelte";
  import { disposeTermHandle, writeToTerm } from "./lib/terminals";
  import { invokeCmd, isTauri } from "./lib/tauri";
  import { buildCdCommand, selectCdTargets } from "./lib/broadcast";
  import type {
    ConfigInfo,
    HostLeadStatus,
    LogicalSession,
    ProjectState,
    SessionInfo,
    WorktreeInfo,
  } from "./lib/types";

  const DEFAULT_COLS = 80;
  const DEFAULT_ROWS = 24;

  let configDirInput = $state("");
  let loadingConfig = $state(false);
  let bulkOpening = $state(false);
  let gitPanelOpen = $state(false);
  let persistenceReady = $state(false);
  let stateSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let demoNextId = 1;

  const LAYOUT_MODES: { value: LayoutMode; label: string; hint: string }[] = [
    { value: "auto", label: "自動", hint: "枚数に応じて格子配置" },
    { value: 1, label: "1列", hint: "縦に積む" },
    { value: 2, label: "2列", hint: "2列で折り返し" },
    { value: 3, label: "3列", hint: "3列で折り返し" },
  ];

  const SHELL_PRESETS: { count: number; label: string; hint: string }[] = [
    { count: 1, label: "＋1", hint: "シェルを1面追加" },
    { count: 4, label: "＋4", hint: "シェルを4面まとめて追加" },
    { count: 9, label: "＋9", hint: "シェルを9面まとめて追加" },
  ];

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
    if (!isTauri()) return "Tauri 実行環境なし（デモモード）";
    const q = ui.queenStatus;
    if (!q) return "Queen MCP サーバー（状態未取得）";
    if (!q.enabled) return "Queen は無効です（ptygrid.yml の queen.enabled: false）";
    const lines: string[] = [];
    if (q.url) lines.push(q.url);
    else if (q.port) lines.push(`http://127.0.0.1:${q.port}/mcp`);
    if (q.error) lines.push(`エラー: ${q.error}`);
    if (!q.running) lines.push("停止中");
    return lines.join("\n") || "Queen MCP サーバー";
  });

  async function copyQueenCommand(): Promise<void> {
    const q = ui.queenStatus;
    if (!isTauri() || !q || (!q.url && !q.port)) return;
    const url = q.url ?? `http://127.0.0.1:${q.port}/mcp`;
    // -s user: デフォルトの local スコープは「実行したディレクトリ限定」のため、
    // ペインの cwd と登録時の cwd が違うと接続できない。user スコープで全体登録する。
    const cmd = `claude mcp add -s user --transport http queen ${url}`;
    try {
      await navigator.clipboard.writeText(cmd);
      addNotice("登録コマンドをコピーしました", cmd);
    } catch (err) {
      ui.errorBanner = `クリップボードへのコピーに失敗しました: ${err}`;
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
  let teammatesPanelPos = $state<{ top: number; right: number }>({ top: 0, right: 0 });
  let registering = $state(false);

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
    if (!isTauri()) return "Tauri 実行環境なし（デモモード）";
    if (hostFallbackActive)
      return "host: フォールバック中（ネイティブペイン化に失敗し observe へ降格）";
    const t = ui.teammateHooks;
    if (!t) return "Teammate hooks（状態未取得）";
    return t.enabled
      ? "Teammate hooks 有効（クリックで設定）"
      : "Teammate hooks 無効（ptygrid.yml の teammates.enabled: true で有効化）";
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
      addNotice("hooks 設定スニペットをコピーしました");
    } catch (err) {
      ui.errorBanner = `クリップボードへのコピーに失敗しました: ${err}`;
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
        result.written
          ? "settings.json に登録しました"
          : "settings.json は既に最新です",
        result.path,
      );
    } catch (err) {
      ui.errorBanner = `hooks の登録に失敗しました (register_teammate_hooks): ${err}`;
    } finally {
      registering = false;
    }
  }

  const TEAMMATE_KIND_TEXT: Record<string, string> = {
    "subagent-start": "起動",
    "subagent-stop": "停止",
    "teammate-idle": "アイドル",
    "task-created": "タスク作成",
    "task-completed": "タスク完了",
  };
  function teammateEventLabel(ev: { kind: string; agentType?: string; agentId?: string; taskName?: string; taskId?: string; sessionId?: string }): string {
    const who =
      ev.agentType ?? ev.agentId ?? ev.taskName ?? ev.taskId ?? ev.sessionId ?? "teammate";
    return `${who} · ${TEAMMATE_KIND_TEXT[ev.kind] ?? ev.kind}`;
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
        ui.errorBanner = `プロジェクト状態の保存に失敗しました: ${err}`;
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
      ui.errorBanner = `シェルの起動に失敗しました (spawn_shell): ${err}`;
    }
  }

  async function openShells(count: number): Promise<void> {
    if (bulkOpening) return;
    const remaining = MAX_PANES - ui.panes.length;
    if (remaining <= 0) {
      ui.errorBanner = `ペイン数が上限（${MAX_PANES}）に達しています。`;
      return;
    }
    let n = count;
    if (n > remaining) {
      ui.errorBanner = `空きが ${remaining} 面のため、${n} 面ではなく ${remaining} 面だけ開きます。`;
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
      ui.errorBanner = `「${name}」の起動に失敗しました (spawn_agent): ${err}`;
    }
  }

  // 設定ファイルの由来を短い日本語ラベルにする（origin バッジ・成功トースト用）。
  function originLabel(origin: ConfigInfo["origin"]): string {
    switch (origin) {
      case "project":
        return "プロジェクト内";
      case "launch":
        return "起動フォルダ";
      case "global":
        return "~/.ptygrid";
      case "default":
        return "既定";
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
        ui.errorBanner = `cd の一括送信に失敗しました (write_pty #${target.id}): ${err}`;
      }
    }
    return sent;
  }

  async function onLoadClick(): Promise<void> {
    if (!isTauri()) {
      ui.errorBanner =
        "設定の読み込み (load_config) には Tauri 実行環境が必要です。";
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
      const cdPart = sent > 0 ? `${sent}ペインに cd を送信` : "cd 対象のペインなし";
      addNotice(
        `作業フォルダ: ${shownDir}（設定: ${originLabel(info.origin)}） / ${cdPart}`,
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
      ui.errorBanner = `再起動に失敗しました (restart_session): ${err}`;
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
        ui.errorBanner = `ペインの停止に失敗しました (kill_pty #${id}): ${err}`;
      });
    }
    ui.panes = ui.panes.filter((p) => p !== id);
    if (ui.maximizedId === id) ui.maximizedId = null;
    disposeTermHandle(id);
    delete ui.resources[id];
    delete ui.sessions[id];
    delete ui.transcripts[id];
  }

  function toggleMaximize(id: number): void {
    ui.maximizedId = ui.maximizedId === id ? null : id;
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
      ui.errorBanner = `ペイン数が上限（${MAX_PANES}）に達しています。既存のペインを閉じてから表示してください。`;
      return;
    }
    addPane(id);
  }

  async function stopOrphanTeammate(id: number): Promise<void> {
    if (isTauri()) {
      try {
        await invokeCmd<void>("kill_pty", { id });
      } catch (err) {
        ui.errorBanner = `teammate の停止に失敗しました (kill_pty #${id}): ${err}`;
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
    teammatesPanelPos = { top: rect.bottom + 6, right };
  }

  async function openTeammatesPanel(): Promise<void> {
    teammatesPanelOpen = !teammatesPanelOpen;
    if (teammatesPanelOpen) {
      positionTeammatesPanel();
      void refreshTeamsHostStatus();
    }
  }

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
      errors.push(
        `ペイン上限(${MAX_PANES})のため${skippedForCap}件を復元しませんでした`,
      );
    }
    if (errors.length > 0) {
      ui.errorBanner = `一部のセッションを復元できませんでした: ${errors.join(" / ")}`;
    }
    return true;
  }

  // ---- startup flow per contract ----
  onMount(() => {
    // Keep the fixed-position Teammates panel anchored to its badge when the
    // window is resized while the panel is open.
    const onReposition = () => {
      if (teammatesPanelOpen) positionTeammatesPanel();
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
      void loadDirSuggestions();
      let restored = false;
      try {
        restored = await restoreProjectState();
      } catch (err) {
        ui.errorBanner = `前回のプロジェクト状態を復元できませんでした: ${err}`;
      }
      try {
        if (!restored) {
          const info = await loadConfig();
          const autostarts = [
            ...info.config.agents,
            ...info.config.processes,
          ].filter((d) => d.autostart);
          for (const def of autostarts) {
            if (ui.panes.length >= MAX_PANES) break;
            await spawnAgent(def.name);
          }
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
  <div class="toolbar" bind:this={toolbarEl}>
    <span class="title">ptygrid</span>

    <div class="tb-group">
      <span class="tb-caption">ターミナル</span>
      <span class="tb-controls" role="group" aria-label="シェル追加">
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
      <span class="tb-caption">レイアウト</span>
      <span class="tb-controls layout-group" role="group" aria-label="列数">
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
      <span class="tb-caption">作業フォルダ</span>
      <span class="tb-controls">
        <input
          class="dir-input"
          type="text"
          list="dir-suggestions"
          placeholder="作業フォルダ（例: ~/works/hoge。先頭 ~ 可）"
          bind:value={configDirInput}
          title={"作業フォルダのパスを入力します（先頭 ~ はホーム展開）。\n" +
            "設定ファイル ptygrid.yml は 作業フォルダ内 → 起動フォルダ → ~/.ptygrid の順に探します。"}
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
          {loadingConfig ? "読み込み中…" : "読み込み"}
        </button>

        {#if ui.configInfo}
          <span
            class="origin-badge"
            title={ui.configInfo.origin === "default"
              ? `設定ファイルなし（組み込みの既定設定）。\n${ui.configInfo.path} を作成すると自動で読み込みます。\n作業フォルダ: ${ui.configInfo.dir}`
              : `設定ファイル: ${ui.configInfo.path}\n作業フォルダ: ${ui.configInfo.dir}`}
          >
            設定: {originLabel(ui.configInfo.origin)}
          </span>
          <span class="project-name" title={ui.configInfo.path}>
            {ui.configInfo.config.project ?? "（名称未設定）"}
          </span>
          <span class="chips">
            {#each agentDefs as def (def.name)}
              <span class="chip chip-agent" title={def.cmd}>
                {def.name}
                <button
                  class="chip-run"
                  onclick={() => spawnAgent(def.name)}
                  disabled={!canAddPane}
                  title={`エージェント ${def.name} を起動`}
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
                  title={`プロセス ${def.name} を起動`}
                >
                  ▶
                </button>
              </span>
            {/each}
          </span>
        {/if}
      </span>
    </div>

    <span class="spacer"></span>
    <button
      class="btn"
      class:seg-active={gitPanelOpen}
      onclick={() => (gitPanelOpen = !gitPanelOpen)}
      title="Git status / diff / commit"
    >
      Git
    </button>
    <button
      class="queen-badge {queenClass}"
      onclick={copyQueenCommand}
      title={queenTooltip}
      aria-label="Queen MCP サーバー状態（クリックで登録コマンドをコピー）"
    >
      <span class="queen-dot"></span>
      {queenLabel}
    </button>
    <div class="teammates-wrap">
      <button
        bind:this={teammatesBadgeEl}
        class="queen-badge {teammatesClass}"
        onclick={openTeammatesPanel}
        title={teammatesTooltip}
        aria-label="Teammate hooks（クリックで設定パネル）"
      >
        <span class="queen-dot"></span>
        Teammates{hostFallbackActive ? " ⚠" : ""}
      </button>
      {#if teammatesPanelOpen}
        <div
          class="teammates-panel"
          role="dialog"
          aria-label="Teammate hooks 設定"
          style="top: {teammatesPanelPos.top}px; right: {teammatesPanelPos.right}px;"
        >
          <div class="tm-head">
            <span class="tm-title">Teammate hooks</span>
            <button
              class="btn btn-small"
              onclick={() => (teammatesPanelOpen = false)}
              title="閉じる"
            >
              ✕
            </button>
          </div>
          {#if !isTauri()}
            <p class="tm-note">Tauri 実行環境でのみ利用できます。</p>
          {:else if !ui.teammateHooks}
            <p class="tm-note">状態を取得中…</p>
          {:else}
            <p class="tm-note">
              状態:
              {ui.teammateHooks.enabled ? "有効" : "無効"} ·
              通知: {ui.teammateHooks.hookNotifications ? "オン" : "オフ"} ·
              ポート :{ui.teammateHooks.port}
            </p>
            <div class="tm-actions">
              <button class="btn btn-small" onclick={copyHooksSnippet}>
                スニペットをコピー
              </button>
              <button
                class="btn btn-small"
                onclick={registerHooks}
                disabled={registering}
                title="~/.claude/settings.json へ登録"
              >
                {registering ? "登録中…" : "settings.json へ登録 (user)"}
              </button>
            </div>
            {#if finishedTeammatePaneIds.length > 0}
              <div class="tm-actions">
                <button
                  class="btn btn-small"
                  onclick={closeFinishedTeammatePanes}
                  title="終了した teammate / transcript ペインをまとめて閉じます（実体には影響しません）"
                >
                  終了したペインを一括で閉じる（{finishedTeammatePaneIds.length}）
                </button>
              </div>
            {/if}
            <div class="tm-events">
              <div class="tm-subhead">host モード（実 PTY teammate）</div>
              {#if (ui.teamsHost?.leads.length ?? 0) === 0 && orphanTeammates.length === 0}
                <div class="tm-empty">
                  稼働中の host lead はありません（ptygrid.yml の teams.mode: host）
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
                        {lead.fallback ? "host: フォールバック中" : "host"}
                      </span>
                    </div>
                    {#if lead.teammates.length === 0}
                      <div class="tm-empty">teammate なし</div>
                    {:else}
                      {#each lead.teammates.map(teammateRow) as tm (tm.id)}
                        <div class="tm-teammate">
                          <span class="tm-teammate-label">
                            #{tm.id}{tm.role ? ` ▸${tm.role}` : ""}
                            {tm.paneless ? "（グリッド外）" : ""}
                          </span>
                          {#if tm.paneless}
                            <button
                              class="btn btn-small"
                              onclick={() => showTeammatePane(tm.id)}
                              disabled={!canAddPane}
                              title="このteammateをグリッドに表示"
                            >
                              グリッドへ表示
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
                      <span class="tm-lead-id">lead 終了済み（孤立 teammate）</span>
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
                          title="この孤立 teammate プロセスを停止"
                        >
                          停止
                        </button>
                      </div>
                    {/each}
                  </div>
                {/if}
              {/if}
            </div>
            <div class="tm-events">
              <div class="tm-subhead">直近のイベント</div>
              {#if ui.teammateEvents.length === 0}
                <div class="tm-empty">まだイベントはありません</div>
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
    <span class="pane-count">{paneCount}/{MAX_PANES} ペイン</span>
  </div>

  {#if gitPanelOpen}
    {#key ui.configInfo?.path}
      <GitPanel
        worktrees={activeWorktrees}
        onclose={() => (gitPanelOpen = false)}
      />
    {/key}
  {/if}

  {#if ui.errorBanner}
    <div class="banner banner-error" role="alert">
      <span class="banner-text">{ui.errorBanner}</span>
      <button
        class="btn btn-small"
        onclick={() => (ui.errorBanner = null)}
        title="閉じる"
      >
        ✕
      </button>
    </div>
  {/if}

  <div class="grid" class:has-max={ui.maximizedId !== null}>
    {#if paneCount === 0}
      <div class="empty-hint">
        ペインがありません — ツールバーの「＋1」でシェルを開くか、エージェントを起動してください。
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
                        <span
                          class="pane-title"
                          title="read-only transcript（観測のみ・入力不可）"
                        >
                          {transcriptTitle(id)}
                        </span>
                        {#if session.teammate}
                          <span class="lead-ref" title="親 lead セッション">
                            ↳#{session.teammate.leadId}
                          </span>
                        {/if}
                        {#if stopped}
                          <span
                            class="finished-tag"
                            title="subagent は終了しました（閉じても影響なし）"
                          >
                            終了
                          </span>
                        {/if}
                        <span class="spacer"></span>
                        <button
                          class="pane-btn"
                          title={ui.maximizedId === id ? "最大化解除" : "最大化"}
                          onclick={() => toggleMaximize(id)}
                        >
                          ⤢
                        </button>
                        <button
                          class="pane-btn pane-btn-close"
                          title="閉じる（tail 停止のみ・subagent には影響しません）"
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
                        <span
                          class="pane-title"
                          title="host teammate（実 PTY・対話可能）"
                        >
                          {hostTeammateTitle(id)}
                        </span>
                        <span class="lead-ref" title="親 lead セッション">
                          ↳#{session.teammate.leadId}
                        </span>
                        {#if session?.state === "exited"}
                          <span
                            class="finished-tag"
                            title="teammate は終了しました"
                          >
                            終了
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
                            <span class="kill-confirm-text">teammate を停止しますか？</span>
                            <button
                              class="btn btn-small tm-stop"
                              onclick={() => confirmCloseHostTeammate(id)}
                            >
                              停止
                            </button>
                            <button
                              class="btn btn-small"
                              onclick={cancelCloseHostTeammate}
                            >
                              取消
                            </button>
                          </span>
                        {:else}
                          <button
                            class="pane-btn"
                            title="再起動"
                            onclick={() => restartSession(id)}
                          >
                            ⟳
                          </button>
                          <button
                            class="pane-btn"
                            title={ui.maximizedId === id ? "最大化解除" : "最大化"}
                            onclick={() => toggleMaximize(id)}
                          >
                            ⤢
                          </button>
                          <button
                            class="pane-btn pane-btn-close"
                            title="閉じる（teammate プロセスを停止します）"
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
                          title="再起動"
                          onclick={() => restartSession(id)}
                        >
                          ⟳
                        </button>
                        <button
                          class="pane-btn"
                          title={ui.maximizedId === id ? "最大化解除" : "最大化"}
                          onclick={() => toggleMaximize(id)}
                        >
                          ⤢
                        </button>
                        <button
                          class="pane-btn pane-btn-close"
                          title="閉じる（セッション終了）"
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
            title="閉じる"
          >
            ✕
          </button>
        </div>
      {/each}
    </div>
  {/if}

  {#if ui.configChangedPath}
    <div class="toast" role="status">
      <span class="toast-text">設定ファイル（ptygrid.yml）が変更されました</span>
      <button class="btn btn-small" onclick={onReloadConfig}>再読み込み</button>
      <button
        class="btn btn-small"
        onclick={() => (ui.configChangedPath = null)}
        title="閉じる"
      >
        ✕
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

  /* ---- grid ---- */

  .grid {
    flex: 1 1 auto;
    min-height: 0;
    position: relative;
    overflow: hidden;
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
