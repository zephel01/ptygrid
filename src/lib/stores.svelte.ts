// Runes-based global state module (Svelte 5). Imported as `.svelte.ts` so
// `$state` works at module scope. Global Tauri event listeners are set up
// exactly once from App via initGlobalListeners().

import type {
  AgentStatus,
  AgentStatusPayload,
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
  WorkflowRun,
} from "./types";
import { msg } from "./i18n.svelte";
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
  /** Untrusted (project/launch) config awaiting a "trust this folder" decision
   * before its autostart commands may run (Finding S2). null when none pending. */
  trustPrompt: null as ConfigInfo | null,
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
  /** Phase 4.4.0: latest semantic status per session id (agent-status event).
   * Separate map from ui.sessions (liveness): only meaningful while running.
   * Cleared when the session leaves `running` (session-state) or on close. */
  agentStatus: {} as Record<number, AgentStatus>,
  /** Phase 4.4.0: matched rule id (regex source) per session id, for tooltips. */
  agentStatusRule: {} as Record<number, string>,
  /** Phase 4.4.3: foreground display detail per session id (currently the ssh
   * destination, e.g. `user@host`). Updated every resource tick; entries are
   * removed when the tick reports no detail and on exit/close. */
  foregroundDetail: {} as Record<number, string>,
  /** Stacked auto-dismiss toasts (top-right). */
  notices: [] as Notice[],
  /** Phase 5.0.1: runs left "running" from before a crash/restart, awaiting
   * the user's resume/discard decision (workflow-resume-pending event). */
  workflowResumePrompts: [] as WorkflowRun[],
  /** Phase 5.0.0.f: latest known WorkflowRun per runId (workflow-state event
   * + the one-shot list_workflow_runs fetch on mount). */
  workflowRuns: {} as Record<string, WorkflowRun>,
});

/** How long a teammate-focus highlight stays on a pane (ms). */
const FOCUS_PULSE_MS = 1600;

/** Active teammate-focus fade-out timers, keyed by session id (BUG-6). */
const focusTimers = new Map<number, ReturnType<typeof setTimeout>>();

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

/**
 * Ids of panes the user just closed. A late `session-state` (running /
 * restarting) event that arrives after we removed the pane locally but before
 * the backend kill lands must NOT resurrect it as a zombie pane (BUG-2). The
 * tombstone is short-lived so a genuinely new session reusing the id later is
 * still tracked normally.
 */
const closedTombstones = new Map<number, number>(); // id -> expiry epoch ms
const TOMBSTONE_TTL_MS = 5000;

/** Register a just-closed pane id so late session-state events are ignored. */
export function markPaneClosed(id: number): void {
  closedTombstones.set(id, Date.now() + TOMBSTONE_TTL_MS);
}

function isTombstoned(id: number): boolean {
  const expiry = closedTombstones.get(id);
  if (expiry === undefined) return false;
  if (Date.now() > expiry) {
    closedTombstones.delete(id);
    return false;
  }
  return true;
}

/**
 * Briefly highlight a pane's frame (reuses the teammate-focus pulse ring).
 * Used by the teammate-focus event and the status sidebar's row click so both
 * share one timer-per-id lifecycle (BUG-6: a re-focus resets, never clears a
 * newer highlight).
 */
export function focusPane(id: number): void {
  ui.focusedTeammates[id] = true;
  const prev = focusTimers.get(id);
  if (prev !== undefined) clearTimeout(prev);
  const timer = setTimeout(() => {
    delete ui.focusedTeammates[id];
    focusTimers.delete(id);
  }, FOCUS_PULSE_MS);
  focusTimers.set(id, timer);
}

/** Drop the semantic status + matched rule for a session (leave running / close). */
export function clearAgentStatus(id: number): void {
  delete ui.agentStatus[id];
  delete ui.agentStatusRule[id];
  // Foreground detail is only meaningful while running; same lifecycle.
  delete ui.foregroundDetail[id];
}

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

