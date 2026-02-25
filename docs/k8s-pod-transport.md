# MCP Pod Transport

## Overview

Pod transport 是一种 MCP server 连接方式：将第三方 MCP server 运行在 K8s ephemeral Pod 中，而非宿主进程内。相比 stdio 和 SSE transport，Pod transport 提供以下优势：

- **安全隔离** -- 第三方 MCP server 在独立 Pod 中运行，crash 或资源泄漏不影响宿主 agent 进程
- **K8s 探针自动健康检查** -- 自动注入 liveness/readiness probe，K8s 负责异常检测与重启
- **资源限制** -- 通过 K8s ResourceRequirements 对 CPU/内存设置 requests 和 limits
- **日志管理** -- Pod 日志由 K8s 统一管理，可通过 `kubectl logs` 或 PodTool 查看

## Prerequisites

1. 可用的 K8s 集群（本地 minikube/kind/Docker Desktop 或远程集群）
2. 正确配置的 kubeconfig（`~/.kube/config` 或集群内自动注入的 ServiceAccount）
3. 运行 rara 的 ServiceAccount 具备 Pod 操作权限（参见 [k8s-setup.md](./k8s-setup.md) 中的 RBAC 配置）

## Enabling the Feature

Pod transport 位于 `k8s` feature flag 之后。编译时需要显式启用：

```bash
cargo build -p rara-mcp --features k8s
```

Feature flag 传递关系：

```
rara-mcp --features k8s
  └── rara-k8s (optional dependency, activated by k8s feature)
        ├── kube 3.0 (runtime, client)
        ├── k8s-openapi 0.27 (v1_31)
        └── uuid (pod name generation)
```

`rara-mcp` 的 `Cargo.toml` 中：

```toml
[features]
default = []
k8s = ["rara-k8s"]

[dependencies]
rara-k8s = { workspace = true, optional = true }
```

## Configuration

当 `transport` 设为 `"pod"` 时，以下字段生效（定义在 `McpServerConfig` 中）：

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `transport` | `string` | 是 | `"stdio"` | 必须设为 `"pod"` |
| `pod_image` | `string` | 是 | 无 | MCP server 容器镜像 |
| `pod_namespace` | `string` | 否 | `"default"` | K8s namespace |
| `pod_port` | `integer` | 否 | `3000` | MCP server 监听端口 |
| `pod_labels` | `object` | 否 | 无 | 额外 Pod 标签（与默认标签合并） |
| `env` | `object` | 否 | `{}` | 传递给容器的环境变量 |
| `enabled` | `boolean` | 否 | `true` | 是否启用此 MCP server |
| `startup_timeout_secs` | `integer` | 否 | 无 | 初始化握手超时（秒） |
| `tool_timeout_secs` | `integer` | 否 | 无 | 单次 tool 调用超时（秒） |
| `tools_enabled` | `array` | 否 | 无 | 工具白名单，仅暴露指定工具 |
| `tools_disabled` | `array` | 否 | `[]` | 工具黑名单，隐藏指定工具 |

注意：`pod_image`、`pod_namespace`、`pod_port`、`pod_labels` 字段仅在 `k8s` feature 启用时编译，未启用时这些字段不存在于序列化结构中。

## Configuration Example

完整的 `McpServerConfig` JSON 示例：

```json
{
  "command": "",
  "transport": "pod",
  "enabled": true,
  "pod_image": "ghcr.io/example/mcp-server:latest",
  "pod_namespace": "mcp-servers",
  "pod_port": 8080,
  "pod_labels": {
    "team": "platform",
    "version": "v1"
  },
  "env": {
    "LOG_LEVEL": "info",
    "API_KEY": "sk-xxx"
  },
  "startup_timeout_secs": 60,
  "tool_timeout_secs": 30,
  "tools_enabled": ["search", "fetch"],
  "tools_disabled": []
}
```

最小配置示例：

```json
{
  "command": "",
  "transport": "pod",
  "pod_image": "ghcr.io/example/mcp-server:latest"
}
```

此时 `pod_namespace` 默认为 `"default"`，`pod_port` 默认为 `3000`。

## How It Works

Pod transport 的完整生命周期如下：

