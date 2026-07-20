// UI localization (frontend only). English is the base dictionary; Japanese
// mirrors it 1:1 (the `ja: Messages` annotation makes a missing/extra key a
// type error). Backend (Rust) strings are NOT translated — they pass through
// verbatim. Comments and log/PTY output are out of scope by design.
//
// Locale resolution: the persisted setting is "auto" | "en" | "ja"
// (localStorage `ptygrid.locale`; absent = "auto"). "auto" follows the
// system language (navigator.languages), mapping ja* → Japanese and
// everything else → English.

export type Locale = "en" | "ja";
export type LocaleSetting = Locale | "auto";

const LOCALE_KEY = "ptygrid.locale";

function systemLocale(): Locale {
  try {
    const langs =
      navigator.languages && navigator.languages.length > 0
        ? navigator.languages
        : [navigator.language];
    for (const lang of langs) {
      if (lang && lang.toLowerCase().startsWith("ja")) return "ja";
    }
  } catch {
    // navigator unavailable (SSR/tests): fall through to English.
  }
  return "en";
}

function loadSetting(): LocaleSetting {
  try {
    const raw = localStorage.getItem(LOCALE_KEY);
    return raw === "en" || raw === "ja" ? raw : "auto";
  } catch {
    return "auto";
  }
}

/** Reactive locale state. Read via `msg()` / `currentLocale()`; write via
 * `setLocaleSetting()`. Exported as an object so the $state stays reactive
 * across module boundaries (Svelte 5 runes in .svelte.ts). */
export const i18n = $state<{ setting: LocaleSetting }>({
  setting: loadSetting(),
});

export function setLocaleSetting(setting: LocaleSetting): void {
  i18n.setting = setting;
  try {
    if (setting === "auto") localStorage.removeItem(LOCALE_KEY);
    else localStorage.setItem(LOCALE_KEY, setting);
  } catch {
    // localStorage unavailable/full: the choice still applies this session.
  }
}

export function currentLocale(): Locale {
  return i18n.setting === "auto" ? systemLocale() : i18n.setting;
}

// ---------------------------------------------------------------------------
// Dictionaries. Grouped by UI area; interpolated strings are functions so
// word order can differ per language.
// ---------------------------------------------------------------------------

