<script lang="ts">
  // Pure pane component: attaches an xterm instance to an EXISTING session.
  // It does not spawn and does not kill_pty on destroy — closing a pane
  // (and disposing the terminal) is handled explicitly by App.
  import { onMount, onDestroy } from "svelte";
  import {
    ensureTermHandle,
    getTermHandle,
    type TermHandle,
  } from "./terminals";

  let { sessionId, title }: { sessionId: number; title: string } = $props();

  let containerEl: HTMLDivElement;
  let handle: TermHandle | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;
  let destroyed = false;

  onMount(async () => {
    handle = await ensureTermHandle(sessionId);
    if (destroyed) return;

    handle.attach(containerEl);

    resizeObserver = new ResizeObserver(() => {
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(() => handle?.fitAndSync(), 50);
    });
    resizeObserver.observe(containerEl);
  });

  onDestroy(() => {
    destroyed = true;
    if (debounceTimer) clearTimeout(debounceTimer);
    resizeObserver?.disconnect();
    // Detach only (keeps the xterm instance + scrollback alive across grid
    // re-layouts). getTermHandle is undefined if App already disposed it.
    getTermHandle(sessionId)?.detach(containerEl);
  });
</script>

<div
  class="terminal-container"
  bind:this={containerEl}
  aria-label={title}
></div>

<style>
  .terminal-container {
    width: 100%;
    height: 100%;
    min-height: 0;
    padding: 2px 0 0 4px;
    background: #1e1e1e;
    overflow: hidden;
  }

  .terminal-container :global(.xterm) {
    height: 100%;
  }
</style>
