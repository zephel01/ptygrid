<script lang="ts">
  // Phase 4.1: read-only transcript view for an observe (teammate/subagent)
  // session. No xterm — just a monospace scroll buffer fed by the
  // `transcript-output` event (accumulated in ui.transcripts). Auto-scrolls
  // while the user is at the bottom; stops following once they scroll up.
  import { tick } from "svelte";
  import { msg } from "./i18n.svelte";
  import { ui } from "./stores.svelte";

  let { sessionId, stopped }: { sessionId: number; stopped: boolean } =
    $props();

  let scrollEl: HTMLDivElement | undefined;
  let following = $state(true);

  let text = $derived(ui.transcripts[sessionId] ?? "");

  function onScroll(): void {
    if (!scrollEl) return;
    const gap = scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight;
    following = gap < 24;
  }

  // Follow the tail on append unless the user scrolled up.
  $effect(() => {
    // Reference `text` so this re-runs on every append.
    text;
    if (!following) return;
    void tick().then(() => {
      if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight;
    });
  });
</script>

<div class="transcript" bind:this={scrollEl} onscroll={onScroll}>
  {#if text.length === 0}
    <div class="transcript-empty">
      {stopped ? msg().trSubStopped : msg().trWaiting}
    </div>
  {:else}
    <pre class="transcript-text">{text}</pre>
  {/if}
</div>

<style>
  .transcript {
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow-y: auto;
    background: #1b1b1b;
    color: #cfcfcf;
    padding: 4px 8px;
    box-sizing: border-box;
  }

  .transcript-text {
    margin: 0;
    font-family: Menlo, "MesloLGS NF", monospace;
    font-size: 12px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .transcript-empty {
    color: #6f6f6f;
    font-size: 12px;
    font-style: italic;
    padding: 6px 2px;
  }
</style>
