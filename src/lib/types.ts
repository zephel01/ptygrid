// TS types per CONTRACT.md (Phase 1 追加契約) — do not change shapes.

/** どの探索場所から設定ファイルを読んだか（作業フォルダ基準）:
 * project=作業フォルダ内 / launch=アプリ起動フォルダ / global=~/.ptygrid /
 * default=どこにも設定が無く組み込みの既定設定を使用（path は作業フォルダ内の
 * ptygrid.yml 第一候補） */
export type ConfigOrigin = "project" | "launch" | "global" | "default";

/** path=実際に読んだ設定ファイル、dir=作業フォルダ（プロジェクト境界）、
 * origin=path の由来。 */
export type ConfigInfo = {
  path: string;
  dir: string;
  origin: ConfigOrigin;
  config: Config;
};

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
  /** Phase 4.1/4.2: teammate セッション（observe transcript / host PTY）に付与 */
  teammate?: TeammateInfo;
};

export type SessionKind = "pty" | "transcript";

/** teammate メタ。observe=read-only transcript（kind:"transcript"）、
 * host=実 PTY teammate（kind:"pty"、Phase 4.2）。 */
export type TeammateInfo = {
  role?: string;
  leadId: number;
  mode: "observe" | "host";
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
  /** Token-free display URL (tooltip). Not usable for connecting. */
  url?: string;
  /** Per-run `/mcp` auth token; appended to build the register URL. */
  token?: string;
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

// Phase 4.2 (host: 実 PTY teammate ペイン)
/** teammate-focus: tmux select-pane 相当。該当ペインを一時的に強調する */
export type TeammateFocusPayload = { id: number };
/** teammate-fallback: host 未使用で observe 降格したときの通知 */
export type TeammateFallbackPayload = {
  leadId: number;
  agentId: string;
  reason: string;
};
/** teams_host_status: 稼働中 host lead ごとの状態と live teammate 一覧 */
export type TeamsHostStatus = { leads: HostLeadStatus[] };
export type HostLeadStatus = {
  id: number;
  mode: string;
  fallback: boolean;
  /** この lead が所有する live host-teammate セッション id */
  teammates: number[];
};

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
