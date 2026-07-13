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
is missing, or if the connection or schema migration fails, rara falls back to
an in-memory repository, which is suitable only for local development because
candle history is not durable.

Local Timescale contract tests use testcontainers and do not require a manually
managed database:

```bash
cargo test -p rara-trading --test timescale_container
```

## Operator setup

The Settings → Data Feeds panel lists built-in finance sources under "Default
finance sources":

- official RSS feeds that can be enabled immediately;
- ready public market-data feeds such as Binance spot candles;
- provider presets such as Longbridge that prefill a `market_candle`
  configuration but still require an operator-managed normalized endpoint or
  credentials.

Enabling a ready source creates or re-enables a deterministic data feed named
`finance-{catalog_id}`; disabling it turns the feed off without deleting the
config row. The same catalog is available through the authenticated admin API:

- `GET /api/v1/data-feeds/catalog`
- `POST /api/v1/data-feeds/catalog/{id}/enable`
- `POST /api/v1/data-feeds/catalog/{id}/disable`

Ready catalog entries can be enabled with an empty body. Provider presets that
require setup return `400 Bad Request` without operator configuration, but can
be materialized by supplying a complete `transport` and optional `auth` body to
`POST /api/v1/data-feeds/catalog/{id}/enable`. The body is merged over the
catalog template, so a Longbridge preset can keep its default symbols/timeframe
while the operator supplies the normalized endpoint and credentials.

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

Latest closed candles through an operator-managed normalized endpoint:

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

Ready Binance catalog entries use Binance's public spot kline API directly and
do not require rara to hold exchange credentials. They still emit only
`market_candle_closed` feed events.

## Conversation tools

The app registers feed-oriented tools for finance data source discovery,
runtime control, subscription management, and inspection:

- `finance_list_feed_sources`
- `finance_list_feed_events`
- `finance_enable_feed_source`
- `finance_disable_feed_source`
- `finance_restart_feed_source`
- `finance_subscribe_news`
- `finance_subscribe_instruments`
- `finance_list_subscriptions`
- `finance_unsubscribe`
- `finance_diagnose_candle_subscriptions`

`finance_list_feed_sources` is the preferred status view before and after
subscription changes. It combines the built-in source catalog, persisted feed
runtime state, event watermarks (`event_count`, `last_event_type`,
`last_event_at`, and `lag_seconds`), configured K-line selectors, and
provider metadata. It also reports source-name subscription matches for the
current user/session and returns action hints for the next safe step:
`enable_hint` for materializing a ready built-in source,
`subscription_hint` for choosing the specialized subscribe tool, and
`events_hint` for checking recent persisted events. RSS sources use
`finance_subscribe_news`; market-candle sources use
`finance_subscribe_instruments`. Sources that require operator credentials or a
custom endpoint do not expose `enable_hint`; they keep `setup_hint` instead.
Persisted sources also expose `restart_hint` and `disable_hint` for runtime
operations. Market-candle sources expose `market_data_hint` for
`finance_list_candle_streams` so rara can move from source status to stored
TSDB stream discovery before querying latest or historical candles.

As the catalog grows, `finance_list_feed_sources` can narrow the returned
catalog by `catalog_source_ids`, `feed_types`, `providers`, `can_enable`,
`requires_configuration`, `persisted`, `enabled`, `running`, `subscribed`, and
`current_session_subscribed`. For example, use
`feed_types=["market_candle"]` and `providers=["binance"]` to inspect ready
Binance K-line sources without mixing in RSS news feeds or operator-only
presets. The result echoes the normalized `filters` plus `count`, so rara can
explain empty or narrowed source lists without reconstructing the query.

`finance_subscribe_news` is the specialized RSS/article entry point. It fixes
`event_kinds` to `rss_article`, rejects non-RSS feed sources, ensures selected
catalog or existing RSS feeds are enabled, starts them by default, creates or
updates the subscription for the current ToolContext identity/session, and
returns unsubscribe and event-query hints.

