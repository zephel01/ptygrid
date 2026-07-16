// TS types per CONTRACT.md (Phase 1 追加契約) — do not change shapes.

export type ConfigInfo = { path: string; config: Config };

export type Config = {
  project?: string;
  agents: AgentDef[];
  processes: AgentDef[];
};

export type AgentDef = {
  name: string;
  cmd: string;
  cwd?: string;
  env?: Record<string, string>;
  autostart?: boolean;
  autorestart?: "never" | "on-failure" | "always";
  instructions?: string;
  worktree?: WorktreeConfig;
};

export type WorktreeConfig = {
  enabled?: boolean;
  base?: string;
  setup?: string;
};

export type SessionInfo = {
  id: number;
  name?: string;
  cmd: string;
  state: SessionState;
  code?: number | null;
  /** フォアグラウンドプロセス名（list_sessions取得時のみ。イベントでは省略） */
  foreground?: string;
  worktree?: WorktreeInfo;
};

export type WorktreeInfo = {
  name: string;
  repoRoot: string;
  path: string;
  branch: string;
  base: string;
  locked: boolean;
};

export type SessionState = "starting" | "running" | "exited" | "restarting";

export type PtyOutputPayload = { id: number; data: string };
export type PtyExitPayload = { id: number; code: number | null };
export type ConfigChangedPayload = { path: string };

// Phase 2 (Queen: 内蔵MCPサーバー)
export type QueenStatus = {
  enabled: boolean;
  running: boolean;
  port?: number;
  url?: string;
  error?: string;
};

export type QueenNotifyPayload = { title: string; message: string };

// Phase 3.1 (read-only Git status/diff)
export type GitFileStatus = {
  path: string;
  originalPath?: string;
  indexStatus: string;
  worktreeStatus: string;
};

export type GitStatusInfo = {
  repoRoot: string;
  branch?: string;
  head: string;
  files: GitFileStatus[];
  truncated: boolean;
};

export type GitDiffInfo = {
  repoRoot: string;
  path?: string;
  staged: boolean;
  text: string;
  truncated: boolean;
};

export type GitCommitInfo = {
  repoRoot: string;
  oid: string;
  summary: string;
  output: string;
};
