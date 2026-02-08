# Buf Integration Guide

This project uses [Buf](https://buf.build/) for managing Protocol Buffer files and generating gRPC stubs for multiple languages.

## Overview

Buf provides:
- **Linting**: Ensures your proto files follow best practices
- **Breaking change detection**: Prevents breaking API changes
- **Code generation**: Generate gRPC client libraries for multiple languages
- **Dependency management**: Manages proto dependencies

### Code Generation Strategy

This project is configured to generate:
- **Rust**: Both client and server code (for the backend service)
- **Other languages** (Go, Java, C++, Python, TypeScript): Client code only (for consuming the API)
- **C**: Requires local setup (see C section below)

## Setup

### Prerequisites

1. Install Buf CLI:
```bash
# macOS
brew install bufbuild/buf/buf

# Linux
curl -sSL https://github.com/bufbuild/buf/releases/latest/download/buf-Linux-x86_64.tar.gz | tar -xvzf - -C /usr/local --strip-components 1

# Windows
choco install buf
```

2. Verify installation:
```bash
buf --version
```

## Configuration Files

All buf configuration files are located in the `api/` directory:

- `api/buf.yaml`: Main configuration file that defines modules and linting rules
- `api/buf.gen.yaml`: General code generation configuration for all languages
- `api/buf.gen.go.yaml`: Go-specific code generation
- `api/buf.gen.java.yaml`: Java-specific code generation  
- `api/buf.gen.cpp.yaml`: C++-specific code generation
- `api/buf.lock`: Dependency lock file (auto-generated)

## Available Commands

### Development Commands (via just)

```bash
# Lint proto files
just proto-lint

# Format proto files
just proto-format

# Check for breaking changes
just proto-breaking

# Generate code for all languages
just proto-generate

# Generate code for specific languages
just proto-generate-go
just proto-generate-java
just proto-generate-cpp
just proto-generate-c      # Requires local protobuf-c setup

# Update dependencies
just proto-deps-update

# Push to Buf Schema Registry (BSR)
just proto-push
```

### Direct Buf Commands

```bash
# Lint
buf lint

# Format (with write)
buf format -w

# Generate all
buf generate

# Generate for specific language
buf generate --template buf.gen.go.yaml

# Breaking change detection
buf breaking --against .git#branch=main

# Update dependencies
buf dep update

# Push to BSR
buf push
```

## Generated Code Structure

Generated code will be placed in the `api/gen/` directory:

```
api/
├── proto/           # Protocol buffer definitions
│   └── hello/
│       └── v1/
│           └── hello.proto
├── gen/             # Generated code for all languages
│   ├── go/          # Go gRPC stubs
│   ├── java/        # Java gRPC stubs  
│   ├── cpp/         # C++ gRPC stubs
│   ├── python/      # Python gRPC stubs
│   ├── typescript/  # TypeScript/JavaScript stubs
│   └── rust/        # Rust gRPC stubs
├── buf.yaml         # Buf configuration
├── buf.gen.yaml     # Code generation config
├── buf.gen.go.yaml  # Go-specific generation
├── buf.gen.java.yaml # Java-specific generation
├── buf.gen.cpp.yaml # C++-specific generation
└── buf.lock         # Dependency lock file
```

## Using Generated Code

### Go Client Example

```go
package main

import (
    "context"
    "log"
    
    hellov1 "github.com/crrow/rsketch/gen/go/hello/v1"
    "google.golang.org/grpc"
    "google.golang.org/grpc/credentials/insecure"
    "google.golang.org/protobuf/types/known/emptypb"
)

func main() {
    // Connect to the Rust gRPC server
    conn, err := grpc.Dial("localhost:50051", grpc.WithTransportCredentials(insecure.NewCredentials()))
    if err != nil {
        log.Fatal(err)
    }
    defer conn.Close()

    // Create client (generated code contains only client)
    client := hellov1.NewHelloServiceClient(conn)
    resp, err := client.Hello(context.Background(), &emptypb.Empty{})
    if err != nil {
        log.Fatal(err)
    }
    
    log.Printf("Response: %v", resp)
}
```

### Java Client Example

```java
import com.rsketch.api.hello.v1.HelloGrpc;
import io.grpc.ManagedChannel;
import io.grpc.ManagedChannelBuilder;
import com.google.protobuf.Empty;

public class HelloClient {
    public static void main(String[] args) {
        // Connect to the Rust gRPC server
        ManagedChannel channel = ManagedChannelBuilder
            .forAddress("localhost", 50051)
            .usePlaintext()
            .build();
            
        // Create client (generated code contains only client)
        HelloGrpc.HelloBlockingStub stub = 
            HelloGrpc.newBlockingStub(channel);
            
        Empty response = stub.hello(Empty.getDefaultInstance());
        System.out.println("Response: " + response);
        
        channel.shutdown();
    }
}
```

### C++ Client Example

```cpp
#include <grpcpp/grpcpp.h>
#include "hello/v1/hello.grpc.pb.h"
#include <google/protobuf/empty.pb.h>

using grpc::Channel;
using grpc::ClientContext;
using grpc::Status;
using rsketch::hello::v1::Hello;
using google::protobuf::Empty;

class HelloClient {
public:
    HelloClient(std::shared_ptr<Channel> channel)
        : stub_(Hello::NewStub(channel)) {}

    void SayHello() {
        Empty request;
        Empty response;
        ClientContext context;

        // Call the Rust gRPC server
        Status status = stub_->Hello(&context, request, &response);
        
        if (status.ok()) {
            std::cout << "Hello successful" << std::endl;
        } else {
            std::cout << "Hello failed: " << status.error_message() << std::endl;
        }
    }

private:
    std::unique_ptr<Hello::Stub> stub_; // Client stub only
};

int main() {
    // Connect to the Rust gRPC server
    auto channel = grpc::CreateChannel("localhost:50051", grpc::InsecureChannelCredentials());
    HelloClient client(channel);
    client.SayHello();
    return 0;
}
```

### C Client Setup & Example

C gRPC support requires local installation of protobuf-c and gRPC-C libraries:

```bash
# Install protobuf-c (Ubuntu/Debian)
sudo apt-get install libprotobuf-c-dev protobuf-c-compiler

# Install protobuf-c (macOS with Homebrew)
brew install protobuf-c

# Install gRPC-C (build from source)
git clone https://github.com/grpc/grpc
cd grpc
git submodule update --init
mkdir -p cmake/build
cd cmake/build
cmake ../..
make grpc
```

Generate C code (requires local setup):
```bash
just proto-generate-c
```

Example C client (conceptual - actual implementation depends on gRPC-C setup):
```c
#include <grpc-c/grpc-c.h>
#include "hello/v1/hello.pb-c.h"

int main() {
    // Initialize gRPC-C
    grpc_c_init(GRPC_C_TYPE_CLIENT, NULL);
    
    // Create client context
    grpc_c_context_t *context = grpc_c_context_init(NULL, 0);
    
    // Connect to Rust gRPC server
    grpc_c_client_t *client = grpc_c_client_init("localhost:50051", NULL, NULL);
    
    // Call Hello service (implementation depends on generated C code)
    // Note: Actual API depends on protobuf-c and gRPC-C generated code
    
    // Cleanup
    grpc_c_client_free(client);
    grpc_c_context_free(context);
    
    return 0;
}
```

**Note**: C gRPC implementation is more complex than other languages. Consider using C++ bindings with C wrappers for easier integration.

## CI/CD Integration

The project includes automated buf validation in GitHub Actions:

- **Linting**: Validates proto file style and best practices
- **Format checking**: Ensures consistent formatting
- **Breaking change detection**: Prevents breaking changes in PRs

## Buf Schema Registry (BSR)

To publish your schemas to BSR:

1. Create an account at [buf.build](https://buf.build)
2. Create a new repository
3. Update `buf.yaml` with your BSR module name
4. Authenticate: `buf registry login`
5. Push schemas: `just proto-push`

## Best Practices

1. **Always run linting** before committing proto changes
2. **Use breaking change detection** for API evolution
3. **Version your APIs** using semantic versioning in proto packages
4. **Generate code regularly** to catch compilation issues early
5. **Keep generated code in .gitignore** - regenerate as needed

## Troubleshooting

### Common Issues

1. **Buf command not found**: Ensure buf is installed and in PATH
2. **Linting errors**: Run `buf lint` to see specific issues
3. **Breaking changes**: Use `buf breaking` to identify breaking changes
4. **Generation failures**: Check plugin versions in buf.gen.yaml files

### Getting Help

- [Buf Documentation](https://docs.buf.build/)
- [Buf Community Slack](https://buf.build/links/slack)
- [Protocol Buffers Guide](https://developers.google.com/protocol-buffers)