/** Short localized toast text for a teammate-lifecycle event. */
function teammateToast(ev: TeammateLifecyclePayload): string {
  const who =
    ev.agentType ??
    ev.agentId ??
    ev.taskName ??
    ev.taskId ??
    ev.sessionId ??
    "teammate";
  return msg().teammateLifecycleToast(who, ev.kind satisfies TeammateLifecycleKind);
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
      if (payload.state !== "running") {
        delete ui.resources[payload.id];
        // Semantic status only applies to a running PTY: drop it on
        // exited/restarting/starting so stale badges never linger (5.1).
        clearAgentStatus(payload.id);
      }
      return;
    }

    // Late event for a session we already closed (kill_pty removes the
    // entry + pane): ignore instead of resurrecting an orphan entry.
    if (payload.state === "exited") return;

    // Recently closed by the user: a delayed running/restarting event (kill
    // not yet reflected, or a restart racing a close) must not resurrect the
    // pane as a zombie (BUG-2). The tombstone expires after a short window.
    if (isTombstoned(payload.id)) return;

    // Unknown live session => spawned outside the UI (Queen's spawn_agent
    // MCP tool). Track it and auto-open a pane.
    ui.sessions[payload.id] = payload;
    if (ui.panes.length >= MAX_PANES) {
      const label = payload.name ?? `shell #${payload.id}`;
      ui.errorBanner = msg().queenSpawnPaneLimit(label, MAX_PANES);
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
    // Live foreground names ride this same tick (Phase 4.4.2): keep
    // ui.sessions[id].foreground fresh so a hand-started claude/codex/grok in a
    // shell pane is labelled by its CLI (header + status sidebar) instead of the
    // shell name. Self-corrects: when the CLI exits, the shell becomes fg again.
    for (const fg of event.payload.foreground ?? []) {
      const session = ui.sessions[fg.id];
      if (session?.state === "running") {
        session.foreground = fg.name;
        // Phase 4.4.3: destination detail (ssh) rides the same entry. Absent
        // detail clears the stored one so `ssh host` → back-to-shell is clean.
        if (fg.detail) ui.foregroundDetail[fg.id] = fg.detail;
        else delete ui.foregroundDetail[fg.id];
      }
    }
  });

  await listen<PtyExitPayload>("pty-exit", (event) => {
    const session = ui.sessions[event.payload.id];
    delete ui.resources[event.payload.id];
    // Already known to be restarting: this is an intentional restart cycle, so
    // suppress the exit banner entirely (avoids duplicate/misleading output).
    if (session && session.state === "restarting") return;
    if (session) {
      session.state = "exited";
      session.code = event.payload.code;
    }
    // Muted grey divider rather than an alarming red "[process exited]"
    // banner: autorestart emits pty-exit before session-state(restarting)
    // arrives, so a red banner falsely reads as a crash every restart (BUG-3).
    writeToTerm(
      event.payload.id,
      `\r\n\x1b[2m— exited (code ${event.payload.code ?? "unknown"}) —\x1b[0m\r\n`,
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
    focusPane(event.payload.id);
  });

  await listen<AgentStatusPayload>("agent-status", (event) => {
    // Phase 4.4.0: semantic status changed for a running session. Kept in a
    // map independent of ui.sessions (liveness); the header badge / status
    // sidebar derive purely from it. Cleared via session-state:exited / close.
    const { id, status, matchedRule } = event.payload;
    ui.agentStatus[id] = status;
    if (matchedRule) ui.agentStatusRule[id] = matchedRule;
    else delete ui.agentStatusRule[id];
  });

  await listen<TeammateFallbackPayload>("teammate-fallback", () => {
    // host が使われず observe 降格した。toast + host 状態を更新する。
    addNotice(msg().hostFallbackNoticeTitle, msg().hostFallbackNoticeMsg);
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

  await listen<WorkflowRun[]>("workflow-resume-pending", (event) => {
    ui.workflowResumePrompts = event.payload;
  });

  await listen<WorkflowRun>("workflow-state", (event) => {
    // Phase 5.0.0.f: the orchestrator driver emits a full run snapshot
    // whenever any field changes; last-write-wins keyed by runId (no merge).
    ui.workflowRuns[event.payload.runId] = event.payload;
  });
}

export function paneTitle(id: number): string {
  const session = ui.sessions[id];
  // Prefer the definition name; for a hand-started session (no spec.name) fall
  // back to the live foreground process name so `claude`/`codex`/`grok` typed
  // into a shell pane shows as `claude #2` rather than `shell #2` (Phase 4.4.2).
  // Phase 4.4.3: when the foreground name is what we show, append its
  // destination detail (`ssh user@host #2`) so the target host is visible in
  // the pane you are typing into.
  const name = session?.name ?? session?.foreground;
  if (!name) return `shell #${id}`;
  const detail =
    !session?.name && session?.foreground ? ui.foregroundDetail[id] : undefined;
  return detail ? `${name} ${detail} #${id}` : `${name} #${id}`;
}
