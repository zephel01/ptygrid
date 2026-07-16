<script lang="ts">
  import { onMount } from "svelte";
  import { Splitpanes, Pane } from "svelte-splitpanes";
  import Terminal from "./lib/Terminal.svelte";
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
    type LayoutMode,
  } from "./lib/stores.svelte";
  import { disposeTermHandle, writeToTerm } from "./lib/terminals";
  import { invokeCmd, isTauri } from "./lib/tauri";
  import type {
    ConfigInfo,
    LogicalSession,
    ProjectState,
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
    if (!q.enabled) return "Queen は無効です（mterm.yml の queen.enabled: false）";
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
  let registering = $state(false);

  let teammatesClass = $derived.by(() => {
    if (!isTauri()) return "queen-off";
    return ui.teammateHooks?.enabled ? "queen-running" : "queen-off";
  });
  let teammatesTooltip = $derived.by(() => {
    if (!isTauri()) return "Tauri 実行環境なし（デモモード）";
    const t = ui.teammateHooks;
    if (!t) return "Teammate hooks（状態未取得）";
    return t.enabled
      ? "Teammate hooks 有効（クリックで設定）"
      : "Teammate hooks 無効（mterm.yml の teammates.enabled: true で有効化）";
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
        : ui.panes.indexOf(ui.maximizedId);
    return {
      version: 1,
      configDir: info.dir,
      layoutMode: savedLayoutMode(),
      sessions: ui.panes.map(logicalSession),
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

  async function loadConfig(dir?: string): Promise<ConfigInfo> {
    loadingConfig = true;
    try {
      const info = await invokeCmd<ConfigInfo>(
        "load_config",
        dir ? { dir } : {},
      );
      ui.configInfo = info;
      ui.configChangedPath = null;
      // Queen may have been restarted if the port changed in mterm.yml.
      void refreshQueenStatus();
      // teammates.enabled / hook_notifications may have changed too.
      void refreshTeammateHooks();
      return info;
    } finally {
      loadingConfig = false;
    }
  }

  async function onLoadClick(): Promise<void> {
    if (!isTauri()) {
      ui.errorBanner =
        "設定の読み込み (load_config) には Tauri 実行環境が必要です。";
      return;
    }
    try {
      await loadConfig(configDirInput.trim() || undefined);
      ui.errorBanner = null;
    } catch (err) {
      ui.errorBanner = String(err);
    }
  }

  async function onReloadConfig(): Promise<void> {
    // Reload the watched mterm.yml; running sessions are NOT respawned.
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
    if (isTauri()) {
      // best-effort; pane close is authoritative on the frontend
      invokeCmd<void>("kill_pty", { id }).catch(() => {});
    }
    ui.panes = ui.panes.filter((p) => p !== id);
    if (ui.maximizedId === id) ui.maximizedId = null;
    disposeTermHandle(id);
    delete ui.resources[id];
    delete ui.sessions[id];
  }

  function toggleMaximize(id: number): void {
    ui.maximizedId = ui.maximizedId === id ? null : id;
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
    for (const [index, session] of saved.sessions.entries()) {
      if (ui.panes.length >= MAX_PANES) break;
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
    if (errors.length > 0) {
      ui.errorBanner = `一部のセッションを復元できませんでした: ${errors.join(" / ")}`;
    }
    return true;
  }

  // ---- startup flow per contract ----
  onMount(() => {
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
  });
</script>

<main>
  <div class="toolbar">
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
      <span class="tb-caption">プロジェクト</span>
      <span class="tb-controls">
        <input
          class="dir-input"
          type="text"
          placeholder="mterm.yml のあるディレクトリ"
          bind:value={configDirInput}
          onkeydown={(e) => {
            if (e.key === "Enter") onLoadClick();
          }}
        />
        <button class="btn" onclick={onLoadClick} disabled={loadingConfig}>
          {loadingConfig ? "読み込み中…" : "読み込み"}
        </button>

        {#if ui.configInfo}
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
        class="queen-badge {teammatesClass}"
        onclick={() => (teammatesPanelOpen = !teammatesPanelOpen)}
        title={teammatesTooltip}
        aria-label="Teammate hooks（クリックで設定パネル）"
      >
        <span class="queen-dot"></span>
        Teammates
      </button>
      {#if teammatesPanelOpen}
        <div class="teammates-panel" role="dialog" aria-label="Teammate hooks 設定">
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
                  <section class="pane" class:is-max={ui.maximizedId === id}>
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
      <span class="toast-text">mterm.yml が変更されました</span>
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
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 120;
    width: 340px;
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

  .pane-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }
</style>
