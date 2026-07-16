// TS types per CONTRACT.md (Phase 1 追加契約) — do not change shapes.

export type ConfigInfo = { path: string; dir: string; config: Config };

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
  resume?: string;
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
  /** Phase 4.1: "pty"（既定）| "transcript"（読み取り専用ペイン） */
  kind?: SessionKind;
  /** Phase 4.1: transcript セッションにのみ付与される teammate メタ */
  teammate?: TeammateInfo;
};

export type SessionKind = "pty" | "transcript";

/** Phase 4.1: transcript ペインの teammate メタ（mode は 4.1 では常に observe） */
export type TeammateInfo = {
  role?: string;
  leadId: number;
  mode: "observe";
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

export type LogicalSession =
  | { kind: "definition"; name: string; worktree?: WorktreeInfo }
  | { kind: "shell" };

export type ProjectState = {
  version: 1;
  configDir: string;
  layoutMode: "auto" | "1" | "2" | "3";
  sessions: LogicalSession[];
  maximizedIndex?: number;
};

export type PtyOutputPayload = { id: number; data: string };
export type PtyExitPayload = { id: number; code: number | null };
export type ConfigChangedPayload = { path: string };

export type SessionResourceUsage = {
  id: number;
  cpuPercent: number;
  memoryBytes: number;
  processCount: number;
};

export type SessionResourcesPayload = {
  sampledAtMs: number;
  sessions: SessionResourceUsage[];
};

// Phase 2 (Queen: 内蔵MCPサーバー)
export type QueenStatus = {
  enabled: boolean;
  running: boolean;
  port?: number;
  url?: string;
  error?: string;
};

export type QueenNotifyPayload = { title: string; message: string };

// Phase 4.0 (teammate hooks 受信基盤)
export type TeammateHooksInfo = {
  enabled: boolean;
  hookNotifications: boolean;
  port: number;
  token: string;
  hooksScope: "user" | "project";
};

export type TeammateLifecycleKind =
  | "subagent-start"
  | "subagent-stop"
  | "teammate-idle"
  | "task-created"
  | "task-completed";

export type TeammateLifecyclePayload = {
  kind: TeammateLifecycleKind;
  sessionId?: string;
  agentId?: string;
  agentType?: string;
  taskId?: string;
  taskName?: string;
  status?: string;
  cwd?: string;
};

// Phase 4.1 (observe: read-only transcript ペイン)
/** transcript-output: 追記された整形済みテキストの差分（既存 pty-output とは別イベント） */
export type TranscriptOutputPayload = { id: number; text: string };
/** teammate-banner: ペイン上限超過などのバナー通知 */
export type TeammateBannerPayload = { message: string };

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
