# Money and rounding conventions

Normative for Phase 0.5–0.11 and anything that stores or aggregates money.
Written for Phase 0.2; revisit if a zero- or three-decimal currency ships.

## 1. Units

- **Amounts are integer minor units** (`BIGINT` / `i64`): `1050` = `10.50` in a
  2-decimal currency. This covers `item.unit_price`, `purchase_fact.amount_*`,
  `budget.amount_minor`, `ledger_entry.amount_minor`.
- **Minor-unit exponent is per currency** (ISO 4217): 2 for USD/EUR/GBP,
  **0 for JPY**, 3 for BHD/KWD. The app currently assumes **2 everywhere**.
  Known limitation: `DEFAULT_CURRENCY`/`wallet.currency` store the code but no
  exponent. Until a `currency` reference table with `minor_unit` exists, JPY
  amounts will be interpreted as 2-decimal. Track this; do not aggregate JPY
  and USD together.
- **Rates are scaled integers**: `rate_scaled = round(rate * RATE_SCALE)` with
  `RATE_SCALE = 1_000_000_000` (1e9). Example: `0.85` → `850_000_000`.
  Stored in `purchase.exchange_rate_scaled` and `exchange_rate_cache.rate_scaled`.
- **`f64` is never used for stored or aggregated money or rates.** It is only
  acceptable for dimensionless analytics output (growth %, ratios, z-scores).

## 2. Conversion

```
amount_base_minor = round_half_away_from_zero(amount_minor * rate_scaled / RATE_SCALE)
```

- **Rust:** compute in `i128` (`amount_minor as i128 * rate_scaled as i128`),
  then round. `i64 * i64` can overflow for large amounts; `i128` cannot.
- **SQL:** compute in `NUMERIC` (never `float8`), then round and cast to `BIGINT`.
- **Rounding rule: half away from zero** (`892.5 → 893`, `-892.5 → -893`).
- **Convert the aggregate, not each line.** The current view converts
  `sum(quantity * unit_price) * rate` once. Keep that: one conversion and one
  rounding per purchase, so item-level rounding drift cannot accumulate.
- **Same-currency purchases store no rate** (`exchange_rate_scaled IS NULL`) and
  `amount_base_minor = amount_minor` exactly.

### Worked examples (rate fixed at `rate_scaled = 850_000_000`)

| `amount_minor` | exact | result |
|---|---|---|
| `1050` (10.50 USD) | `892.5` | `893` |
| `1000` | `850` | `850` |
| `1` (0.01 USD) | `0.85` | `1` |
| `0` | `0` | `0` |
| `-1050` (refund) | `-892.5` | `-893` |

Rounding up from `1` to `1` is expected: the smallest representable base
amount is one minor unit. It is a documented artifact, not a bug.

## 3. Where each rule applies

| Stage | Representation | Rounding |
|---|---|---|
| OLTP write (`purchase`, `item`) | source currency, minor units | none |
| OLTP read (`purchase_items` view) | `total` `NUMERIC`; `total_wallet` converted | one, per purchase (0.7) |
| Fact projection (0.10/0.11) | `amount_minor` + `rate_scaled` + `amount_base_minor` | one, at projection time |
| Analytics aggregation | sums of `amount_base_minor` | none (integers) |

`amount_base_minor` is materialized **only in the analytics projection**. OLTP
totals stay derived, per `prompts/multi-currency.md`.

## 4. Aggregation rules

- **Always group by currency.** Never `SUM` across currencies or across base
  currencies. A per-currency breakdown is required for every aggregate.
- Rates are stored with the transaction, so historical amounts are stable when
  the rate cache changes. Re-projecting a purchase must produce the same
  `amount_base_minor` (`rate_scaled` is copied, never re-fetched).

## 5. Reconciliation criteria (test contract)

For each `(user_id, base_currency, day)`:

```
sum(purchase_fact.amount_base_minor for non-deleted purchases)
  == sum(round(quantity * unit_price * rate_scaled / RATE_SCALE))
       over the same purchases read from OLTP
```

Tests must cover: same-currency, foreign currency, `NULL` rate, refund/negative
amount, `NULL` category, edit that changes `purchase_date`, and soft delete.
Any mismatch is a projection bug, not a rounding tolerance.

## 6. Open items

- Currency reference table with `minor_unit` (blocks correct JPY/KWD handling).
- Decide whether to store `amount_base_minor` for `item_fact` (item-level
  anomaly detection, investigation A0.10) — same rules apply.
