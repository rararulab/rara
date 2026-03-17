# Load environment variables from .env.local if it exists
set dotenv-load
set dotenv-filename := ".env"

# Environment variables with defaults
RUST_TOOLCHAIN := `grep 'channel = ' rust-toolchain.toml | cut -d '"' -f 2`
RARA__DATABASE__DATABASE_URL := env("DATABASE_URL", "postgres://postgres:postgres@localhost:5432/rara")
RARA__DATABASE__MIGRATION_DIR := env("RARA__DATABASE__MIGRATION_DIR", "crates/rara-model/migrations")

# ========================================================================================
# Default Recipe & Help
# ========================================================================================

[group("📒 Help")]
[private]
default:
    @just --list --list-heading '🦀 rara justfile manual page:\n'

[doc("show help")]
[group("📒 Help")]
help: default

[doc("show environment variables")]
[group("📒 Help")]
env:
    @echo "🔧 Environment Configuration:"
    @echo "  RUST_TOOLCHAIN: {{RUST_TOOLCHAIN}}"

# ========================================================================================
# Code Quality
# ========================================================================================

[doc("run `cargo fmt` to format Rust code")]
[group("👆 Code Quality")]
fmt: fmt-proto
    @echo "🔧 Formatting Rust code..."
    cargo +nightly fmt --all
    @echo "🔧 Formatting TOML files..."
    taplo format
    @echo "🔧 Formatting YAML files..."
    yamllint-rs --fix --recursive .
    @echo "🔧 Formatting with hawkeye..."
    hawkeye format
    @echo "✅ All formatting complete!"

[doc("format protobuf files")]
[group("👆 Code Quality")]
[working-directory: 'api']
fmt-proto:
    @echo "🔧 Formatting protobuf files..."
    buf format -w

[doc("run `cargo fmt` in check mode")]
[group("👆 Code Quality")]
fmt-check:
    @echo "📝 Checking Rust code formatting..."
    cargo +nightly fmt --all --check
    @echo "📝 Checking TOML formatting..."
    taplo format --check

[doc("run `cargo clippy`")]
[group("👆 Code Quality")]
clippy:
    @echo "🔍 Running clippy checks..."
    cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings

[doc("run `cargo check`")]
[group("👆 Code Quality")]
check:
    @echo "🔨 Running compilation check..."
    cargo check --all --all-targets

alias c := check

[doc("run `cargo test`")]
[group("👆 Code Quality")]
test:
    @echo "🧪 Running tests with nextest..."
    cargo nextest run --workspace --all-features

alias t := test

[doc("run linting checks (clippy, docs, buf, zizmor, yamllint-rs, cargo-deny)")]
[group("👆 Code Quality")]
lint:
    @echo "🔍 Running clippy..."
    cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings
    @echo "📚 Building documentation..."
    cargo doc --workspace --all-features --no-deps
    @echo "🔍 Linting protobuf..."
    cd api && buf lint
    @echo "🔍 Linting YAML files..."
    yamllint-rs .
    @echo "🔍 Linting GitHub Actions..."
    find .github/workflows -name '*.yml' ! -name 'release.yml' -exec zizmor {} +
    @echo "🔍 Checking dependencies (advisories & bans)..."
    cargo deny check
    @echo "✅ All linting checks passed!"

[doc("run `fmt` `clippy` `check` `test` at once")]
[group("👆 Code Quality")]
pre-commit: fmt clippy check test
    @echo "✅ All pre-commit checks passed!"

[doc("install prek pre-commit hooks into .git/hooks")]
[group("👆 Code Quality")]
setup-hooks:
    @command -v prek >/dev/null 2>&1 || (echo "❌ prek is required. Install with: brew install prek" && exit 1)
    prek install
    @echo "✅ Pre-commit hooks installed!"

[doc("clean build artifacts")]
[group("👆 Code Quality")]
clean:
    @echo "🧹 Cleaning build artifacts..."
    cargo clean

[doc("count lines of code")]
[group("👆 Code Quality")]
cloc:
    @echo "📊 Counting lines of code..."
    cloc . --exclude-dir=vendor,docs,tests,examples,build,scripts,tools,target

# ========================================================================================
# Build
# ========================================================================================

[doc("build rara binary")]
[group("🔨 Build")]
build:
    @echo "🔨 Building rara..."
    cargo build -p rara-cli
    @echo "📦 Moving binary to bin/ directory..."
    mkdir -p bin/ && cp target/debug/rara bin/

[doc("build in release mode")]
[group("🔨 Build")]
build-release:
    @echo "🔨 Building rara (release mode)..."
    cargo build -p rara-cli --release

# ========================================================================================
# Release & Changelog
# ========================================================================================

[doc("generate full changelog")]
[group("📦 Release")]
changelog:
    @echo "📝 Generating full changelog..."
    git cliff -o CHANGELOG.md

[doc("generate changelog for a specific tag")]
[group("📦 Release")]
changelog-tag tag:
    @echo "📝 Generating changelog for {{ tag }}..."
    git cliff --tag {{ tag }} -o CHANGELOG-{{ tag }}.md

[doc("preview unreleased changes")]
[group("📦 Release")]
changelog-unreleased:
    @echo "📝 Preview unreleased changes..."
    git cliff --unreleased

