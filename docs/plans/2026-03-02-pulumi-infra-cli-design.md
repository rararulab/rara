# Pulumi Infra CLI Design

## Overview

用 Pulumi + Go 替代现有的 `deploy/helm/justfile` 编排逻辑，实现 "给一个 K8s 集群，一条命令搭建完所有依赖" 的体验。

## 技术栈

- **Pulumi SDK v3** (`github.com/pulumi/pulumi/sdk/v3`)
- **Pulumi Kubernetes Provider v4** (`github.com/pulumi/pulumi-kubernetes/sdk/v4`)
- **Pulumi Command Provider** (`github.com/pulumi/pulumi-command/sdk`)
- **Go 1.25+**
- **State**: 本地 (`pulumi login --local`, `~/.pulumi`)

## 项目结构

```
infra/
├── cmd/main.go              # CLI 入口
├── go.mod
├── pkg/
│   ├── infra/               # infra stack
│   │   ├── stack.go         # 编排入口：注册所有组件
│   │   ├── network.go       # Traefik + cert-manager
│   │   ├── data.go          # PostgreSQL + MinIO + Consul
│   │   ├── observability.go # Prometheus stack + Tempo + Alloy + Quickwit + Langfuse
│   │   ├── services.go      # ChromaDB, Crawl4AI, Memos, Hindsight, Mem0, Ollama
│   │   ├── consul.go        # Consul KV seeding
│   │   └── config.go        # Pulumi config → Go struct
│   └── app/                 # app stack
│       ├── stack.go         # backend + frontend Deployment/Service
│       ├── ingress.go       # Traefik IngressRoute CRD
│       └── config.go        # app 配置
├── Pulumi.yaml              # 项目定义 (name: rara)
├── Pulumi.infra-dev.yaml    # infra stack 配置值
├── Pulumi.app-dev.yaml      # app stack 配置值
└── justfile                 # 便捷命令
```

## Stack 划分

| Stack | 名称 | 职责 |
|-------|------|------|
| infra | `infra-dev` | 所有基础设施 + Consul KV seeding |
| app   | `app-dev`   | rara-app backend + frontend |

`app-dev` 通过 `pulumi.StackReference` 读取 `infra-dev` 的 output（namespace、Consul 地址等）。

## Infra Stack 组件

### 第一层：网络 + 证书 (Helm Release)
- **Traefik** — Ingress controller, LoadBalancer
- **cert-manager** — TLS 管理 + ClusterIssuer + CA Issuer + 通配符证书 (Pulumi K8s 资源)

### 第二层：数据 + 配置 (Helm Release)
- **PostgreSQL** — Bitnami chart, pgmq 镜像
- **MinIO** — 对象存储
- **Consul** — KV 配置中心

### 第三层：自定义组件 (纯 Pulumi K8s 资源)
- **ChromaDB** — Deployment + Service + PVC
- **Crawl4AI** — Deployment + Service
- **Memos** — Deployment + Service + PVC + 独立 PostgreSQL
- **Hindsight** — Deployment + Service + 独立 PostgreSQL (pgvector)
- **Mem0** — Deployment + Service + ConfigMap
- **Ollama** — Deployment + Service + PVC

### 第三层：可观测性 (Helm Release)
- **kube-prometheus-stack** — Prometheus + Grafana + AlertManager
- **Tempo** — 分布式追踪
- **Alloy** — OpenTelemetry collector
- **Quickwit** — 日志搜索
- **Langfuse** — LLM 可观测性

### 最后：Consul KV Seeding
- 依赖所有组件部署完成
- 通过 `pulumi-command` 执行 `kubectl exec consul-server-0` 写入
- KV 值直接引用组件的 Pulumi output (service name, endpoint)

```
rara/config/database/database_url        ← PostgreSQL service + password
rara/config/database/migration_dir       ← "crates/rara-model/migrations"
rara/config/http/bind_address            ← "0.0.0.0:25555"
rara/config/grpc/bind_address            ← "0.0.0.0:50051"
rara/config/main_service_http_base       ← app backend service URL
rara/config/object_store/endpoint        ← MinIO service URL
rara/config/object_store/access_key_id   ← Pulumi config
rara/config/object_store/secret_access_key ← Pulumi config
rara/config/object_store/bucket          ← "rara"
rara/config/memory/mem0_base_url         ← Mem0 service URL
rara/config/memory/memos_base_url        ← Memos service URL
rara/config/memory/memos_token           ← ""
rara/config/memory/hindsight_base_url    ← Hindsight service URL
rara/config/memory/hindsight_bank_id     ← "default"
rara/config/crawl4ai/base_url            ← Crawl4AI service URL
rara/config/langfuse/host                ← Langfuse-web service URL
rara/config/langfuse/public_key          ← Pulumi config
rara/config/langfuse/secret_key          ← Pulumi config
```

## App Stack 组件 (纯 Pulumi K8s 资源)

- **Backend Deployment** — `ghcr.io/rararulab/rara:{tag}`, ports: 25555 (HTTP) + 50051 (gRPC)
- **Backend Service** — ClusterIP
- **Frontend Deployment** — `ghcr.io/rararulab/rara-web:{tag}`, port: 80
- **Frontend Service** — ClusterIP
- **IngressRoute** — Traefik CRD: `app.rara.local` → frontend, `api.rara.local` → backend
- **ServiceMonitor** — Prometheus CRD: scrape backend metrics

## 开发者工作流 (justfile)

```justfile
init:          # pulumi login --local + stack init
infra-up:      # pulumi up -s infra-dev
app-up:        # pulumi up -s app-dev
setup:         # infra-up + app-up (一键搭建)
infra-preview: # pulumi preview -s infra-dev
app-preview:   # pulumi preview -s app-dev
infra-destroy: # pulumi destroy -s infra-dev
app-destroy:   # pulumi destroy -s app-dev
```

核心体验：**拿到 K8s 集群 → `just setup` → 全部就绪**

## 决策汇总

| 决策 | 选择 |
|------|------|
| 工具 | Pulumi + Go |
| 第三方组件 | Helm Release via Pulumi |
| 自定义组件 | 纯 Pulumi K8s 资源 |
| rara-app | 纯 Pulumi K8s 资源 |
| Stack 划分 | infra-dev + app-dev |
| Consul KV | Pulumi 内通过 command provider 写入 |
| State | 本地 (~/.pulumi) |
| 所有组件 | 全部必须部署，无可选开关 |
