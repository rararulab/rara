CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE TABLE IF NOT EXISTS market_candles (
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

CREATE INDEX IF NOT EXISTS idx_market_candles_venue_symbol_time
  ON market_candles(venue, symbol, timeframe, open_time DESC);

CREATE INDEX IF NOT EXISTS idx_market_candles_source_symbol_time
  ON market_candles(source_name, symbol, timeframe, open_time DESC);

CREATE TABLE IF NOT EXISTS market_candle_corrections (
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
