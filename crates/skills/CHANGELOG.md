# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-03-29

### Bug Fixes

- **skills**: Add ~/.claude/skills/ and bundled skills to discovery paths
- **skills,telegram**: Improve skill prompt injection and markdown rendering
- **skills**: Add config_dir skills path to discoverer ([#128](https://github.com/rararulab/rara/issues/128))
- **skills**: Harden ClawhubClient — zip traversal, snafu errors, trust policy ([#224](https://github.com/rararulab/rara/issues/224))
- **skills**: Harden ClawhubClient — retry, cleanup, conflict, encoding ([#224](https://github.com/rararulab/rara/issues/224))
- **telegram**: Harden tool-call XML stripping for streaming edge cases ([#314](https://github.com/rararulab/rara/issues/314))
- Resolve all clippy warnings across codebase ([#313](https://github.com/rararulab/rara/issues/313))
- **skills**: Universal marketplace install support ([#354](https://github.com/rararulab/rara/issues/354))
- **skills**: Normalize source in install_repo to fix manifest lookup ([#645](https://github.com/rararulab/rara/issues/645)) ([#646](https://github.com/rararulab/rara/issues/646))
- **skills**: Fallback to SKILL.md scanning for hybrid repos ([#714](https://github.com/rararulab/rara/issues/714)) ([#716](https://github.com/rararulab/rara/issues/716))
- **kernel**: Align deferred tool catalog with executable tool registry ([#941](https://github.com/rararulab/rara/issues/941)) ([#942](https://github.com/rararulab/rara/issues/942))
- Resolve contract violations found by code quality scan ([#1026](https://github.com/rararulab/rara/issues/1026)) ([#1030](https://github.com/rararulab/rara/issues/1030))

### Documentation

- **skills**: Add README and module doc comments ([#176](https://github.com/rararulab/rara/issues/176))
- **skills**: Add AGENT.md for ClawHub integration design constraints ([#224](https://github.com/rararulab/rara/issues/224))
- Update README to reflect tape-based architecture ([#783](https://github.com/rararulab/rara/issues/783)) ([#784](https://github.com/rararulab/rara/issues/784))

### Features

- **skills**: Add core skill types, loader, and registry crate ([#160](https://github.com/rararulab/rara/issues/160))
- **skills**: Add PG-backed skill cache for fast startup ([#182](https://github.com/rararulab/rara/issues/182))
- **skills**: Add marketplace data types ([#218](https://github.com/rararulab/rara/issues/218))
- **skills**: MarketplaceService fetch_index from GitHub API ([#218](https://github.com/rararulab/rara/issues/218))
- **skills**: MarketplaceService browse/search/install/enable ([#218](https://github.com/rararulab/rara/issues/218))
- **skills**: Add ClawHub API response types
- **skills**: Add ClawhubClient retry scaffold and serde tests
- **skills**: Add ClawhubClient search browse and detail methods
- **skills**: Add ClawhubClient download and install
- **kernel**: Inject installed skills into agent system prompt ([#487](https://github.com/rararulab/rara/issues/487))
- **kernel**: Discover-tools finds skills ([#833](https://github.com/rararulab/rara/issues/833)) ([#835](https://github.com/rararulab/rara/issues/835))

### Miscellaneous Tasks

- Establish job backend baseline
- Change default HTTP port from 3000 to 25555
- Format
- Format
- Format
- **skills**: Upgrade notify-debouncer-full 0.5 → 0.7, rstest 0.25 → 0.26 ([#286](https://github.com/rararulab/rara/issues/286)) ([#291](https://github.com/rararulab/rara/issues/291))

### Refactor

- Decouple telegram bot with grpc/http boundary ([#94](https://github.com/rararulab/rara/issues/94))
- **skills**: Replace anyhow with snafu, add new modules ([#171](https://github.com/rararulab/rara/issues/171))
- **skills**: Update backend consumers for new InMemoryRegistry API ([#173](https://github.com/rararulab/rara/issues/173))
- **skills**: Make InMemoryRegistry cloneable with internal Arc<RwLock>
- Add keyring-store crate, process group utils, layer READMEs, and dep upgrades
- **model**: Inline rara-model types into remaining crates ([#238](https://github.com/rararulab/rara/issues/238))
- **kernel**: Consolidate queue/KV/LLM subsystems, remove rara-queue crate
- Replace Consul with YAML config, continue SQLite migration, add session resumption
- **kernel**: Prompt review — fix 12 findings ([#755](https://github.com/rararulab/rara/issues/755)) ([#758](https://github.com/rararulab/rara/issues/758))
- **agents**: Prompt diet — cut tokens ~49% ([#823](https://github.com/rararulab/rara/issues/823)) ([#824](https://github.com/rararulab/rara/issues/824))
- **skills**: Skill install hardening — manifest locking, GitHub auth, cleanup ([#817](https://github.com/rararulab/rara/issues/817)) ([#821](https://github.com/rararulab/rara/issues/821))
- **tools**: Split monolithic marketplace tool into 6 independent tools ([#922](https://github.com/rararulab/rara/issues/922)) ([#923](https://github.com/rararulab/rara/issues/923))

<!-- generated by git-cliff -->
