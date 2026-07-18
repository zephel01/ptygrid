// TS types per CONTRACT.md (Phase 1 追加契約) — do not change shapes.

/** どの探索場所から設定ファイルを読んだか（作業フォルダ基準）:
 * project=作業フォルダ内 / launch=アプリ起動フォルダ / global=~/.ptygrid /
 * default=どこにも設定が無く組み込みの既定設定を使用（path は作業フォルダ内の
 * ptygrid.yml 第一候補） */
export type ConfigOrigin = "project" | "launch" | "global" | "default";

/** path=実際に読んだ設定ファイル、dir=作業フォルダ（プロジェクト境界）、
 * origin=path の由来。
 * trusted=自動コマンド実行（autostart / worktree.setup）を許可してよいか
 * （Finding S2）。global/default は常に true、project/launch は該当フォルダが
 * 信頼済み集合にある場合のみ true。false でも load 自体は成功する。 */
export type ConfigInfo = {
  path: string;
  dir: string;
  origin: ConfigOrigin;
  trusted: boolean;
  config: Config;
};

export type Config = {
  project?: string;
  agents: AgentDef[];
  processes: AgentDef[];
  /** Phase 4.4.0: セマンティック状態検出の設定（省略で既定・検出は既定 on） */
  agent_status?: AgentStatusConfig;
  /** Phase 4.3: 名前付きチーム構成（一括起動）。検証は backend の parse 時。 */
  team_presets?: Record<string, TeamPreset>;
};

// Phase 4.3 (Queen team preset: 一括起動)
/** team_presets.<name>。members は agents: 定義名の参照のみ（allowlist 整合）。 */
export type TeamPreset = {
  /** kickoff の宛先。省略時は最初の非 standby メンバー。 */
  lead?: string;
  members: TeamMember[];
  /** 起動後に lead の inbox へ投函される初回メッセージ。 */
  kickoff?: string;
};

export type TeamMember = {
  agent: string;
  /** default false。true はチーム起動時に立ち上げない待機層。 */
  standby?: boolean;
  /** チーム起動時に inbox（mailbox=定義名）へ配送される役割指示。 */
  instructions?: string;
};

/** spawn_team（command / Queen tool 共通）の起動レポート（wire: camelCase）。 */
export type TeamStartReport = {
  preset: string;
  lead?: string;
  members: TeamMemberOutcome[];
  /** kickoff あり かつ started>0 かつ inbox 送信成功のときのみ true。 */
  kickoffDelivered: boolean;
};

export type TeamMemberOutcome = {
  agent: string;
  standby: boolean;
  /** skipped=既に生存中（id は既存セッション）。failed は error 必須。 */
  status: "started" | "skipped" | "failed" | "standby";
  id?: number;
  error?: string;
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

// Phase 4.4.0 (agent-status: セマンティック状態検出)
/** 意味的状態。SessionState（プロセス生死）とは別レイヤの「推定（意見）」。
 * unknown はルールセット未割当／評価前（UI ではバッジ非表示）。 */
export type AgentStatus = "working" | "blocked" | "done" | "idle" | "unknown";

/** agent-status: 状態が変化したときだけ emit（agent_status.enabled 時のみ）。
 * exited 時はイベントを出さず、frontend が session-state:exited で
 * ui.agentStatus[id] を削除する。 */
export type AgentStatusPayload = {
  id: number;
  status: AgentStatus;
  /** マッチしたルールの id（＝正規表現ソース。tooltip / デバッグ用、任意） */
  matchedRule?: string;
  /** 適用したルールセットキー（claude / codex / "*" 等、任意） */
  ruleSet?: string;
};

/** agent_status.patterns.<key>。既定は内蔵ルールへの merge、replace:true で置換。 */
export type AgentStatusPatternSet = {
  replace?: boolean;
  blocked?: string[];
  working?: string[];
  done?: string[];
};

/** グローバル agent_status ブロック（すべて任意）。キーは agent 定義名 or
 * フォアグラウンドプロセス名（+ opt-in の "*"）。wire は snake_case。 */
export type AgentStatusConfig = {
  enabled?: boolean; // default true
  tail_lines?: number; // default 24, clamp 4..200
  debounce_ms?: number; // default 250, clamp 100..2000
  done_linger_ms?: number; // default 6000, clamp 0..60000
  patterns?: Record<string, AgentStatusPatternSet>;
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

/** フォアグラウンドプロセス名の実時間更新（session-resources に相乗り、
 * Phase 4.4.2）。手打ち起動の claude/codex/grok を表示名・状態バッジに反映する。 */
/** Phase 4.4.3: detail は表示用の補足（現状 ssh の接続先 `user@host`。無い場合は省略）。 */
export type SessionForeground = { id: number; name: string; detail?: string };

export type SessionResourcesPayload = {
  sampledAtMs: number;
  sessions: SessionResourceUsage[];
  /** 実行中 PTY セッションごとの foreground プロセス名（解決できたものだけ）。 */
  foreground?: SessionForeground[];
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
