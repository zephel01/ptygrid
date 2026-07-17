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
  // Left, collapsible status sidebar (Phase 4.4.1 / spec 5.3). A pure derived
  // view: no backend / IPC. Open state + width are persisted by the parent
  // (localStorage) and passed in as bindable props.
  let {
    open = $bindable(true),
    width = $bindable(200),
    rows,
    onFocus,
    onToggleMax,
    onClose,
  }: {
    open?: boolean;
    width?: number;
    rows: StatusRow[];
    onFocus: (id: number) => void;
    onToggleMax: (id: number) => void;
    onClose: (id: number) => void;
  } = $props();

  const MIN_WIDTH = 150;
  const MAX_WIDTH = 480;

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

  let blockedCount = $derived(
    rows.filter((r) => r.status === "blocked").length,
  );

  const STATUS_LABEL: Record<AgentStatus, string> = {
    blocked: "blocked（承認待ち）",
    working: "working（実行中）",
    done: "done（完了）",
    idle: "idle（待機）",
    unknown: "unknown（判定なし）",
  };

  function rowTitle(r: StatusRow): string {
    const base = `${STATUS_LABEL[r.status]} · #${r.id}`;
    return r.matchedRule ? `${base} · ${r.matchedRule}` : base;
  }

  // ---- width resize (pointer drag on the right edge) ----
  let resizing = $state(false);

  function startResize(ev: PointerEvent): void {
    ev.preventDefault();
    resizing = true;
    const startX = ev.clientX;
    const startW = width;
    const move = (e: PointerEvent) => {
      const next = startW + (e.clientX - startX);
      width = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, next));
    };
    const up = () => {
      resizing = false;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
</script>

{#if open}
  <aside
    class="status-sidebar"
    class:resizing
    style="width: {width}px;"
    aria-label="ステータスサイドバー"
  >
    <div class="ss-head">
      <button
        class="ss-collapse"
        onclick={() => (open = false)}
        title="サイドバーを畳む"
        aria-label="サイドバーを畳む"
      >
        ‹
      </button>
      <span
        class="ss-blocked"
        class:muted={blockedCount === 0}
        title={blockedCount > 0
          ? `${blockedCount} ペインが承認待ち（blocked）`
          : "承認待ちのペインはありません"}
      >
        🔴 {blockedCount}
      </span>
      <span class="ss-count">{rows.length}</span>
    </div>

    <div class="ss-list">
      {#if sorted.length === 0}
        <div class="ss-empty">実行中のペインはありません</div>
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
              title="最大化トグル"
              aria-label="最大化トグル"
              onclick={(e) => {
                e.stopPropagation();
                onToggleMax(r.id);
              }}
            >
              ⤢
            </button>
            <button
              class="ss-btn ss-btn-close"
              title="閉じる"
              aria-label="閉じる"
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

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="ss-resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="サイドバーの幅を変更"
      onpointerdown={startResize}
    ></div>
  </aside>
{/if}

<style>
  .status-sidebar {
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

  .ss-head {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    background: #252526;
    border-bottom: 1px solid #333;
    font-size: 11px;
  }

  .ss-collapse {
    background: transparent;
    border: 1px solid #444;
    border-radius: 4px;
    color: #bbb;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
    padding: 1px 6px;
  }

  .ss-collapse:hover {
    background: #353535;
    color: #eee;
  }

  .ss-blocked {
    font-variant-numeric: tabular-nums;
    color: #f0b8b8;
    font-weight: 700;
  }

  .ss-blocked.muted {
    color: #6f6f6f;
    font-weight: 400;
  }

  .ss-count {
    margin-left: auto;
    color: #888;
    font-variant-numeric: tabular-nums;
  }

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

  .ss-resizer {
    position: absolute;
    top: 0;
    right: -3px;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 5;
  }

  .ss-resizer:hover {
    background: #4a6b9a;
  }

  .status-sidebar.resizing {
    cursor: col-resize;
  }
</style>
