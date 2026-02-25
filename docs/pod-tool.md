# PodTool -- Agent K8s Pod Management

## Overview

`PodTool` 是一个实现了 `AgentTool` trait 的 agent 工具，允许 agent 在对话过程中动态创建、删除、查询和获取日志 K8s Pod。典型用途包括：在隔离环境中执行不可信代码、编译测试、运行第三方工具等。

工具名称：`pod`

工具描述：`Manage Kubernetes pods. Actions: create, delete, status, logs. Use for running isolated workloads in the cluster.`

## Prerequisites

1. 可用的 K8s 集群（本地或远程）
2. 正确配置的 kubeconfig
3. ServiceAccount 具备 Pod 操作权限（参见 [k8s-setup.md](./k8s-setup.md)）

## Enabling

PodTool 位于 `tool-core` crate 的 `k8s` feature flag 之后：

```bash
cargo build -p tool-core --features k8s
```

Feature flag 传递关系：

```toml
# tool-core/Cargo.toml
[features]
default = []
k8s = ["rara-k8s"]

[dependencies]
rara-k8s = { workspace = true, optional = true }
```

构造方式：

```rust
use std::sync::Arc;
use tool_core::domain_primitives::pod::PodTool;

let manager = Arc::new(rara_k8s::PodManager::new().await?);
let pod_tool = PodTool::new(manager);
```

## Tool Schema

PodTool 的 `parameters_schema()` 返回以下 JSON Schema：

```json
{
  "type": "object",
  "required": ["action"],
  "properties": {
    "action": {
      "type": "string",
      "enum": ["create", "delete", "status", "logs"],
      "description": "The operation to perform"
    },
    "image": {
      "type": "string",
      "description": "Container image (required for create)"
    },
    "name_prefix": {
      "type": "string",
      "description": "Pod name prefix (for create)",
      "default": "rara-pod"
    },
    "namespace": {
      "type": "string",
      "description": "K8s namespace",
      "default": "default"
    },
    "port": {
      "type": "integer",
      "description": "Container port to expose (for create)"
    },
    "command": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Override container entrypoint (for create)"
    },
    "args": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Container arguments (for create)"
    },
    "env": {
      "type": "object",
      "description": "Environment variables (for create)"
    },
    "labels": {
      "type": "object",
      "description": "Extra pod labels (for create)"
    },
    "pod_name": {
      "type": "string",
      "description": "Pod name (required for delete/status/logs)"
    },
    "tail_lines": {
      "type": "integer",
      "description": "Number of log lines to return (for logs)"
    }
  }
}
```

## Actions

### create

创建一个新的 K8s Pod 并等待其进入 Running 状态。

**参数：**

| 参数 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `action` | `string` | 是 | - | `"create"` |
| `image` | `string` | 是 | - | 容器镜像 |
| `name_prefix` | `string` | 否 | `"rara-pod"` | Pod 名称前缀，生成格式为 `{prefix}-{8位uuid}` |
| `namespace` | `string` | 否 | `"default"` | K8s namespace |
| `port` | `integer` | 否 | 无 | 暴露的容器端口 |
| `command` | `string[]` | 否 | 无 | 覆盖容器 entrypoint |
| `args` | `string[]` | 否 | 无 | 容器启动参数 |
| `env` | `object` | 否 | `{}` | 环境变量 |
| `labels` | `object` | 否 | `{}` | 额外 Pod 标签 |

注意：通过 PodTool 创建的 Pod，`restart_policy` 固定为 `Never`，`timeout_secs` 固定为 `120` 秒，不注入 probe，不设置 resource limits。所有 Pod 自动附加 `app.kubernetes.io/managed-by: rara` 标签。

**调用示例：**

```json
{
  "action": "create",
  "image": "python:3.12-slim",
  "name_prefix": "sandbox",
  "namespace": "workloads",
  "command": ["python"],
  "args": ["-c", "import time; time.sleep(3600)"],
  "env": {
    "PYTHONDONTWRITEBYTECODE": "1"
  },
  "labels": {
    "task": "code-exec"
  }
}
```

**返回值：** `PodHandle` 对象

```json
{
  "name": "sandbox-a1b2c3d4",
  "namespace": "workloads",
  "ip": "10.244.0.15",
  "port": null
}
```

### delete

删除指定的 Pod。如果 Pod 已经不存在（NotFound），静默忽略不报错。

**参数：**

| 参数 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `action` | `string` | 是 | - | `"delete"` |
| `pod_name` | `string` | 是 | - | Pod 名称 |
| `namespace` | `string` | 否 | `"default"` | K8s namespace |