const en = {
  // ---- toolbar ----
  tbTerminal: "Terminal",
  tbLayout: "Layout",
  tbWorkingFolder: "Working folder",
  ariaAddShells: "Add shells",
  ariaColumns: "Columns",
  layoutAuto: "Auto",
  layoutAutoHint: "Grid shape follows pane count",
  layout1: "1 col",
  layout1Hint: "Stack panes vertically",
  layout2: "2 cols",
  layout2Hint: "Wrap after 2 columns",
  layout3: "3 cols",
  layout3Hint: "Wrap after 3 columns",
  shellAddHint: (count: number) =>
    count === 1 ? "Open 1 shell pane" : `Open ${count} shell panes at once`,
  dirPlaceholder: "Working folder (e.g. ~/works/hoge; leading ~ ok)",
  dirInputTitle:
    "Enter the working-folder path (a leading ~ expands to home).\n" +
    "ptygrid.yml is searched in: working folder → launch folder → ~/.ptygrid.",
  btnLoad: "Load",
  btnLoading: "Loading…",
  configBadge: (origin: string) => `Config: ${origin}`,
  originProject: "project folder",
  originLaunch: "launch folder",
  originGlobal: "~/.ptygrid",
  originDefault: "built-in default",
  originBadgeTitleDefault: (path: string, dir: string) =>
    `No config file (using the built-in default).\nCreate ${path} and it will be loaded automatically.\nWorking folder: ${dir}`,
  originBadgeTitle: (path: string, dir: string) =>
    `Config file: ${path}\nWorking folder: ${dir}`,
  unnamedProject: "(unnamed)",
  runAgentTitle: (name: string) => `Start agent ${name}`,
  runProcessTitle: (name: string) => `Start process ${name}`,
  teamChipTitle: (name: string, members: string) =>
    `Launch team ${name}\nMembers: ${members}`,

  // ---- generic ----
  btnClose: "Close",
  btnCancel: "Cancel",
  btnStop: "Stop",
  btnReload: "Reload",
  titleMaximize: "Maximize",
  titleUnmaximize: "Restore",
  titleRestart: "Restart",
  tauriOnly: "Available only in the Tauri runtime.",

  // ---- errors / banners ----
  clipboardCopyFailed: (err: unknown) => `Failed to copy to clipboard: ${err}`,
  spawnShellFailed: (err: unknown) =>
    `Failed to start a shell (spawn_shell): ${err}`,
  paneLimitReached: (max: number) => `Pane limit (${max}) reached.`,
  paneLimitReachedClose: (max: number) =>
    `Pane limit (${max}) reached. Close an existing pane first.`,
  openShellsCapped: (remaining: number, requested: number) =>
    `Only ${remaining} slots left; opening ${remaining} panes instead of ${requested}.`,
  spawnAgentFailed: (name: string, err: unknown) =>
    `Failed to start "${name}" (spawn_agent): ${err}`,
  spawnTeamFailed: (name: string, err: unknown) =>
    `Failed to launch team "${name}" (spawn_team): ${err}`,
  teamMembersFailed: (name: string, members: string) =>
    `Team ${name}: failed to start ${members}.`,
  unknownError: "unknown error",
  restartFailed: (err: unknown) => `Restart failed (restart_session): ${err}`,
  killPaneFailed: (id: number, err: unknown) =>
    `Failed to stop the pane (kill_pty #${id}): ${err}`,
  stopTeammateFailed: (id: number, err: unknown) =>
    `Failed to stop the teammate (kill_pty #${id}): ${err}`,
  trustFailed: (err: unknown) =>
    `Failed to trust the folder (trust_working_folder): ${err}`,
  cdBroadcastFailed: (id: number, err: unknown) =>
    `Failed to broadcast cd (write_pty #${id}): ${err}`,
  saveStateFailed: (err: unknown) =>
    `Failed to save the project state: ${err}`,
  restorePrevFailed: (err: unknown) =>
    `Could not restore the previous project state: ${err}`,
  restoreSomeFailed: (details: string) =>
    `Some sessions could not be restored: ${details}`,
  restoreSkippedForCap: (max: number, skipped: number) =>
    `${skipped} session(s) not restored due to the pane limit (${max})`,
  loadConfigNeedsTauri:
    "Loading a config (load_config) requires the Tauri runtime.",
  queenSpawnPaneLimit: (label: string, max: number) =>
    `Queen started "${label}", but it cannot be shown: pane limit (${max}) reached.`,

  // ---- notices / toasts ----
  queenCmdCopied:
    "Register command copied (remove→add, idempotent. Survives restarts; safe to re-run).",
  codexSnippetCopied:
    "codex snippet copied (paste into ~/.codex/config.toml. Uses the QUEEN_TOKEN env, so it survives token regeneration).",
  grokSnippetCopied:
    "grok snippet copied (paste into ~/.grok/config.toml. Uses the QUEEN_TOKEN env, so it survives token regeneration).",
  universalUrlCopied:
    "Token-bearing URL copied. Paste it into any MCP client's HTTP server URL field (no headers/env needed). Re-copy after regenerating the token.",
  universalJsonCopied:
    "Standard mcpServers JSON copied (type: http, token embedded in the URL). Works with most JSON-config tools (Cursor / Cline / VS Code / Gemini CLI).",
  rawValuesCopied:
    "Raw values copied (endpoint URL / token / QUEEN_TOKEN env name / token-bearing URL) for hand-configuring any tool.",
  hooksSnippetCopied: "hooks settings snippet copied",
  hooksRegistered: "Registered in settings.json",
  hooksAlreadyCurrent: "settings.json is already up to date",
  hooksRegisterFailed: (err: unknown) =>
    `Failed to register hooks (register_teammate_hooks): ${err}`,
  tokensRegenerated: (labels: string) =>
    `Regenerated the ${labels} token(s). Re-registration required (hook: register settings.json / Queen: copy the register command).`,
  tokenRegenFailed: (err: unknown) =>
    `Failed to regenerate tokens (regenerate_auth_tokens): ${err}`,
  teamNoticeTitle: (name: string) => `Team ${name}`,
  lblStarted: "started",
  lblSkippedExisting: "existing",
  lblFailed: "failed",
  lblStandby: "standby",
  kickoffSent: ", kickoff sent to lead",
  loadedNotice: (dir: string, origin: string, cdPart: string) =>
    `Working folder: ${dir} (config: ${origin}) / ${cdPart}`,
  cdSent: (n: number) => `sent cd to ${n} pane(s)`,
  cdNoTargets: "no panes to cd",
  configChanged: "The config file (ptygrid.yml) has changed",

  // ---- trust prompt ----
  trustAria: "Trust confirmation for this folder",
  trustText: (dir: string) =>
    `The config in this folder (${dir}) has not been reviewed. Auto-start its defined commands?`,
  btnTrust: "Trust & start",
  titleTrust: "Trust this folder and start its autostart entries",
  btnLater: "Later",
  titleLater:
    "Close without starting (agents can still be started manually from the chips)",

  // ---- Queen badge / panel ----
  queenAria: "Queen MCP server status (click for the register menu)",
  queenTooltipNoTauri: "No Tauri runtime (demo mode)",
  queenTooltipUnknown: "Queen MCP server (status not fetched)",
  queenTooltipDisabled:
    "Queen is disabled (queen.enabled: false in ptygrid.yml)",
  queenTooltipError: (err: string) => `Error: ${err}`,
  queenTooltipStopped: "stopped",
  queenTooltipClickHint:
    "Click to copy the register command with the token (remove→add, idempotent. Survives restarts; safe to re-run).",
  queenTooltipFallback: "Queen MCP server",
  queenPanelTitle: "Queen MCP registration",
  queenPanelIntro:
    "Copy the command/snippet that registers Queen in each agent CLI. The token is persisted and survives restarts.",
  btnClaudeCmd: "claude register command",
  titleClaudeCmd: "claude mcp add (remove→add, idempotent)",
  btnCodexSnippet: "codex snippet",
  titleCodexSnippet: "Snippet for ~/.codex/config.toml (reads QUEEN_TOKEN env)",
  btnGrokSnippet: "grok snippet",
  titleGrokSnippet: "Snippet for ~/.grok/config.toml (reads QUEEN_TOKEN env)",
  queenPanelFootnote:
    "codex / grok read the QUEEN_TOKEN env, so they stay valid after token regeneration. Re-copy the claude command after regenerating.",
  queenPanelUniversalLabel: "Generic (for other / new agent CLIs):",
  btnUniversalUrl: "copy URL",
  titleUniversalUrl:
    "Token-bearing URL — paste into any MCP client that takes an HTTP URL (no headers/env needed)",
  btnUniversalJson: "copy JSON",
  titleUniversalJson:
    "Standard mcpServers JSON (type: http, token in URL) for Cursor / Cline / VS Code / Gemini CLI, etc.",
  btnRawValues: "copy raw values",
  titleRawValues:
    "Endpoint URL / token / QUEEN_TOKEN env / token-bearing URL for hand configuration",
  queenPanelUniversalNote:
    "The URL form is the most portable. For a regeneration-proof setup on TOML CLIs, prefer the codex / grok snippets (QUEEN_TOKEN env).",

  // ---- Teammates badge / panel ----
  teammatesAria: "Teammate hooks (click for the settings panel)",
  tmTooltipFallback:
    "host: fallback active (failed to host native panes; downgraded to observe)",
  tmTooltipUnknown: "Teammate hooks (status not fetched)",
  tmTooltipEnabled: "Teammate hooks enabled (click to configure)",
  tmTooltipDisabled:
    "Teammate hooks disabled (set teammates.enabled: true in ptygrid.yml)",
  tmPanelTitle: "Teammate hooks",
  tmFetching: "Fetching status…",
  tmStatusLine: (enabled: boolean, notifications: boolean, port: number) =>
    `State: ${enabled ? "enabled" : "disabled"} · notifications: ${notifications ? "on" : "off"} · port :${port}`,
  btnCopySnippet: "Copy snippet",
  btnRegisterSettings: "Register in settings.json (user)",
  btnRegistering: "Registering…",
  titleRegisterSettings: "Register in ~/.claude/settings.json",
  tmTokenNote:
    "The token is saved and survives restarts. Register once (re-register only after regenerating).",
  btnRegenHook: "Regenerate hook token",
  titleRegenHook:
    "Regenerate the hook token (rotation after a leak. Re-register settings.json afterwards)",
  btnRegenQueen: "Regenerate Queen token",
  titleRegenQueen:
    "Regenerate the Queen /mcp token (rotation after a leak. Re-register the MCP afterwards)",
  btnCloseFinished: (n: number) => `Close finished panes (${n})`,
  titleCloseFinished:
    "Close all finished teammate/transcript panes at once (does not affect the processes)",
  tmHostHead: "host mode (real PTY teammates)",
  tmNoHostLeads:
    "No active host leads (teams.mode: host in ptygrid.yml)",
  tmLeadBadgeFallback: "host: fallback active",
  tmLeadBadgeHost: "host",
  tmNoTeammates: "no teammates",
  tmPaneless: "(off-grid)",
  btnShowOnGrid: "Show on grid",
  titleShowOnGrid: "Show this teammate on the grid",
  tmOrphanHead: "lead exited (orphaned teammates)",
  titleStopOrphan: "Stop this orphaned teammate process",
  tmEventsHead: "Recent events",
  tmNoEvents: "No events yet",
  teammateKindText: {
    "subagent-start": "started",
    "subagent-stop": "stopped",
    "teammate-idle": "idle",
    "task-created": "task created",
    "task-completed": "task completed",
  } as Record<string, string>,
  teammateLifecycleToast: (who: string, kind: string) => {
    const label =
      (
        {
          "subagent-start": "started",
          "subagent-stop": "stopped",
          "teammate-idle": "went idle",
          "task-created": "created a task",
          "task-completed": "completed a task",
        } as Record<string, string>
      )[kind] ?? kind;
    return `🤝 teammate ${who} ${label}`;
  },
  hostFallbackNoticeTitle: "Could not host the teammate",
  hostFallbackNoticeMsg:
    "Failed to host it as a native pane; fell back to the read-only view.",

  // ---- panes ----
  emptyHint:
    "No panes — open a shell with “+1” in the toolbar, or start an agent.",
  transcriptROTitle: "read-only transcript (observe only, no input)",
  leadRefTitle: "parent lead session",
  finishedTag: "exited",
  finishedTagSubTitle: "The subagent has exited (closing has no effect on it)",
  finishedTagTeammateTitle: "The teammate has exited",
  titleCloseTranscript:
    "Close (stops the tail only; does not affect the subagent)",
  hostPTYTitle: "host teammate (real PTY, interactive)",
  killConfirmText: "Stop this teammate?",
  titleCloseTeammate: "Close (stops the teammate process)",
  titleClosePane: "Close (ends the session)",

  // ---- semantic status ----
  astatusBlocked: "blocked (waiting for approval)",
  astatusWorking: "working (running)",
  astatusDone: "done (finished)",
  astatusIdle: "idle (waiting)",
  astatusUnknown: "unknown (not evaluated)",

  // ---- dock / status sidebar ----
  dockAria: "Left dock (status / Git)",
  dockCollapse: "Collapse the dock",
  tabStatus: "Status",
  tabGit: "Git",
  dockResizeAria: "Resize the dock",
  ssAria: "Status list",
  ssEmpty: "No running panes",
  ssMaxToggle: "Toggle maximize",

  // ---- footer ----
  sbCollapse: "Collapse the status sidebar",
  sbOpen: "Open the status sidebar",
  sbLabel: "Sidebar",
  sbBlockedTitle: (n: number) => `${n} pane(s) blocked (waiting for approval)`,
  paneCount: (n: number, max: number) => `${n}/${max} panes`,

  // ---- settings menu ----
  settingsTitle: "Settings",
  settingsAria: "App settings",
  settingsLanguage: "Language",
  langAuto: "Auto (system)",
  langEn: "English",
  langJa: "日本語",

  // ---- Git panel ----
  gitRefresh: "Refresh",
  gitAllChanges: "All changes",
  gitSelectedCount: (n: number) => `${n} selected`,
  gitNoChanges: "No changes",
  gitTruncated: "List truncated at 10,000 files",
  gitSelectAria: (path: string) => `Select ${path} for stage/unstage`,
  gitMutated: (n: number, staged: boolean) =>
    staged ? `Staged ${n} file(s).` : `Unstaged ${n} file(s).`,
  gitCommitTitleReady: "Commit the currently staged changes",
  gitCommitTitleEmpty: "No staged changes",
  gitWorking: "Working…",
  gitLoading: "Loading…",
  gitNoDiff: "No diff in this scope",

  // ---- transcript pane ----
  trSubStopped:
    "This subagent has stopped (no further transcript output).",
  trWaiting: "Waiting for transcript… (read-only, observe only)",
};

