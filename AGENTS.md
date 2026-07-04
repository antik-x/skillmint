# Project conventions for agents (ZCode / Claude Code / etc.)

## Node version

The frontend (Vite 6) requires Node 18+; the toolchain is pinned to **Node 22 via nvm**. Before running any `npm`/`node` command:

```sh
export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"; nvm use 22
```

(Repo has an `.nvmrc`.)

## Repository layout

```
skillmint/        # the app (Tauri 2 + React)
  src/            # frontend
  src-tauri/      # Rust backend
Makefile          # build / install / verify targets
tools/            # dev-only Python helpers (mock backend, coverage check, kg generator)
```

## Build & test

- `make build-install` — release build + install to `/Applications/SkillMint.app` + verify.
- `cd skillmint && npm run test:ci` — frontend vitest suite.
- `cd skillmint/src-tauri && cargo test` — backend unit tests.

## Local data layout (macOS)

SkillMint stores everything on the user's device:

| Purpose | Path |
|---|---|
| Center skill repository | `~/.skillmint/repo/` |
| SQLite database | `~/Library/Application Support/com.skillmint/skillmint.db` |
| Settings | `~/Library/Application Support/com.skillmint/settings.json` |
| Keychain service name | `com.skillmint` |
| Per-project skill dir | `<project>/.skillmint/skills/` |
| Deep-link scheme | `skillmint://` |

Collection is strictly read-only against installed agents' own data directories; SkillMint never writes into an agent directory except via a user-initiated sync (which creates a symlink back to `~/.skillmint/repo`).
