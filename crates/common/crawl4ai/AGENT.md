# crawl4ai — Agent Guidelines

## Purpose

HTTP client for the Crawl4AI service — converts web pages to markdown via a local Docker container running the Crawl4AI REST API.

## Architecture

### Key module

- `src/lib.rs` — `Crawl4AiClient` with a single public method `crawl_md(url)` that POSTs to the Crawl4AI `/md` endpoint and returns extracted markdown. Validates URLs before sending. Error types cover HTTP failures, empty results, and remote errors.

### Default endpoint

`http://localhost:11235` — the standard Crawl4AI Docker container port.

## Critical Invariants

- URLs are validated with the `validator` crate before sending — invalid URLs fail early.
- Empty markdown responses are treated as errors (`EmptyMarkdown`).
- The client has a 60-second timeout per request.

## What NOT To Do

- Do NOT use this client without a running Crawl4AI container — it will fail with connection errors.
- Do NOT bypass URL validation — it prevents sending malformed requests.

## Dependencies

**Upstream:** `reqwest` (HTTP), `validator` (URL validation), `snafu`.

**Downstream:** `rara-kernel` or `rara-app` (browser/fetch tools may use this for markdown extraction).
