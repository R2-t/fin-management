# Exchange rate cache stays unlogged; durable history gets its own table

`exchange_rate_cache` is UNLOGGED, so it loses contents on crash or immediate restart and keeps no long
term history. That was acceptable while rates were purely transient and always re-fetchable. Recording
that purchases may now have an unresolved rate filled in later means we need durable rates, so a logged
history table (keyed by currency pair and time) is added, and fetching both going forward and for older
gaps relies on it plus the provider's historical endpoint. The cache remains for hot lookups.