**调用示例：**

```json
{
  "action": "delete",
  "pod_name": "sandbox-a1b2c3d4",
  "namespace": "workloads"
}
```

**返回值：**

```json
{
  "deleted": "sandbox-a1b2c3d4"
}
```

### status

查询 Pod 当前状态。

**参数：**

| 参数 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `action` | `string` | 是 | - | `"status"` |
| `pod_name` | `string` | 是 | - | Pod 名称 |
| `namespace` | `string` | 否 | `"default"` | K8s namespace |

**调用示例：**

```json
{
  "action": "status",
  "pod_name": "sandbox-a1b2c3d4",
  "namespace": "workloads"
}
```

**返回值：** `PodStatus` 对象

```json
{
  "name": "sandbox-a1b2c3d4",
  "namespace": "workloads",
  "phase": "Running",
  "ready": true,
  "ip": "10.244.0.15"
}
```

`phase` 可能的值包括：`Pending`、`Running`、`Succeeded`、`Failed`、`Unknown`。`ready` 字段在 `phase` 为 `"Running"` 时为 `true`，其他情况为 `false`。

### logs

获取 Pod 的容器日志。

**参数：**

| 参数 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `action` | `string` | 是 | - | `"logs"` |
| `pod_name` | `string` | 是 | - | Pod 名称 |
| `namespace` | `string` | 否 | `"default"` | K8s namespace |
| `tail_lines` | `integer` | 否 | 无（全部日志） | 返回最近 N 行日志 |

**调用示例：**

```json
{
  "action": "logs",
  "pod_name": "sandbox-a1b2c3d4",
  "namespace": "workloads",
  "tail_lines": 50
}
```

**返回值：**

```json
{
  "logs": "Starting server on :3000\nReady to accept connections\n..."
}
```

## Use Cases

### 在 Pod 中运行不可信代码

Agent 接收用户提交的代码片段，在隔离 Pod 中执行，避免影响宿主环境：

```json
{
  "action": "create",
  "image": "python:3.12-slim",
  "name_prefix": "code-sandbox",
  "command": ["python", "-c"],
  "args": ["print('Hello from sandbox')"]
}
```

执行完成后查看日志获取输出，最后删除 Pod：

```json
{
  "action": "logs",
  "pod_name": "code-sandbox-a1b2c3d4"
}
```

```json
{
  "action": "delete",
  "pod_name": "code-sandbox-a1b2c3d4"
}
```

### 编译和测试

在 Pod 中执行项目编译和测试任务：

```json
{
  "action": "create",
  "image": "rust:1.83-slim",
  "name_prefix": "build-task",
  "command": ["bash", "-c"],
  "args": ["cd /workspace && cargo test --release 2>&1"],
  "env": {
    "CARGO_HOME": "/tmp/cargo",
    "RUSTFLAGS": "-D warnings"
  }
}
```

### 隔离运行第三方工具

将不受信任的第三方工具运行在 Pod 中，通过网络与 agent 通信：

```json
{
  "action": "create",
  "image": "ghcr.io/example/analysis-tool:v2",
  "name_prefix": "analysis",
  "port": 8080,
  "env": {
    "ANALYSIS_MODE": "deep"
  },
  "labels": {
    "tool": "analysis",
    "trust-level": "low"
  }
}
```

## Security

### RBAC 最小权限

PodTool 需要的最小 K8s 权限：

```yaml
rules:
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["create", "delete", "get", "list", "watch"]
  - apiGroups: [""]
    resources: ["pods/log"]
    verbs: ["get"]
```

参见 [k8s-setup.md](./k8s-setup.md) 获取完整 RBAC 配置。

### NetworkPolicy 建议

对于运行不可信代码的 Pod，建议配置 NetworkPolicy 限制网络访问：

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: deny-sandbox-egress
  namespace: workloads
spec:
  podSelector:
    matchLabels:
      task: code-exec
  policyTypes:
    - Egress
  egress: []  # 禁止所有出站流量
```

### 建议实践

- 使用专用 namespace 隔离 agent 创建的 Pod（如 `workloads`、`sandboxes`）
- 为 Pod 设置 resource limits 防止资源滥用（注意：PodTool 当前版本不暴露 resource limits 配置，需要通过 K8s LimitRange 或 ResourceQuota 在 namespace 级别设置）
- 定期清理残留 Pod：`kubectl delete pods -l app.kubernetes.io/managed-by=rara --field-selector status.phase!=Running`
- 使用 `pod_name` 精确操作，避免误删其他 Pod