`finance_subscribe_instruments` is the specialized closed-candle entry point.
It fixes `event_kinds` to `market_candle_closed`, rejects non-`market_candle`
feed sources, derives `venue` from a selected market-candle source when omitted,
persists requested `symbols` and `timeframes` into the feed transport, starts or
restarts the runtime feed by default when needed, echoes the persisted delivery
policy and budget, and returns market-data, diagnostic, and single-stream
candle-query hints. Use
`finance_list_candle_streams` after subscription creation to discover stored
TSDB streams, then call the single-stream candle tools for latest, recent,
freshness, gaps, or bounded ranges.

The lower-level `finance_subscribe` tool remains available for callers that
already know the exact selectors. Its result echoes the persisted normalized
subscription selectors (`event_kinds`, `source_names`, `category_tags`,
`watch_terms`, `venues`, `symbols`, and `timeframes`) plus the delivery policy,
so rara can explain the created subscription without an immediate follow-up list
call.

`finance_list_subscriptions` is the preferred status view after a subscription
exists. Each subscription includes source/runtime details, an
`unsubscribe_hint` for `finance_unsubscribe`, and an `events_hint` for
`finance_list_feed_events`. Market-candle subscriptions also include a
`diagnostic_hint` for `finance_diagnose_candle_subscriptions`, prefilled with
the subscription id, and a `market_data_hint` for
`finance_list_candle_streams`, defaulting only to selectors that are
unambiguous for the subscription. Fully specified
single-stream market-candle subscriptions also include `latest_candle_hint` for
`finance_get_latest_candle`, so rara can jump directly from the subscription to
the latest stored bar without first discovering streams, `recent_candles_hint`
for `finance_get_recent_candles`, and `freshness_hint` for
`finance_get_candle_freshness`, so rara can check recent bars and whether that
stream is fresh or stale with the same exact selectors. They also include
`gaps_hint` for `finance_find_candle_gaps`; this pre-fills the stream selectors
but still requires the user or agent to supply the `start` and `end` range.
`query_candles_hint` does the same for `finance_query_candles`, with a default
bounded result limit. Use the events hint when the user asks what recent finance
news or closed-candle notifications were actually received; use the market-data
hint when the user asks what closed candle streams are stored in the TSDB before
picking a stream. Event queries read persisted events by
`catalog_source_ids`, `source_names`, or `feed_ids`, can narrow mixed sources by
`event_kinds` (`rss_article`, `market_candle_closed`), and return per-source
pages with `total`, `has_more`, `query_limit`, and `query_offset`. Pages with
`has_more=true` also include `next_page_hint`, which calls
`finance_list_feed_events` with the same source selector and filters plus the
next `offset`. Empty market-candle pages include a `diagnostic_hint` for
`finance_diagnose_candle_subscriptions`, prefilled with the same
`catalog_source_ids`, `source_names`, or `feed_ids` selector, so rara can move
from "no raw closed candle events" to source-scoped subscription/runtime
diagnosis. The result also echoes a normalized `query` with resolved unique
sources, event-kind strings, `since`, `query_limit`, and `query_offset`, so rara
can explain empty pages or paginated event lookups without reconstructing the
request. Top-level `source_count`, `event_count`, `total`, and `has_more`
summarize the returned pages across all selected sources.

For built-in catalog entries, `provider` is catalog metadata used by the agent
and UI to label fixed data sources such as Binance and Longbridge. It is
separate from `transport.provider`, which selects a runtime transport driver.

Stored closed candles are queryable through read-only market-data tools:

- `finance_list_candle_streams`
- `finance_get_latest_candle`
- `finance_get_recent_candles`
- `finance_query_candles`
- `finance_find_candle_gaps`
- `finance_get_candle_freshness`

Operators can also inspect stored stream watermarks without going through the
agent via:

- `GET /api/v1/data-feeds/market-data/candle-streams`
- `GET /api/v1/data-feeds/market-data/candles/latest`
- `GET /api/v1/data-feeds/market-data/candles/recent`
- `GET /api/v1/data-feeds/market-data/candles`
- `GET /api/v1/data-feeds/market-data/candles/freshness`
- `GET /api/v1/data-feeds/market-data/candles/gaps`