export type Messages = typeof en;

const ja: Messages = {
  // ---- toolbar ----
  tbTerminal: "ターミナル",
  tbLayout: "レイアウト",
  tbWorkingFolder: "作業フォルダ",
  ariaAddShells: "シェル追加",
  ariaColumns: "列数",
  layoutAuto: "自動",
  layoutAutoHint: "枚数に応じて格子配置",
  layout1: "1列",
  layout1Hint: "縦に積む",
  layout2: "2列",
  layout2Hint: "2列で折り返し",
  layout3: "3列",
  layout3Hint: "3列で折り返し",
  shellAddHint: (count: number) =>
    count === 1 ? "シェルを1面追加" : `シェルを${count}面まとめて追加`,
  dirPlaceholder: "作業フォルダ（例: ~/works/hoge。先頭 ~ 可）",
  dirInputTitle:
    "作業フォルダのパスを入力します（先頭 ~ はホーム展開）。\n" +
    "設定ファイル ptygrid.yml は 作業フォルダ内 → 起動フォルダ → ~/.ptygrid の順に探します。",
  btnLoad: "読み込み",
  btnLoading: "読み込み中…",
  configBadge: (origin: string) => `設定: ${origin}`,
  originProject: "プロジェクト内",
  originLaunch: "起動フォルダ",
  originGlobal: "~/.ptygrid",
  originDefault: "既定",
  originBadgeTitleDefault: (path: string, dir: string) =>
    `設定ファイルなし（組み込みの既定設定）。\n${path} を作成すると自動で読み込みます。\n作業フォルダ: ${dir}`,
  originBadgeTitle: (path: string, dir: string) =>
    `設定ファイル: ${path}\n作業フォルダ: ${dir}`,
  unnamedProject: "（名称未設定）",
  runAgentTitle: (name: string) => `エージェント ${name} を起動`,
  runProcessTitle: (name: string) => `プロセス ${name} を起動`,
  teamChipTitle: (name: string, members: string) =>
    `チーム ${name} を一括起動\nメンバー: ${members}`,

  // ---- generic ----
  btnClose: "閉じる",
  btnCancel: "取消",
  btnStop: "停止",
  btnReload: "再読み込み",
  titleMaximize: "最大化",
  titleUnmaximize: "最大化解除",
  titleRestart: "再起動",
  tauriOnly: "Tauri 実行環境でのみ利用できます。",

  // ---- errors / banners ----
  clipboardCopyFailed: (err: unknown) =>
    `クリップボードへのコピーに失敗しました: ${err}`,
  spawnShellFailed: (err: unknown) =>
    `シェルの起動に失敗しました (spawn_shell): ${err}`,
  paneLimitReached: (max: number) =>
    `ペイン数が上限（${max}）に達しています。`,
  paneLimitReachedClose: (max: number) =>
    `ペイン数が上限（${max}）に達しています。既存のペインを閉じてから表示してください。`,
  openShellsCapped: (remaining: number, requested: number) =>
    `空きが ${remaining} 面のため、${requested} 面ではなく ${remaining} 面だけ開きます。`,
  spawnAgentFailed: (name: string, err: unknown) =>
    `「${name}」の起動に失敗しました (spawn_agent): ${err}`,
  spawnTeamFailed: (name: string, err: unknown) =>
    `チーム「${name}」の起動に失敗しました (spawn_team): ${err}`,
  teamMembersFailed: (name: string, members: string) =>
    `チーム ${name}: ${members} の起動に失敗しました。`,
  unknownError: "不明なエラー",
  restartFailed: (err: unknown) =>
    `再起動に失敗しました (restart_session): ${err}`,
  killPaneFailed: (id: number, err: unknown) =>
    `ペインの停止に失敗しました (kill_pty #${id}): ${err}`,
  stopTeammateFailed: (id: number, err: unknown) =>
    `teammate の停止に失敗しました (kill_pty #${id}): ${err}`,
  trustFailed: (err: unknown) =>
    `フォルダの信頼設定に失敗しました (trust_working_folder): ${err}`,
  cdBroadcastFailed: (id: number, err: unknown) =>
    `cd の一括送信に失敗しました (write_pty #${id}): ${err}`,
  saveStateFailed: (err: unknown) =>
    `プロジェクト状態の保存に失敗しました: ${err}`,
  restorePrevFailed: (err: unknown) =>
    `前回のプロジェクト状態を復元できませんでした: ${err}`,
  restoreSomeFailed: (details: string) =>
    `一部のセッションを復元できませんでした: ${details}`,
  restoreSkippedForCap: (max: number, skipped: number) =>
    `ペイン上限(${max})のため${skipped}件を復元しませんでした`,
  loadConfigNeedsTauri:
    "設定の読み込み (load_config) には Tauri 実行環境が必要です。",
  queenSpawnPaneLimit: (label: string, max: number) =>
    `Queen が「${label}」を起動しましたが、ペイン上限(${max})のため表示できません。`,

  // ---- notices / toasts ----
  queenCmdCopied:
    "登録コマンドをコピーしました（remove→add で冪等。再起動後も有効、再クリックでも安全に再登録）",
  codexSnippetCopied:
    "codex スニペットをコピーしました（~/.codex/config.toml に貼付。QUEEN_TOKEN env 参照でトークン再生成後も有効）",
  grokSnippetCopied:
    "grok スニペットをコピーしました（~/.grok/config.toml に貼付。QUEEN_TOKEN env 参照でトークン再生成後も有効）",
  universalUrlCopied:
    "token 込み URL をコピーしました（任意の MCP クライアントの HTTP URL 欄に貼付。ヘッダ/env 不要。再生成時は再コピー）",
  universalJsonCopied:
    "標準 mcpServers JSON をコピーしました（type: http、URL に token 埋め込み。Cursor / Cline / VS Code / Gemini CLI 等の JSON 設定向け）",
  rawValuesCopied:
    "生の値をコピーしました（エンドポイント URL / token / QUEEN_TOKEN env 変数名 / token 込み URL）。未対応形式のツールに手貼り用",
  hooksSnippetCopied: "hooks 設定スニペットをコピーしました",
  hooksRegistered: "settings.json に登録しました",
  hooksAlreadyCurrent: "settings.json は既に最新です",
  hooksRegisterFailed: (err: unknown) =>
    `hooks の登録に失敗しました (register_teammate_hooks): ${err}`,
  tokensRegenerated: (labels: string) =>
    `${labels} トークンを再生成しました。再登録が必要です（hook: settings.json へ登録 / Queen: 登録コマンドをコピー）。`,
  tokenRegenFailed: (err: unknown) =>
    `トークンの再生成に失敗しました (regenerate_auth_tokens): ${err}`,
  teamNoticeTitle: (name: string) => `チーム ${name}`,
  lblStarted: "起動",
  lblSkippedExisting: "既存",
  lblFailed: "失敗",
  lblStandby: "待機",
  kickoffSent: "、kickoff を lead へ送信",
  loadedNotice: (dir: string, origin: string, cdPart: string) =>
    `作業フォルダ: ${dir}（設定: ${origin}） / ${cdPart}`,
  cdSent: (n: number) => `${n}ペインに cd を送信`,
  cdNoTargets: "cd 対象のペインなし",
  configChanged: "設定ファイル（ptygrid.yml）が変更されました",

  // ---- trust prompt ----
  trustAria: "フォルダの信頼確認",
  trustText: (dir: string) =>
    `このフォルダ（${dir}）の設定は未確認です。定義されたコマンドを自動起動しますか？`,
  btnTrust: "信頼して起動",
  titleTrust: "このフォルダを信頼し、autostart 対象を起動します",
  btnLater: "後で",
  titleLater: "起動せずに閉じる（エージェントチップから手動起動は可能）",

  // ---- Queen badge / panel ----
  queenAria: "Queen MCP サーバー状態（クリックで登録メニュー）",
  queenTooltipNoTauri: "Tauri 実行環境なし（デモモード）",
  queenTooltipUnknown: "Queen MCP サーバー（状態未取得）",
  queenTooltipDisabled:
    "Queen は無効です（ptygrid.yml の queen.enabled: false）",
  queenTooltipError: (err: string) => `エラー: ${err}`,
  queenTooltipStopped: "停止中",
  queenTooltipClickHint:
    "クリックで token 込み登録コマンドをコピー（remove→add で冪等。再起動後も有効。再クリックでも安全に再登録）",
  queenTooltipFallback: "Queen MCP サーバー",
  queenPanelTitle: "Queen MCP 登録",
  queenPanelIntro:
    "各エージェント CLI に Queen を登録するコマンド/スニペットをコピーします。トークンは永続化され再起動後も有効です。",
  btnClaudeCmd: "claude 登録コマンド",
  titleClaudeCmd: "claude mcp add（remove→add で冪等）",
  btnCodexSnippet: "codex スニペット",
  titleCodexSnippet:
    "~/.codex/config.toml 用スニペット（QUEEN_TOKEN env 参照）",
  btnGrokSnippet: "grok スニペット",
  titleGrokSnippet:
    "~/.grok/config.toml 用スニペット（QUEEN_TOKEN env 参照）",
  queenPanelFootnote:
    "codex / grok は QUEEN_TOKEN env 参照なのでトークン再生成後もそのまま有効。claude は再生成時に再コピーしてください。",
  queenPanelUniversalLabel: "汎用（他の / 新しいエージェント CLI 向け）:",
  btnUniversalUrl: "URL をコピー",
  titleUniversalUrl:
    "token 込み URL。HTTP URL を受け付ける任意の MCP クライアントに貼れる（ヘッダ/env 不要）",
  btnUniversalJson: "JSON をコピー",
  titleUniversalJson:
    "標準 mcpServers JSON（type: http、URL に token）。Cursor / Cline / VS Code / Gemini CLI 等向け",
  btnRawValues: "生の値をコピー",
  titleRawValues:
    "エンドポイント URL / token / QUEEN_TOKEN env / token 込み URL（手動設定用）",
  queenPanelUniversalNote:
    "URL 形式が最も汎用。TOML 系 CLI で再生成にも強くしたい場合は codex / grok スニペット（QUEEN_TOKEN env 参照）を推奨。",

  // ---- Teammates badge / panel ----
  teammatesAria: "Teammate hooks（クリックで設定パネル）",
  tmTooltipFallback:
    "host: フォールバック中（ネイティブペイン化に失敗し observe へ降格）",
  tmTooltipUnknown: "Teammate hooks（状態未取得）",
  tmTooltipEnabled: "Teammate hooks 有効（クリックで設定）",
  tmTooltipDisabled:
    "Teammate hooks 無効（ptygrid.yml の teammates.enabled: true で有効化）",
  tmPanelTitle: "Teammate hooks",
  tmFetching: "状態を取得中…",
  tmStatusLine: (enabled: boolean, notifications: boolean, port: number) =>
    `状態: ${enabled ? "有効" : "無効"} · 通知: ${notifications ? "オン" : "オフ"} · ポート :${port}`,
  btnCopySnippet: "スニペットをコピー",
  btnRegisterSettings: "settings.json へ登録 (user)",
  btnRegistering: "登録中…",
  titleRegisterSettings: "~/.claude/settings.json へ登録",
  tmTokenNote:
    "トークンは保存され、再起動後も有効です。初回のみ登録が必要（再生成したときだけ再登録）。",
  btnRegenHook: "hook トークン再生成",
  titleRegenHook:
    "hook トークンを再生成（漏洩時のローテーション用。再生成後は settings.json の再登録が必要）",
  btnRegenQueen: "Queen トークン再生成",
  titleRegenQueen:
    "Queen /mcp トークンを再生成（漏洩時のローテーション用。再生成後は MCP の再登録が必要）",
  btnCloseFinished: (n: number) => `終了したペインを一括で閉じる（${n}）`,
  titleCloseFinished:
    "終了した teammate / transcript ペインをまとめて閉じます（実体には影響しません）",
  tmHostHead: "host モード（実 PTY teammate）",
  tmNoHostLeads:
    "稼働中の host lead はありません（ptygrid.yml の teams.mode: host）",
  tmLeadBadgeFallback: "host: フォールバック中",
  tmLeadBadgeHost: "host",
  tmNoTeammates: "teammate なし",
  tmPaneless: "（グリッド外）",
  btnShowOnGrid: "グリッドへ表示",
  titleShowOnGrid: "このteammateをグリッドに表示",
  tmOrphanHead: "lead 終了済み（孤立 teammate）",
  titleStopOrphan: "この孤立 teammate プロセスを停止",
  tmEventsHead: "直近のイベント",
  tmNoEvents: "まだイベントはありません",
  teammateKindText: {
    "subagent-start": "起動",
    "subagent-stop": "停止",
    "teammate-idle": "アイドル",
    "task-created": "タスク作成",
    "task-completed": "タスク完了",
  } as Record<string, string>,
  teammateLifecycleToast: (who: string, kind: string) => {
    const label =
      (
        {
          "subagent-start": "が起動",
          "subagent-stop": "が停止",
          "teammate-idle": "がアイドル",
          "task-created": "のタスクを作成",
          "task-completed": "のタスクが完了",
        } as Record<string, string>
      )[kind] ?? kind;
    return `🤝 teammate ${who} ${label}`;
  },
  hostFallbackNoticeTitle: "teammate を host できませんでした",
  hostFallbackNoticeMsg:
    "ネイティブペインにホストできず、読み取り専用ビューにフォールバックしました。",

  // ---- panes ----
  emptyHint:
    "ペインがありません — ツールバーの「＋1」でシェルを開くか、エージェントを起動してください。",
  transcriptROTitle: "read-only transcript（観測のみ・入力不可）",
  leadRefTitle: "親 lead セッション",
  finishedTag: "終了",
  finishedTagSubTitle: "subagent は終了しました（閉じても影響なし）",
  finishedTagTeammateTitle: "teammate は終了しました",
  titleCloseTranscript:
    "閉じる（tail 停止のみ・subagent には影響しません）",
  hostPTYTitle: "host teammate（実 PTY・対話可能）",
  killConfirmText: "teammate を停止しますか？",
  titleCloseTeammate: "閉じる（teammate プロセスを停止します）",
  titleClosePane: "閉じる（セッション終了）",

  // ---- semantic status ----
  astatusBlocked: "blocked（承認待ち）",
  astatusWorking: "working（実行中）",
  astatusDone: "done（完了）",
  astatusIdle: "idle（待機）",
  astatusUnknown: "unknown（判定なし）",

  // ---- dock / status sidebar ----
  dockAria: "左ドック（ステータス / Git）",
  dockCollapse: "ドックを畳む",
  tabStatus: "ステータス",
  tabGit: "Git",
  dockResizeAria: "ドックの幅を変更",
  ssAria: "ステータス一覧",
  ssEmpty: "実行中のペインはありません",
  ssMaxToggle: "最大化トグル",

  // ---- footer ----
  sbCollapse: "ステータスサイドバーを畳む",
  sbOpen: "ステータスサイドバーを開く",
  sbLabel: "サイドバー",
  sbBlockedTitle: (n: number) => `${n} ペインが承認待ち（blocked）`,
  paneCount: (n: number, max: number) => `${n}/${max} ペイン`,

  // ---- settings menu ----
  settingsTitle: "設定",
  settingsAria: "アプリ設定",
  settingsLanguage: "言語",
  langAuto: "自動（システム）",
  langEn: "English",
  langJa: "日本語",

  // ---- Git panel ----
  gitRefresh: "更新",
  gitAllChanges: "すべての変更",
  gitSelectedCount: (n: number) => `${n}件選択`,
  gitNoChanges: "変更はありません",
  gitTruncated: "10,000ファイルで表示を打ち切りました",
  gitSelectAria: (path: string) => `${path}を操作対象に選択`,
  gitMutated: (n: number, staged: boolean) =>
    `${n}件を${staged ? "stage" : "unstage"}しました。`,
  gitCommitTitleReady: "現在stageされている変更をcommit",
  gitCommitTitleEmpty: "stageされた変更がありません",
  gitWorking: "処理中…",
  gitLoading: "読み込み中…",
  gitNoDiff: "この範囲にdiffはありません",

  // ---- transcript pane ----
  trSubStopped:
    "この subagent は停止しました（transcript の追記はありません）。",
  trWaiting: "transcript を待機中…（read-only・観測のみ）",
};

const DICTS: Record<Locale, Messages> = { en, ja };

/** Current dictionary. Reading this inside a component's template / $derived
 * tracks the locale setting, so the UI re-renders on switch. */
export function msg(): Messages {
  return DICTS[currentLocale()];
}
