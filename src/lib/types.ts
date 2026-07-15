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
};

export type SessionInfo = {
  id: number;
  name?: string;
  cmd: string;
  state: SessionState;
  code?: number | null;
  /** フォアグラウンドプロセス名（list_sessions取得時のみ。イベントでは省略） */
  foreground?: string;
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
