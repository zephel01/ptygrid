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
  /** Stacked auto-dismiss toasts (top-right). */
  notices: [] as Notice[],
});

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
}

export function paneTitle(id: number): string {
  return ui.sessions[id]?.name ?? `shell #${id}`;
}