The admin feed-events endpoint mirrors the same event-kind split for UI and
operator views: `GET /api/v1/data-feeds/{id}/events?event_kinds=rss_article`
or `event_kinds=market_candle_closed`. Multiple values are comma-separated.

The candle endpoints are read-only. They require canonical selectors
(`venue`, `symbol`, `timeframe`) and return decimal OHLCV values as strings.
The range endpoint uses `start` as an inclusive open-time lower bound and `end`
as an exclusive open-time upper bound. Freshness defaults its stale threshold to
2x the timeframe step. Gap checks use the same inclusive/exclusive range
semantics and are capped at 10,000 expected candles.

Identity and session routing are always taken from `ToolContext`. Finance tool
schemas do not accept `owner`, `user_id`, `session`, or `session_key` identity
parameters, so an agent cannot subscribe or unsubscribe on behalf of another
user or conversation by passing forged IDs. `current_session_only` is only a
scope flag for listing/removing the current user's subscriptions; it does not
select an arbitrary session. Subscription list results also omit internal
`owner` and `session_key` routing fields; they expose only the subscription ID,
normalized selectors, source/runtime context, hints, and delivery policy.

Example article subscription:

```json
{
  "event_kinds": ["rss_article"],
  "source_names": ["fed-news"],
  "watch_terms": ["BTC", "NVDA", "Federal Reserve"],
  "delivery": "immediate"
}
```

For built-in RSS sources, `finance_subscribe_news` is the preferred
conversation entry point because it narrows the call surface to article
subscriptions, materializes the selected RSS catalog feeds when needed, and
expands catalog IDs into subscription source names:

