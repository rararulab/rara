# Release Pipeline

Rara uses [release-plz](https://release-plz.ieni.dev/) for version management and [cargo-dist](https://opensource.axo.dev/cargo-dist/) for binary builds. The entire process is automated — no manual tagging or version bumping required.

## Flow

```text
push to main
    │
    ▼
CI (lint + test)
    │
    ▼
release-plz creates Release PR
  • bumps version in Cargo.toml (workspace-wide)
  • updates CHANGELOG.md via git-cliff
  • labels PR with "release"
    │
    ▼
human reviews & merges Release PR
    │
    ▼
release-plz.yml triggers (on PR merge with "release" label)
  • runs release-plz release (currently a no-op: tag/release/publish all disabled)
  • extracts rara-cli version via cargo metadata
  • calls release.yml via workflow_call with tag=vX.Y.Z
    │
    ▼
release.yml (cargo-dist)
  • cargo dist plan --tag=vX.Y.Z
  • builds binaries: aarch64-apple-darwin (macos runner), x86_64-unknown-linux-gnu (linux runner)
  • uploads artifacts to MinIO S3
  • gh release create vX.Y.Z — creates git tag + GitHub Release with binaries attached
  • pushes Homebrew formula (rara-cli.rb) to rararulab/homebrew-tap
    │
    ▼
done — users can install via:
  brew install rararulab/tap/rara-cli
  # or download from GitHub Releases
```

## Key Configuration Files

| File | Purpose |
|------|---------|
| `release-plz.toml` | release-plz behavior: changelog, tag/release/publish toggles |
| `cliff.toml` | git-cliff changelog generation rules |
| `Cargo.toml` `[workspace.metadata.dist]` | cargo-dist config: targets, installers, Homebrew tap |
| `.github/workflows/release-pr.yml` | Creates the Release PR (called by CI after main push) |
| `.github/workflows/release-plz.yml` | Runs on Release PR merge → triggers release.yml |
| `.github/workflows/release.yml` | cargo-dist build + GitHub Release + Homebrew |

## Why workflow_call Instead of Tag Push?

GitHub Actions security restriction: tags created by `GITHUB_TOKEN` within a workflow **do not trigger** `on: push: tags` events (prevents infinite loops). So release-plz.yml calls release.yml directly via `workflow_call` instead of relying on tag-push triggers.

## Version Source of Truth

- All crates inherit `version.workspace = true` from root `Cargo.toml`
- `rara-cli` is the only distributable binary (defined in `crates/cmd/Cargo.toml`)
- Version extraction in release-plz.yml uses `cargo metadata` to read the resolved rara-cli version

## Homebrew

- Tap repo: [rararulab/homebrew-tap](https://github.com/rararulab/homebrew-tap)
- Formula pushed by cargo-dist during release
- Requires `HOMEBREW_TAP_TOKEN` secret with write access to the tap repo
