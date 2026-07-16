// Runes-based global state module (Svelte 5). Imported as `.svelte.ts` so
// `$state` works at module scope. Global Tauri event listeners are set up
// exactly once from App via initGlobalListeners().

import type {
  ConfigChangedPayload,
  ConfigInfo,
  PtyExitPayload,
  QueenNotifyPayload,
  QueenStatus,
  SessionResourcesPayload,
  SessionResourceUsage,
  SessionInfo,
  TeammateBannerPayload,
  TeammateFallbackPayload,
  TeammateFocusPayload,
  TeammateHooksInfo,
  TeammateLifecycleKind,
  TeammateLifecyclePayload,
  TeamsHostStatus,
  TranscriptOutputPayload,
} from "./types";
import { isTauri } from "./tauri";
import { writeToTerm } from "./terminals";

export type LayoutMode = "auto" | 1 | 2 | 3;

/** Auto-dismissing stacked toast (queen-notify, copy confirmations, ...). */
export type Notice = { key: number; title: string; message: string };

export const ui = $state({
  /** All known sessions keyed by id ({id, name?, cmd, state, code}). */
  sessions: {} as Record<number, SessionInfo>,
  /** Latest process-tree CPU/memory sample keyed by session id. */
  resources: {} as Record<number, SessionResourceUsage>,
  /** Ordered list of session ids that have an open pane (max 9). */
  panes: [] as number[],
  /** Session id of the maximized pane, or null. */
  maximizedId: null as number | null,
  /** Grid column mode: "auto" heuristic, or a fixed column count. */
  layoutMode: "auto" as LayoutMode,
  /** Last successfully loaded config. */
  configInfo: null as ConfigInfo | null,
  /** Dismissible error banner text. */
  errorBanner: null as string | null,
  /** Path from the last `config-changed` event (shows the reload toast). */
  configChangedPath: null as string | null,
  /** Queen MCP server status (null until first queen_status fetch). */
  queenStatus: null as QueenStatus | null,
  /** Teammate hooks info (null until first teammate_hooks_info fetch). */
  teammateHooks: null as TeammateHooksInfo | null,
  /** Most recent teammate-lifecycle events (newest first, capped). */
  teammateEvents: [] as TeammateEvent[],
  /** Accumulated read-only transcript text keyed by transcript session id. */
  transcripts: {} as Record<number, string>,
  /** Phase 4.2: latest teams_host_status (host leads + live teammate ids). */
  teamsHost: null as TeamsHostStatus | null,
  /** Phase 4.2: session ids briefly highlighted by a teammate-focus event. */
  focusedTeammates: {} as Record<number, true>,
  /** Stacked auto-dismiss toasts (top-right). */
  notices: [] as Notice[],
});

/** How long a teammate-focus highlight stays on a pane (ms). */
const FOCUS_PULSE_MS = 1600;

/** Per-transcript rolling text cap (chars); oldest is dropped past this. */
const TRANSCRIPT_CAP = 256 * 1024;

/** A received teammate-lifecycle event, kept for the Teammates panel. */
export type TeammateEvent = TeammateLifecyclePayload & {
  key: number;
  atMs: number;
};

const MAX_TEAMMATE_EVENTS = 10;
let nextTeammateKey = 0;

export const MAX_PANES = 9;

const NOTICE_TTL_MS = 5000;
let nextNoticeKey = 0;

export function addNotice(title: string, message = ""): void {
  const key = ++nextNoticeKey;
  ui.notices.push({ key, title, message });
  setTimeout(() => dismissNotice(key), NOTICE_TTL_MS);
}

export function dismissNotice(key: number): void {
  ui.notices = ui.notices.filter((n) => n.key !== key);
}

/** Fetch queen_status (startup + after each successful load_config). */
export async function refreshQueenStatus(): Promise<void> {
  if (!isTauri()) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    ui.queenStatus = await invoke<QueenStatus>("queen_status");
  } catch (err) {
    ui.queenStatus = { enabled: true, running: false, error: String(err) };
  }
}

/** Fetch teammate_hooks_info (startup + after each successful load_config). */
export async function refreshTeammateHooks(): Promise<void> {
  if (!isTauri()) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    ui.teammateHooks = await invoke<TeammateHooksInfo>("teammate_hooks_info");
  } catch {
    ui.teammateHooks = null;
  }
}

/** Fetch teams_host_status (Teammates panel open + on teammate-fallback). */
export async function refreshTeamsHostStatus(): Promise<void> {
  if (!isTauri()) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    ui.teamsHost = await invoke<TeamsHostStatus>("teams_host_status");
  } catch {
    ui.teamsHost = null;
  }
}

const TEAMMATE_KIND_LABELS: Record<TeammateLifecycleKind, string> = {
  "subagent-start": "が起動",
  "subagent-stop": "が停止",
  "teammate-idle": "がアイドル",
  "task-created": "のタスクを作成",
  "task-completed": "のタスクが完了",
};

