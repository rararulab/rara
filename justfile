# Load environment variables from .env.local if it exists
set dotenv-load
set dotenv-filename := ".env.local"

# Environment variables with defaults
RUST_TOOLCHAIN := `grep 'channel = ' rust-toolchain.toml | cut -d '"' -f 2`
TARGET_PLATFORM := env("TARGET_PLATFORM", "linux/arm64")
DISTRI_PLATFORM := env("DISTRI_PLATFORM", "ubuntu")
DOCKER_TAG := env("DOCKER_TAG", "job:latest")

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
    cargo check --all --all-features --all-targets

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
    @echo "🏃 Running rsketch binary..."
    cargo run --package binary hello

[doc("run hello-world example")]
[group("🏃 Running")]
example-hello:
    @echo "🏃 Running hello-world example..."
    cargo run --example hello-world

# ========================================================================================
# Docker
# ========================================================================================

[doc("build Docker image")]
[group("🐳 Docker")]
build-docker:
    @echo "🐳 Building Docker image..."
    docker buildx build \
        --build-arg RUST_TOOLCHAIN={{RUST_TOOLCHAIN}} \
        --tag {{DOCKER_TAG}} \
        --file docker/Dockerfile \
        --output type=docker \
        .

[doc("build Docker image for multiple platforms")]
[group("🐳 Docker")]
build-docker-multiarch:
    @echo "🐳 Building multi-arch Docker image..."
    docker buildx build \
        --platform linux/amd64,linux/arm64 \
        --build-arg RUST_TOOLCHAIN={{RUST_TOOLCHAIN}} \
        --tag {{DOCKER_TAG}} \
        --file docker/Dockerfile \
        .

[group("🐳 Docker")]
up:
    docker compose up

# ========================================================================================
# Development Tools
# ========================================================================================

[doc("update dependencies interactively")]
[group("🔧 Development")]
deps-update:
    @echo "📦 Updating dependencies..."
    ./scripts/update-deps.sh
