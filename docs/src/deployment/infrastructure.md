# Infrastructure Components

Reference for all infrastructure services deployed by `rara-infra`.

## Core

| Component | Version / Image | Port | Purpose |
|-----------|----------------|------|---------|
| PostgreSQL | `ghcr.io/pgmq/pg18-pgmq:v1.10.0` (Bitnami chart v18.4.0) | 5432 | Primary database with pgmq message queue extension |
| MinIO | Chart v5.4.0 | 9000 (API), 9001 (console) | S3-compatible object storage. Buckets: `rara`, `langfuse`, `quickwit` |
| ChromaDB | `chromadb/chroma:latest` (custom templates) | 8000 | Vector database for memory/embedding search |
| Crawl4AI | `unclecode/crawl4ai:latest` (custom templates) | 11235 | Web crawling and content extraction |
| Consul | Hashicorp chart v1.9.3 | 8500 | Configuration KV store. Single-server, no service mesh |

**Default credentials:**

| Service | User | Password | Console URL |
|---------|------|----------|-------------|
| PostgreSQL | `postgres` | `postgres` | — |
| MinIO | `minioadmin` | `minioadmin` | `https://minio.rara.local` (port 9001) |

## Ingress & TLS

| Component | Chart Version | Purpose |
|-----------|--------------|---------|
| Traefik | v39.0.2 | Ingress controller. HTTP automatically redirects to HTTPS. IngressRoute CRDs for routing |
| cert-manager | v1.19.3 | Manages self-signed CA chain. Issues wildcard certificate for `*.rara.local` |

The TLS chain: self-signed ClusterIssuer creates a root CA, which signs a wildcard certificate referenced by all IngressRoutes.

**Endpoints:**

| URL | Service |
|-----|---------|
| `https://traefik.rara.local` | Traefik dashboard |

## Observability

| Component | Chart Version | Purpose |
|-----------|--------------|---------|
| Prometheus | kube-prometheus-stack v82.2.1 | Metrics collection and alerting rules. 7-day retention (default) |
| Grafana | (bundled) | Dashboards. Pre-configured datasources: Prometheus, Tempo, Quickwit |
| AlertManager | (bundled) | Alert routing and notification |
| Tempo | v1.24.4 | Distributed tracing backend (single binary mode). Receives OTLP via gRPC/HTTP |
| Alloy | v1.6.0 | OpenTelemetry collector. Routes traces to Tempo, metrics to Prometheus, logs to Quickwit |
| Quickwit | v0.7.21 | Log search engine. Uses MinIO for index storage |

**Data flow:**

```
App (OTLP) --> Alloy --> Tempo      (traces)
                     --> Prometheus  (metrics via remote write)
                     --> Quickwit   (logs via OTLP HTTP)
```

**Default credentials:**

| Service | Access |
|---------|--------|
| Grafana | Anonymous access enabled (Admin role). Login form disabled. Fallback: `admin` / `admin` |

**Endpoints:**

| URL | Service |
|-----|---------|
| `https://grafana.rara.local` | Grafana dashboards |
| `https://prometheus.rara.local` | Prometheus query UI |
| `https://alertmanager.rara.local` | AlertManager UI |
| `https://tempo.rara.local` | Tempo API |
| `https://quickwit.rara.local` | Quickwit search UI |

## Platform

| Component | Chart Version | Purpose |
|-----------|--------------|---------|
| Langfuse | v1.5.20 | LLM observability -- trace, evaluate, and monitor LLM calls. Deploys its own PostgreSQL, ClickHouse, and Redis instances. Uses shared MinIO for S3 storage |

**Default credentials:**

| Service | Access |
|---------|--------|
| Langfuse | Create account on first visit at `https://langfuse.rara.local`. After creating a project, copy the API keys to `consulSeed.langfuse.publicKey/secretKey` in `values.yaml` |

**Endpoints:**

| URL | Service |
|-----|---------|
| `https://langfuse.rara.local` | Langfuse web UI |
| `https://consul.rara.local` | Consul KV UI |
| `https://minio.rara.local` | MinIO console |

## Consul KV — Service Discovery

Consul KV stores all infrastructure connection info under `rara/config/`. The rara app reads this at startup when `CONSUL_HTTP_ADDR` is set, eliminating the need for individual environment variables.

**Seeded keys:**

| Key prefix | Component | Example value |
|------------|-----------|---------------|
| `database/` | PostgreSQL | `postgres://postgres:postgres@rara-infra-postgresql:5432/rara` |
| `object_store/` | MinIO | endpoint, access_key_id, secret_access_key, bucket |
| `memory/` | ChromaDB | `http://rara-infra-chromadb:8000` |
| `crawl4ai/` | Crawl4AI | `http://rara-infra-crawl4ai:11235` |
| `langfuse/` | Langfuse | host, public_key, secret_key |

Service URLs default to cluster-internal DNS. Override via `consulSeed.overrides.*` in `values.yaml` when the app runs outside K8s. See [Kubernetes deployment](kubernetes.md#url-override-out-of-cluster-app) for details.

**Useful commands:**

```bash
cd deploy/helm
just seed-consul   # seed/re-seed all keys
just consul-keys   # list current keys
```
