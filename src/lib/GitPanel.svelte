<script lang="ts">
  import { onMount } from "svelte";
  import { invokeCmd } from "./tauri";
  import type {
    GitCommitInfo,
    GitDiffInfo,
    GitFileStatus,
    GitStatusInfo,
    WorktreeInfo,
  } from "./types";

  let {
    dir,
    worktrees = [],
    onclose,
  }: {
    dir?: string;
    worktrees?: WorktreeInfo[];
    onclose: () => void;
  } = $props();

  let activeDir = $state<string | undefined>();

  let status = $state<GitStatusInfo | null>(null);
  let diff = $state<GitDiffInfo | null>(null);
  let selectedPath = $state<string | null>(null);
  let selectedPaths = $state<string[]>([]);
  let staged = $state(false);
  let loadingStatus = $state(false);
  let loadingDiff = $state(false);
  let mutating = $state(false);
  let error = $state<string | null>(null);
  let operationMessage = $state<string | null>(null);
  let commitMessage = $state("");

  let hasStagedChanges = $derived(
    status?.files.some((file) => hasIndexChange(file)) ?? false,
  );

  // Monotonic generation tokens: after an await we only apply a result if its
  // request is still the latest one, so a slow earlier request can't overwrite
  // newer status/diff/error state (BUG-4).
  let statusGen = 0;
  let diffGen = 0;

  function dirArgs(): { dir?: string } {
    return activeDir ? { dir: activeDir } : {};
  }

  function statusCode(file: GitFileStatus): string {
    return `${file.indexStatus}${file.worktreeStatus}`;
  }

  function hasIndexChange(file: GitFileStatus): boolean {
    return file.indexStatus !== " " && file.indexStatus !== "?";
  }

  function hasWorktreeChange(file: GitFileStatus): boolean {
    return file.worktreeStatus !== " ";
  }

  function toggleSelected(path: string): void {
    selectedPaths = selectedPaths.includes(path)
      ? selectedPaths.filter((selected) => selected !== path)
      : [...selectedPaths, path];
  }

  function selectedForStage(): string[] {
    return (status?.files ?? [])
      .filter(
        (file) =>
          selectedPaths.includes(file.path) && hasWorktreeChange(file),
      )
      .map((file) => file.path);
  }

  function selectedForUnstage(): string[] {
    return (status?.files ?? [])
      .filter(
        (file) => selectedPaths.includes(file.path) && hasIndexChange(file),
      )
      .map((file) => file.path);
  }

  async function refresh(): Promise<void> {
    const gen = ++statusGen;
    loadingStatus = true;
    error = null;
    operationMessage = null;
    try {
      const next = await invokeCmd<GitStatusInfo>("git_status", dirArgs());
      if (gen !== statusGen) return; // superseded by a newer refresh
      status = next;
      if (
        selectedPath &&
        !status.files.some((file) => file.path === selectedPath)
      ) {
        selectedPath = null;
      }
      selectedPaths = selectedPaths.filter((path) =>
        status?.files.some((file) => file.path === path),
      );
      await loadDiff(selectedPath, staged);
    } catch (err) {
      if (gen !== statusGen) return;
      status = null;
      diff = null;
      error = String(err);
    } finally {
      if (gen === statusGen) loadingStatus = false;
    }
  }

  async function selectFile(file: GitFileStatus): Promise<void> {
    selectedPath = file.path;
    // Prefer working-tree diff when both sides changed. A purely staged file
    // opens its staged diff immediately.
    staged = !hasWorktreeChange(file) && hasIndexChange(file);
    await loadDiff(file.path, staged);
  }

  async function loadDiff(
    path: string | null,
    nextStaged: boolean,
  ): Promise<void> {
    const gen = ++diffGen;
    loadingDiff = true;
    error = null;
    staged = nextStaged;
    try {
      const next = await invokeCmd<GitDiffInfo>("git_diff", {
        ...dirArgs(),
        ...(path ? { path } : {}),
        staged: nextStaged,
      });
      if (gen !== diffGen) return; // a newer diff request superseded this one
      diff = next;
    } catch (err) {
      if (gen !== diffGen) return;
      diff = null;
      error = String(err);
    } finally {
      if (gen === diffGen) loadingDiff = false;
    }
  }

  async function mutatePaths(
    command: "git_stage" | "git_unstage",
    paths: string[],
  ): Promise<void> {
    if (paths.length === 0 || mutating) return;
    mutating = true;
    error = null;
    operationMessage = null;
    try {
      status = await invokeCmd<GitStatusInfo>(command, {
        ...dirArgs(),
        paths,
      });
      operationMessage = `${paths.length}件を${command === "git_stage" ? "stage" : "unstage"}しました。`;
      selectedPaths = [];
      if (
        selectedPath &&
        !status.files.some((file) => file.path === selectedPath)
      ) {
        selectedPath = null;
      }
      await loadDiff(selectedPath, staged);
    } catch (err) {
      error = String(err);
    } finally {
      mutating = false;
    }
  }

  async function commitChanges(): Promise<void> {
    if (!commitMessage.trim() || !hasStagedChanges || mutating) return;
    mutating = true;
    error = null;
    operationMessage = null;
    try {
      const committed = await invokeCmd<GitCommitInfo>("git_commit", {
        ...dirArgs(),
        message: commitMessage,
      });
      commitMessage = "";
      selectedPath = null;
      selectedPaths = [];
      staged = false;
      await refresh();
      operationMessage = `Committed ${committed.oid.slice(0, 12)} — ${committed.summary}`;
    } catch (err) {
      error = String(err);
    } finally {
      mutating = false;
    }
  }

  onMount(() => {
    activeDir = dir;
    void refresh();
  });

  async function changeWorkspace(event: Event): Promise<void> {
    const value = (event.currentTarget as HTMLSelectElement).value;
    activeDir = value || dir;
    selectedPath = null;
    selectedPaths = [];
    staged = false;
    await refresh();
  }
