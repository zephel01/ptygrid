**English** · [日本語 (Japanese)](competitive-landscape.md)

# Competitive Research: Comparing Similar Tools (Competitive Landscape)

Survey date: 2026-07-16 (a cross-web survey conducted via Grok, covering 32+ sites; updated from the baseline description in docs/design.md)

> Figures for external projects are a snapshot as of the survey date. ptygrid's implementation status has been updated through Phase 3.8.

## Updates Since design.md (2026-07-15)

| design.md's description | This survey's findings |
|---|---|
| cmux ~17k stars | Grown to ~24.5k. A leading contender for macOS-native terminal dominance |
| Superset ~10.7k, described as a "terminal" | ~12.4k. In reality an agent IDE (Electron, ELv2 license) |
| Crystal — needs verification | MIT-licensed. Migrated to Nimbalyst (Crystal is now considered legacy) |
| Parallel Code ~633 | ~850, MIT, Electron+Solid — a staple of the worktree camp |
| Only a few similar tools existed | Architect (Zig grid + MCP), Agent Deck, and Conductor are now also significant additions |
| Building from scratch rather than forking | Still the right call. Comparable OSS projects are either GPL/AGPL/ELv2-licensed or take a different approach to worktrees |

## Positioning (Conclusion)

The market has split broadly into two camps:

- **The worktree-isolation camp**: Claude Squad / Parallel Code / Conductor / Superset
  — spins up a separate git worktree per agent, runs them in parallel, and merges the results via diff review
- **The same-screen collaboration camp**: HiveTerm / **ptygrid**
  — a multi-pane layout plus an orchestrator (Queen) lets agents read and write to each other directly

```
                Strong on collaboration / MCP / config-as-code
                          ↑
              HiveTerm ●  │  ● ptygrid  ← filling this gap
                          │
    Architect ●           │            ● cmux
  ← Strong on worktree parallelism ──┼── Strong on terminal primitives →
 Claude Squad ●           │
Parallel Code ●           │
    Conductor ●           │
     Superset ●           ↓ (IDE-ification / max feature set)
                Orchestration is thin / a different model
```

**The white space**: An OSS equivalent of HiveTerm's "Tauri + YAML config + built-in MCP" combination barely exists.
cmux is the strongest terminal around, but it isn't Queen-style. Superset and Conductor are worktree IDEs.

## Codebases Worth Studying

| Priority | Repository | What to Learn From It |
|---|---|---|
| High | Architect | Grid terminal, status highlighting, thin MCP layer |
| High | Claude Squad | Session management and worktree handling (**AGPL-licensed, so reference for design only**) |
| Medium | Parallel Code | worktree UI, diff review (reference for Phase 3) |
| Medium | cmux | Notification UX, CLI programmability (GPL-licensed, macOS-only) |
| Low (spec only) | HiveTerm | Product UX, the upper bound on Queen tools' spec |

## ptygrid's Winning Angle / Next Features to Build

**Strengths already in place (Phase 3.8)**: Beyond multi-pane layout, mterm.yml, and Queen's 18 tools, ptygrid already integrates Git review/commit, optional worktree isolation, logical resume, process-tree resource monitoring, race-safe Pins/Notes, and even persistent Inbox/Reply — all in one package.

| Priority | Item | Where Competitors Are Strong |
|---|---|---|
| Implemented in Phase 3 | Git, worktree, pins/notes, inbox/reply, cancellable await | Superset / Parallel Code / Conductor / HiveTerm |
| UX | Notification ring / "needs approval" highlighting | cmux / Architect |
| Maintaining differentiation | config-as-code plus allowlist-gated spawn | Almost no one else combines both |

## What Not to Do

- Compete with cmux on being a "full terminal emulator" (native Ghostty will win that fight)
- Compete with Superset on being a "full IDE plus cloud remote" (scope creep)
- Fork Claude Squad's code (it's AGPL-licensed)

## In a Nutshell

- **Closest product match**: HiveTerm (closed-source, the original spec)
- **Highest OSS star counts**: cmux (terminal primitives) and Superset (worktree IDE)
- **Philosophical split**: the worktree-isolation camp vs. the same-screen collaboration camp — ptygrid is built around the latter while also offering opt-in isolation
- **Why build it ourselves**: comparable OSS projects are off either on license or on design philosophy, and an OSS implementation of "Tauri + mterm.yml + Queen" remains essentially unclaimed territory
