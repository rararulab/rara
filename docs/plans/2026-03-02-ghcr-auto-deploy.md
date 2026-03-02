# GHCR Auto Deploy Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 当 GHCR 有新镜像时，内网 K8s 自动拉取并滚动更新 rara 应用。

**Architecture:** 分两层实现 — GitHub Actions 负责构建并推送 Docker 镜像到 GHCR（外网），Keel operator 在内网 K8s 集群中轮询 GHCR 检测 digest 变化并自动触发滚动更新。

**Tech Stack:** GitHub Actions, Docker buildx, GHCR, Keel (keel.sh), Pulumi Go

---

## Issue 拆分

两个独立 issue，可并行执行：

| Issue | 标题 | 范围 |
|-------|------|------|
| #1 | `feat(ci): add Docker image publish workflow` | `.github/workflows/` + `docker/Dockerfile` |
| #2 | `feat(infra): add Keel for automatic image updates` | `infra/pkg/infra/` + `infra/pkg/app/` |

---

## Task 1: GitHub Actions Docker 镜像发布 Workflow

**Issue:** `feat(ci): add Docker image publish workflow`

**Files:**
- Create: `.github/workflows/docker-publish.yml`
- Modify: `docker/Dockerfile` (使 base image 可配置)

### Step 1: 修改 Dockerfile 支持 CI 构建

当前 `docker/Dockerfile` 硬编码 `FROM rara-base:latest`，这在 CI 环境中不存在。需要改为可配置的 base image。

修改 `docker/Dockerfile` 第 7 行：

```dockerfile
# 之前:
FROM rara-base:latest AS chef

# 之后:
ARG BASE_IMAGE=rara-base:latest
FROM ${BASE_IMAGE} AS chef
```

这样本地开发不受影响（仍然用 `rara-base:latest`），CI 可以传入 `--build-arg BASE_IMAGE=...`。

### Step 2: 创建 docker-publish.yml workflow

创建 `.github/workflows/docker-publish.yml`:

```yaml
name: Docker Publish

on:
  push:
    branches: [main]
    paths-ignore:
      - "docs/**"
      - "*.md"
      - ".github/workflows/web.yml"

concurrency:
  group: docker-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read
  packages: write

env:
  REGISTRY: ghcr.io
  BACKEND_IMAGE: ghcr.io/rararulab/rara
  FRONTEND_IMAGE: ghcr.io/rararulab/rara-web
  BASE_IMAGE: ghcr.io/rararulab/rara-base

jobs:
  # Gate on CI passing
  ci:
    uses: ./.github/workflows/ci.yml
    secrets:
      CODECOV_TOKEN: ${{ secrets.CODECOV_TOKEN }}

  build-base:
    name: Build Base Image
    needs: [ci]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - uses: docker/setup-buildx-action@v3

      - uses: docker/build-push-action@v6
        with:
          context: .
          file: docker/base.Dockerfile
          push: true
          tags: |
            ${{ env.BASE_IMAGE }}:latest
          cache-from: type=registry,ref=${{ env.BASE_IMAGE }}:buildcache
          cache-to: type=registry,ref=${{ env.BASE_IMAGE }}:buildcache,mode=max

  build-backend:
    name: Build Backend Image
    needs: [build-base]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - uses: docker/setup-buildx-action@v3

      - uses: docker/build-push-action@v6
        with:
          context: .
          file: docker/Dockerfile
          push: true
          build-args: |
            BASE_IMAGE=${{ env.BASE_IMAGE }}:latest
          tags: |
            ${{ env.BACKEND_IMAGE }}:latest
            ${{ env.BACKEND_IMAGE }}:sha-${{ github.sha }}
          cache-from: type=registry,ref=${{ env.BACKEND_IMAGE }}:buildcache
          cache-to: type=registry,ref=${{ env.BACKEND_IMAGE }}:buildcache,mode=max

  build-frontend:
    name: Build Frontend Image
    needs: [ci]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - uses: docker/setup-buildx-action@v3

      # Frontend needs docs/book for the embedded docs
      - name: Setup mdBook
        uses: jontze/action-mdbook@v4
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
          use-linkcheck: true
          use-mermaid: true
          use-toc: true

      - name: Build mdbook
        run: mdbook build docs

      - uses: docker/build-push-action@v6
        with:
          context: .
          file: docker/web.Dockerfile
          push: true
          tags: |
            ${{ env.FRONTEND_IMAGE }}:latest
            ${{ env.FRONTEND_IMAGE }}:sha-${{ github.sha }}
          cache-from: type=registry,ref=${{ env.FRONTEND_IMAGE }}:buildcache
          cache-to: type=registry,ref=${{ env.FRONTEND_IMAGE }}:buildcache,mode=max
```