</script>

<aside class="git-panel" aria-label="Git changes">
  <header class="git-header">
    <div>
      <div class="git-title">Git</div>
      {#if status}
        <div class="git-ref">
          {status.branch ?? "detached HEAD"} · {status.head}
        </div>
      {/if}
    </div>
    <span class="spacer"></span>
    <button class="icon-btn" onclick={refresh} disabled={loadingStatus || mutating} title="更新">
      ⟳
    </button>
    <button class="icon-btn" onclick={onclose} title="閉じる">✕</button>
  </header>

  {#if status}
    {#if worktrees.length > 0}
      <label class="workspace-select">
        <span>Workspace</span>
        <select value={activeDir ?? ""} onchange={changeWorkspace} disabled={mutating}>
          <option value={dir ?? ""}>Project workspace</option>
          {#each worktrees as worktree (worktree.path)}
            <option value={worktree.path}>{worktree.branch}</option>
          {/each}
        </select>
      </label>
    {/if}
    <div class="repo-root" title={status.repoRoot}>{status.repoRoot}</div>
    <div class="file-heading">
      <button
        class:active={selectedPath === null}
        onclick={() => {
          selectedPath = null;
          void loadDiff(null, staged);
        }}
      >
        すべての変更
      </button>
      <span>{status.files.length}</span>
    </div>
    <div class="mutation-bar">
      <span>{selectedPaths.length}件選択</span>
      <span class="spacer"></span>
      <button
        onclick={() => mutatePaths("git_stage", selectedForStage())}
        disabled={selectedForStage().length === 0 || mutating}
      >Stage</button>
      <button
        onclick={() => mutatePaths("git_unstage", selectedForUnstage())}
        disabled={selectedForUnstage().length === 0 || mutating}
      >Unstage</button>
    </div>
    <div class="file-list">
      {#if status.files.length === 0}
        <div class="empty">変更はありません</div>
      {:else}
        {#each status.files as file (file.path)}
          <div
            class="file-row"
            class:active={selectedPath === file.path}
            title={file.originalPath
              ? `${file.originalPath} → ${file.path}`
              : file.path}
          >
            <input
              type="checkbox"
              checked={selectedPaths.includes(file.path)}
              onchange={() => toggleSelected(file.path)}
              aria-label={`${file.path}を操作対象に選択`}
            />
            <button class="file-select" onclick={() => selectFile(file)}>
              <span class="status-code">{statusCode(file)}</span>
              <span class="file-path">{file.path}</span>
            </button>
          </div>
        {/each}
      {/if}
      {#if status.truncated}
        <div class="warning">10,000ファイルで表示を打ち切りました</div>
      {/if}
    </div>
  {/if}

  <div class="commit-box">
    <textarea
      bind:value={commitMessage}
      rows="2"
      placeholder="Commit message"
      aria-label="Commit message"
      disabled={mutating}
    ></textarea>
    <button
      class="commit-btn"
      onclick={commitChanges}
      disabled={!commitMessage.trim() || !hasStagedChanges || mutating}
      title={hasStagedChanges
        ? "現在stageされている変更をcommit"
        : "stageされた変更がありません"}
    >{mutating ? "処理中…" : "Commit staged changes"}</button>
  </div>

  {#if operationMessage}
    <div class="operation-message" role="status">{operationMessage}</div>
  {/if}

  <div class="diff-tabs" role="group" aria-label="Diff target">
    <button
      class:active={!staged}
      onclick={() => loadDiff(selectedPath, false)}
      disabled={loadingDiff || mutating}
    >Working tree</button>
    <button
      class:active={staged}
      onclick={() => loadDiff(selectedPath, true)}
      disabled={loadingDiff || mutating}
    >Staged</button>
  </div>

  {#if error}
    <div class="git-error" role="alert">{error}</div>
  {:else if loadingDiff || loadingStatus}
    <div class="empty">読み込み中…</div>
  {:else if diff?.text}
    <pre class="diff-view">{diff.text}</pre>
  {:else if status}
    <div class="empty">この範囲にdiffはありません</div>
  {/if}
</aside>

<style>
  .git-panel {
    position: fixed;
    z-index: 100;
    top: 43px;
    right: 0;
    bottom: 0;
    width: min(520px, 48vw);
    min-width: 360px;
    display: flex;
    flex-direction: column;
    background: #1e1e1e;
    border-left: 1px solid #444;
    box-shadow: -8px 0 24px #0008;
    color: #ccc;
    font-size: 12px;
  }

  .git-header,
  .mutation-bar,
  .commit-box {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .git-header {
    padding: 8px 10px;
    background: #252526;
    border-bottom: 1px solid #3a3a3a;
  }

  .git-title {
    color: #fff;
    font-weight: 600;
  }

  .git-ref,
  .repo-root {
    color: #888;
    font-size: 10px;
  }

  .workspace-select {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    color: #888;
    border-bottom: 1px solid #333;
  }

  .workspace-select select {
    flex: 1;
    min-width: 0;
    border: 1px solid #444;
    border-radius: 3px;
    padding: 3px 5px;
    background: #181818;
    color: #ddd;
  }

  .repo-root {
    padding: 5px 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border-bottom: 1px solid #333;
  }

  .spacer {
    flex: 1;
  }

  button {
    border: 0;
    color: #bbb;
    background: transparent;
    cursor: pointer;
  }

  button:hover:not(:disabled) {
    color: #fff;
    background: #333;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .icon-btn {
    padding: 3px 7px;
    border-radius: 3px;
  }

  .file-heading {
    display: flex;
    align-items: center;
    border-bottom: 1px solid #333;
  }

  .file-heading button {
    flex: 1;
    padding: 7px 10px;
    text-align: left;
  }

  .file-heading span {
    padding-right: 10px;
    color: #888;
  }

  .mutation-bar {
    padding: 5px 8px;
    color: #888;
    border-bottom: 1px solid #333;
  }

  .mutation-bar button,
  .commit-btn {
    padding: 4px 8px;
    border: 1px solid #444;
    border-radius: 3px;
    background: #2d2d2d;
  }

  .file-list {
    flex: 0 1 30%;
    min-height: 80px;
    overflow: auto;
    border-bottom: 1px solid #444;
  }

  .file-row {
    display: flex;
    align-items: center;
    width: 100%;
    padding-left: 8px;
    font-family: Menlo, monospace;
  }

  .file-row input {
    flex: 0 0 auto;
    margin: 0 5px 0 0;
  }

  .file-select {
    display: flex;
    flex: 1;
    min-width: 0;
    gap: 8px;
    padding: 4px 10px 4px 3px;
    text-align: left;
  }

  .active {
    color: #fff !important;
    background: #094771 !important;
  }

  .status-code {
    flex: 0 0 2.2em;
    color: #dcdcaa;
    white-space: pre;
  }

  .file-path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .commit-box {
    flex: 0 0 auto;
    padding: 7px 8px;
    border-bottom: 1px solid #444;
    background: #252526;
  }

  .commit-box textarea {
    flex: 1;
    min-width: 0;
    resize: vertical;
    border: 1px solid #444;
    border-radius: 3px;
    padding: 5px 6px;
    background: #181818;
    color: #ddd;
    font: 11px/1.4 Menlo, monospace;
  }

  .commit-btn {
    align-self: stretch;
    color: #ddd;
  }

  .operation-message {
    padding: 5px 8px;
    color: #9cdcfe;
    border-bottom: 1px solid #333;
  }

  .diff-tabs {
    display: flex;
    flex: 0 0 auto;
    border-bottom: 1px solid #444;
  }

  .diff-tabs button {
    flex: 1;
    padding: 6px;
  }

  .diff-view {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 10px;
    overflow: auto;
    background: #181818;
    color: #d4d4d4;
    font: 11px/1.45 Menlo, monospace;
    white-space: pre;
    tab-size: 4;
  }

  .empty,
  .git-error,
  .warning {
    padding: 12px;
    color: #888;
  }

  .git-error {
    color: #f1b0b0;
    white-space: pre-wrap;
  }

  .warning {
    color: #d7ba7d;
  }
</style>
