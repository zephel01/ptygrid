<script lang="ts">
  // Pure pane component: attaches an xterm instance to an EXISTING session.
  // It does not spawn and does not kill_pty on destroy — closing a pane
  // (and disposing the terminal) is handled explicitly by App.
  import { onMount, onDestroy } from "svelte";
  import {
    ensureTermHandle,
    getTermHandle,
    COPY_SHORTCUT,
    PASTE_SHORTCUT,
    type TermHandle,
  } from "./terminals";
  import { msg } from "./i18n.svelte";
  import { ui } from "./stores.svelte";

  let { sessionId, title }: { sessionId: number; title: string } = $props();

  let m = $derived(msg());

  let containerEl: HTMLDivElement;
  let handle: TermHandle | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;
  let destroyed = false;

  // ---- right-click menu (copy / paste) ----
  // Fixed-position like the toolbar popovers in App. `canCopy` is sampled when
  // the menu opens: the selection cannot change while it is open.
  const MENU_WIDTH = 190;
  const MENU_HEIGHT = 74;
  let menu = $state<{ x: number; y: number } | null>(null);
  let canCopy = $state(false);
  let menuEl = $state<HTMLDivElement | null>(null);

  function openMenu(event: MouseEvent): void {
    event.preventDefault();
    canCopy = getTermHandle(sessionId)?.hasSelection() ?? false;
    menu = {
      x: Math.min(event.clientX, window.innerWidth - MENU_WIDTH - 6),
      y: Math.min(event.clientY, window.innerHeight - MENU_HEIGHT - 6),
    };
  }

  function closeMenu(): void {
    menu = null;
  }

  // Dismiss on Esc / click outside. Listeners exist only while the menu is
  // open and are removed by the effect's teardown (also on component destroy).
  $effect(() => {
    if (!menu) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        closeMenu();
      }
    };
    const onPointerDown = (event: MouseEvent) => {
      // Ignore presses on the menu itself, otherwise it would close before the
      // click event that runs the action.
      if (menuEl && event.target instanceof Node && menuEl.contains(event.target)) return;
      closeMenu();
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("mousedown", onPointerDown, true);
    window.addEventListener("blur", closeMenu);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("mousedown", onPointerDown, true);
      window.removeEventListener("blur", closeMenu);
    };
  });

  async function copyFromMenu(): Promise<void> {
    closeMenu();
    try {
      await getTermHandle(sessionId)?.copySelection();
    } catch (err) {
      ui.errorBanner = m.clipboardCopyFailed(err);
    }
  }

  async function pasteFromMenu(): Promise<void> {
    closeMenu();
    const h = getTermHandle(sessionId);
    try {
      await h?.pasteFromClipboard();
    } catch (err) {
      ui.errorBanner = m.clipboardPasteFailed(err);
    }
    // Right-clicking may have left focus elsewhere; typing should continue.
    h?.term.focus();
  }

  onMount(async () => {
    // Bound imperatively (and removed in onDestroy): as a Svelte attribute an
    // `oncontextmenu` on this plain <div> would demand an ARIA role it should
    // not carry — xterm builds its own accessible tree inside it.
    containerEl.addEventListener("contextmenu", openMenu);

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
    menu = null;
    if (debounceTimer) clearTimeout(debounceTimer);
    resizeObserver?.disconnect();
    containerEl?.removeEventListener("contextmenu", openMenu);
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

{#if menu}
  <div
    class="pane-menu"
    bind:this={menuEl}
    role="menu"
    aria-label={m.ctxMenuAria}
    style="left: {menu.x}px; top: {menu.y}px; width: {MENU_WIDTH}px;"
  >
    <button
      class="pane-menu-item"
      role="menuitem"
      disabled={!canCopy}
      title={canCopy ? m.ctxCopyTitle : m.ctxCopyDisabledTitle}
      onclick={copyFromMenu}
    >
      <span>{m.ctxCopy}</span>
      <span class="pane-menu-key">{COPY_SHORTCUT}</span>
    </button>
    <button
      class="pane-menu-item"
      role="menuitem"
      title={m.ctxPasteTitle}
      onclick={pasteFromMenu}
    >
      <span>{m.ctxPaste}</span>
      <span class="pane-menu-key">{PASTE_SHORTCUT}</span>
    </button>
  </div>
{/if}

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

  /* Same popover chrome as the toolbar panels in App.svelte. */
  .pane-menu {
    position: fixed;
    z-index: 130;
    background: #2d2d30;
    border: 1px solid #4a4a4a;
    border-radius: 6px;
    padding: 4px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.55);
  }

  .pane-menu-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    background: transparent;
    border: none;
    border-radius: 4px;
    padding: 4px 8px;
    color: #e8e8e8;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }

  .pane-menu-item:hover:not(:disabled) {
    background: #3a3a3a;
  }

  .pane-menu-item:disabled {
    color: #6f6f6f;
    cursor: default;
  }

  .pane-menu-key {
    color: #8a8a8a;
    font-size: 11px;
  }
</style>
