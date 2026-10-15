# A purchase is recorded even when its exchange rate cannot be resolved

Creating a cross-currency purchase must not fail because the FX provider is unavailable. Previously such
a purchase returned an error and the insert was rejected. We invert that: the transaction is recorded with
no conversion and marked unresolved, analytics exclude or approximate it, and resolution happens later
through a bounded retry job whose results land in the OLTP row via the normal versioned update path and
re-project the fact. Bookkeeping must not depend on a third-party service being up.