/** Short Japanese toast text for a teammate-lifecycle event. */
function teammateToast(ev: TeammateLifecyclePayload): string {
  const who =
    ev.agentType ??
    ev.agentId ??
    ev.taskName ??
    ev.taskId ??
    ev.sessionId ??
    "teammate";
  return `🤝 teammate ${who} ${TEAMMATE_KIND_LABELS[ev.kind]}`;
}

let listenersInitialized = false;

/**
 * Set up the global listeners once:
 * session-state / session-resources / pty-exit / config-changed / queen-notify.
 */
export async function initGlobalListeners(): Promise<void> {
  if (listenersInitialized || !isTauri()) return;
  listenersInitialized = true;

  const { listen } = await import("@tauri-apps/api/event");

  await listen<SessionInfo>("session-state", (event) => {
    const payload = event.payload;
    // Dedup guard: a session is "known" if we already track it or a pane
    // exists for it — state transitions (restarting/exited) for known ids
    // must never re-add a pane.
    const known =
      payload.id in ui.sessions || ui.panes.includes(payload.id);

    if (known) {
      ui.sessions[payload.id] = payload;
      if (payload.state !== "running") delete ui.resources[payload.id];
      return;
    }

    // Late event for a session we already closed (kill_pty removes the
    // entry + pane): ignore instead of resurrecting an orphan entry.
    if (payload.state === "exited") return;

    // Unknown live session => spawned outside the UI (Queen's spawn_agent
    // MCP tool). Track it and auto-open a pane.
    ui.sessions[payload.id] = payload;
    if (ui.panes.length >= MAX_PANES) {
      const label = payload.name ?? `shell #${payload.id}`;
      ui.errorBanner = `Queen が「${label}」を起動しましたが、ペイン上限(${MAX_PANES})のため表示できません。`;
      return; // session keeps running headless
    }
    ui.panes.push(payload.id);
  });

  await listen<SessionResourcesPayload>("session-resources", (event) => {
    const next: Record<number, SessionResourceUsage> = {};
    for (const usage of event.payload.sessions) {
      if (ui.sessions[usage.id]?.state === "running") {
        next[usage.id] = usage;
      }
    }
    // One assignment per sampler tick keeps all pane values consistent.
    ui.resources = next;
  });

  await listen<PtyExitPayload>("pty-exit", (event) => {
    const session = ui.sessions[event.payload.id];
    if (session && session.state !== "restarting") {
      session.state = "exited";
      session.code = event.payload.code;
    }
    delete ui.resources[event.payload.id];
    writeToTerm(
      event.payload.id,
      `\r\n\x1b[1;31m[process exited with code ${event.payload.code ?? "unknown"}]\x1b[0m\r\n`,
    );
  });

  await listen<ConfigChangedPayload>("config-changed", (event) => {
    ui.configChangedPath = event.payload.path;
  });

  await listen<QueenNotifyPayload>("queen-notify", (event) => {
    addNotice(event.payload.title, event.payload.message);
  });

  await listen<TranscriptOutputPayload>("transcript-output", (event) => {
    const { id, text } = event.payload;
    const prev = ui.transcripts[id] ?? "";
    let next = prev + text;
    if (next.length > TRANSCRIPT_CAP) {
      next = next.slice(next.length - TRANSCRIPT_CAP);
    }
    ui.transcripts[id] = next;
  });

  await listen<TeammateBannerPayload>("teammate-banner", (event) => {
    // Follows the Phase 2 9-pane banner path (ui.errorBanner).
    ui.errorBanner = event.payload.message;
  });

  await listen<TeammateFocusPayload>("teammate-focus", (event) => {
    // tmux select-pane 相当: 該当ペインの枠を短時間ハイライトする。
    const { id } = event.payload;
    ui.focusedTeammates[id] = true;
    setTimeout(() => {
      delete ui.focusedTeammates[id];
    }, FOCUS_PULSE_MS);
  });

  await listen<TeammateFallbackPayload>("teammate-fallback", () => {
    // host が使われず observe 降格した。toast + host 状態を更新する。
    addNotice(
      "teammate を host できませんでした",
      "ネイティブペインにホストできず、読み取り専用ビューにフォールバックしました。",
    );
    void refreshTeamsHostStatus();
  });

  await listen<TeammateLifecyclePayload>("teammate-lifecycle", (event) => {
    const ev: TeammateEvent = {
      ...event.payload,
      key: ++nextTeammateKey,
      atMs: Date.now(),
    };
    ui.teammateEvents = [ev, ...ui.teammateEvents].slice(
      0,
      MAX_TEAMMATE_EVENTS,
    );
    // Toast only when hook notifications are on (backend still emits so the
    // Teammates panel stays current regardless).
    if (ui.teammateHooks?.hookNotifications) {
      addNotice(teammateToast(event.payload));
    }
  });
}

export function paneTitle(id: number): string {
  const name = ui.sessions[id]?.name;
  return name ? `${name} #${id}` : `shell #${id}`;
}
