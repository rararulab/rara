# Finance information subscriptions

This guide covers the first rara trading deliverable: financial information
subscriptions for RSS/Atom articles and latest closed market candles. It does
not enable strategy deployment, account access, orders, or execution.

## Runtime model

`rara-trading` provides finance-specific feed transports, a TSDB-backed market
data repository, and conversation tools. The kernel `data_feed` layer only owns
generic feed config, lifecycle, event IDs, persistence, and status reporting.

The ingestion path is:

1. An operator enables a built-in finance RSS source or creates an
   authenticated `rss` / `market_candle` data feed through the existing admin
   data-feed API.
2. The app starts the configured source in the background.
3. The source emits normalized `FeedEvent`s:
   - `rss_article`
   - `market_candle_closed`
4. Every event is written to the feed-event store.
5. Closed candles are also upserted into `MarketDataRepository`.
6. `FinanceSubscriptionRegistry` matches event metadata against conversation
   subscriptions and either wakes the session once or appends silently to tape.

The conversation can subscribe to already configured feeds. It cannot add feed
URLs, provider URLs, credentials, account identifiers, orders, deployments, or
arbitrary ticker fetches.

## Storage

Feed events are stored in the existing SQLite-backed `data_feed_events` table.
They are notification facts, not the OHLCV history store.

Closed candles are also stored through `MarketDataRepository`. Production uses
TimescaleDB/PostgreSQL when `RARA_MARKET_DATA_DATABASE_URL` is set:

```bash
export RARA_MARKET_DATA_DATABASE_URL='postgres://user:pass@host:5432/rara'
```

On startup, rara applies the market-candle migration from
`crates/extensions/rara-trading/migrations/0001_market_candles.sql`. The
TimescaleDB extension must be installed in the target database. If the variable
is missing, rara falls back to an in-memory repository, which is suitable only
for local development because candle history is not durable.

Local Timescale contract tests use testcontainers and do not require a manually
managed database:

```bash
cargo test -p rara-trading --test timescale_container
```

## Operator setup

The Settings → Data Feeds panel lists built-in finance sources under "Default
finance sources":

- official RSS feeds that can be enabled immediately;
- provider presets such as Binance and Longbridge that prefill a
  `market_candle` configuration but still require an operator-managed endpoint
  or credentials.

Enabling a ready source creates or re-enables a deterministic data feed named
`finance-{catalog_id}`; disabling it turns the feed off without deleting the
config row. The same catalog is available through the authenticated admin API:

- `GET /api/v1/data-feeds/catalog`
- `POST /api/v1/data-feeds/catalog/{id}/enable`
- `POST /api/v1/data-feeds/catalog/{id}/disable`

Built-in provider presets are not directly enabled by the catalog API. A direct
enable attempt returns `400 Bad Request` until the operator supplies a complete
feed configuration. Market-candle feeds still require an operator-managed
normalized endpoint.

Custom finance feeds can also be created through the authenticated admin API.
Example payloads are documented in `config.example.yaml`.

RSS/Atom:

```json
{
  "name": "fed-news",
  "feed_type": "rss",
  "enabled": true,
  "tags": ["finance", "macro"],
  "transport": {
    "url": "https://trusted-publisher.example/feed.xml",
    "interval_secs": 300,
    "max_entries_per_poll": 20
  }
}
```

Latest closed candles:

```json
{
  "name": "binance-spot",
  "feed_type": "market_candle",
  "enabled": true,
  "tags": ["finance", "market-data", "crypto"],
  "transport": {
    "url": "https://trusted-market-data.example/candles/latest",
    "interval_secs": 60,
    "venue": "binance",
    "symbols": ["BTCUSDT", "ETHUSDT"],
    "timeframes": ["15m", "1h"],
    "max_candles_per_poll": 1000
  }
}
```

The market-candle endpoint must return normalized JSON with decimal strings.
Only `closed: true` bars are emitted. In-progress bars are ignored in the MVP.

## Conversation tools

The app registers three deferred tools:

- `finance_subscribe`
- `finance_unsubscribe`
- `finance_list_subscriptions`

Identity and session routing are always taken from `ToolContext`. The tool
schema has no `owner` or `session` parameter, so an agent cannot subscribe or
unsubscribe on behalf of another user by passing forged IDs.

Example article subscription:

```json
{
  "event_kinds": ["rss_article"],
  "source_names": ["fed-news"],
  "watch_terms": ["BTC", "NVDA", "Federal Reserve"],
  "delivery": "immediate"
}
```

Example candle subscription:

```json
{
  "event_kinds": ["market_candle_closed"],
  "venues": ["binance"],
  "symbols": ["BTCUSDT"],
  "timeframes": ["15m"],
  "delivery": "silent"
}
```

Default delivery is `silent`. Immediate delivery is bounded by per-subscription
cooldown and hourly budget; events above the budget are appended to tape rather
than waking the session.

## Acceptance checklist

Use this checklist before considering a deployment ready:

1. Enable a built-in RSS source from Settings → Data Feeds, or create a custom
   authenticated `rss` data feed with tags `["finance", "macro"]`.
2. Create an authenticated `market_candle` data feed with tags
   `["finance", "market-data"]`.
3. Confirm each closed candle is upserted into `market_candles` exactly once
   for `(source_name, venue, symbol, timeframe, open_time)`.
4. Use a conversation to call `finance_subscribe` for article `source_names`
   and `watch_terms`.
5. Use a conversation to call `finance_subscribe` for candle `symbols` and
   `timeframes`.
6. Confirm the admin event endpoint stores one event per article and one event
   per closed candle across two polls.
7. Confirm one matching item wakes the same session at most once.
8. Confirm a seventh matching item in an hour is tape-only under the default
   immediate-delivery budget.
9. Confirm no feed URL, ticker provider URL, credentials, order, deployment, or
   account tool is agent-callable.

## Non-goals

This MVP does not include arbitrary URL ingestion, arbitrary ticker/provider
fetches, LLM entity extraction at ingestion, article scraping, portfolio-aware
impact scoring, price-threshold alerts, backtests, account access, order
placement, or strategy deployment.
