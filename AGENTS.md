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
| SQLite database (rebuildable index cache) | `~/Library/Application Support/com.skillmint/skillmint.db` |
| Settings | `~/Library/Application Support/com.skillmint/settings.json` |
| Keychain service name | `com.skillmint` |
| Global private hub (authored skills, own git) | `~/.skillmint/hub/` |
| Per-project private hub (committed with the project) | `<project>/.skillmint/hub/` |
| Trash snapshots before removal | `~/.skillmint/trash/` |
| npx skills global lock | `~/.agents/.skill-lock.json` |
| npx skills global canonical store | `~/.agents/skills/` |
| npx skills project lock | `<project>/skills-lock.json` |
| npx skills project canonical store | `<project>/.agents/skills/` |
| Deep-link scheme | `skillmint://` (`skillmint://sync` = rebuild the index) |

## P3 architecture: SkillMint is a GUI over `npx skills`

The skills domain is driven by the `skills` CLI (npm `skills`, vercel-labs), invoked as
`npx -y <npx_package> …` (default `skills@latest`, requires Node ≥ 22.20). Design invariants:

- **The CLI owns install state.** Every install/update/remove runs the real binary
  (`src-tauri/src/npx.rs`); SkillMint keeps no parallel state. The DB (`skill_index`
  tables) is a disposable cache rebuilt from the locks + agent dirs + hubs.
- **Provenance is read-only across boundaries.** Rows whose canonical path lands in
  `.agents/skills` (or that appear in a lock) are `npx`-managed: SkillMint never
  re-links or re-imports them. Untracked agent-dir entries are `unmanaged`
  (collect-to-hub copies them, source untouched). Hub rows are authored content.
- **Private hubs replace the center repo** (retired in P3-6): authoring happens in
  `~/.skillmint/hub` (own git, optional private remote) or `<project>/.skillmint/hub`
  (committed with the project, no nested .git). Distribution = `npx skills add <hub>`.
- **China acceleration** (settings): `skills_api_url` overrides the skills.sh search
  API (`SKILLS_API_URL`), `proxy_env` injects `HTTPS_PROXY` for the CLI's git steps
  (the CLI's own fetch calls ignore proxy env; api.github.com failures fall back to
  git clone), and telemetry is disabled by default (`DISABLE_TELEMETRY=1`).

Legacy note: the old center-repo sync/import/version engine still compiles but has no
remaining UI entry point; mechanical deletion is tracked in `docs/OPTIMIZATION-2026-07.md` (P3-6b).

Collection is strictly read-only against installed agents' own data directories.
