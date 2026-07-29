<script lang="ts">
  // Phase 5.0.2 `ptygrid init` — the preview modal (spec-init-5.0.2.md §6).
  //
  // Shows what was detected (found AND not found), where the file would go,
  // the generated YAML (editable), the self-check badge, and — when a config
  // already exists — the sidecar destination plus a line-by-line two-pane diff.
  // Nothing here writes on its own: `init_scan`/`init_preview` never touch the
  // disk and `init_write` re-runs `parse_config` on whatever the textarea holds.
  //
  // `dir` is always passed explicitly by the caller: `init_dir()` deliberately
  // refuses to fall back to the launch cwd (`no_target_dir:`), so an omitted
  // `dir` is a frontend bug, not a user error.
  import { onMount } from "svelte";
  import { msg } from "./i18n.svelte";
  import { invokeCmd, isTauri } from "./tauri";
  import type {
    ConfigOrigin,
    InitPreview,
    InitTarget,
    InitWriteResult,
  } from "./types";

  let {
    dir,
    onclose,
    onwritten,
  }: {
    /** Absolute working folder to scan and write into. Never omitted. */
    dir: string;
    onclose: () => void;
    /** Called after a successful write, before the modal closes. */
    onwritten: (result: InitWriteResult) => void;
  } = $props();

  const SIDECAR_FILE_NAME = "ptygrid.init.yml";
  /** Above this many DP cells the diff falls back to unaligned side-by-side. */
  const DIFF_MAX_CELLS = 4_000_000;

  let m = $derived(msg());

  let target = $state<InitTarget>("project");
  let preview = $state<InitPreview | null>(null);
  /** The text that will actually be written (`init_write` takes it verbatim). */
  let content = $state("");
  /** True once the textarea diverges from what `init_preview` generated. There
   * is no check-only command, so an edited buffer cannot show ✅/❌ until the
   * write re-runs `parse_config` — the badge says so instead of guessing. */
  let edited = $state(false);
  let loading = $state(false);
  let writing = $state(false);
  /** Preview-side failure (scan/generate). Write failures use `writeError`. */
  let loadError = $state<string | null>(null);
  let writeError = $state<string | null>(null);
  /** Backend text of the last refused write (shown verbatim under the reason). */
  let writeErrorDetail = $state<string | null>(null);
  /** Inline confirmation (GitPanel's `operationMessage` pattern). The global
   * toast stack sits below this modal, so the panel reports its own outcome. */
  let statusMessage = $state<string | null>(null);

  let scan = $derived(preview?.scan ?? null);

  /** `init_write` refuses when the destination is `<dir>/ptygrid.yml` and a
   * legacy `mterm.yml` is still there. `existing.legacy` can only be true when
   * `<dir>/ptygrid.yml` is absent (search order), so `!sidecar` + target
   * `project` is exactly the refused case — predictable without writing. */
  let legacyBlocks = $derived(
    preview !== null &&
      preview.target === "project" &&
      !preview.sidecar &&
      preview.scan.existing?.legacy === true,
  );

  let canWrite = $derived(
    preview !== null && !writing && !loading && (edited || preview.valid),
  );

  function originLabel(origin: ConfigOrigin): string {
    switch (origin) {
      case "project":
        return m.originProject;
      case "launch":
        return m.originLaunch;
      case "global":
        return m.originGlobal;
      case "default":
        return m.originDefault;
    }
  }

  /** Map the CONTRACT error prefixes to a human reason; unknown (plain I/O)
   * errors fall through to the raw backend text. */
  function errorReason(raw: string): string | null {
    if (raw.startsWith("legacy_config:")) return m.initErrLegacy;
    if (raw.startsWith("invalid_config:")) return m.initErrInvalid;
    if (raw.startsWith("no_home:")) return m.initErrNoHome;
    if (raw.startsWith("no_target_dir:")) return m.initErrNoTargetDir;
    return null;
  }

  // --- two-pane line diff (spec §3.4: line based, never a structural merge) ---

  type DiffRow = {
    leftNo: number | null;
    left: string | null;
    rightNo: number | null;
    right: string | null;
    kind: "same" | "removed" | "added" | "changed";
  };

  function splitLines(text: string): string[] {
    return text.replace(/\n$/, "").split("\n");
  }

  function lineDiff(left: string[], right: string[]): DiffRow[] {
    const n = left.length;
    const k = right.length;
    const rows: DiffRow[] = [];
    if ((n + 1) * (k + 1) > DIFF_MAX_CELLS) {
      for (let i = 0; i < Math.max(n, k); i += 1) {
        const l = i < n ? left[i] : null;
        const r = i < k ? right[i] : null;
        rows.push({
          leftNo: l === null ? null : i + 1,
          left: l,
          rightNo: r === null ? null : i + 1,
          right: r,
          kind: l !== null && r !== null && l === r ? "same" : "changed",
        });
      }
      return rows;
    }
    // Classic LCS table, walked forward so equal runs stay aligned.
    const width = k + 1;
    const lcs = new Uint32Array((n + 1) * width);
    for (let i = n - 1; i >= 0; i -= 1) {
      for (let j = k - 1; j >= 0; j -= 1) {
        lcs[i * width + j] =
          left[i] === right[j]
            ? lcs[(i + 1) * width + j + 1] + 1
            : Math.max(lcs[(i + 1) * width + j], lcs[i * width + j + 1]);
      }
    }
    let i = 0;
    let j = 0;
    while (i < n && j < k) {
      if (left[i] === right[j]) {
        rows.push({
          leftNo: i + 1,
          left: left[i],
          rightNo: j + 1,
          right: right[j],
          kind: "same",
        });
        i += 1;
        j += 1;
      } else if (lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
        rows.push({
          leftNo: i + 1,
          left: left[i],
          rightNo: null,
          right: null,
          kind: "removed",
        });
        i += 1;
      } else {
        rows.push({
          leftNo: null,
          left: null,
          rightNo: j + 1,
          right: right[j],
          kind: "added",
        });
        j += 1;
      }
    }
    while (i < n) {
      rows.push({
        leftNo: i + 1,
        left: left[i],
        rightNo: null,
        right: null,
        kind: "removed",
      });
      i += 1;
    }
    while (j < k) {
      rows.push({
        leftNo: null,
        left: null,
        rightNo: j + 1,
        right: right[j],
        kind: "added",
      });
      j += 1;
    }
    return rows;
  }

  let diffLines = $derived.by(() => {
    if (!preview?.sidecar || preview.existingContent === undefined) return null;
    return {
      left: splitLines(preview.existingContent),
      right: splitLines(content),
    };
  });

  let diffTooLarge = $derived(
    diffLines !== null &&
      (diffLines.left.length + 1) * (diffLines.right.length + 1) >
        DIFF_MAX_CELLS,
  );

  let diffRows = $derived(
    diffLines === null ? [] : lineDiff(diffLines.left, diffLines.right),
  );

  // --- commands ---

  async function refreshPreview(next: InitTarget): Promise<void> {
    if (!isTauri()) {
      loadError = m.tauriOnly;
      return;
    }
    loading = true;
    loadError = null;
    writeError = null;
    writeErrorDetail = null;
    statusMessage = null;
    try {
      const result = await invokeCmd<InitPreview>("init_preview", {
        dir,
        target: next,
      });
      preview = result;
      target = result.target;
      // Keep hand edits across a destination switch: the generated body does
      // not depend on `target` (only `path`/`sidecar` do), so discarding the
      // user's text here would lose work for no reason.
      if (!edited) content = result.content;
    } catch (err) {
      // Keep the last good preview (and therefore `target`): a failed switch to
      // Global — `no_home:` — must leave the Project choice reachable.
      const raw = String(err);
      loadError = errorReason(raw) ?? m.initPreviewFailed(raw);
    } finally {
      loading = false;
    }
  }

  function selectTarget(next: InitTarget): void {
    if (next === target || loading || writing) return;
    void refreshPreview(next);
  }

  async function copyContent(): Promise<void> {
    try {
      await navigator.clipboard.writeText(content);
      writeError = null;
      writeErrorDetail = null;
      statusMessage = m.initCopied;
    } catch (err) {
      statusMessage = null;
      writeError = m.clipboardCopyFailed(err);
      writeErrorDetail = null;
    }
  }

  async function write(): Promise<void> {
    if (!canWrite || preview === null) return;
    if (!isTauri()) {
      writeError = m.tauriOnly;
      return;
    }
    writing = true;
    writeError = null;
    writeErrorDetail = null;
    statusMessage = null;
    try {
      const result = await invokeCmd<InitWriteResult>("init_write", {
        dir,
        target,
        content,
      });
      onwritten(result);
      onclose();
    } catch (err) {
      const raw = String(err);
      const reason = errorReason(raw);
      writeError = reason ?? raw;
      writeErrorDetail = reason === null ? null : raw;
    } finally {
      writing = false;
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== "Escape" || writing) return;
    // Escape inside the editable preview would throw away hand edits; only the
    // explicit Cancel button does that.
    const el = event.target;
    if (el instanceof HTMLElement && el.tagName === "TEXTAREA") return;
    onclose();
  }

  onMount(() => {
    void refreshPreview(target);
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="init-overlay">
  <div class="init-modal" role="dialog" aria-modal="true" aria-label={m.initAria}>
    <header class="init-head">
      <span class="init-title">{m.initTitle}</span>
      <span class="init-spacer"></span>
      <button class="btn btn-small" onclick={onclose} title={m.btnClose}>✕</button>
    </header>

    <div class="init-body">
      {#if loadError}
        <div class="init-error" role="alert">{loadError}</div>
      {:else if loading && preview === null}
        <div class="init-muted">{m.initLoading}</div>
      {/if}

      {#if scan}
        <!-- 1. detection: found AND not found, so an absent guidance block
             always has a visible reason (spec §6-1). -->
        <section class="init-section">
          <div class="init-section-title">{m.initScanHead}</div>
          <dl class="init-facts">
            <dt>{m.initScanDir}</dt>
            <dd class="init-path" title={scan.dir}>{scan.dir}</dd>

            <dt>{m.initScanAgents}</dt>
            <dd class:init-absent={scan.agents.length === 0}>
              {scan.agents.length > 0 ? scan.agents.join(" / ") : m.initValueNone}
            </dd>

            <dt>{m.initScanProject}</dt>
            <dd class:init-absent={scan.projectKinds.length === 0}>
              {scan.projectKinds.length > 0
                ? scan.projectKinds.join(" / ")
                : m.initValueNone}
            </dd>

            <dt>{m.initScanGit}</dt>
            <dd class:init-absent={!scan.gitRepo}>
              {scan.gitRepo ? m.initValueYes : m.initValueNo}
            </dd>

            <dt>{m.initScanRouter}</dt>
            <dd class:init-absent={scan.routerPort === null}>
              {scan.routerPort === null
                ? m.initRouterNone
                : m.initRouterFound(scan.routerPort)}
            </dd>

            <dt>{m.initScanExisting}</dt>
            <dd class:init-absent={scan.existing === null}>
              {#if scan.existing}
                <span class="init-path" title={scan.existing.path}
                  >{scan.existing.path}</span
                >
                <span class="init-tag">{originLabel(scan.existing.origin)}</span>
                {#if scan.existing.legacy}
                  <span class="init-tag init-tag-warn">{m.initLegacyTag}</span>
                {/if}
              {:else}
                {m.initValueNo}
              {/if}
            </dd>
          </dl>
          <div class="init-note">{m.initScanNote}</div>
        </section>
      {/if}

      {#if preview}
        <!-- 2. destination + Project/Global choice. The Global note states the
             consequence (trust-free autostart) without recommending it. -->
        <section class="init-section">
          <div class="init-section-title">{m.initTargetHead}</div>
          <div class="init-row">
            <span
              class="init-seg"
              role="group"
              aria-label={m.initTargetAria}
            >
              <button
                class="seg-btn"
                class:seg-active={target === "project"}
                onclick={() => selectTarget("project")}
                disabled={loading || writing}
              >
                {m.initTargetProject}
              </button>
              <button
                class="seg-btn"
                class:seg-active={target === "global"}
                onclick={() => selectTarget("global")}
                disabled={loading || writing}
              >
                {m.initTargetGlobal}
              </button>
            </span>
            <span class="init-muted">{m.initDestLabel}</span>
            <span class="init-path init-dest" title={preview.path}
              >{preview.path}</span
            >
          </div>
          {#if target === "global"}
            <div class="init-warn" role="note">{m.initGlobalWarn}</div>
          {:else}
            <div class="init-note">{m.initTargetProjectNote}</div>
          {/if}
        </section>

        {#if legacyBlocks && scan?.existing}
          <div class="init-warn" role="alert">
            {m.initLegacyWarn(scan.existing.path)}
          </div>
        {/if}

        {#if preview.sidecar}
          <!-- 4. sidecar: name of the destination, the untouched existing file,
               and the line-by-line two-pane diff. -->
          <section class="init-section">
            <div class="init-section-title">{m.initSidecarHead}</div>
            <div class="init-note">{m.initSidecarNote(SIDECAR_FILE_NAME)}</div>
          </section>
        {/if}

        <!-- 3. editable preview + self-check badge + the autostart statement. -->
        <section class="init-section">
          <div class="init-section-title">{m.initPreviewHead}</div>
          <textarea
            class="init-preview"
            aria-label={m.initPreviewAria}
            spellcheck="false"
            value={content}
            oninput={(e) => {
              content = e.currentTarget.value;
              edited = content !== preview?.content;
            }}
            disabled={writing}
          ></textarea>
          <div class="init-row">
            {#if edited}
              <span class="init-badge init-badge-edited">{m.initCheckEdited}</span>
            {:else if preview.valid}
              <span class="init-badge init-badge-ok">{m.initCheckOk}</span>
            {:else}
              <span class="init-badge init-badge-ng">{m.initCheckNg}</span>
            {/if}
          </div>
          {#if !edited && !preview.valid && preview.error}
            <pre class="init-error-text">{preview.error}</pre>
          {/if}
          <div class="init-note">{m.initAutostartNote}</div>
        </section>

        {#if diffLines}
          <section class="init-section">
            <div class="init-section-title">{m.initDiffHead}</div>
            {#if diffTooLarge}
              <div class="init-note">{m.initDiffTooLarge}</div>
            {/if}
            <div class="init-diff-head">
              <span>{m.initDiffLeft}</span>
              <span>{m.initDiffRight}</span>
            </div>
            <div class="init-diff">
              {#each diffRows as row, index (index)}
                <div class="init-diff-row init-diff-{row.kind}">
                  <span class="init-diff-no">{row.leftNo ?? ""}</span>
                  <span class="init-diff-cell">{row.left ?? ""}</span>
                  <span class="init-diff-no">{row.rightNo ?? ""}</span>
                  <span class="init-diff-cell">{row.right ?? ""}</span>
                </div>
              {/each}
            </div>
          </section>
        {/if}

        {#if statusMessage}
          <div class="init-status" role="status">{statusMessage}</div>
        {/if}

        {#if writeError}
          <div class="init-error" role="alert">
            {writeError}
            {#if writeErrorDetail}
              <pre class="init-error-text">{writeErrorDetail}</pre>
            {/if}
          </div>
        {/if}
      {/if}
    </div>

    <footer class="init-foot">
      <button
        class="btn"
        onclick={write}
        disabled={!canWrite}
        title={m.btnInitWrite}
      >
        {writing ? m.btnInitWriting : m.btnInitWrite}
      </button>
      <!-- The escape hatch: always available, even when the self-check fails. -->
      <button
        class="btn"
        onclick={copyContent}
        disabled={content === ""}
        title={m.btnInitCopyTitle}
      >
        {m.btnInitCopy}
      </button>
      <span class="init-spacer"></span>
      <button class="btn" onclick={onclose} disabled={writing}>
        {m.btnCancel}
      </button>
    </footer>
  </div>
</div>

<style>
  .init-overlay {
    position: fixed;
    inset: 0;
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #000000aa;
  }

  .init-modal {
    display: flex;
    flex-direction: column;
    width: min(920px, 92vw);
    max-height: 88vh;
    background: #1e1e1e;
    border: 1px solid #444;
    border-radius: 6px;
    box-shadow: 0 8px 24px #0008;
    color: #ccc;
    font-size: 12px;
  }

  .init-head,
  .init-foot {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
  }

  .init-head {
    background: #252526;
    border-bottom: 1px solid #3a3a3a;
    border-radius: 6px 6px 0 0;
  }

  .init-foot {
    background: #252526;
    border-top: 1px solid #3a3a3a;
    border-radius: 0 0 6px 6px;
  }

  .init-title {
    color: #fff;
    font-weight: 600;
  }

  .init-spacer {
    flex: 1 1 auto;
  }

  .init-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 10px 10px;
  }

  .init-section {
    padding: 8px 0;
    border-bottom: 1px solid #333;
  }

  .init-section-title {
    color: #999;
    font-weight: 600;
    text-transform: uppercase;
    font-size: 10px;
    letter-spacing: 0.03em;
    padding-bottom: 4px;
  }

  .init-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 4px 0;
  }

  .init-facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px 10px;
    margin: 0;
  }

  .init-facts dt {
    color: #888;
  }

  .init-facts dd {
    margin: 0;
    color: #ddd;
    overflow-wrap: anywhere;
  }

  .init-absent {
    color: #888;
  }

  .init-path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    overflow-wrap: anywhere;
  }

  .init-dest {
    color: #ddd;
  }

  .init-tag {
    display: inline-block;
    margin-left: 6px;
    padding: 0 5px;
    border-radius: 8px;
    background: #333;
    color: #bbb;
    font-size: 10px;
  }

  .init-tag-warn {
    background: #4b3a1e;
    color: #d7ba7d;
  }

  .init-note {
    color: #888;
    padding: 4px 0 0;
  }

  .init-muted {
    color: #888;
  }

  .init-warn {
    color: #d7ba7d;
    background: #3a301e;
    border: 1px solid #5a4a2a;
    border-radius: 4px;
    padding: 5px 8px;
    margin: 6px 0;
  }

  .init-status {
    color: #9cc4e4;
    background: #23303a;
    border: 1px solid #35506b;
    border-radius: 4px;
    padding: 5px 8px;
    margin: 6px 0;
  }

  .init-error {
    color: #f1b0b0;
    background: #4b1e1e;
    border: 1px solid #6b2b2b;
    border-radius: 4px;
    padding: 5px 8px;
    margin: 6px 0;
  }

  .init-error-text {
    margin: 4px 0 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    color: #f1b0b0;
  }

  .init-seg {
    display: inline-flex;
    align-items: center;
    border: 1px solid #444;
    border-radius: 4px;
    overflow: hidden;
  }

  .seg-btn {
    background: #2a2a2a;
    color: #aaa;
    border: none;
    border-right: 1px solid #444;
    padding: 3px 8px;
    font-size: 11px;
    cursor: pointer;
  }

  .seg-btn:last-child {
    border-right: none;
  }

  .seg-btn:hover:not(.seg-active):not(:disabled) {
    background: #353535;
    color: #ddd;
  }

  .seg-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .seg-active {
    background: #3b5b7a;
    color: #fff;
  }

  .init-preview {
    width: 100%;
    min-height: 240px;
    resize: vertical;
    background: #1b1b1b;
    color: #ddd;
    border: 1px solid #444;
    border-radius: 4px;
    padding: 6px 8px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    line-height: 1.5;
    white-space: pre;
  }

  .init-preview:disabled {
    opacity: 0.6;
  }

  .init-badge {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 8px;
    font-size: 11px;
  }

  .init-badge-ok {
    background: #23361f;
    color: #4caf50;
  }

  .init-badge-ng {
    background: #4b1e1e;
    color: #f1b0b0;
  }

  .init-badge-edited {
    background: #3a301e;
    color: #d7ba7d;
  }

  .init-diff-head {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    color: #888;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding-bottom: 2px;
  }

  .init-diff {
    max-height: 260px;
    overflow: auto;
    border: 1px solid #333;
    border-radius: 4px;
    background: #1b1b1b;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    line-height: 1.45;
  }

  .init-diff-row {
    display: grid;
    grid-template-columns: 3.5em 1fr 3.5em 1fr;
    gap: 0 6px;
  }

  .init-diff-no {
    color: #666;
    text-align: right;
    -webkit-user-select: none;
    user-select: none;
  }

  .init-diff-cell {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: #ccc;
  }

  .init-diff-same .init-diff-cell {
    color: #999;
  }

  .init-diff-removed {
    background: #3a2323;
  }

  .init-diff-added {
    background: #23361f;
  }

  .init-diff-changed {
    background: #2f2a1e;
  }

  .btn {
    background: #333;
    color: #ddd;
    border: 1px solid #444;
    border-radius: 4px;
    padding: 3px 8px;
    font-size: 12px;
    cursor: pointer;
  }

  .btn:hover:not(:disabled) {
    background: #3d3d3d;
  }

  .btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .btn-small {
    padding: 1px 6px;
    font-size: 11px;
  }
</style>