[doc("show release workflow (managed by release-plz)")]
[group("📦 Release")]
release:
    @echo "Release workflow (automated via release-plz):"
    @echo ""
    @echo "  1. Push/merge to main"
    @echo "  2. CI runs lint + test"
    @echo "  3. release-plz creates a Release PR (version bump + changelog)"
    @echo "  4. Review and merge the Release PR"
    @echo "  5. cargo-dist builds binaries + creates GitHub Release (with tag)"
    @echo "  6. Homebrew formula auto-published to rararulab/homebrew-tap"
    @echo ""
    @echo "To preview unreleased changes: just changelog-unreleased"

# ========================================================================================
# Protobuf/gRPC
# ========================================================================================

[doc("generate code from protobuf definitions")]
[group("🔌 Protobuf")]
[working-directory: 'api']
proto:
    @echo "🔌 Generating code from protobuf..."
    buf generate

# ========================================================================================
# Documentation
# ========================================================================================

[doc("serve documentation with mdbook")]
[group("📚 Documentation")]
book:
    @echo "📚 Serving documentation..."
    mdbook serve docs --port 13000

[doc("build documentation with mdbook")]
[group("📚 Documentation")]
docs-build:
    @echo "📚 Building documentation..."
    mdbook build docs

[doc("open cargo docs in browser")]
[group("📚 Documentation")]
docs-open:
    @echo "📚 Opening cargo documentation..."
    cargo doc --workspace --all-features --no-deps --document-private-items --open

# ========================================================================================
# Running & Examples
# ========================================================================================

[doc("run rara via gateway supervisor")]
[group("🏃 Running")]
run:
    @echo "🏃 Starting rara gateway (supervised mode)..."
    cargo run -p rara-cli -- gateway

[doc("run rara server standalone (no supervisor)")]
[group("🏃 Running")]
run-standalone:
    @echo "🏃 Running rara server (standalone)..."
    cargo run -p rara-cli -- server

[doc("run hello-world example")]
[group("🏃 Running")]
example-hello:
    @echo "🏃 Running hello-world example..."
    cargo run --example hello-world


[doc("start frontend dev server (proxies /api to localhost:25555)")]
[group("🔧 Development")]
web:
    cd web && bun run dev

[doc("start Electrobun desktop shell (connects to existing frontend/backend)")]
[group("🔧 Development")]
desktop:
    cd desktop && bun install && bun run start

[doc("start Electrobun desktop shell and also spawn backend + frontend dev servers")]
[group("🔧 Development")]
desktop-managed:
    cd desktop && RARA_DESKTOP_REPO_ROOT=$(cd .. && pwd) RARA_DESKTOP_START_BACKEND=1 RARA_DESKTOP_START_FRONTEND=1 bun install && bun run start

[doc("build Electrobun desktop app for current platform")]
[group("🔧 Development")]
desktop-build:
    cd desktop && bun install && bun run build

[doc("start backend + frontend dev servers together")]
[group("🔧 Development")]
dev:
    just run &
    sleep 2
    just web

# ========================================================================================
# Development Tools
# ========================================================================================

[doc("create a new reversible SQL migration via sqlx cli")]
[group("🗄️ Database")]
migrate-add name:
    @command -v sqlx >/dev/null 2>&1 || (echo "❌ sqlx-cli is required. Install with: cargo install sqlx-cli --no-default-features --features rustls,postgres" && exit 1)
    @echo "🗄️ Creating migration '{{name}}' in {{RARA__DATABASE__MIGRATION_DIR}}..."
    DATABASE_URL={{RARA__DATABASE__DATABASE_URL}} sqlx migrate add -r --source {{RARA__DATABASE__MIGRATION_DIR}} {{name}} 

[doc("run pending SQL migrations")]
[group("🗄️ Database")]
migrate-run:
    DATABASE_URL={{RARA__DATABASE__DATABASE_URL}} sqlx migrate run --source {{RARA__DATABASE__MIGRATION_DIR}}

[doc("revert the latest SQL migration")]
[group("🗄️ Database")]
migrate-revert:
    DATABASE_URL={{RARA__DATABASE__DATABASE_URL}} sqlx migrate revert --source {{RARA__DATABASE__MIGRATION_DIR}}

[doc("show migration status")]
[group("🗄️ Database")]
migrate-info:
    DATABASE_URL={{RARA__DATABASE__DATABASE_URL}} sqlx migrate info --source {{RARA__DATABASE__MIGRATION_DIR}}

[doc("reset rara data directory (drops database, agentfs, etc.)")]
[group("🗄️ Database")]
[confirm("⚠️ This will delete ALL rara data (database, sessions, agentfs). Continue?")]
nuke:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" = "Darwin" ]; then
        DATA_DIR="$HOME/Library/Application Support/rara"
    else
        DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/rara"
    fi
    echo "🧹 Removing $DATA_DIR..."
    rm -rf "$DATA_DIR"
    echo "✅ Clean slate — next startup will re-create everything."

alias ma := migrate-add

# ========================================================================================
# Worktree Management
# ========================================================================================

DEVTOOL := "scripts/bin/devtool"

[doc("build devtool binary")]
[group("🔧 Development")]
devtool-build:
    @cd scripts && go build -o bin/devtool ./cmd/devtool/

[doc("interactive worktree manager (TUI)")]
[group("🌳 Worktree")]
wt: devtool-build
    @{{DEVTOOL}} wt

# ========================================================================================
# Dependency Management
# ========================================================================================

[doc("update dependencies interactively")]
[group("🔧 Development")]
deps-update:
    @echo "📦 Updating dependencies..."
    ./scripts/update-deps.sh
