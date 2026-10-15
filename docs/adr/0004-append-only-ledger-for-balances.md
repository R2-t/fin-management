# Wallet balances derive from an append-only ledger, not a mutable column

Balances will be computed by summing ledger entries (opening balance, income, transfers, adjustments)
rather than storing an editable balance on the wallet. Runway forecasting must be explainable against
recorded activity, and a mutable column offers no way to reconcile balance against starting value minus
spend. Enforcing that each transfer's entries net to zero ships with the feature, since a partial write
would silently create or destroy money in aggregate figures.