```
                    McpPodManager
                         |
  1. create_mcp_pod()    |
     +---------+---------+
     |                   |
     v                   |
  构建 k8s_core::Pod     |
  (MCP 默认标签 +        |
   HTTP probe +          |
   restart: Never)       |
     |                   |
     v                   |
  PodManager.create_pod(pod, namespace, timeout)
     |
     v
  K8s API: POST /api/v1/namespaces/{ns}/pods
     |
     v
  kube-runtime await_condition(is_pod_running)
  超时: 120 秒 (timeout_secs)
     |
     +--- 超时 --> 自动删除 Pod --> 返回 PodTimeout 错误
     |
     v (Running)
  提取 Pod IP
     |
     +--- 无 IP --> 返回 NoPodIp 错误
     |
     v
  返回 (pod_name, pod_ip, port)
     |
     v
  调用方构建 HTTP URL: http://{pod_ip}:{port}
     |
     v
  rmcp Streamable HTTP Client 连接
     |
     v
  MCP handshake (initialize)
     |
     v
  正常 MCP 工具调用
     |
     ...
     |
  2. delete_mcp_pod()
     |
     v
  K8s API: DELETE /api/v1/namespaces/{ns}/pods/{name}
  (NotFound 静默忽略)
```

关键步骤说明：

1. **Pod 名称生成** -- 格式为 `mcp-{server_name}-{8位uuid}`，自动小写、特殊字符替换为 `-`，上限 253 字符
2. **默认标签** -- 所有 MCP Pod 自动附加以下标签：
   - `app.kubernetes.io/managed-by: rara`
   - `rara.dev/component: mcp-server`
   - `rara.dev/server-name: {server_name}`
3. **Restart Policy** -- 固定为 `Never`，Pod 退出不重启（ephemeral 语义）
4. **等待条件** -- 使用 `kube-runtime` 的 `is_pod_running()` 条件等待，超时后自动清理

## Probes

`McpPodManager` 自动为每个 MCP Pod 注入 liveness 和 readiness probe，参数如下：

| 参数 | 值 | 说明 |
|------|----|------|
| `http_get.path` | `/` | HTTP GET 探测路径 |
| `http_get.port` | 与 `pod_port` 相同 | 探测端口 |
| `http_get.scheme` | `HTTP` | 使用 HTTP 协议 |
| `initial_delay_seconds` | `5` | 容器启动后 5 秒开始探测 |
| `period_seconds` | `10` | 每 10 秒探测一次 |
| `timeout_seconds` | `5` | 单次探测超时 5 秒 |
| `failure_threshold` | `3` | 连续失败 3 次标记为不健康 |
| `success_threshold` | `1` | 成功 1 次即恢复 |

liveness 和 readiness probe 使用相同配置。MCP server 容器需要在 `pod_port` 端口的 `/` 路径返回 HTTP 200 响应。

## Troubleshooting

### Pod 启动超时（PodTimeout）

错误信息：`Pod mcp-xxx-12345678 failed to become ready within 120s`

可能原因：
- 镜像拉取失败（检查镜像名称、registry 认证）
- 容器启动缓慢（增大 `timeout_secs`）
- readiness probe 失败（MCP server 未在指定端口响应）

排查步骤：
```bash
kubectl describe pod mcp-xxx-12345678 -n <namespace>
kubectl logs mcp-xxx-12345678 -n <namespace>
kubectl get events -n <namespace> --field-selector involvedObject.name=mcp-xxx-12345678
```

### Pod 无 IP（NoPodIp）

错误信息：`Pod mcp-xxx-12345678 has no IP assigned`

可能原因：
- Pod 达到 Running 状态但 CNI 尚未分配 IP
- 节点网络问题

排查步骤：
```bash
kubectl get pod mcp-xxx-12345678 -n <namespace> -o wide
kubectl describe node <node-name>
```

### K8s API 权限不足（KubeClient）

错误信息：`K8s client error: ...`

可能原因：
- ServiceAccount 缺少 Pod 操作权限
- kubeconfig 未正确配置

排查步骤：
```bash
# 检查当前身份
kubectl auth whoami

# 测试 Pod 创建权限
kubectl auth can-i create pods -n <namespace>
kubectl auth can-i delete pods -n <namespace>
kubectl auth can-i get pods -n <namespace>
```

参见 [k8s-setup.md](./k8s-setup.md) 中的 RBAC 配置。

### 查看 rara 管理的所有 MCP Pod

```bash
kubectl get pods -l app.kubernetes.io/managed-by=rara -l rara.dev/component=mcp-server --all-namespaces
```
