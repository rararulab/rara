# rara-infra Helm Chart

Umbrella chart that deploys all infrastructure dependencies for the rara platform. The rara application itself is **not** included.

## Components

| Component | Chart Source | Default Domain | Purpose |
|-----------|-------------|----------------|---------|
| Traefik | `traefik/traefik` | `traefik.rara.local` | Ingress controller |
| cert-manager | `jetstack/cert-manager` | — | Self-signed CA + TLS certificates |
| kube-prometheus-stack | `prometheus-community` | `grafana.rara.local` `prometheus.rara.local` `alertmanager.rara.local` | Monitoring (Prometheus + Grafana + AlertManager) |
| Tempo | `grafana/tempo` | `tempo.rara.local` | Distributed tracing |
| Alloy | `grafana/alloy` | — | OpenTelemetry collector |
| Quickwit | `quickwit/quickwit` | `quickwit.rara.local` | Log search |
| Langfuse | `langfuse/langfuse` | `langfuse.rara.local` | LLM observability |
| Consul | `hashicorp/consul` | `consul.rara.local` | Configuration KV store |
| MinIO | `minio/minio` | `minio.rara.local` | Object storage |
| PostgreSQL | `bitnami/postgresql` | — | Database (pgmq image) |
| ChromaDB | custom templates | — | Vector database |
| Crawl4AI | custom templates | — | Web scraping |

## Prerequisites

- Kubernetes 1.33+
- [Helm 4.0+](https://helm.sh/blog/helm-4-released/)
- [just](https://github.com/casey/just) (optional, for shortcuts)

## Quick Start

```bash
# Download subchart dependencies
just deps

# Install with dev values (minimal resources)
just install-dev

# Or install with production values
just install
```

### Manual (without just)

```bash
cd deploy/helm
helm dependency build rara-infra
helm install rara-infra rara-infra -n rara --create-namespace -f rara-infra/values-dev.yaml
```

## Configuration

All components can be toggled on/off via `values.yaml`:

```yaml
traefik:
  enabled: true    # set to false to skip
postgresql:
  enabled: true
minio:
  enabled: true
# ...
```

### Values Files

| File | Description |
|------|-------------|
| `rara-infra/values.yaml` | Default values — all components enabled, production-sized resources |
| `rara-infra/values-dev.yaml` | Dev overlay — minimal CPU/memory requests, smaller PVCs |

### Key Defaults

| Setting | Value |
|---------|-------|
| Domain | `*.rara.local` (self-signed TLS) |
| PostgreSQL credentials | `postgres` / `postgres` |
| PostgreSQL databases | `rara` (app), `langfuse` (auto-created) |
| PostgreSQL image | `ghcr.io/pgmq/pg18-pgmq:v1.10.0` |
| MinIO credentials | `minioadmin` / `minioadmin` |
| MinIO buckets | `rara`, `langfuse`, `quickwit` |
| Grafana credentials | `admin` / `admin` |

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HELM_RELEASE` | `rara-infra` | Helm release name |
| `HELM_NAMESPACE` | `rara` | Kubernetes namespace |

## TLS Certificates

The chart creates a self-signed CA chain via cert-manager:

1. `ClusterIssuer` (self-signed) → creates root CA certificate
2. Root CA `Certificate` → stored in `*-ca-tls` secret
3. `ClusterIssuer` (CA) → uses root CA to sign service certificates
4. Wildcard `Certificate` for `*.rara.local`

All Traefik IngressRoutes reference the wildcard TLS secret.

To trust the CA locally:

```bash
kubectl get secret rara-infra-ca-tls -n rara -o jsonpath='{.data.ca\.crt}' | base64 -d > rara-ca.crt
# macOS
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain rara-ca.crt
# Linux
sudo cp rara-ca.crt /usr/local/share/ca-certificates/ && sudo update-ca-certificates
```

## Observability Pipeline

```
App (OTLP) → Alloy → Tempo    (traces)
                    → Prometheus (metrics)
                    → Quickwit  (logs)

Grafana dashboards query all three backends.
```

## Justfile Commands

Run `just` from `deploy/helm/` to see all available commands:

```
⎈ helm justfile:

📊 Status
    history     show release history
    list        list all releases in namespace
    status      show release status
    values      show deployed values

📦 Dependencies
    deps        download / update subchart dependencies
    deps-list   list current subchart dependency versions
    deps-update update subchart dependencies to latest allowed versions

✅ Validate
    diff        diff upcoming changes against live release
    lint        lint chart with default values
    lint-dev    lint chart with dev overlay
    template    render templates locally (dry-run)
    template-dev render templates with dev overlay

🚀 Deploy
    install      install chart (production values, server-side apply)
    install-dev  install chart (dev values, server-side apply)
    uninstall    uninstall release
    upgrade      upgrade existing release (server-side apply)
    upgrade-dev  upgrade with dev overlay (server-side apply)
    upgrade-safe upgrade with rollback on failure

🩺 Doctor
    doctor       check infrastructure health across all components
```

### Doctor

`just doctor` checks every component's health in one shot:

```
🩺 rara-infra doctor — namespace: rara
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📦 Helm Release
 ✔  release deployed             deployed

🗄️  Core Infrastructure
 ✔  PostgreSQL                   1/1 Running
 ✔  MinIO                        1/1 Running
 ✔  ChromaDB                     1/1 Running
 ✔  Crawl4AI                     1/1 Running

🌐 Ingress & TLS
 ✔  Traefik                      1/1 Running
 ✔  cert-manager                 1/1 Running
 ✔  wildcard certificate         Ready

📈 Observability
 ✔  Prometheus                   2/2 Running
 ✔  Grafana                      1/1 Running
 ...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ✔ 16 passed
```
