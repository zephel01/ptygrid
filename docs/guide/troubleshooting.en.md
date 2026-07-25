**English** · [日本語 (Japanese)](troubleshooting.md)

# Troubleshooting

## Can't Connect to Another Pane via Queen MCP (2026-07-16 Incident)

When trying to send a review request to codex from Claude Code (a background job) using
Queen's `send_message` / `read_output`, the tools couldn't be called out of the box.
There were multiple causes, each isolated and fixed as described below. The initial
investigation worked around the issues within the session; the resulting implementation
fixes are summarized at the end.

### 1. Queen's MCP tools aren't present in the session

- **Symptom**: ToolSearch returns "No matching deferred tools found" for
  `mcp__queen__send_message` / `mcp__queen__read_output`.
- **Cause**: The Queen server was registered under a different Claude Code project scope.
  The session's project was `~/works/project/ptygrid`, which had no Queen registration,
  and only `hiveterm` was registered globally. Because the project scope didn't match,
  Queen was never connected as an MCP client and its tools never loaded.
- **Fix (this time)**: Identified Queen's endpoint, `http://127.0.0.1:39237/mcp`
  (Streamable HTTP), from the registration info and hit the MCP protocol directly with
  `curl`.
- **Permanent fix**: To use Queen across directories, register it at user scope with
  `claude mcp add -s user --transport http queen http://127.0.0.1:39237/mcp`. Use
  `-s project` if you only want to share it within a single project. Note: if Queen's
  port isn't fixed, it can change on each startup.
- **Additional symptom**: Even when asked to "have grok #2 do the work", Claude would
  suggest running headless or having the user paste it into another tab themselves.
- **Diagnosis**: This isn't a model-capability problem — `queen` doesn't show up in
  `claude mcp list`, so the tool that resolves `#2` as a ptygrid session ID simply isn't
  visible.
- **Recovery**: After registering at user scope as above, **restart or resume the
  Claude Code session**. The MCP tool list is loaded at startup, so it won't take effect
  immediately in a session that was already running before the registration.

### 2. curl fails due to sandbox network restrictions

- **Symptom**: `curl http://127.0.0.1:39237/mcp` exits with code 7 (connection failed).
- **Cause**: Claude Code's Bash sandbox has an empty `allowedHosts`, which blocks network
  connections to every host, including localhost.
- **Fix (this time)**: Ran just that one curl command with the sandbox disabled.
- **Candidate permanent fix**: Add `127.0.0.1` / `localhost` to the allowed hosts via the
  `/sandbox` command.

### 3. Protocol steps for talking to MCP directly

Queen runs on rmcp 1.8.0 / Streamable HTTP (SSE responses). Steps for talking to it with
curl:

```sh
# 1) initialize — note the mcp-session-id from the response header
curl -s -D - -X POST http://127.0.0.1:39237/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"1.0"}}}'

# 2) initialized notification (every subsequent request needs the mcp-session-id header)
curl -s -X POST http://127.0.0.1:39237/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: $SID" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'

# 3) tools/call (the response is SSE; extract the JSON from the `data:` line)
curl -s -X POST http://127.0.0.1:39237/mcp ... \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_output","arguments":{"agent":"#2","lines":300}}}'
```

Notes:

- Requests are rejected without `Accept: application/json, text/event-stream`.
- Responses come back as `text/event-stream`, so you need to extract the `data:` lines
  and parse them as JSON.

### 4. Don't know the session ID of a manually launched Codex

