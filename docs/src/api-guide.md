# API Module

This directory contains all Protocol Buffer definitions and generated gRPC stubs for the rsketch project.

## Architecture

This project uses a **client-server architecture**:
- **Rust**: Implements the gRPC server with both client and server code generated
- **Other languages**: Generate client-only code to consume the Rust gRPC API

This approach allows you to:
- Build the main service in Rust (performance, safety)
- Create client libraries for multiple languages (ecosystem integration)
- Maintain a single source of truth for the API definition

## Directory Structure

```
api/
├── proto/                    # Protocol Buffer definitions
│   └── hello/
│       └── v1/
│           └── hello.proto
├── gen/                      # Generated code (git-ignored)
│   ├── go/                   # Go client libraries
│   ├── java/                 # Java client libraries
│   ├── cpp/                  # C++ client libraries
│   ├── c/                    # C client libraries (requires local setup)
│   ├── python/               # Python client libraries
│   ├── typescript/           # TypeScript client libraries
│   └── rust/                 # Rust server + client code
├── buf.yaml                  # Main Buf configuration
├── buf.gen.yaml              # Multi-language generation config
├── buf.gen.go.yaml           # Go-specific generation
├── buf.gen.java.yaml         # Java-specific generation
├── buf.gen.cpp.yaml          # C++-specific generation
├── buf.gen.c.yaml            # C-specific generation
├── buf.lock                  # Dependency lock file
├── build.rs                  # Rust build script (for tonic/prost)
├── Cargo.toml                # Rust crate configuration
└── src/                      # Rust API crate source
    └── lib.rs
```

## Available Commands

All commands should be run from the project root:

### Code Generation

```bash
# Generate code for all languages
just proto-generate

# Generate for specific languages
just proto-generate-go
just proto-generate-java  
just proto-generate-cpp
just proto-generate-c      # Requires local protobuf-c setup

# Generate directly with buf (from api/ directory)
cd api && buf generate
cd api && buf generate --template buf.gen.go.yaml
```

### Validation

```bash
# Lint proto files
just proto-lint

# Format proto files
just proto-format

# Check for breaking changes (in CI/PR)
just proto-breaking
```

### Dependency Management

```bash
# Update proto dependencies
just proto-deps-update
```

## Language-Specific Setup

### Go

Generated Go code is in `gen/go/` with proper go_package options:

```go
import hellov1 "github.com/crrow/rsketch/gen/go/hello/v1"
```

### Java

Generated Java code is in `gen/java/` with package `com.rsketch.api.hello.v1`:

```java
import com.rsketch.api.hello.v1.HelloProto;
import com.rsketch.api.hello.v1.HelloGrpc;
```

### C++

Generated C++ code is in `gen/cpp/`:

```cpp
#include "hello/v1/hello.grpc.pb.h"
#include "hello/v1/hello.pb.h"
```

### Rust

The Rust crate is built with tonic and prost, available as:

```rust
use rsketch_api::pb::hello::v1::*;
```

## Adding New Services

1. Create a new `.proto` file in the appropriate directory under `proto/`
2. Add language-specific options (go_package, java_package, etc.)
3. Update `build.rs` to include the new proto file for Rust compilation
4. Run code generation: `just proto-generate`
5. Update your application code to use the new generated stubs

## Buf Schema Registry

To publish schemas to the Buf Schema Registry:

1. Configure your module name in `buf.yaml`
2. Authenticate: `buf registry login`
3. Push: `just proto-push`

See the main project documentation for more details on buf integration.
