# Docker Compose (Local Dev)

The root `docker-compose.yml` provides infrastructure services for local development. The rara backend and frontend run natively on the host.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) with Compose v2
- Rust toolchain (for building the backend)
- Node.js 20+ (for building the frontend)

## Quick Start

```bash
# Start infrastructure services
docker compose up -d

# Run the backend (reads RARA__ env vars or .env)
cargo run -p rara-cmd

# Run the frontend dev server (proxies /api to localhost:25555)
cd web && npm run dev
```

## Services

| Service | Image | Port | Purpose |
|---------|-------|------|---------|
| `postgres` | `ghcr.io/pgmq/pg18-pgmq:v1.10.0` | 5432 | PostgreSQL with pgmq extension |
| `minio` | `minio/minio` | 9000 (API), 9001 (console) | S3-compatible object storage |
| `minio-init` | `minio/mc` | -- | Creates the `job-markdown` bucket on startup |
| `chroma` | `chromadb/chroma` | 8000 | Vector database |
| `crawl4ai` | `unclecode/crawl4ai` | 11235 | Web crawler service |

All services use Docker-managed named volumes for data persistence.

## Configuration

In local dev mode (without Consul), the backend reads `RARA__`-prefixed environment variables. Sensible defaults match the docker-compose services:

| Variable | Default | Notes |
|----------|---------|-------|
| `RARA__DATABASE__DATABASE_URL` | `postgres://postgres:postgres@localhost:5432/rara` | Matches postgres service |
| `RARA__OBJECT_STORE__ENDPOINT` | `http://localhost:9000` | Matches minio service |
| `RARA__OBJECT_STORE__ACCESS_KEY` | `minioadmin` | MinIO root user |
| `RARA__OBJECT_STORE__SECRET_KEY` | `minioadmin` | MinIO root password |
| `RARA__MEMORY__CHROMA_URL` | `http://localhost:8000` | Matches chroma service |
| `RARA__CRAWL4AI__BASE_URL` | `http://localhost:11235` | Matches crawl4ai service |

**Default credentials:**

| Service | User | Password |
|---------|------|----------|
| PostgreSQL | `postgres` | `postgres` |
| MinIO | `minioadmin` | `minioadmin` |

See [Configuration](../configuration.md) for the full list of config keys.

## Accessing Services

| Service | URL |
|---------|-----|
| Backend API | <http://localhost:25555> |
| Frontend (Vite dev) | <http://localhost:5173> |
| MinIO Console | <http://localhost:9001> |
| ChromaDB API | <http://localhost:8000> |
| Crawl4AI | <http://localhost:11235> |

## Troubleshooting

**Port conflict on 5432**

A local PostgreSQL instance may already be using port 5432. Either stop it or remap:

```bash
# In docker-compose.yml, change:
ports:
  - "5433:5432"
# Then set:
RARA__DATABASE__DATABASE_URL=postgres://postgres:postgres@localhost:5433/rara
```

**MinIO bucket not created**

The `minio-init` sidecar depends on `minio` being healthy. If it fails, re-run:

```bash
docker compose up -d minio-init
```

**Chroma healthcheck failing**

ChromaDB may take 10-15 seconds to become ready. Check status:

```bash
docker compose ps
curl http://localhost:8000/api/v1/heartbeat
```

**Reset all data**

```bash
docker compose down -v   # removes containers AND volumes
docker compose up -d
```
