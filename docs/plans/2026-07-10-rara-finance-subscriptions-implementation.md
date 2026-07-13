# Rara Finance Information Subscriptions Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let a user ask rara to subscribe a conversation to selected financial RSS/Atom news and latest closed market candles, then receive only deduplicated, rate-limited matching updates as a proactive message or silent memory entry.

**Architecture:** Add normalized financial feed events through `rara-trading`: RSS/Atom articles and latest closed market candles both become individually deduplicated `FeedEvent`s rather than raw polling blobs. The kernel `data_feed` layer keeps only generic feed types/lifecycle/storage; source-specific parsing lives in `crates/extensions/rara-trading`. Closed candles are also upserted into a TimescaleDB/PostgreSQL-backed `MarketDataRepository` for durable OHLCV history. `rara-trading` owns finance-specific subscription matching; the app exposes specialized Deferred tools for source discovery, RSS subscription, market-candle subscription, listing, event inspection, diagnostics, and unsubscribe. The app injects that registry into the existing feed-dispatch loop, which already persists every event and can deliver a proactive turn to the source session.

**Tech Stack:** Rust 2024, `feed-rs`, `reqwest`, `rust_decimal`, TimescaleDB/PostgreSQL hypertables, `sqlx-core`/`sqlx-postgres`, `rara-kernel::data_feed`, `rara-kernel::notification::NotifyAction`, `rara-tool-macro::ToolDef`, Tokio, JSON-file persistence, SQLite feed-event store.

---

## MVP boundary

The first user experience is:

> “订阅 BTC、NVDA 和美联储相关消息；重要的直接告诉我。”

> “订阅 BTC 15m K 线收盘；有 NVDA 和美联储相关新闻也告诉我。”

The agent uses `finance_list_feed_sources` to discover trusted sources, then
`finance_subscribe_news` for RSS/Atom articles or
`finance_subscribe_instruments` for latest closed market candles. An operator
has previously registered one or more trusted RSS/Atom sources and market candle
sources via the existing admin data-feed API or built-in finance catalog. New
articles and latest closed candles are normalized and persisted, then matched
against the subscriber's source/category/watch-term/symbol/timeframe filters. A
matching event is delivered to the originating session as either `immediate`
(one proactive rara turn) or `silent` (tape only).

MVP deliberately excludes arbitrary user-supplied feed URLs, arbitrary ticker/provider fetches, LLM-based entity extraction at ingestion, article scraping, portfolio-aware impact scoring, digest scheduling, derived price-threshold alerts, strategy backtests, orders, accounts, and all execution capability.

### Product decisions

