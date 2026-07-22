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
  import { invokeCmd, isTauri } from "./tauri";
  import type { WorkflowDef, WorkflowRun } from "./types";

  const DEFAULT_COLS = 80;
  const DEFAULT_ROWS = 24;
  /** Recently-ended runs shown alongside every still-running run. */
  const MAX_RECENT_ENDED = 10;

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

  onMount(() => {
    void refresh();
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
                <div class="wf-step">
                  <span class={`wf-badge wf-badge-sm wf-badge-${step.state}`}>{step.state}</span>
                  <span class="wf-step-id">{step.stepId}</span>
                  <span class="wf-step-agent">{step.agent}</span>
                  {#if step.error}
                    <span class="wf-step-error" title={step.error}>⚠</span>
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
