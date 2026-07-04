# Contributing to SkillMint

Thanks for your interest in improving SkillMint! This is a small project, so the process is lightweight.

## Setup

SkillMint is a Tauri 2 app: a Rust backend (`skillmint/src-tauri/`) and a React + TypeScript + Vite frontend (`skillmint/src/`).

```sh
# Requirements: macOS 14+, Node 22 (nvm), Rust stable
nvm use 22
cd skillmint
npm install
npm run tauri dev
```

## Before you open a PR

Please run both test suites and confirm they're green:

```sh
# Frontend (vitest)
cd skillmint && npm run test:ci

# Backend (cargo)
cd skillmint/src-tauri && cargo test
```

For changes that touch packaging, also confirm the bundle builds:

```sh
make build-install verify
```

## What's welcome

- Bug reports with repro steps (please open an issue first).
- Agent collector support: if you use an agent SkillMint doesn't auto-detect yet, a read-only collector PR is a great first contribution. Open an issue to discuss the source layout before coding.
- New skill bundle sources (GitHub-hosted, read-only, signed).
- Documentation improvements.

## What's not in scope

- Telemetry / analytics that phone home. SkillMint is local-first; please don't introduce network calls without an explicit, opt-in user setting.
- Changes that write outside `~/.skillmint` or the app's own App Support directory.

## License

By contributing you agree that your contributions are licensed under the Apache License 2.0, same as the rest of the project.
