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

**Ledger entry**:
One dated movement of money for one Wallet: an opening balance, an income receipt, a transfer
leg, a purchase debit, or an adjustment. A Wallet's balance is the sum of its ledger entries.
_Avoid_: transaction, journal row, movement

**Budget**:
A spending limit for one Category within one Wallet, over a Budget period.
_Avoid_: cap, allowance, envelope, limit

**Budget template**:
The recurring definition of a Budget: its amount, category, wallet, cadence and reset day.
_Avoid_: recurring budget, budget rule

**Budget period**:
One materialised occurrence of a Budget template, holding the limit that actually applies to
that period. Editing a period does not change the template.
_Avoid_: budget cycle, budget instance

**Reset day**:
The day a Budget period starts, within its cadence. One for a calendar month, fifteen for a
budget that follows a mid-month payday, the last day of the month for one that follows a
month-end payday.
_Avoid_: period start, anchor date

**Rollover**:
Carrying unspent Budget limit into the next Budget period. Independent of carrying an
overspend into it: a user can want one without the other.
_Avoid_: carry over, accumulation

**Unspent money**:
Money in a Budget period that was not spent. Reported beside the limit rather than
added to the next period's limit, because the user may be deliberately saving it.
_Avoid_: rollover, carried balance, remaining allowance

**Effective limit**:
The limit that actually applies during a Budget period: the template's amount, lowered by
a carried overspend, or replaced by a user edit. Nothing raises it automatically.
_Avoid_: current limit, live limit

**Income schedule**:
An expectation that money will arrive on a cadence. It is not money until an occurrence is
received.
_Avoid_: salary, payroll, expected income

**Income receipt**:
A recorded Income occurrence: ledger credit entries in the wallets it was directed to.
_Avoid_: booked income, confirmed payment

**Runway**:
How long a Wallet's money lasts at its current spending pace and commitments: the trough
the balance reaches, and the date it recovers. Computed per Wallet, never combined across
currencies.
_Avoid_: cash-out date, burn date, depletion date

**Burn rate**:
The daily spending pace a Runway projection assumes, measured over a trailing window of up
to 30 days. Excludes recurring charges, which are projected separately as dated movements.
_Avoid_: spend rate, velocity

**Trough**:
The lowest balance a Runway projection reaches. Reported alongside the recovery date
because the depth of the hole is what makes it urgent, not only when it ends.
_Avoid_: low point, minimum balance

**Recovery date**:
The date a Wallet's projected balance returns above zero. `None` when the balance never
recovers within the horizon.
_Avoid_: payback date, break-even date

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

**User**:
The person whose money the records describe. Identified by a UUID that is the same value in
every table and in the `users` row that holds their login.
_Avoid_: Account, customer, member, owner

**Credential**:
The password a User presents to prove who they are. The service stores only its Argon2id hash,
never the password itself.
_Avoid_: Password (when the stored form is meant), secret, hash

**Session**:
A logged-in browser's proof of identity, carried in a cookie and exchanged for the User on every
request. Ends when it expires or when the User logs out, and the proof is destroyed at that
moment.
_Avoid_: Login token, cookie, auth token

**Service token**:
A short-lived credential the Rust service mints for machine-to-machine calls, naming one User.
The analytics sidecar trusts the tenant in the token, never the tenant a caller asks about.
_Avoid_: API key, internal token, machine credential

**Login attempt**:
One try at presenting a Credential, recorded whether it succeeded or failed, so repeated failures
can be counted against the email and the address that made them.
_Avoid_: Auth record, sign-in event

**Suggestion**:
The category the system proposes for a Purchase the user has not categorized. It is a
proposal, never a decision: the user's own choice is final, and a Suggestion that arrives
after the purchase changed is dropped rather than applied.
_Avoid_: Prediction, auto-category, guess

**Suggestion sweep**:
A periodic pass that finds Purchases which never received a Suggestion and asks for one.
It exists because the first request to suggest can fail, and a Purchase nobody touches
again would otherwise stay uncategorized forever.
_Avoid_: Retry job, backfill, reconciliation