**关键设计决策:**
- `build-frontend` 不依赖 `build-base`，可与 `build-base` 并行运行
- `build-backend` 依赖 `build-base`（需要 base image）
- 双 tag 策略: `latest` + `sha-{commit}` — Keel 用 `latest` 检测 digest 变化，`sha-*` 提供可追溯性
- 使用 registry cache 加速后续构建
- `paths-ignore` 避免纯文档变更触发构建

### Step 3: Commit

```bash
git add docker/Dockerfile .github/workflows/docker-publish.yml
git commit -m "feat(ci): add Docker image publish workflow

Build and push backend/frontend images to GHCR on push to main.
- Base image published separately for layer caching
- Dual tag: latest + sha-{commit}
- Frontend build runs parallel to base image build"
```

---

## Task 2: Pulumi 添加 Keel 自动更新

**Issue:** `feat(infra): add Keel for automatic image updates`

**Files:**
- Create: `infra/pkg/infra/keel.go`
- Modify: `infra/pkg/infra/stack.go` (添加 Keel 到部署链)
- Modify: `infra/pkg/infra/config.go` (无需改动，Keel 无特殊配置)
- Modify: `infra/pkg/app/config.go` (默认 `imagePullPolicy` 改为 `Always`)
- Modify: `infra/pkg/app/stack.go` (添加 Keel annotations)

### Step 1: 创建 `infra/pkg/infra/keel.go`

Keel 通过 Helm chart 部署。参考现有的 `network.go` / `data.go` 模式：

```go
package infra

import (
	"fmt"

	helmv4 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/helm/v4"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// KeelResult holds references to the Keel deployment.
type KeelResult struct {
	Keel *helmv4.Chart
}

// DeployKeel deploys Keel image update controller via Helm.
func DeployKeel(ctx *pulumi.Context, cfg *InfraConfig) (*KeelResult, error) {
	name := fmt.Sprintf("%s-keel", cfg.Prefix())

	keel, err := helmv4.NewChart(ctx, name, &helmv4.ChartArgs{
		Chart:     pulumi.String("keel"),
		Namespace: pulumi.String(cfg.Namespace),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://charts.keel.sh"),
		},
		Values: pulumi.Map{
			// Polling mode — no webhooks needed (internal network)
			"polling": pulumi.Map{
				"enabled":  pulumi.Bool(true),
				"schedule": pulumi.String("@every 2m"),
			},
			// Disable webhook — internal network, no inbound access
			"webhook": pulumi.Map{
				"enabled": pulumi.Bool(false),
			},
			// Basic auth for dashboard (optional)
			"basicauth": pulumi.Map{
				"enabled": pulumi.Bool(false),
			},
			// Resource limits
			"resources": pulumi.Map{
				"requests": pulumi.StringMap{
					"cpu":    pulumi.String("25m"),
					"memory": pulumi.String("64Mi"),
				},
				"limits": pulumi.StringMap{
					"cpu":    pulumi.String("100m"),
					"memory": pulumi.String("128Mi"),
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return &KeelResult{Keel: keel}, nil
}
```

### Step 2: 修改 `infra/pkg/infra/stack.go`

在 observability 之后、Consul KV seeding 之前添加 Keel 部署：

在 `// Layer 3: Observability (Helm)` 块之后添加:

```go
	// Keel — automatic image update controller
	keel, err := DeployKeel(ctx, cfg)
	if err != nil {
		return err
	}
```

把 `keel.Keel` 加入 `consulDeps` 不是必须的（Keel 不依赖 Consul），但为了一致性可以不加。无需改动 `consulDeps`。