```json
{
  "catalog_source_ids": ["fed-press-releases", "sec-press-releases"],
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

`finance_subscribe_instruments` derives `venue` from the selected
`market_candle` feed when omitted. It also persists requested `symbols` and
`timeframes` into the feed transport before creating the subscription. If a
caller supplies `venue`, it must match the feed transport venue; mismatches are
rejected before subscription creation because the feed cannot emit candles for
another venue.

Default delivery is `silent`. Immediate delivery is bounded by per-subscription
cooldown and hourly budget; events above the budget are appended to tape rather
than waking the session.

After subscribing to candle instruments, call
`finance_diagnose_candle_subscriptions` to verify the path end to end. It can
inspect all current-user candle subscriptions, one explicit `subscription_id`,
or the subset whose source matches `catalog_source_ids`, `source_names`, or
`feed_ids`. Each subscription diagnostic includes:

- feed config/runtime state, including whether the source is registered and
  running, plus configured `venue`, `symbols`, and `timeframes`;
- selector coverage (`covered`, `missing_selectors`, or `unavailable`) with a
  deterministic diagnostic such as `missing symbols: ETHUSDT; missing
  timeframes: 5m`, so a fresh-runtime/no-data case can be separated from a
  feed configuration that cannot emit the subscribed stream;
- persisted feed-event summary (`event_count`, `last_event_type`,
  `last_event_at`, and `lag_seconds`) to confirm the source is emitting the
  expected event kind;
- latest stored closed candle and freshness per
  `(source_name, venue, symbol, timeframe)`;
- a `next_action_hint` when there is an obvious follow-up: missing or disabled
  built-in sources point to `finance_enable_feed_source`, stopped sources point to
  `finance_restart_feed_source`, selector mismatches point back to
  `finance_subscribe_instruments` with the subscription selectors so the feed
  transport can be extended idempotently, and running sources with
  missing/stale candle data point to `finance_list_feed_events` for raw event
  inspection.

For ad hoc market-data inspection, use `finance_list_candle_streams` to discover
stored streams, `finance_get_latest_candle` for one newest closed bar,
`finance_get_recent_candles` for the newest N closed bars, and
`finance_query_candles` for bounded historical windows. `finance_find_candle_gaps`
checks completeness over a bounded range, while `finance_get_candle_freshness`
answers whether a stream is currently stale. All prices and volumes are returned
as strings to preserve decimal precision. Single-stream candle tools return a
normalized `selector` (`source_name`, `venue`, `symbol`, `timeframe`) even when
no rows match, so rara can tie empty or missing results back to the stream it
checked. `finance_list_candle_streams` returns normalized `filters` with the
same fields, including for empty broad-discovery results, so rara can explain
which stream set it inspected before narrowing to a single stream. Each stream
returned by `finance_list_candle_streams` includes ready-to-call hints for
`finance_get_latest_candle`, `finance_get_recent_candles`,
`finance_get_candle_freshness`, `finance_find_candle_gaps`, and
`finance_query_candles`; range tools still require explicit `start`/`end`.
`finance_list_candle_streams`, `finance_get_recent_candles`, and
`finance_query_candles` return `has_more` when additional rows match the
selectors beyond the returned `query_limit`; narrow by `source_name`, `venue`,
`symbol`, `timeframe`, or time range before assuming a broad query is
exhaustive. `finance_list_candle_streams` also returns `query_offset`; when
`has_more` is true, `next_page_hint` calls the same tool with the same filters
and the next `offset`. When `finance_query_candles.has_more` is true,
`next_start` contains the next candle open time to use as the following query's
inclusive `start`, and `next_page_hint` calls `finance_query_candles` with that
`start`, the same selectors, the same exclusive `end`, and the same limit.
When `finance_get_recent_candles.has_more` is true, `next_end` contains the
oldest returned candle open time to use as an exclusive `end` when paging older
history via another `finance_get_recent_candles` call; its `next_page_hint`
pre-fills that exclusive `end`. The
stream-list, recent-candle, and candle-range limits are capped at 9,999 so the
tools can probe one extra row and report pagination state accurately.

## Acceptance checklist

Use this checklist before considering a deployment ready:

1. Enable a built-in RSS source from Settings → Data Feeds, or create a custom
   authenticated `rss` data feed with tags `["finance", "macro"]`.
2. Create an authenticated `market_candle` data feed with tags
   `["finance", "market-data"]`.
3. Confirm each closed candle is upserted into `market_candles` exactly once
   for `(source_name, venue, symbol, timeframe, open_time)`.
4. Use a conversation to call `finance_subscribe_news` for article RSS sources
   and `watch_terms`.
5. Use a conversation to call `finance_subscribe_instruments` for candle
   `symbols` and `timeframes`.
6. Confirm `finance_list_feed_sources` reports the subscribed source with
   `subscriptions.session_subscribed = true`.
7. Confirm the admin event endpoint stores one event per article and one event
   per closed candle across two polls, and `finance_list_feed_events` returns
   recent persisted events for the subscribed source, including filtered views
   by `event_kinds`.
8. Confirm `finance_diagnose_candle_subscriptions` reports a running feed, a
   recent feed event, `selector_coverage = covered`, and a fresh latest candle
   for the subscribed stream.
9. Confirm `finance_get_latest_candle` returns the latest stored closed candle
   for the subscribed `(venue, symbol, timeframe)`.
10. Confirm
   `GET /api/v1/data-feeds/market-data/candle-streams?venue=...&symbol=...`
   returns the stored stream watermark.
11. Confirm `finance_get_recent_candles`,
   `GET /api/v1/data-feeds/market-data/candles/recent?venue=...&symbol=...&timeframe=...`,
   `finance_query_candles`, and
   `GET /api/v1/data-feeds/market-data/candles?venue=...&symbol=...&timeframe=...&start=...&end=...`
   return ordered candles with decimal values encoded as strings.
12. Confirm one matching item wakes the same session at most once.
13. Confirm a seventh matching item in an hour is tape-only under the default
   immediate-delivery budget.
14. Confirm no feed URL, ticker provider URL, credentials, order, deployment, or
   account tool is agent-callable.

## Non-goals

This MVP does not include arbitrary URL ingestion, arbitrary ticker/provider
fetches, LLM entity extraction at ingestion, article scraping, portfolio-aware
impact scoring, price-threshold alerts, backtests, account access, order
placement, or strategy deployment.
