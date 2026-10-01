-- Analytics substrate: TimescaleDB extension (Phase 0.1).
-- The compose `postgresdb` service runs timescale/timescaledb:2.23.0-pg18, which
-- preloads the library. This migration only has to register the extension.
-- Edition check: `SHOW timescaledb.license;` (community/apache builds do not
-- include continuous aggregates or compression).
CREATE EXTENSION IF NOT EXISTS timescaledb;
