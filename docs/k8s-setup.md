# K8s Environment Setup

本文档说明如何为 rara 的 K8s Pod 功能（MCP Pod transport 和 PodTool）配置 Kubernetes 环境。

## Local Development

### minikube

```bash
# 安装 minikube (macOS)
brew install minikube

# 启动集群
minikube start --driver=docker

# 验证集群可用
kubectl cluster-info
kubectl get nodes
```

### kind (Kubernetes in Docker)

```bash
# 安装 kind
brew install kind

# 创建集群
kind create cluster --name rara-dev

# 验证
kubectl cluster-info --context kind-rara-dev
```

### Docker Desktop

在 Docker Desktop 的 Settings > Kubernetes 中启用 Kubernetes，等待集群启动完成。

```bash
# 验证
kubectl config use-context docker-desktop
kubectl get nodes
```

### 验证 kubeconfig

rara 使用 `kube` crate 的 `Client::try_default()` 获取 K8s 客户端连接：
- 集群内运行时：自动使用 ServiceAccount 的 in-cluster config
- 本地开发时：使用 `~/.kube/config` 中的当前 context

```bash
# 检查当前 context
kubectl config current-context

# 测试 Pod 操作
kubectl run test-pod --image=alpine --restart=Never -- sleep 10
kubectl get pod test-pod
kubectl delete pod test-pod
```

## RBAC Configuration

rara 需要创建、删除、查询 Pod 以及读取 Pod 日志的权限。以下是最小权限配置。

### ServiceAccount

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: rara
  namespace: default
```

### Role

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: rara-pod-manager
  namespace: default
rules:
  # Pod 生命周期管理
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["create", "delete", "get", "list", "watch"]
  # 读取 Pod 日志
  - apiGroups: [""]
    resources: ["pods/log"]
    verbs: ["get"]
```

### RoleBinding

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: rara-pod-manager-binding
  namespace: default
subjects:
  - kind: ServiceAccount
    name: rara
    namespace: default
roleRef:
  kind: Role
  name: rara-pod-manager
  apiGroup: rbac.authorization.k8s.io
```

### 多 namespace 场景

如果 Pod 需要在多个 namespace 中创建（如 `default` 和 `mcp-servers`），需要在每个 namespace 创建 Role + RoleBinding，或者使用 ClusterRole + ClusterRoleBinding：

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: rara-pod-manager
rules:
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["create", "delete", "get", "list", "watch"]
  - apiGroups: [""]
    resources: ["pods/log"]
    verbs: ["get"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: rara-pod-manager-binding
subjects:
  - kind: ServiceAccount
    name: rara
    namespace: default
roleRef:
  kind: ClusterRole
  name: rara-pod-manager
  apiGroup: rbac.authorization.k8s.io
```

### 一键应用

将上述 YAML 保存为 `k8s-rbac.yaml` 后：

```bash
kubectl apply -f k8s-rbac.yaml
```

### 验证权限

```bash
# 使用 rara ServiceAccount 检查权限
kubectl auth can-i create pods --as=system:serviceaccount:default:rara -n default
kubectl auth can-i delete pods --as=system:serviceaccount:default:rara -n default
kubectl auth can-i get pods --as=system:serviceaccount:default:rara -n default
kubectl auth can-i list pods --as=system:serviceaccount:default:rara -n default
kubectl auth can-i watch pods --as=system:serviceaccount:default:rara -n default
kubectl auth can-i get pods/log --as=system:serviceaccount:default:rara -n default
```

所有命令应返回 `yes`。

## Network Policies

### 禁止 sandbox Pod 所有出站流量

适用于运行不可信代码的场景，完全禁止 Pod 的出站网络访问：

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: deny-sandbox-egress
  namespace: default
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/managed-by: rara
  policyTypes:
    - Egress
  egress: []
```

### 仅允许 DNS 查询

如果 Pod 需要解析域名但不应访问外部服务：

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: sandbox-dns-only
  namespace: default
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/managed-by: rara
  policyTypes:
    - Egress
  egress:
    - to: []
      ports:
        - protocol: UDP
          port: 53
        - protocol: TCP
          port: 53
```

### 允许 MCP Pod 被 agent 访问

MCP Pod transport 场景中，agent 需要通过 HTTP 连接 Pod。确保入站流量策略不会阻止：

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: allow-mcp-ingress
  namespace: default
