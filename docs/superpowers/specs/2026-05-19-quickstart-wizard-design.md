# githttp quickstart Wizard — Design Spec

## Overview

Add `githttp quickstart` subcommand — an interactive setup wizard for first-time users. Release packages include `quickstart.bat`/`quickstart.sh` scripts that invoke this subcommand.

## Trigger

- `githttp quickstart` — the **only** entry point
- `githttp` without args and no config.toml — NO auto-trigger (existing behavior preserved)
- Release scripts: `quickstart.bat` (Windows), `quickstart.sh` (Linux), both run `githttp quickstart`

## Wizard Flow

```
Welcome message
  |
Step 1: Repos root directory
  |-- Invalid path → loop retry (don't exit)
  |-- Valid path → scan *.git repos, list them
  |-- Generate config.toml immediately
  |
Step 2: Create user
  |-- Username (reject duplicates)
  |-- Password + confirm (hidden, reject empty/mismatch)
  |-- Update config.toml
  |
Step 3: Add more users? [y/N] → loop back to Step 2
  |
Step 4: Summary display (read-only)
  |-- Config file location
  |-- Repos directory, listen addr, users
  |-- Usage examples (git clone/push commands)
  |
Step 5: Start server now? [Y/n]
  |-- Y → run server in current process
  |-- N → exit
```

## Code Changes

| File | Change |
|------|--------|
| `src/quickstart.rs` | **New** — `pub fn run_quickstart(config_path: &str) -> Option<Config>` |
| `src/main.rs` | **Modify** — add `quickstart` subcommand parsing + handler branch |
| `quickstart.bat` | **New** — Windows one-click: `githttp quickstart` |
| `quickstart.sh` | **New** — Linux one-click: `githttp quickstart` |
| `.github/workflows/build.yml` | **Modify** — package quickstart.bat/sh, remove config.example.toml |

## Design Decisions

- Generated config.toml omits `backend` (default native) and `logging` (default disabled)
- Reuses `auth::hash_password()` and `users::read_password_pair()` from existing code
- Config maintained in memory, saved incrementally after each step
- No .git repos found → non-blocking, show creation command
- Ctrl+C at any point → exit immediately
