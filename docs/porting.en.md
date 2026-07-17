**English** · [日本語 (Japanese)](porting.md)

# Cross-Platform Porting Guide (Linux / Windows)

Currently, **macOS is the primary target**, and Linux is in **test-support (beta)** status.
This document covers the Linux build and distribution steps established in Phase 3.9, along
with a roadmap for the Windows work ahead. The code was written with porting in mind, so no
major rewrite is required. Here's the rough scope:

| Target | Scope | Main work |
|---|---|---|
| **Linux** | **Phase 3.9 test support** | Ubuntu CI + PATH restoration + `.deb` / AppImage. Real-device compatibility testing is ongoing |
| **Windows** | Medium (1-2 weeks) | Implement `process_name` + absorb PowerShell differences + add tests + installer |

> Note: The Windows code path already has `#[cfg(windows)]` branches in place and is expected
> to compile, but it has never been exercised by automated tests (unverified) — that is
> effectively where most of the remaining work lies.

---

## Already in place (porting groundwork)

The biggest hurdle — the PTY layer — is handled by `portable-pty 0.9` (from wezterm), which
internally absorbs the Windows=ConPTY / Unix=forkpty difference. Beyond that, platform
branches exist at the key points.

| Area | File | What's handled |
|---|---|---|
| Default shell | `src-tauri/src/pty.rs` | Windows=`powershell.exe` / others=`$SHELL` (fallback `/bin/bash`) |
| Home directory | `src-tauri/src/pty.rs` | `USERPROFILE` / `HOME` |
| Shell-wrap execution | `src-tauri/src/session.rs` | `/bin/sh -c` / `powershell.exe -Command` |
| Resize | `src-tauri/src/session.rs` | Guarded by `#[cfg(unix)]` (a non-unix path also exists) |
| Console suppression | `src-tauri/src/main.rs` | `windows_subsystem = "windows"` attribute |
| Process-name resolution | `src-tauri/src/pty.rs` | Linux=`/proc/<pid>/comm` / other unix=`ps` / **Windows=not implemented (`None`)** |

- Dependencies (tokio / serde / notify / rmcp / axum) and the frontend (Svelte 5 + xterm.js)
  are all portable.
- Key bindings aren't intercepted on the JS side, so no Cmd/Ctrl differentiation is needed.
- Tauri's capability set is just `core:default`, with no OS-specific plugin dependencies.
- `scripts/queen-send.py` is Python, so it's portable as-is.

---

## Linux test-support status (Phase 3.9)

Linux has the runtime path fully implemented, including the `/proc`-only path for
`process_name`. Ongoing platform-specific verification and distribution are pinned to
Ubuntu CI. At this stage this is not yet stable-release support — it's treated as beta,
with verification continuing on users' real machines, centered on Ubuntu / Debian-based
systems.

- [x] Brought Tauri v2's Linux build dependencies into Ubuntu 22.04 CI
- [x] Run frontend check/build, Rust test/clippy, and Tauri native build across the
      macOS / Ubuntu matrix
- [x] Obtain the Linux foreground process name from `/proc/<pid>/comm`
- [x] Bind Queen to 127.0.0.1 only, and inject `QUEEN_URL` into every PTY
- [x] Restore the user shell's `PATH`, which the desktop launcher drops, right after startup
- [x] Enable `bundle.active` and pin the Linux target to `.deb` / AppImage
- [x] Generate the Linux package via the tag / manual workflow and upload it as an artifact
- [x] Add Linux install and build instructions to the README / user guide

### Compatibility baseline and build

To avoid inadvertently raising the minimum required glibc version, the Linux bundle uses
the Ubuntu 22.04 runner as its baseline. Debian 12 is also a compatible baseline, since it
ships WebKitGTK 4.1 by default.

```bash
npm run tauri dev
npm run bundle:linux
```

The regular GitHub Actions CI builds the native application with `--no-bundle` on both
macOS and Linux. `.deb` / AppImage generation runs on tag push or via `workflow_dispatch`.

## Windows support checklist

The branching scaffolding is in place, but there are gaps to fill in on both functionality
and quality.

- [ ] **Windows implementation of `process_name()`** (top priority)
      It currently returns `None`, so the path that lets Queen's `read_output` /
      `send_message` / `spawn_agent` target a session by "foreground process name" (e.g.,
      `codex`) doesn't work — only `#<id>` and the defined name can be used. `SessionInfo.foreground`
      also ends up empty.
      Implement this with the Windows API (`QueryFullProcessImageNameW`) or the `sysinfo` crate.
- [ ] **Absorb PowerShell differences**
      `shell_wrap` uses `powershell.exe -Command`. Quoting, piping, and variable-expansion
      semantics differ from `/bin/sh -c`, so `sh`-oriented `cmd` entries in `mterm.yml` won't
      work as written. Either document a caveat about `cmd` in `mterm.yml`, or consider letting
      users choose between `cmd.exe` and `pwsh`.
- [ ] **Windows test support**
      Existing tests depend on `/bin/cat` and `/bin/sh` and are Unix-only. Switch to Windows
      equivalents (e.g., `cmd /c type`, `more`), or branch with `#[cfg]`, and get Windows CI green.
- [ ] Verify on real hardware how `ansi.rs`'s CR (`\r`) folding behaves against ConPTY's VT stream
- [ ] Path-absoluteness detection (`Path::is_absolute` is cross-platform, but `resolve_cwd`
      still needs real-hardware verification)
- [ ] Icon: currently only `src-tauri/icons/icon.png` exists. Prepare a Windows `.ico`
      (Tauri can generate one)
- [ ] Installer: MSI / NSIS, code signing
- [ ] Add Windows to the platform badges in the README

---

## Common groundwork needed

- [ ] Extend GitHub Actions to all 3 OSes (macOS and Ubuntu are done; add Windows)
- [x] Enable `bundle.active` in `tauri.conf.json` and set the Linux target
- [ ] Binary distribution at release time (an installer for each OS)

## Verification checkpoints (always exercise these after porting)

- PTY: spawn / I/O / resize / kill / autorestart (never/on-failure/always, abort after
  5 consecutive failures)
- Queen: `list_agents` / `read_output` (cursor/erase/alternate-screen reconstruction) /
  `send_message` / `spawn_agent` (allow list) / `notify`, and whether bind is 127.0.0.1 only
- config: loading `mterm.yml`, `${VAR}` expansion, autostart, reload on change watch
- UI: up to 9 panes, layout, status dots (running / exited / restarting + exit code)

## References

- [Tauri v2 Linux prerequisites](https://v2.tauri.app/start/prerequisites/#linux)
- [Tauri v2 AppImage guide](https://v2.tauri.app/distribute/appimage/)
- [Tauri v2 Debian package guide](https://v2.tauri.app/distribute/debian/)
- [tauri-apps/fix-path-env-rs](https://github.com/tauri-apps/fix-path-env-rs)
