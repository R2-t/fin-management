# An allocation rolls over, a limit does not

ADR-0010 says unspent money is reported and never raises the next period's limit, because a
limit that rose on its own would spend money the user was deliberately saving. Envelope
budgeting says the opposite: what you did not spend is still in the envelope.

Both are right about different numbers, so the model gets two of them.

**The limit** (`budget_period.amount_minor`) is the plan. It feeds the overspend forecast, the
carry-overspend rule and every projection. It never rolls: if the plan for Groceries is £500, it
is £500 next period too. ADR-0010 stands, unchanged and unqualified.

**The allocation** (`budget_period.budgeted_minor`) is money put into the envelope. Spending
comes out of it and whatever remains is `available`, which does roll forward:

```
available = previous_available + budgeted − spent
```

The two coexist because they answer different questions. "What did I plan to spend?" is a
limit, and a plan that drifted upward on its own would be a plan nobody wrote. "How much money
is in this envelope?" is an allocation, and that money is real, so it must not evaporate at a
period boundary.

The number that makes envelopes worth having is **to be budgeted**:

```
to_be_budgeted = income_this_period + rollover_in − Σ budgeted
```

That figure — income not yet given a job — has no meaning in the limit model, and it is the
single most useful number in envelope budgeting. It is reported, never enforced: the app does
not stop a user allocating money they do not have; it tells them the number is negative.

Because allocation rolls, `available` may be negative. That is overspending carried forward,
and it is reported rather than clamped. A negative envelope is the most honest statement
available about money that has already been spent.

`available` is snapshotted when the period closes, and the period closes **when the next one is
first materialised** — the same lazy moment that creates periods at all (ADR-0009). There is no
scheduler in this service, and inventing one to fix a figure that can be fixed at a boundary
the service already observes would be a process to maintain forever.

One consequence is deliberate: editing a purchase from a closed period moves the *current*
period's available, not the closed one's. The closed figure is what the user saw and reconciled
against; rewriting it later would make a reconciled month stop matching its statement with no
record that anything changed.
