# Introduction

Welcome to the rsketch documentation! This is a Rust project template that includes modern tooling for gRPC API development and multi-language code generation.

## What's Included

- **Rust Workspace**: Organized crate structure with API, server, command-line, and common utilities
- **gRPC/Protobuf**: Protocol Buffer definitions with [Buf](https://buf.build/) integration
- **Multi-language Code Generation**: Generate gRPC stubs for Go, Java, C++, Python, TypeScript, and more
- **Development Tools**: Just recipes, automated formatting, linting, and testing
- **CI/CD**: GitHub Actions with comprehensive testing and documentation deployment
- **Documentation**: MDBook-based documentation with automatic deployment

## Quick Start

1. **Clone and Setup**:
   ```bash
   git clone <your-repo>
   cd rsketch
   ```

2. **Install Dependencies**:
   ```bash
   # Install Rust if you haven't already
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   
   # Install additional tools
   cargo install just mdbook
   
   # Install buf for protobuf
   # See: https://docs.buf.build/installation
   ```

3. **Build and Test**:
   ```bash
   just build
   just test
   ```

4. **Generate Multi-language Code**:
   ```bash
   just proto-generate        # All languages
   just proto-generate-go     # Go only
   just proto-generate-java   # Java only
   ```

## Project Structure

```
rsketch/
├── api/                    # Protocol Buffer definitions and generated code
├── crates/                 # Rust workspace crates
│   ├── cmd/               # Command-line interface
│   ├── common/            # Shared utilities
│   └── server/            # gRPC server implementation
├── docs/                  # Documentation (this site)
└── examples/              # Usage examples
```

## Next Steps

- [API Guide](api-guide.md) - Learn about the gRPC API structure
- [Buf Integration](buf-integration.md) - Multi-language code generation setup