在 `ServicesResult` 中不需要添加 Keel，因为 Keel 属于基础设施层而不是 services 层。

添加 export:

```go
	ctx.Export("keelEnabled", pulumi.Bool(true))
```

注意：`keel` 变量如果没有被使用需要处理（Go 不允许未使用变量）。可以用 `_ = keel` 或将其加入 export。

### Step 3: 修改 `infra/pkg/app/config.go`

将 `BackendPullPolicy` 和 `FrontendPullPolicy` 的默认值从 `IfNotPresent` 改为 `Always`:

```go
// 之前:
backendPull := cfg.Get("backend.imagePullPolicy")
if backendPull == "" {
    backendPull = "IfNotPresent"
}

// 之后:
backendPull := cfg.Get("backend.imagePullPolicy")
if backendPull == "" {
    backendPull = "Always"
}
```

frontend 同理。

### Step 4: 修改 `infra/pkg/app/stack.go`

为 backend 和 frontend Deployment 的 Pod template 添加 Keel annotations。

在 backend Deployment 的 `Template > Metadata` 中添加 annotations:

```go
Template: &corev1.PodTemplateSpecArgs{
    Metadata: &metav1.ObjectMetaArgs{
        Labels: pulumi.ToStringMap(backendLabels),
        Annotations: pulumi.StringMap{
            "keel.sh/policy":       pulumi.String("force"),
            "keel.sh/trigger":      pulumi.String("poll"),
            "keel.sh/pollSchedule": pulumi.String("@every 2m"),
        },
    },
```

**注意：** Keel annotations 需要放在 **Deployment 的 metadata** 上，不是 Pod template 的 metadata。

修正：annotations 应在 Deployment 的顶层 `Metadata` 中：

```go
_, err := appsv1.NewDeployment(ctx, fmt.Sprintf("%s-backend", prefix), &appsv1.DeploymentArgs{
    Metadata: &metav1.ObjectMetaArgs{
        Name:      pulumi.String(fmt.Sprintf("%s-backend", prefix)),
        Namespace: pulumi.String(ns),
        Labels:    pulumi.ToStringMap(backendLabels),
        Annotations: pulumi.StringMap{
            "keel.sh/policy":       pulumi.String("force"),
            "keel.sh/trigger":      pulumi.String("poll"),
            "keel.sh/pollSchedule": pulumi.String("@every 2m"),
        },
    },
```

Frontend Deployment 同理。

### Step 5: 验证编译

```bash
cd infra && go build ./...
```

### Step 6: Commit

```bash
git add infra/pkg/infra/keel.go infra/pkg/infra/stack.go infra/pkg/app/config.go infra/pkg/app/stack.go
git commit -m "feat(infra): add Keel for automatic image updates

- Deploy Keel via Helm in infra stack (polling mode, 2m interval)
- Add keel.sh annotations to backend/frontend Deployments
- Change default imagePullPolicy to Always
- No inbound network access required — Keel polls GHCR outbound"
```

---

## 部署流程（完成后）

```
Developer pushes to main
    │
    ├─→ GitHub Actions: CI (lint + test)
    │       │
    │       ├─→ build-base → push ghcr.io/rararulab/rara-base:latest
    │       │       │
    │       │       └─→ build-backend → push ghcr.io/rararulab/rara:latest + :sha-xxx
    │       │
    │       └─→ build-frontend → push ghcr.io/rararulab/rara-web:latest + :sha-xxx
    │
    └─→ (≤2 min later) Keel detects new digest on GHCR
            │
            └─→ K8s rolling update: pull new image → start new pod → drain old pod
```

## 验证清单

- [ ] Push to main 后，GitHub Actions 成功构建并推送 backend/frontend 镜像到 GHCR
- [ ] `pulumi up` infra stack 成功部署 Keel
- [ ] `pulumi up` app stack 的 Deployment 包含 Keel annotations 和 `imagePullPolicy: Always`
- [ ] Keel 日志显示正在轮询 GHCR（`kubectl logs -l app=keel`）
- [ ] 手动 push 新镜像后，Keel 在 2 分钟内触发滚动更新
