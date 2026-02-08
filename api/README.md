# API Module

This directory contains Protocol Buffer definitions and generated gRPC stubs.

📚 **For complete documentation, see the [project docs](../docs/src/api-guide.md)**

## Quick Start

```bash
# Generate code for all languages
just proto-generate

# Generate for specific languages  
just proto-generate-go
just proto-generate-java
just proto-generate-cpp

# Lint and validate
just proto-lint
just proto-format
```

## Structure

- `proto/` - Protocol Buffer definitions
- `../bindings/` - Generated code for all languages (git-ignored)
- `buf.*.yaml` - Buf configuration files
- `src/` - Rust API crate source