spec:
  podSelector:
    matchLabels:
      rara.dev/component: mcp-server
  policyTypes:
    - Ingress
  ingress:
    - from:
        - podSelector:
            matchLabels:
              app: rara
      ports:
        - protocol: TCP
          port: 3000
```

注意：NetworkPolicy 需要集群安装了支持的 CNI 插件（如 Calico、Cilium）。minikube 默认不支持 NetworkPolicy，可通过 `minikube start --cni=calico` 启用。

## Resource Limits

rara 的 `PodSpec` 支持通过 `ResourceSpec` 设置容器资源 requests 和 limits。

### ResourceSpec 字段

| 字段 | 类型 | 说明 | 示例 |
|------|------|------|------|
| `cpu_request` | `string` | CPU 请求量 | `"100m"` (0.1 核) |
| `cpu_limit` | `string` | CPU 上限 | `"500m"` (0.5 核) |
| `memory_request` | `string` | 内存请求量 | `"64Mi"` |
| `memory_limit` | `string` | 内存上限 | `"256Mi"` |

所有字段均为可选。值格式遵循 K8s 资源量表示法。

### 建议值

| 场景 | CPU Request | CPU Limit | Memory Request | Memory Limit |
|------|-------------|-----------|----------------|--------------|
| MCP Server (轻量) | `100m` | `500m` | `64Mi` | `256Mi` |
| MCP Server (标准) | `250m` | `1000m` | `128Mi` | `512Mi` |
| 代码执行 sandbox | `200m` | `1000m` | `128Mi` | `512Mi` |
| 编译任务 | `500m` | `2000m` | `512Mi` | `2Gi` |

### Namespace 级别 ResourceQuota

即使单个 Pod 未设置 resource limits，也可以通过 ResourceQuota 在 namespace 级别限制总资源消耗：

```yaml
apiVersion: v1
kind: ResourceQuota
metadata:
  name: rara-pod-quota
  namespace: default
spec:
  hard:
    pods: "20"
    requests.cpu: "4"
    requests.memory: "4Gi"
    limits.cpu: "8"
    limits.memory: "8Gi"
```

### Namespace 级别 LimitRange

为未设置 resource limits 的 Pod 自动注入默认值：

```yaml
apiVersion: v1
kind: LimitRange
metadata:
  name: rara-pod-limits
  namespace: default
spec:
  limits:
    - type: Container
      default:
        cpu: "500m"
        memory: "256Mi"
      defaultRequest:
        cpu: "100m"
        memory: "64Mi"
```

## Monitoring

### 查看 rara 管理的所有 Pod

```bash
# 所有 rara 管理的 Pod
kubectl get pods -l app.kubernetes.io/managed-by=rara --all-namespaces

# 仅 MCP server Pod
kubectl get pods -l rara.dev/component=mcp-server --all-namespaces

# 详细信息（含 IP、节点）
kubectl get pods -l app.kubernetes.io/managed-by=rara --all-namespaces -o wide
```

### 查看单个 Pod 状态

```bash
# Pod 详情（事件、条件、容器状态）
kubectl describe pod <pod-name> -n <namespace>

# Pod YAML 完整定义
kubectl get pod <pod-name> -n <namespace> -o yaml
```

### 查看 Pod 日志

```bash
# 全部日志
kubectl logs <pod-name> -n <namespace>

# 最近 100 行
kubectl logs <pod-name> -n <namespace> --tail=100

# 实时跟踪
kubectl logs <pod-name> -n <namespace> -f

# 已退出容器的日志
kubectl logs <pod-name> -n <namespace> --previous
```

### 查看事件

```bash
# namespace 级别事件
kubectl get events -n <namespace> --sort-by=.lastTimestamp

# 特定 Pod 事件
kubectl get events -n <namespace> --field-selector involvedObject.name=<pod-name>
```

### 清理残留 Pod

```bash
# 删除所有已完成/失败的 rara Pod
kubectl delete pods -l app.kubernetes.io/managed-by=rara \
  --field-selector status.phase=Succeeded --all-namespaces

kubectl delete pods -l app.kubernetes.io/managed-by=rara \
  --field-selector status.phase=Failed --all-namespaces

# 删除所有 rara Pod（谨慎使用）
kubectl delete pods -l app.kubernetes.io/managed-by=rara --all-namespaces
```
