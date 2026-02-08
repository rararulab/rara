# Release Process

This project uses [cargo-dist](https://opensource.axo.dev/cargo-dist/) for automated releases and [git-cliff](https://git-cliff.org/) for changelog generation.

## Quick Release (Recommended)

Use the `just release` command to create and push a release tag:

```bash
just release v0.1.0
```

This will:

1. Show a preview of unreleased changes
2. Create an annotated git tag

Then push the tag to trigger the automated release:

```bash
git push origin v0.1.0
```

**The CI will automatically:**

- Update Cargo.toml version (removes `v` prefix from tag)
- Generate complete CHANGELOG.md
- Commit these changes to main branch
- Build binaries for all platforms
- Create GitHub Release with artifacts

## Manual Release Process

If you prefer to do it manually:

### 1. Update Version

Update version in `Cargo.toml`:

```toml
[workspace.package]
version = "0.1.0"  # Update this
```

### 2. Generate Changelog

Preview unreleased changes:

```bash
just changelog-unreleased
```

Update changelog and create release commit:

```bash
# This updates CHANGELOG.md with unreleased changes
git cliff --unreleased --tag v0.1.0 --prepend CHANGELOG.md
git add CHANGELOG.md
git commit -m "chore(release): prepare for v0.1.0"
```

### 3. Create and Push Tag

```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin main
git push origin v0.1.0
```

### 4. Automated Release Workflow

GitHub Actions will automatically:

- Generate **incremental** release notes using git-cliff
- Build binaries for all supported platforms
- Create installers (shell, PowerShell, MSI)
- Generate checksums
- Create a GitHub Release with all artifacts and release notes

## Changelog Management

### Full Changelog (CHANGELOG.md)

The full changelog is automatically updated on every push to `main` via CI. It contains the complete project history.

You can also generate it manually:
```bash
just changelog
```

### Incremental Release Notes

Each GitHub Release gets incremental release notes containing only changes since the last release. This is automatically generated during the release workflow using:
```bash
git cliff --latest --strip all
```

## Supported Platforms

- **macOS**: x86_64 (Intel), aarch64 (Apple Silicon)
- **Linux**: x86_64 (glibc), x86_64 (musl)
- **Windows**: x86_64

## Installation Methods

After a release is published, users can install via:

### Shell (macOS/Linux)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/crrow/rsketch/releases/latest/download/rsketch-installer.sh | sh
```

### PowerShell (Windows)

```powershell
irm https://github.com/crrow/rsketch/releases/latest/download/rsketch-installer.ps1 | iex
```

### MSI Installer (Windows)

Download the `.msi` file from the [releases page](https://github.com/crrow/rsketch/releases).

## Testing Release Workflow

You can test the release workflow on pull requests without creating a real release:

```bash
git tag v0.1.0-test
git push origin v0.1.0-test
```

The workflow will run but won't publish artifacts.

## Useful Just Commands

```bash
just changelog                # Generate full changelog
just changelog-unreleased     # Preview unreleased changes
just release v0.1.0          # Prepare release (recommended)
```
