<div align="center">

# CC Switch Kimi

### Provider switcher & usage monitor for Kimi Code, Claude Code and Codex

[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](../../releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)

English | [中文](README_ZH.md) | [Changelog](CHANGELOG.md)

</div>

A personal fork of [cc-switch](https://github.com/farion1231/cc-switch), rebuilt around **Kimi Code**: it manages Kimi Code as a first-class app alongside Claude Code and Codex, streams local session usage into the existing usage pipeline, and ships a deliberately trimmed feature set.

## Core Features

- **Provider Management** — Create/switch/remove providers for Kimi Code (`~/.kimi-code/config.toml`), Claude Code and Codex with one click. Seven Kimi Code presets built in: Kimi For Coding, GLM, MiniMax, Claude, Codex, DeepSeek, Xiaomi MiMo. CLI and the VSCode extension pick up the same config instantly (shared config directory).
- **Usage Monitoring** — Kimi Code sessions (CLI + VSCode extension) are ingested by incrementally parsing `~/.kimi-code/sessions/**/wire.jsonl` into the local usage database: token breakdown (input / output / cache read / cache creation), TTFT, stream duration, USD cost. Visible in the usage dashboard with near-real-time refresh.
- **Real-time Tray (macOS)** — Menu bar title shows quota windows and generation speed, e.g. `7D 42% · 5H 87% · 31 t/s`, refreshed per second while a session is running. Lightweight mode (tray-only, no main window) keeps it alive.
- **MCP Management** — Unified MCP server list synced into each app's config (`~/.kimi-code/mcp.json`, Claude, Codex).
- **Proxy & Failover** — Local gateway with takeover routing, multi-endpoint failover, and global proxy support.

## What Was Trimmed vs Upstream

Physical removal (not feature flags) of everything outside the four pillars above: the gemini / grokbuild / opencode / openclaw / hermes / claudeDesktop apps, session browser, workspace, prompts & skills management, WebDAV/S3 backup, speedtest, deep links, profiles, and the upstream sponsor/R2-mirror release infrastructure. The upstream `migration.rs` is untouched, so an existing `~/.cc-switch` config directory and database open fine — leftover rows from removed apps are safely ignored.

## Build & Development

Requirements: Node.js 18+, pnpm 8+, Rust 1.85+, Tauri CLI 2.8+.

```bash
pnpm install

pnpm dev                # dev mode (hot reload)
pnpm typecheck          # TS type check
pnpm test:unit          # frontend tests (vitest)
pnpm build              # build installers via tauri

cd src-tauri
cargo test              # backend tests
cargo clippy            # lint
```

## Releases & Auto-Update

Releases are built by `.github/workflows/release.yml` on `v*` tags and published to this repository's GitHub Releases. The updater polls `releases/latest/download/latest.json` from this repo and verifies artifacts against the minisign public key baked into `src-tauri/tauri.conf.json`.

Signing keys are **not** in the repo:

- Private key: generated locally (`tauri signer generate`), kept outside the working tree.
- For local builds that produce updater artifacts, point the env var at it:

  ```bash
  export TAURI_SIGNING_PRIVATE_KEY=~/.tauri-keys/cc-switch-kimi.key
  pnpm build
  ```

- For CI releases, set the repository secrets `TAURI_SIGNING_PRIVATE_KEY` (and optionally `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) under Settings → Secrets and variables → Actions.

## Attribution

Forked from [farion1231/cc-switch](https://github.com/farion1231/cc-switch) (v3.19.1). All credit for the original design and the vast majority of the code goes to its author and contributors; this fork's changes (Kimi Code integration, usage pipeline, presets, trimming) are documented in `CHANGELOG.md`.

## License

MIT — see [LICENSE](LICENSE). The upstream project's MIT license and attribution obligations are preserved.
