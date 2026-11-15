# Reconciliation locks the money, not the row

A reconciliation says one thing: on this date, this wallet's balance matched the bank's. It
vouches for an **amount**, a **date** and a **wallet**. It does not vouch for which category a
purchase belonged to, and it never claims the row is now perfect.

So reconciling locks exactly what it verified:

- **Locked:** `amount_minor`, the purchase date, the wallet. Changing any of them makes the
  reconciliation untrue, so the write is refused until the user un-reconciles.
- **Still editable:** category, splits, notes. None of these changes the wallet balance the
  statement matched.

The alternative — locking the whole row — has one rule and no reasoning for the user to do, and
it was the obvious choice until the cost was named: a user who mis-categorised a checked
purchase would have to un-reconcile a month that was, in fact, perfectly reconciled. That
teaches them that "reconciled" means "frozen", which is not what it means, and makes them
reluctant to reconcile at all.

Locking money and categories together is the worst of the three: it refuses a split, which
cannot break the balance, so the user has a rule they cannot predict.

Reconciling never creates an adjustment row. A difference is either a purchase the user has not
recorded or one they typed wrong, and both are theirs to decide. The app reports the
difference, offers the candidates that might explain it, and declines to balance it for them.

A reconciliation is history, not state: each one is a row, a later one supersedes an earlier one
by being later, and nothing deletes a past reconciliation.
