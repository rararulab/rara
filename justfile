# Load environment variables from .env.local if it exists
set dotenv-load
set dotenv-filename := ".env"

# Environment variables with defaults
RUST_TOOLCHAIN := `grep 'channel = ' rust-toolchain.toml | cut -d '"' -f 2`
TARGET_PLATFORM := env("TARGET_PLATFORM", "linux/arm64")
DISTRI_PLATFORM := env("DISTRI_PLATFORM", "ubuntu")
DOCKER_TAG := env("DOCKER_TAG", "job:latest")
PYO3_PYTHON := `uv python find 3.10`
RARA__DATABASE__DATABASE_URL := env("DATABASE_URL", "postgres://postgres:postgres@localhost:5432/rara")
RARA__DATABASE__MIGRATION_DIR := env("RARA__DATABASE__MIGRATION_DIR", "crates/rara-model/migrations")

# ========================================================================================
# Default Recipe & Help
# ========================================================================================

[group("📒 Help")]
[private]
default:
    @just --list --list-heading '🦀 job justfile manual page:\n'

[doc("show help")]
[group("📒 Help")]
help: default

[doc("show environment variables")]
[group("📒 Help")]
env:
    @echo "🔧 Environment Configuration:"
    @echo "  RUST_TOOLCHAIN: {{RUST_TOOLCHAIN}}"
    @echo "  TARGET_PLATFORM: {{TARGET_PLATFORM}}"
    @echo "  DISTRI_PLATFORM: {{DISTRI_PLATFORM}}"
    @echo "  DOCKER_TAG: {{DOCKER_TAG}}"
    @echo "  PYO3_PYTHON: {{PYO3_PYTHON}}"

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

[doc("run memory integration tests against deployed services")]
[group("🧪 Testing")]
test-memory MEM0_URL="" MEMOS_URL="" MEMOS_TOKEN="" HINDSIGHT_URL="" HINDSIGHT_BANK="default":
    #!/usr/bin/env bash
    set -euo pipefail
    # Use provided args or fall back to env vars or defaults
    export MEM0_BASE_URL="${MEM0_URL:-${MEM0_BASE_URL:-http://localhost:8888}}"
    export MEMOS_BASE_URL="${MEMOS_URL:-${MEMOS_BASE_URL:-http://localhost:5230}}"
    export MEMOS_TOKEN="${MEMOS_TOKEN:-}"
    export HINDSIGHT_BASE_URL="${HINDSIGHT_URL:-${HINDSIGHT_BASE_URL:-http://localhost:8100}}"
    export HINDSIGHT_BANK_ID="${HINDSIGHT_BANK:-${HINDSIGHT_BANK_ID:-default}}"
    echo "Running memory integration tests..."
    echo "  MEM0_BASE_URL=$MEM0_BASE_URL"
    echo "  MEMOS_BASE_URL=$MEMOS_BASE_URL"
    echo "  HINDSIGHT_BASE_URL=$HINDSIGHT_BASE_URL"
    echo "  HINDSIGHT_BANK_ID=$HINDSIGHT_BANK_ID"
    cargo test -p rara-memory -- --ignored --nocapture

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

[doc("build job binary")]
[group("🔨 Build")]
build:
    @echo "🔨 Building job..."
    cargo build -p job-cli
    @echo "📦 Moving binary to bin/ directory..."
    mkdir -p bin/ && cp target/debug/job bin/

[doc("build in release mode")]
[group("🔨 Build")]
build-release:
    @echo "🔨 Building job (release mode)..."
    cargo build -p job-cli --release

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

[doc("create release tag (CI will update version and changelog)")]
[group("📦 Release")]
release version:
    @echo "🚀 Creating release tag {{ version }}..."
    @echo "📝 Preview of unreleased changes:"
    @git cliff --unreleased --strip all || true
    @echo ""
    @echo "Creating tag {{ version }}..."
    git tag -a {{ version }} -m "Release {{ version }}"
    @echo "✅ Tag {{ version }} created!"
    @echo ""
    @echo "⚠️  CI will automatically:"
    @echo "  1. Update Cargo.toml version"
    @echo "  2. Generate CHANGELOG.md"
    @echo "  3. Commit changes to main"
    @echo "  4. Build and publish release"
    @echo ""
    @echo "Next step: Push the tag to trigger release"
    @git push origin {{ version }}

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

[doc("run the binary")]
[group("🏃 Running")]
run:
    @echo "🏃 Running rara binary..."
    PYO3_PYTHON={{PYO3_PYTHON}} cargo run -p rara-cli -- server

[doc("run hello-world example")]
[group("🏃 Running")]
example-hello:
    @echo "🏃 Running hello-world example..."
    cargo run --example hello-world

# ========================================================================================
# Docker
# ========================================================================================

[doc("build base image with Rust toolchain and cargo tools (run once)")]
[group("🐳 Docker")]
build-base:
    @echo "🐳 Building base image (job-base)..."
    docker build \
        --build-arg RUST_TOOLCHAIN={{RUST_TOOLCHAIN}} \
        --tag job-base:latest \
        --file docker/base.Dockerfile \
        .

[doc("build app Docker image (requires base image)")]
[group("🐳 Docker")]
build-docker:
    @echo "🐳 Building app Docker image..."
    docker build \
        --tag {{DOCKER_TAG}} \
        --file docker/Dockerfile \
        .

[group("🐳 Docker")]
up:
    docker compose up --build -d

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

alias ma := migrate-add

[doc("update dependencies interactively")]
[group("🔧 Development")]
deps-update:
    @echo "📦 Updating dependencies..."
    ./scripts/update-deps.sh