- **Symptom**: Expected to be able to target `codex` as the recipient of `send_message`,
  but `list_agents` came back with empty definitions — just four `/bin/zsh` sessions
  (#1, #2, #4, #5).
- **Cause**: `codex` wasn't an agent defined in mterm.yml — it was a CLI manually
  launched inside a zsh pane (session #2).
- **Fix (this time)**: Checked the pane header and `list_agents` to find the ID Codex
  was running in. Used a session-ID-style address like `"#2"`. `list_agents` now also
  returns the foreground process name, so a manually launched CLI inside a shell can be
  discovered this way.

### 5. `agent: "codex"` triggers an ambiguous error

- **Symptom**: The message fails to send, with an error like `use one of: #3, #5`.
- **Cause**: Multiple definitions share the same name, or multiple matching foreground
  processes are running.
- **Fix**: Check the pane header or `list_agents` to identify the target, and specify it
  exactly, e.g. `agent: "#3"`.
- **Design intent**: This is expected safe behavior — it prevents guessing at the most
  recent session and misdirecting a message to the wrong pane.

### 6. Unsent text was left in the input box

- **Symptom**: Reading pane #2 showed `› src/session.rs をレビューして問題点を挙げて`
  (Japanese for "review src/session.rs and list the issues") already typed into the
  composer but not yet submitted.
- **Risk**: Sending the same text via `send_message` as-is would double up the input.
- **Fix (this time)**: Called `send_message` with `{"text": "", "submit": true}` to send
  just an Enter keystroke, submitting the text that was already there.
- **Lesson**: Always check the composer's state with `read_output` before sending
  anything through Queen.

### Other notes

- TUI raw output gets cluttered with leftover spinner artifacts from ANSI control
  sequences and `\r` overwrites. The normal `read_output` reconstructs cursor movement,
  erase sequences, and alternate-screen behavior to match the pane's dimensions. Use
  `raw: true` only when investigating.
- Completion detection turned out to be most stable when polling (every 10 seconds) for
  "the tail no longer contains `esc to interrupt`, and the output hasn't changed across
  two consecutive checks."
- After receiving the request, Codex also read the instructions in the repo's RTK.md
  before starting work.

---

## Implementation-side fixes (2026-07-16, Phase 3.6)

Based on the investigation above, the following changes were made to ptygrid itself:

- **1. Scope issue** → Changed the registration command copied by the Queen badge to
  `claude mcp add -s user --transport http ...` (also documented as a gotcha in the
  README).
- **4. Identifying manually launched CLIs** → `list_agents` / `list_sessions` now return
  the foreground process name (`foreground`). Recipient resolution prioritizes `#<id>`
  first, then a unique definition/session name, and finally a unique foreground name.
  Multiple matches are rejected with the candidate IDs returned.
- **Other notes (spinner artifacts)** → `read_output` originally just stripped ANSI
  codes and collapsed `\r`; following the Grok TUI incident (below), it was extended to
  reconstruct cursor movement, erase sequences, and alternate-screen behavior using the
  pane's dimensions (`raw: true` still gives the old raw output).
- **6. Composer double-input** → `send_message`'s tool description now explicitly
  recommends checking the composer with `read_output` before sending, and notes that
  `text: ""` + `submit: true` sends just an Enter keystroke.
- **2. Sandbox localhost restriction** → Not something the app can fix on its own. The
  permanent fix is to allow `127.0.0.1` via `/sandbox` on the Claude Code side
  (documented in the README).
- **Completion-detection polling (other notes)** → Phase 3.8's cancellable Queen `await`
  already provides cursor-based waiting with a bounded timeout. Use `await` for waiting
  on the Inbox, and keep that distinct from terminal-output completion detection.

---

## `queen-send.py` appears to hang (Grok TUI, 2026-07-16 incident)

When a Phase 3.8 Note draft was requested from Grok `#2`, the background command
appeared to run for a long time on the Claude Code side. Investigating the logs showed
the following sequence of events:

- `sent to #2` was logged — the request reached its destination on the very first
  `send_message`.
- `no activity — sent extra Enter` was not logged, so the extra-Enter nudge never fired.
- About a minute later, Grok wrote the file and printed `執筆完了` ("writing complete").
- `queen-send.py` then confirmed the output had stayed static for two consecutive checks
  and exited normally.

So this incident wasn't a case of the command failing to send — it was a **delay in
output monitoring and completion detection**. Grok's alternate-screen TUI pumps out
spinners, elapsed-time counters, and full-screen redraws, which bloated the
`read_output` result to around 45 KiB and kept it looking like it was still updating.
Even after stripping ANSI codes and collapsing `\r`, rendering fragments can remain, so
the "output has gone static" determination lags behind the actual completion of the
response body.

When triaging, check the following:

1. Does stderr contain `sent to #<id>`? If so, the send call to Queen succeeded.
2. Does it contain `no activity — sent extra Enter`? If so, there was no output change
   after the initial send, and the nudge fired.
3. Check not just `read_output` on the destination pane, but also the update time and
   contents of the requested deliverable itself.
4. If it's still waiting after completion, don't assume it's a send problem — suspect
   ongoing TUI redraws instead.

As a result of this incident, `read_output` was changed from simply stripping ANSI codes
to a lightweight VT reconstruction that uses the pane's rows/cols to apply cursor
movement, screen/line erasure, save/restore cursor, and alternate-screen handling. This
prevents past full-screen redraws from getting concatenated into one giant line.

Note that `queen-send.py` uses terminal-output stillness as a generic completion signal,
so it can lag on TUIs that keep updating their screen content. `await` is for the
Inbox — it isn't a tool for directly waiting on a terminal TUI's response completion.
Future work should consider explicit completion notifications, checking the deliverable
itself, or per-TUI completion conditions.

---

## Pins / Notes return a `conflict`

- **Symptom**: `set_pin` / `update_note` / delete-family tools return a revision
  conflict.
- **Cause**: Another agent updated or deleted the same record after it was read.
- **Fix**: Fetch the latest version and revision with `list_pins` or `get_note`, merge in
  your own changes, then retry with the new `expectedRevision`.
- **Note**: On conflict, the whole mutation transaction is rolled back — the other
  agent's newer content is not overwritten or deleted. Don't ignore the error and
  blindly retry.

---

## Can't find a message in the Inbox

- **Used `#3` as the mailbox**: The Inbox rejects session IDs because it needs to persist
  across app restarts. Call `send_inbox` / `list_inbox` with a stable role name instead,
  such as `codex-review`.
- **Already acknowledged**: `list_inbox` returns only unacknowledged messages by default.
  Pass `includeAcknowledged: true` to review history.
- **Different project**: The Inbox is partitioned by the canonical directory of the
  loaded `mterm.yml`. Even with the same sender and recipient, messages aren't visible
  from a different project.
- **Can't reply**: `reply_inbox.sender` must exactly match the original message's
  recipient. Check the message's `recipient` field and don't substitute a display name
  or session ID.

### `await` returns empty

- `timedOut: true` isn't an error — it's a normal deadline reached. Wait again using the
  returned `nextCursor`.
- IDs smaller than `afterId` are never returned. To check history, use `list_inbox` with
  `afterId: 0`.
- Acknowledged messages are excluded by default. Pass `includeAcknowledged: true` when
  you need them.
- A cancellation error means the MCP client canceled the request — it doesn't change any
  message or ack state.

## The ptygrid window "randomly dies" (case from 2026-07-17)

- **Symptom**: while running agent verification with ptygrid's own repository as the
  working folder, the whole ptygrid window disappeared several times, seemingly right
  after pasting long text into a pane.
- **Triage**: macOS crash reports (`~/Library/Logs/DiagnosticReports`) contained no
  `.ips` for ptygrid at all — so this was not a real crash (segfault/abort). Meanwhile
  the mtime of `src-tauri/target/debug/ptygrid` had been refreshed right after each
  "crash".
- **Cause**: the **file watcher of `npm run tauri dev`**. When anything under
  `src-tauri/` changes (a test file being created, an agent editing sources), tauri dev
  kills the app, rebuilds, and relaunches it. When you dogfood ptygrid on its own
  repository, every agent touch of src-tauri makes the window drop (and come back after
  minutes of Rust build). The paste itself is unrelated — the timing just coincides.
- **Fix**: disable the watcher for verification sessions:

  ```bash
  npm run tauri -- dev --no-watch
  ```

  or verify against a built app (`npm run tauri build` artifact). Restart manually when
  you actually want source changes picked up.
- **Lesson**: when "the app crashed", check DiagnosticReports first. A "crash" with no
  `.ips` means an external kill (dev watcher, or some supervising process) — not a
  crash.