- **Sources are operator configured.** The conversation can choose among enabled source names, but cannot make rara fetch an arbitrary URL. This keeps ingestion configuration, SSRF exposure, licensing review, and rate limits out of the LLM surface.
- **RSS source parsing is deterministic.** The parser emits title, canonical URL, summary, author, published time, categories, and a stable per-entry `FeedEventId`. `feed-rs` supplies a unified model for RSS, Atom and JSON Feed inputs. [feed-rs documentation](https://docs.rs/feed-rs/latest/feed_rs/index.html)
- **Market candle sources are operator configured.** The conversation can choose among enabled symbols, venues and timeframes for a configured source, but cannot make rara query an arbitrary ticker/provider pair.
- **Market ingestion is batched by source.** One `MarketCandleSource` covers a provider/venue/timeframe and a symbol allowlist. User subscriptions never create provider polling tasks; they only match against already-ingested candle events and TSDB rows.
- **Hundreds of symbols are in scope.** The MVP schema and ingestion loop assume at least hundreds of symbols across common bar timeframes. This is still bar data, not tick data; fan-out happens inside subscription matching, not at the provider request layer.
- **Only closed candles wake the user by default.** MVP emits `market_candle_closed` for the latest completed bar. Provider updates for an in-progress bar are either ignored or coalesced into the final close; high-frequency `market_candle_update` is a later explicit mode.
- **Closed candles are dual-written.** Each closed candle becomes a `FeedEvent` for notification matching and is upserted into `MarketDataRepository` for OHLCV history. The feed-event store is not the historical market-data store.
- **TSDB choice is TimescaleDB/PostgreSQL for MVP.** It gives us hypertables, SQL, transactions, precise upsert/correction semantics and straightforward Rust `sqlx` integration. ClickHouse or QuestDB can be added later behind `MarketDataRepository` for high-throughput market-data lake workloads.
- **Market values are decimal strings.** Open/high/low/close/volume are parsed into `Decimal`, serialized as strings in feed events, and stored as `NUMERIC` in TSDB. Do not use `f64` for prices or volumes.
- **Watch terms are literal normalized matches.** They match title plus summary after Unicode case folding and whitespace normalization; no LLM runs for each incoming item.
- **Filter groups are ANDed; values in a group are ORed.** A subscription constrained to `event_kinds=["rss_article"]`, `sources=["fed"]` and `watch_terms=["BTC", "NVDA"]` must be an article from `fed` and mention either term. A subscription constrained to `event_kinds=["market_candle_closed"]`, `symbols=["BTCUSDT"]` and `timeframes=["15m"]` must be the latest closed 15m candle for BTCUSDT. Empty optional groups impose no constraint.
- **Immediate delivery is bounded.** Default is `silent`; `immediate` has a fixed per-subscription cooldown and maximum-per-hour budget. Items exceeding the budget are silently appended, never discarded from the feed store.
- **No financial decision is made.** Proactive prompts say that the event is information, include source metadata, and ask rara to summarize or report facts; they must not recommend trades by default.

### Normalized event shape

Every RSS item becomes one normal `FeedEvent`:

```json
{
  "source_name": "fed-news",
  "event_type": "rss_article",
  "tags": ["finance", "source:fed-news", "category:monetary-policy"],
  "payload": {
    "title": "...",
    "url": "https://example.org/article",
    "summary": "...",
    "author": "...",
    "published_at": "2026-07-10T08:30:00Z",
    "categories": ["Monetary Policy"]
  }
}
```

The event ID is `FeedEventId::deterministic("<feed-url>:<guid-or-url-or-content-fallback>")`, never a poll timestamp. This makes repeated polls and process restarts idempotent.

Every closed candle becomes one normal `FeedEvent`:

```json
{
  "source_name": "binance-spot",
  "event_type": "market_candle_closed",
  "tags": [
    "finance",
    "market-data",
    "source:binance-spot",
    "venue:binance",
    "symbol:BTCUSDT",
    "timeframe:15m"
  ],
  "payload": {
    "venue": "binance",
    "symbol": "BTCUSDT",
    "timeframe": "15m",
    "open_time": "2026-07-10T08:15:00Z",
    "close_time": "2026-07-10T08:30:00Z",
    "open": "61500.12",
    "high": "61640.00",
    "low": "61480.50",
    "close": "61610.30",
    "volume": "124.551"
  }
}
```

The event ID is `FeedEventId::deterministic("<source>:<venue>:<symbol>:<timeframe>:<open-time>")`. This preserves exactly-once notification semantics across polling, websocket reconnects and process restarts.

The TSDB primary key is `(source_name, venue, symbol, timeframe, open_time)`. Corrections for the same candle update the current row and append a small correction/audit record with `ingested_at`, provider sequence if available, and the previous OHLCV values. Consumers that need immutable snapshots use exported dataset artifacts with content hashes.

## Task 1: Add a normalized RSS/Atom feed transport

Implementation note: this transport belongs in `crates/extensions/rara-trading/src/feed/rss.rs`.
`crates/kernel/src/data_feed/config.rs` only adds the generic `FeedType::Rss`
variant so the admin/feed registry can route it.

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/kernel/Cargo.toml`
- Modify: `crates/kernel/src/data_feed/config.rs`
- Modify: `crates/kernel/src/data_feed/mod.rs`
- Create: `crates/kernel/src/data_feed/rss.rs`
- Test: `crates/kernel/src/data_feed/rss.rs`

**Step 1: Write failing parser tests**

Add fixtures inline for RSS 2.0 and Atom. Test canonical extraction, category tags, and duplicate identity:

```rust
#[test]
fn rss_item_becomes_one_normalized_article_event() { /* title/url/categories */ }

#[test]
fn same_guid_has_same_event_id_across_polls() { /* parse twice */ }

#[test]
fn missing_guid_uses_link_then_content_fallback() { /* stable fallback */ }
```

**Step 2: Run focused tests to verify they fail**

Run: `cargo test -p rara-kernel rss`

Expected: FAIL because `FeedType::Rss` and `rss` module do not exist.

**Step 3: Implement the transport**

- Add workspace dependency `feed-rs = "2.3.1"` and consume it only from `rara-kernel`.
- Add `FeedType::Rss` and `pub mod rss` to the data-feed module.
- Define `RssTransport { url, interval_secs, headers, max_entries_per_poll }`; reject non-HTTPS URLs, zero intervals, zero/max-excessive entry counts, and invalid headers before starting a task.
- Implement `RssSource::from_config`, `DataFeed::run`, and `poll_once` with the same cancellation/error-status behavior as `PollingSource`.
- Cap response bodies before parsing, parse with `feed_rs::parser::parse`, and emit at most `max_entries_per_poll` normalized `rss_article` events per poll.
- Build tags as `config.tags + ["source:<name>"] + normalized feed categories`; deduplicate and sort tags before constructing the event.
- Use `reqwest` timeouts matching the existing polling source. Do not add a generic arbitrary-URL agent tool.

**Step 4: Run focused tests**

Run: `cargo test -p rara-kernel rss`

Expected: PASS.

**Step 5: Commit**

```bash
git add Cargo.toml crates/kernel/Cargo.toml crates/kernel/src/data_feed
git commit -m "feat(data-feed): add normalized RSS ingestion"
```

## Task 2: Start RSS feeds through the existing admin and boot paths

**Files:**
- Modify: `crates/extensions/backend-admin/src/data_feeds/router.rs`
- Test: `crates/extensions/backend-admin/src/data_feeds/router.rs`
- Modify: `config.example.yaml`

**Step 1: Write failing task-start tests**

Add a test that creates an enabled `FeedType::Rss` configuration, calls `start_feed_task`, and observes a running task which stops when the registry cancels it. Add a request test that accepts valid RSS transport JSON and rejects an HTTP URL.

**Step 2: Run the tests to verify they fail**

Run: `cargo test -p rara-backend-admin data_feeds::router::tests`

Expected: FAIL because the router only starts polling feeds.

**Step 3: Add RSS task dispatch**

Extend `start_feed_task` to build `RssSource` for `FeedType::Rss`, attach the existing status reporter, and reuse the registry cancellation token. Leave webhook and polling behavior unchanged.

Add a commented operator example to `config.example.yaml` and the data-feed admin docs:

```yaml
# Managed through the authenticated /api/v1/data-feeds admin API:
# feed_type: rss
# tags: [finance, macro]
# transport:
#   url: "https://trusted-publisher.example/feed.xml"
#   interval_secs: 300
#   max_entries_per_poll: 20
```

**Step 4: Run focused tests**

Run: `cargo test -p rara-backend-admin data_feeds::router::tests`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/extensions/backend-admin/src/data_feeds/router.rs config.example.yaml
git commit -m "feat(data-feed): start RSS feeds from admin config"
```

## Task 3: Add normalized market candle ingestion

Implementation note: this transport belongs in
`crates/extensions/rara-trading/src/feed/market_candle.rs`.
`crates/kernel/src/data_feed/config.rs` only adds the generic
`FeedType::MarketCandle` variant.

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/kernel/Cargo.toml`
- Modify: `crates/kernel/src/data_feed/config.rs`
- Modify: `crates/kernel/src/data_feed/mod.rs`
- Create: `crates/kernel/src/data_feed/market_candle.rs`
- Modify: `crates/extensions/backend-admin/src/data_feeds/router.rs`
- Test: `crates/kernel/src/data_feed/market_candle.rs`
- Test: `crates/extensions/backend-admin/src/data_feeds/router.rs`

**Step 1: Write failing candle normalization tests**

Use an inline fixture for an operator-managed normalized endpoint:

```json
{
  "candles": [
    {
      "venue": "binance",
      "symbol": "BTCUSDT",
      "timeframe": "15m",
      "open_time": "2026-07-10T08:15:00Z",
      "close_time": "2026-07-10T08:30:00Z",
      "open": "61500.12",
      "high": "61640.00",
      "low": "61480.50",
      "close": "61610.30",
      "volume": "124.551",
      "closed": true
    }
  ]
}
```

Add tests:

```rust
#[test]
fn closed_candles_for_many_symbols_become_batched_events() { /* symbol/timeframe/decimal strings */ }

#[test]
fn same_candle_has_same_event_id_across_polls() { /* parse twice */ }

#[test]
fn open_or_invalid_decimal_candles_are_not_emitted() { /* closed=false, bad price */ }
```

**Step 2: Run focused tests to verify they fail**

Run: `cargo test -p rara-kernel market_candle`

Expected: FAIL because `FeedType::MarketCandle` and `market_candle` module do not exist.

**Step 3: Implement the market candle transport**

- Add `rust_decimal` as a workspace dependency if it is not already present.
- Add `FeedType::MarketCandle` and `pub mod market_candle` to the data-feed module.
- Define `MarketCandleTransport { url, interval_secs, headers, venue, symbols, timeframes, max_candles_per_poll }`; reject non-HTTPS URLs, zero intervals, empty symbol/timeframe allowlists, and excessive candle counts. Set `max_candles_per_poll` high enough for hundreds of symbols across the configured timeframes, but bounded to protect memory.
- Poll only operator-configured URLs. Do not add a tool that lets the LLM request arbitrary market data endpoints. Do not create one task per symbol; one task should fetch a batch for its configured symbol/timeframe allowlist.
- Parse the normalized endpoint response into `MarketCandle` values using `Decimal` for OHLCV fields.
- Emit only `closed == true` candles as `market_candle_closed`; ignore in-progress candles for MVP.
- Build tags as `config.tags + ["finance", "market-data", "source:<name>", "venue:<venue>", "symbol:<symbol>", "timeframe:<timeframe>"]`, deduplicated and sorted.
- Use deterministic event IDs from `source_name`, `venue`, `symbol`, `timeframe`, and `open_time`.
- Return normalized closed candles to the caller so the app can both persist a notification `FeedEvent` and upsert the candle into `MarketDataRepository`.

**Step 4: Start market candle feeds from admin config**

Add backend-admin tests that accept a valid `market_candle` feed config and reject an HTTP URL or a symbol outside the configured allowlist. Extend `start_feed_task` to build `MarketCandleSource` for `FeedType::MarketCandle`, attach the existing status reporter, and reuse the registry cancellation token.

Add a commented operator example to `config.example.yaml`:

```yaml
# Managed through the authenticated /api/v1/data-feeds admin API:
# feed_type: market_candle
# tags: [finance, market-data, crypto]
# transport:
#   url: "https://trusted-market-data.example/candles/latest"
#   interval_secs: 60
#   venue: "binance"
#   symbols: ["BTCUSDT", "ETHUSDT"]
#   timeframes: ["15m", "1h"]
#   max_candles_per_poll: 1000
```

**Step 5: Run focused tests**

Run:

```bash
cargo test -p rara-kernel market_candle
cargo test -p rara-backend-admin data_feeds::router::tests
```

Expected: PASS.

**Step 6: Commit**

```bash
git add Cargo.toml crates/kernel/Cargo.toml crates/kernel/src/data_feed crates/extensions/backend-admin/src/data_feeds/router.rs config.example.yaml
git commit -m "feat(data-feed): add latest market candle ingestion"
```

## Task 4: Add the TSDB-backed market data repository

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/extensions/rara-trading/Cargo.toml`
- Create: `crates/extensions/rara-trading/src/market_data/mod.rs`
- Create: `crates/extensions/rara-trading/src/market_data/model.rs`
- Create: `crates/extensions/rara-trading/src/market_data/repository.rs`
- Create: `crates/extensions/rara-trading/src/market_data/timescale.rs`
- Create: `crates/extensions/rara-trading/migrations/0001_market_candles.sql`
- Test: `crates/extensions/rara-trading/src/market_data/repository.rs`

**Step 1: Write failing repository tests**

Use repository-contract tests that run against an in-memory fake and a local TimescaleDB testcontainer:

```rust
#[tokio::test]
async fn upsert_candle_is_idempotent_for_same_primary_key() { /* insert twice */ }

#[tokio::test]
async fn corrected_candle_updates_current_row_and_records_audit() { /* changed close */ }

#[tokio::test]
async fn query_range_returns_ordered_candles_for_symbol_timeframe() { /* range scan */ }

#[tokio::test]
async fn gap_detection_reports_missing_open_times() { /* missing 15m bar */ }
```

**Step 2: Run focused tests to verify they fail**

Run: `cargo test -p rara-trading market_data`

Expected: FAIL because `market_data` module and repository do not exist.

**Step 3: Define the domain model and repository trait**

Create:

```rust
pub struct MarketCandle {
    pub source_name: String,
    pub venue: String,
    pub symbol: String,
    pub timeframe: Timeframe,
    pub open_time: OffsetDateTime,
    pub close_time: OffsetDateTime,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub ingested_at: OffsetDateTime,
}

#[async_trait]
pub trait MarketDataRepository: Send + Sync {
    async fn upsert_closed_candle(&self, candle: MarketCandle) -> Result<UpsertOutcome>;
    async fn candles(&self, query: CandleRangeQuery) -> Result<Vec<MarketCandle>>;
    async fn missing_open_times(&self, query: CandleRangeQuery) -> Result<Vec<OffsetDateTime>>;
}
```

`UpsertOutcome` distinguishes inserted, duplicate unchanged, and corrected.

**Step 4: Add TimescaleDB schema**

Migration:

```sql
CREATE TABLE market_candles (
  source_name TEXT NOT NULL,
  venue TEXT NOT NULL,
  symbol TEXT NOT NULL,
  timeframe TEXT NOT NULL,
  open_time TIMESTAMPTZ NOT NULL,
  close_time TIMESTAMPTZ NOT NULL,
  open NUMERIC NOT NULL,
  high NUMERIC NOT NULL,
  low NUMERIC NOT NULL,
  close NUMERIC NOT NULL,
  volume NUMERIC NOT NULL,
  ingested_at TIMESTAMPTZ NOT NULL,
  provider_sequence TEXT,
  PRIMARY KEY (source_name, venue, symbol, timeframe, open_time)
);

SELECT create_hypertable('market_candles', 'open_time', if_not_exists => TRUE);

CREATE TABLE market_candle_corrections (
  id UUID PRIMARY KEY,
  source_name TEXT NOT NULL,
  venue TEXT NOT NULL,
  symbol TEXT NOT NULL,
  timeframe TEXT NOT NULL,
  open_time TIMESTAMPTZ NOT NULL,
  corrected_at TIMESTAMPTZ NOT NULL,
  previous_payload JSONB NOT NULL,
  new_payload JSONB NOT NULL
);
```

Use `NUMERIC`, not floating-point columns. Add indexes for `(venue, symbol, timeframe, open_time DESC)` and `(source_name, symbol, timeframe, open_time DESC)`.

**Step 5: Implement TimescaleDB repository**

- Use `sqlx` with PostgreSQL.
- Upsert by primary key.
- If existing OHLCV equals the incoming candle, return `DuplicateUnchanged`.
- If OHLCV differs, write one correction row, update the current row, and return `Corrected`.
- Keep retention/compression policy as operator config, not hard-coded in the migration.

**Step 6: Run focused tests**

Run:

```bash
cargo test -p rara-trading market_data
```

Expected: PASS for fake repository tests and the TimescaleDB testcontainer contract.

**Step 7: Commit**

```bash
git add Cargo.toml crates/extensions/rara-trading
git commit -m "feat(trading): add market data repository"
```

## Task 5: Create the finance subscription domain registry

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/extensions/rara-trading/Cargo.toml`
- Create: `crates/extensions/rara-trading/src/lib.rs`
- Create: `crates/extensions/rara-trading/src/finance/mod.rs`
- Create: `crates/extensions/rara-trading/src/finance/model.rs`
- Create: `crates/extensions/rara-trading/src/finance/registry.rs`
- Test: `crates/extensions/rara-trading/src/finance/registry.rs`

**Step 1: Write failing registry tests**

```rust
#[tokio::test]
async fn article_source_and_watch_terms_are_anded() { /* source=Fed, term=BTC */ }

#[tokio::test]
async fn candle_symbol_and_timeframe_are_anded() { /* BTCUSDT + 15m */ }

#[tokio::test]
async fn values_inside_filter_groups_are_ored() { /* BTC OR NVDA; 15m OR 1h */ }

#[tokio::test]
async fn duplicate_event_is_delivered_once_after_reload() { /* JSON persistence */ }

#[tokio::test]
async fn immediate_budget_downgrades_excess_events_to_silent() { /* clock injection */ }
```

**Step 2: Run focused tests to verify they fail**

Run: `cargo test -p rara-trading finance::registry`

Expected: FAIL because the extension and registry do not exist.

**Step 3: Implement persistence, matching and delivery decisions**

Create a JSON-backed `FinanceSubscriptionRegistry` at `<data_dir>/trading/finance-subscriptions.json`. It owns:

```rust
pub struct FinanceSubscription {
    pub id: Uuid,
    pub owner: UserId,
    pub session_key: SessionKey,
    pub event_kinds: Vec<FinanceEventKind>, // RssArticle | MarketCandleClosed
    pub source_names: Vec<String>,
    pub category_tags: Vec<String>,
    pub watch_terms: Vec<String>,
    pub venues: Vec<String>,
    pub symbols: Vec<String>,
    pub timeframes: Vec<String>,
    pub delivery: FinanceDelivery, // Immediate | Silent
    pub cooldown_secs: u64,
    pub max_immediate_per_hour: u16,
}
```

Persist a bounded delivery ledger `(subscription_id, event_id, delivered_at, action)` with the registry. `match_event` returns `FinanceDeliveryDecision`s, not a notification side effect. It must:

1. accept only `event_type == "rss_article"` or `event_type == "market_candle_closed"` with a `finance` tag;
2. match event kind, sources, categories, watch terms, venues, symbols and timeframes using the stated AND-group/OR-value rule;
3. normalize article title + summary with Unicode lowercase and collapsed whitespace;
4. match candle venues, symbols and timeframes from structured payload fields, not from free text;
5. suppress an already-seen `(subscription,event)` pair;
6. return `Silent` when immediate cooldown/budget is exhausted and record why.

No LLM dependency belongs in this crate.

**Step 4: Run focused tests**

Run: `cargo test -p rara-trading finance::registry`

Expected: PASS.

**Step 5: Commit**

```bash
git add Cargo.toml crates/extensions/rara-trading
git commit -m "feat(trading): add finance subscription registry"
```

## Task 6: Add conversation-native finance subscription tools

**Files:**
- Create: `crates/extensions/rara-trading/src/finance/tools.rs`
- Modify: `crates/extensions/rara-trading/src/finance/mod.rs`
- Test: `crates/extensions/rara-trading/src/finance/tools.rs`

**Step 1: Write failing tool tests**

```rust
#[tokio::test]
async fn subscribe_uses_context_owner_and_session_not_llm_fields() { /* ToolContext fixture */ }

#[tokio::test]
async fn unsubscribe_cannot_remove_another_users_subscription() { /* two owners */ }

#[test]
fn list_schema_has_no_identity_or_session_parameter() { /* ToolDef schema */ }
```

**Step 2: Run focused tests to verify they fail**

Run: `cargo test -p rara-trading finance::tools`

Expected: FAIL because the tools do not exist.

**Step 3: Implement Deferred subscription tools**

Implement specialized subscription tools,
`finance_subscribe_news` and `finance_subscribe_instruments`, plus
`finance_unsubscribe` and `finance_list_subscriptions`, using
`ToolContext.user_id` and `ToolContext.session_key` as the only
identity/routing inputs. Never accept owner or session from LLM parameters.

`finance_subscribe_news` accepts built-in `catalog_source_ids` or existing RSS
`feed_ids`, plus `category_tags`, `watch_terms`, optional `delivery` (`silent`
default; `immediate` explicit), `cooldown_secs` (default 900), and
`max_immediate_per_hour` (default 6). It ensures the selected RSS feeds are
enabled and optionally running, then returns a subscription ID and exact
delivery policy.

`finance_subscribe_instruments` accepts a built-in market-candle
`catalog_source_id` or existing market-candle `feed_id`, plus `venue`,
`symbols`, `timeframes`, optional `delivery`, `cooldown_secs`, and
`max_immediate_per_hour`. It ensures the market-candle feed is enabled,
persists the requested symbols/timeframes into the feed config, optionally
restarts the feed, and returns a subscription ID plus diagnostic identifiers.

Both subscribe tools validate non-empty selectors, bounded selector
counts/lengths, and normalized deduplicated terms.

Mark subscribe/unsubscribe non-read-only; mark list read-only. Do not mark them destructive: they only manage the caller's notification preference and never alter a financial account.

**Step 4: Run focused tests**

Run: `cargo test -p rara-trading finance::tools`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/extensions/rara-trading/src/finance
git commit -m "feat(trading): add finance subscription tools"
```

## Task 7: Register the extension, market-data store and finance dispatch safely

**Files:**
- Modify: `crates/app/Cargo.toml`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/boot.rs`
- Modify: `crates/app/src/lib.rs`
- Test: `crates/app/src/lib.rs`

**Step 1: Write failing app-level tests**

Extract feed delivery into a testable async helper. Verify:

```rust
#[tokio::test]
async fn matched_immediate_finance_article_creates_one_synthetic_turn() { /* fake handle */ }

#[tokio::test]
async fn matched_immediate_candle_creates_compact_market_update_turn() { /* fake handle */ }

#[tokio::test]
async fn closed_candle_is_upserted_before_subscription_delivery() { /* fake repo */ }

#[tokio::test]
async fn silent_finance_event_appends_to_tape_without_turn() { /* fake tape */ }

#[tokio::test]
async fn unmatched_finance_event_does_not_wake_a_session() { /* empty decisions */ }
```

**Step 2: Run focused tests to verify they fail**

Run: `cargo test -p rara-app finance_event`

Expected: FAIL because the finance registry and market-data repository are not constructed or dispatched.

**Step 3: Wire the registry once**

- Add `rara-trading` as an app dependency and construct `Arc<FinanceSubscriptionRegistry>` at boot using `rara_paths::data_dir().join("trading/finance-subscriptions.json")`.
- Construct one `Arc<dyn MarketDataRepository>` at boot. Production config points it at TimescaleDB/PostgreSQL; tests use a fake repository.
- Add the finance registry `Arc` to `ToolDeps` and register all three tools as Deferred. Do not put them in the core manifest.
- Refactor the existing feed dispatch loop in `crates/app/src/lib.rs` into a helper that receives generic tag subscriptions, market-data repository and finance delivery decisions. Keep current generic `SubscriptionRegistry` behavior byte-for-byte equivalent.
- When the event is `market_candle_closed`, parse its structured payload into `MarketCandle` and call `upsert_closed_candle` before notification delivery. A TSDB failure should mark the data-feed task unhealthy and skip proactive delivery for that candle; do not wake the user for a candle that failed durable storage.
- For `Immediate`, issue the existing synthetic inbound message to the matched session. Article directives contain source, title, canonical URL, published time, matching selectors and the instruction: “Summarize factual relevance; do not give trade advice unless the user asks.” Candle directives contain source, venue, symbol, timeframe, close time, OHLCV values and the instruction: “Report the market update factually; do not infer a trade unless the user asks.”
- For `Silent`, append the normalized event and matching metadata to that session's tape using `TapEntryKind::FeedEvent`.
- If the target session is absent from the process table, downgrade immediate to silent exactly as the existing generic dispatch path does.

**Step 4: Run focused tests**

Run: `cargo test -p rara-app finance_event`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/Cargo.toml crates/app/src/tools/mod.rs crates/app/src/boot.rs crates/app/src/lib.rs
git commit -m "feat(trading): deliver matched finance information to sessions"
```

## Task 8: Document the new first milestone and verify the whole feature

**Files:**
- Modify: `docs/plans/2026-07-10-rara-trading-design.md`
- Modify: `config.example.yaml`
- Create: `docs/guides/finance-subscriptions.md`

**Step 1: Write an operator acceptance checklist**

Document this exact manual lane:

1. Create an authenticated `rss` data feed with tags `["finance", "macro"]`.
2. Create an authenticated `market_candle` data feed with tags `["finance", "market-data"]`.
3. Confirm each closed candle is upserted into `market_candles` exactly once for `(source, venue, symbol, timeframe, open_time)`.
4. Use a conversation to call `finance_subscribe_news` for article sources and `watch_terms`.
5. Use a conversation to call `finance_subscribe_instruments` for candle `symbols` and `timeframes`.
6. Confirm the admin event endpoint stores one event per article and one event per closed candle across two polls.
7. Confirm one matching item wakes the same session at most once.
8. Confirm a seventh matching item in an hour is tape-only under the default budget.
9. Confirm no feed URL, ticker provider URL, credentials, order, deployment, or account tool is agent-callable.

**Step 2: Update product sequencing**

Amend the trading design: finance information subscriptions for news and latest closed candles become the first deliverable, the isolated research bench becomes the second deliverable, and execution remains gated behind the existing mandate/OMS/reconciliation criteria. Link both implementation plans.

**Step 3: Run verification**

Run:

```bash
cargo fmt --check
cargo clippy -p rara-kernel -p rara-backend-admin -p rara-trading -p rara-app -- -D warnings
cargo test -p rara-trading feed
cargo test -p rara-kernel data_feed::registry::tests
cargo test -p rara-backend-admin data_feeds::router::tests
cargo test -p rara-trading market_data
cargo test -p rara-trading --test timescale_container
cargo test -p rara-trading finance
cargo test -p rara-app finance_event
```

Expected: all commands PASS.

**Step 4: Commit**

```bash
git add docs/plans/2026-07-10-rara-trading-design.md docs/guides/finance-subscriptions.md config.example.yaml
git commit -m "docs(trading): prioritize finance information subscriptions"
```

## Follow-up milestones (not part of this MVP)

1. Scheduled per-user digests that summarize silent matches without re-reading the entire feed history.
2. Curated ticker aliases and deterministic named-entity extraction; no regex guesswork over all-caps titles.
3. Article fetching with publisher-specific policy, robots/licensing review and content-size limits.
4. Portfolio-aware relevance only after positions are durable and consented; it must remain informational, never an order trigger.
5. The isolated backtest research bench defined in `2026-07-10-rara-trading-research-mvp-implementation.md`.
