<script lang="ts">
  // Phase 5.0.0.f: minimal workflow control panel for the left dock's third
  // tab ("Workflows"). Pure store-driven view — no props: workflow
  // declarations come from ui.configInfo.config.workflows, live/recent runs
  // from ui.workflowRuns (kept current by the global `workflow-state`
  // listener in stores.svelte.ts, seeded once here via list_workflow_runs).
  // Launch/cancel call the Tauri commands directly (same self-contained
  // pattern GitPanel uses), independent of App.svelte's own toolbar chips.
  import { onMount } from "svelte";
  import { ui } from "./stores.svelte";
  import { formatDurationMs, msg } from "./i18n.svelte";
  import { invokeCmd, isTauri } from "./tauri";
  import type {
    ScheduleView,
    StepOutcome,
    WorkflowDef,
    WorkflowRun,
  } from "./types";

  const DEFAULT_COLS = 80;
  const DEFAULT_ROWS = 24;
  /** Recently-ended runs shown alongside every still-running run. */
  const MAX_RECENT_ENDED = 10;

  /** Same pattern the other panels use: reading it in the template tracks the
   * locale setting, so a language switch re-renders these labels. */
  let m = $derived(msg());

  /** Phase 5.0.8. Polled rather than pushed: the only field that moves on its
   * own is the countdown, and that changes by the minute. A `schedule-state`
   * event would be a new wire contract for something a 60s poll answers. */
  let schedules = $state<ScheduleView[]>([]);
  const SCHEDULE_POLL_MS = 60_000;

  let launchingName = $state<string | null>(null);
  let cancellingRunId = $state<string | null>(null);
  let error = $state<string | null>(null);

  let workflowDefs = $derived(
    Object.entries(ui.configInfo?.config.workflows ?? {}),
  );

  /** Every known run, most recently started first. */
  let sortedRuns = $derived(
    Object.values(ui.workflowRuns).sort((a, b) => b.startedAtMs - a.startedAtMs),
  );

  /** Every currently running/pending run, plus the most recent
   * MAX_RECENT_ENDED terminated runs — re-sorted by recency for display. */
  let visibleRuns = $derived.by(() => {
    const active = sortedRuns.filter(
      (r) => r.state === "running" || r.state === "pending",
    );
    const ended = sortedRuns
      .filter((r) => r.state !== "running" && r.state !== "pending")
      .slice(0, MAX_RECENT_ENDED);
    return [...active, ...ended].sort((a, b) => b.startedAtMs - a.startedAtMs);
  });

  let scheduleByName = $derived(
    new Map(schedules.map((s) => [s.name, s] as const)),
  );

  /** The one line under a scheduled workflow's name.
   *
   * Ordered by what an operator needs first when they ask why nothing
   * happened: stopped beats skipped beats the countdown, because a stopped
   * schedule has no countdown worth reading. The last run's outcome is always
   * appended — a "last run: 3 days ago" next to a healthy-looking "next 09:00"
   * is how a closed laptop shows up here, and there is no other signal for it
   * (nothing is caught up on launch, by design).
   *
   * Every word of it is built here (5.0.8 fix M5). `list_schedules` used to
   * hand over English sentences (`"stopped after 3 consecutive failures"`)
   * that were printed verbatim, so this line came out half-translated in a
   * Japanese UI. The backend now sends the declaration and tagged reasons; the
   * wording lives in `i18n.svelte.ts` on both sides. */
  function scheduleLine(s: ScheduleView): string {
    const parts: string[] = [m.wfScheduleEvery(s.every, s.at)];
    if (!s.enabled) {
      parts.push(m.wfScheduleDisabled);
    } else if (s.stoppedReason) {
      parts.push(stopLabel(s.stoppedReason));
    } else if (s.lastSkipReason) {
      parts.push(skipLabel(s.lastSkipReason));
    } else if (s.nextFireAtMs !== undefined) {
      parts.push(m.wfScheduleNext(fmtSchedTime(s.nextFireAtMs), untilLabel(s.nextFireAtMs)));
    }
    // A streak that has not yet reached the stop. Shown only while the
    // schedule is still live, because "1/3" next to "自動停止" would be
    // saying the same thing twice.
    if (s.enabled && !s.stoppedReason && s.consecutiveFailures > 0) {
      parts.push(m.wfScheduleFailureStreak(s.consecutiveFailures, s.maxConsecutiveFailures));
    }
    if (s.lastFireAtMs !== undefined) {
      parts.push(
        m.wfScheduleLast(
          fmtSchedStamp(s.lastFireAtMs),
          s.lastResult ? resultLabel(s.lastResult) : m.wfScheduleRunning,
        ),
      );
    }
    return parts.join(" · ");
  }

  function skipLabel(skip: NonNullable<ScheduleView["lastSkipReason"]>): string {
    switch (skip.kind) {
      case "overlap":
        return m.wfScheduleSkipOverlap;
      case "noRoom":
        return m.wfScheduleSkipNoRoom(skip.occupied, skip.cap, skip.needed);
      case "late":
        return m.wfScheduleSkipLate(skip.lateMinutes);
    }
  }

  function stopLabel(stop: NonNullable<ScheduleView["stoppedReason"]>): string {
    return m.wfScheduleStopped(stop.failures);
  }

  function resultLabel(result: NonNullable<ScheduleView["lastResult"]>): string {
    switch (result.kind) {
      case "succeeded":
        return m.wfScheduleResultSucceeded;
      case "failed":
        return m.wfScheduleResultFailed;
      case "cancelled":
        return m.wfScheduleResultCancelled;
      case "spawnFailed":
        return m.wfScheduleResultSpawnFailed(result.error);
    }
  }

  /** "3 時間 20 分" / "3h 20m" — rounded to the minute, which is the
   * resolution the schedule vocabulary has. Deliberately not
   * `formatDurationMs`: that one is for step timings and its "3h 20m" has no
   * translated form. */
  function untilLabel(atMs: number): string {
    const minutes = Math.round(Math.max(0, atMs - Date.now()) / 60_000);
    return m.wfScheduleUntil(Math.floor(minutes / 60), minutes % 60);
  }

  /** The last fire, to the minute. A schedule cannot resolve finer, and
   * seconds would only make the line longer.
   *
   * `undefined` locale, i.e. the SYSTEM one, not the app's language setting —
   * same as `fmtTime` on the run rows, and changing only the schedule line
   * would leave the panel formatting dates two ways. So a Japanese UI on an
   * English system still shows "8/5, 09:00 AM" here. */
  function fmtSchedStamp(ms: number): string {
    try {
      return new Date(ms).toLocaleString(undefined, {
        month: "numeric",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return String(ms);
    }
  }

  /** The next fire. Carries the date whenever it is not today, so "next 09:00"
   * no longer leaves "today or tomorrow?" to be inferred from the countdown. */
  function fmtSchedTime(ms: number): string {
    try {
      const d = new Date(ms);
      const sameDay = d.toDateString() === new Date().toDateString();
      return d.toLocaleString(undefined, {
        ...(sameDay ? {} : { month: "numeric", day: "numeric" }),
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return String(ms);
    }
  }

  function stepCountLabel(def: WorkflowDef): string {
    const n = def.steps.length;
    return `${n} step${n === 1 ? "" : "s"}`;
  }

  function defTitle(name: string, def: WorkflowDef): string {
    return `${name} [${def.pattern}] — ${stepCountLabel(def)}`;
  }

  function fmtTime(ms: number): string {
    try {
      return new Date(ms).toLocaleTimeString();
    } catch {
      return String(ms);
    }
  }

  /** Rendered timing for one step row, or null when there is nothing to say.
   * Phase 5.0.6: `endedAtMs` / `waitedForPaneMs` are both optional on the wire
   * (`skip_serializing_if` / `default`), so a run persisted before 5.0.6 and
   * resumed produces null here and the row looks exactly as it did before. */
  type StepTiming = { text: string; title: string };

  /** Derive the timing label for a step. Read-only and cheap, so it runs from
   * the template on every re-render: `workflow-state` events already repaint
   * the panel, and deliberately no timer/interval is added — a running step
   * simply shows nothing until it reaches a terminal state. */
  function stepTiming(step: StepOutcome): StepTiming | null {
    // A terminal step has `endedAtMs`; `startedAtMs === 0` means it was never
    // spawned, in which case the difference would be an epoch timestamp rather
    // than a duration. Guard the reversed-clock case too.
    const ended = step.endedAtMs;
    const ran =
      typeof ended === "number" &&
      Number.isFinite(ended) &&
      step.startedAtMs > 0 &&
      ended >= step.startedAtMs;
    const dur = ran ? formatDurationMs(ended - step.startedAtMs) : null;

    // Pane wait is a separate, cumulative quantity — shown whenever it is
    // non-zero, including on a step that has not terminated yet. It only moves
    // when a deferral is settled, so it never ticks on its own either.
    const waitedMs = step.waitedForPaneMs ?? 0;
    const wait =
      Number.isFinite(waitedMs) && waitedMs > 0
        ? formatDurationMs(waitedMs)
        : null;

    if (dur === null && wait === null) return null;
    const text =
      dur !== null && wait !== null
        ? m.wfStepDurationWaited(dur, wait)
        : dur !== null
          ? dur
          : m.wfStepWaitOnly(wait as string);
    return { text, title: m.wfStepTimingTitle(dur, wait) };
  }

  async function runWorkflow(name: string): Promise<void> {
    if (launchingName !== null || !isTauri()) return;
    launchingName = name;
    error = null;
    try {
      const run = await invokeCmd<WorkflowRun>("spawn_workflow", {
        name,
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
      });
      ui.workflowRuns[run.runId] = run;
    } catch (err) {
      error = `Failed to launch "${name}": ${err}`;
    } finally {
      launchingName = null;
    }
  }

  async function cancelRun(runId: string): Promise<void> {
    if (cancellingRunId !== null || !isTauri()) return;
    cancellingRunId = runId;
    error = null;
    try {
      const run = await invokeCmd<WorkflowRun>("cancel_workflow", { runId });
      ui.workflowRuns[run.runId] = run;
    } catch (err) {
      error = `Failed to cancel run: ${err}`;
    } finally {
      cancellingRunId = null;
    }
  }

  async function refresh(): Promise<void> {
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

  async function refreshSchedules(): Promise<void> {
    if (!isTauri()) return;
    try {
      schedules = await invokeCmd<ScheduleView[]>("list_schedules");
    } catch {
      // Same posture as `refresh`: a failed poll leaves the last known state
      // on screen rather than blanking a row that was telling the operator
      // something.
    }
  }

  onMount(() => {
    void refresh();
    void refreshSchedules();
    const timer = setInterval(() => void refreshSchedules(), SCHEDULE_POLL_MS);
    return () => clearInterval(timer);
  });
</script>

<div class="wf-panel">
  <div class="wf-section">
    <div class="wf-section-title">Workflows</div>
    {#if workflowDefs.length === 0}
      <div class="wf-empty">No workflows declared in ptygrid.yml</div>
    {:else}
      <div class="wf-def-list">
        {#each workflowDefs as [name, def] (name)}
          <div class="wf-def-row">
            <span class="wf-def-name" title={defTitle(name, def)}>
              {name}
              <span class="wf-def-meta">{def.pattern} · {stepCountLabel(def)}</span>
              {#if scheduleByName.get(name)}
                <span class="wf-def-schedule">
                  🕒 {scheduleLine(scheduleByName.get(name) as ScheduleView)}
                </span>
              {/if}
            </span>
            <button
              class="wf-btn wf-btn-run"
              onclick={() => runWorkflow(name)}
              disabled={launchingName !== null}
              title={`Run workflow "${name}"`}
            >
              ▶ Run
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  {#if error}
    <div class="wf-error">{error}</div>
  {/if}

  <div class="wf-section wf-runs-section">
    <div class="wf-section-title">Runs</div>
    {#if visibleRuns.length === 0}
      <div class="wf-empty">No workflow runs yet</div>
    {:else}
      <div class="wf-run-list">
        {#each visibleRuns as run (run.runId)}
          <div class="wf-run">
            <div class="wf-run-head">
              <span class={`wf-badge wf-badge-${run.state}`}>{run.state}</span>
              <span class="wf-run-name" title={run.runId}>{run.name}</span>
              <span class="wf-run-time">{fmtTime(run.startedAtMs)}</span>
              {#if run.state === "running" || run.state === "pending"}
                <button
                  class="wf-btn wf-btn-cancel"
                  onclick={() => cancelRun(run.runId)}
                  disabled={cancellingRunId !== null}
                  title="Cancel this run"
                >
                  ⏹ Cancel
                </button>
              {/if}
            </div>
            <div class="wf-steps">
              {#each run.steps as step (step.stepId)}
                {@const timing = stepTiming(step)}
                <div class="wf-step">
                  <span class={`wf-badge wf-badge-sm wf-badge-${step.state}`}>{step.state}</span>
                  <span class="wf-step-id">{step.stepId}</span>
                  <span class="wf-step-agent">{step.agent}</span>
                  {#if step.state === "pending" && step.error}
                    <!-- A step that is waiting rather than failing (the pane cap
                         queue, 5.0.4 hardening) explains itself in `error`. Show
                         it as text: folded into the ⚠ tooltip it was effectively
                         invisible, and "Pending with no reason" reads as stuck. -->
                    <span class="wf-step-wait" title={step.error}>{step.error}</span>
                  {:else if step.error}
                    <span class="wf-step-error" title={step.error}>⚠</span>
                  {/if}
                  {#if timing}
                    <!-- Phase 5.0.6: execution time (and cumulative pane wait)
                         for steps that reported them. Pinned to the right of
                         the row so it never competes with the wait reason
                         above, which keeps its ellipsis and shrinks first. -->
                    <span class="wf-step-timing" title={timing.title}
                      >{timing.text}</span
                    >
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .wf-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    font-size: 11px;
    color: #ccc;
  }

  .wf-section {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-bottom: 1px solid #333;
  }

  .wf-runs-section {
    flex: 1 1 auto;
    overflow-y: auto;
    border-bottom: none;
  }

  .wf-section-title {
    padding: 6px 8px 4px;
    color: #999;
    font-weight: 600;
    text-transform: uppercase;
    font-size: 10px;
    letter-spacing: 0.03em;
  }

  .wf-empty {
    color: #666;
    font-size: 11px;
    padding: 4px 8px 10px;
  }

  .wf-def-list {
    display: flex;
    flex-direction: column;
    max-height: 160px;
    overflow-y: auto;
  }

  .wf-def-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
  }

  .wf-def-row:hover {
    background: #2a2a2a;
  }

  .wf-def-name {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    color: #ddd;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .wf-def-schedule {
    display: block;
    font-size: 0.75rem;
    opacity: 0.75;
    margin-top: 1px;
  }

  .wf-def-meta {
    color: #888;
    font-size: 10px;
  }

  .wf-error {
    color: #f0b8b8;
    background: #3a2323;
    padding: 5px 8px;
    font-size: 11px;
  }

  .wf-run-list {
    display: flex;
    flex-direction: column;
  }

  .wf-run {
    padding: 5px 8px;
    border-bottom: 1px solid #2a2a2a;
  }

  .wf-run-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .wf-run-name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #ddd;
  }

  .wf-run-time {
    color: #777;
    font-size: 10px;
    font-variant-numeric: tabular-nums;
  }

  .wf-steps {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 4px;
    padding-left: 4px;
  }

  .wf-step {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 10px;
  }

  .wf-step-id {
    color: #bbb;
  }

  .wf-step-agent {
    color: #888;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .wf-step-error {
    color: #e0574a;
    cursor: help;
  }
  .wf-step-wait {
    color: #d7ba7d;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: help;
  }

  /* Phase 5.0.6 step timing. `margin-left: auto` right-aligns it and
     `flex: 0 0 auto` keeps it intact when a long wait reason shares the row —
     the reason ellipsises instead. Only rendered when the run carries the
     5.0.6 fields, so older rows are byte-identical to before. */
  .wf-step-timing {
    flex: 0 0 auto;
    margin-left: auto;
    padding-left: 6px;
    color: #7f9f7f;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    cursor: help;
  }

  .wf-badge {
    flex: 0 0 auto;
    padding: 1px 6px;
    border-radius: 8px;
    font-size: 10px;
    text-transform: uppercase;
    color: #111;
    background: #888;
  }

  .wf-badge-sm {
    padding: 0 5px;
  }

  .wf-badge-pending {
    background: #888;
  }

  .wf-badge-running {
    background: #e5c07b;
  }

  .wf-badge-succeeded {
    background: #4caf50;
  }

  .wf-badge-failed {
    background: #e0574a;
    color: #fff;
  }

  .wf-badge-skipped {
    background: #666;
    color: #ddd;
  }

  .wf-badge-cancelled {
    background: #666;
    color: #ddd;
  }

  .wf-btn {
    flex: 0 0 auto;
    background: transparent;
    border: 1px solid #444;
    border-radius: 3px;
    color: #bbb;
    cursor: pointer;
    font-size: 10px;
    padding: 1px 6px;
  }

  .wf-btn:hover:not(:disabled) {
    color: #fff;
    background: #333;
  }

  .wf-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .wf-btn-cancel {
    border-color: #6b2b2b;
    color: #e0a0a0;
  }

  .wf-btn-cancel:hover:not(:disabled) {
    background: #6b2b2b;
    color: #fff;
  }
</style>
