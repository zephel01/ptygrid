<script lang="ts" module>
  import type { AgentStatus } from "./types";

  /** One derived row of the status sidebar (a running pane). Pure view over
   * ui.sessions / ui.agentStatus / ui.panes — computed by the parent. */
  export type StatusRow = {
    id: number;
    /** Semantic status ("unknown" when no ruleset / not yet evaluated). */
    status: AgentStatus;
    /** Matched rule id (regex source) for the tooltip, if any. */
    matchedRule?: string;
    /** Display name (definition name / foreground; teammate & RO markers). */
    name: string;
    /** SessionState of the pane (all rows are running, kept for the liveness dot). */
    alive: boolean;
  };
</script>

<script lang="ts">
  // Status list body for the left dock's "ステータス" tab (Phase 4.4.1 / spec
  // 5.3 + Git 移設). A pure derived view: no backend / IPC. The frame (open
  // state, width, tabs, resize) is owned by the dock in App.svelte; this
  // component renders only the scrollable list.
  import { msg } from "./i18n.svelte";

  let {
    rows,
    onFocus,
    onToggleMax,
    onClose,
  }: {
    rows: StatusRow[];
    onFocus: (id: number) => void;
    onToggleMax: (id: number) => void;
    onClose: (id: number) => void;
  } = $props();

  // blocked > working > done > idle > unknown, then #id asc (spec 5.3 ソート).
  const STATUS_ORDER: Record<AgentStatus, number> = {
    blocked: 0,
    working: 1,
    done: 2,
    idle: 3,
    unknown: 4,
  };

  let sorted = $derived(
    [...rows].sort(
      (a, b) =>
        STATUS_ORDER[a.status] - STATUS_ORDER[b.status] || a.id - b.id,
    ),
  );

  let m = $derived(msg());

  let STATUS_LABEL = $derived<Record<AgentStatus, string>>({
    blocked: m.astatusBlocked,
    working: m.astatusWorking,
    done: m.astatusDone,
    idle: m.astatusIdle,
    unknown: m.astatusUnknown,
  });

  function rowTitle(r: StatusRow): string {
    const base = `${STATUS_LABEL[r.status]} · #${r.id}`;
    return r.matchedRule ? `${base} · ${r.matchedRule}` : base;
  }
</script>

<div class="ss-list" aria-label={m.ssAria}>
  {#if sorted.length === 0}
    <div class="ss-empty">{m.ssEmpty}</div>
  {:else}
    {#each sorted as r (r.id)}
      <div
        class="ss-row astatus-{r.status}"
        role="button"
        tabindex="0"
        title={rowTitle(r)}
        onclick={() => onFocus(r.id)}
        ondblclick={() => onToggleMax(r.id)}
        onkeydown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onFocus(r.id);
          }
        }}
      >
        <span class="ss-dot" aria-hidden="true"></span>
        <span class="ss-id">#{r.id}</span>
        <span class="ss-name">{r.name}</span>
        <span
          class="ss-live"
          class:alive={r.alive}
          title={r.alive ? "running" : "stopped"}
          aria-hidden="true"
        ></span>
        <button
          class="ss-btn"
          title={m.ssMaxToggle}
          aria-label={m.ssMaxToggle}
          onclick={(e) => {
            e.stopPropagation();
            onToggleMax(r.id);
          }}
        >
          ⤢
        </button>
        <button
          class="ss-btn ss-btn-close"
          title={m.btnClose}
          aria-label={m.btnClose}
          onclick={(e) => {
            e.stopPropagation();
            onClose(r.id);
          }}
        >
          ✕
        </button>
      </div>
    {/each}
  {/if}
</div>

<style>
  .ss-list {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 3px;
  }

  .ss-empty {
    color: #666;
    font-size: 11px;
    padding: 8px 6px;
    text-align: center;
  }

  .ss-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 5px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 11px;
  }

  .ss-row:hover {
    background: #2f2f31;
  }

  .ss-dot {
    flex: 0 0 auto;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #666;
  }

  /* semantic status color (shared class names with the header badge) */
  .astatus-blocked .ss-dot {
    background: #e0574a;
  }
  .astatus-working .ss-dot {
    background: #e5c07b;
  }
  .astatus-done .ss-dot {
    background: #4a9be0;
  }
  .astatus-idle .ss-dot {
    background: #4caf50;
  }
  .astatus-unknown .ss-dot {
    background: #666;
  }

  .ss-id {
    flex: 0 0 auto;
    color: #9aa4ad;
    font-family: Menlo, monospace;
    font-variant-numeric: tabular-nums;
  }

  .ss-name {
    flex: 1 1 auto;
    color: #d0d0d0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ss-live {
    flex: 0 0 auto;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #888;
  }

  .ss-live.alive {
    background: #4caf50;
  }

  .ss-btn {
    flex: 0 0 auto;
    background: transparent;
    border: none;
    color: #999;
    cursor: pointer;
    font-size: 11px;
    line-height: 1;
    padding: 1px 3px;
    border-radius: 3px;
    opacity: 0;
  }

  .ss-row:hover .ss-btn,
  .ss-row:focus-within .ss-btn {
    opacity: 1;
  }

  .ss-btn:hover {
    background: #3a3a3a;
    color: #eee;
  }

  .ss-btn-close:hover {
    background: #6b2b2b;
    color: #fff;
  }
</style>
