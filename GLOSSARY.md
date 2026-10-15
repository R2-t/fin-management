# Financial Management Domain

The context is personal finance: a user records spending against wallets, and the system
derives analytics and forecasts from those records. This glossary names the domain concepts
only; how they are stored or computed lives in specs and ADRs.

## Language

**Purchase**:
A record of something the user bought, denominated in the currency it was paid in.
_Avoid_: Transaction, expense, charge, spend

**Wallet**:
A single pot of money the user holds, in one currency.
_Avoid_: Account, bank, card, pocket

**Transfer**:
The user moving money between two of their own wallets. Not spending and not income; it changes
what each wallet holds but not the user's total.
_Avoid_: Payment, movement, internal purchase

**Income**:
Money arriving to the user from outside their wallets, such as a salary. May be split across
several wallets as one intentional act.
_Avoid_: Deposit, top-up, earnings entry

**Correction**:
A user changing or removing a previously recorded Purchase, Transfer, or Income after the fact.
Expected and routine in a record-your-spending app, not an exceptional case.
_Avoid_: Undo, amendment, audit event

**Point-in-time value**:
An amount recorded using the exchange rate in force when it happened, kept unchanged even if the
user later alters currencies or settings.
_Avoid_: Current value, live conversion

**Wallet currency**:
The currency a Wallet holds. The target that a Purchase amount is converted into.
_Avoid_: Account currency, local currency

**Purchase currency**:
The currency a Purchase was actually paid in, which may differ from its wallet's currency.
_Avoid_: Foreign currency, original currency

**Base currency**:
The currency a user nominates for defaults. It sets the currency of newly created wallets and
nothing more.
_Avoid_: Reporting currency, consolidation currency

**Reporting scope**:
Currency figures are compared within one Wallet only. Amounts are never summed across wallets
holding different currencies.
_Avoid_: Consolidated total, net worth figure

**Projection**:
A derived analytics record rebuilt from current truth whenever the underlying record changes.
Never edited directly.
_Avoid_: Cache, materialized view, snapshot

**Unresolved rate**:
A Purchase whose conversion could not be determined when it was recorded. Its existence is still
recorded; the conversion is filled in later.
_Avoid_: Missing data, failed import

**Rate completion**:
Filling in a conversion that was absent at recording time. Treated as completing an incomplete
record, not as restating history.
_Avoid_: Backfill mutation, historical rewrite
