# A Purchase debits the ledger

Balances derive from an append-only ledger (ADR-0004), and a Purchase records its debit ledger
entry in the same transaction that records the Purchase. We considered computing balances from the
analytics projection instead, and subtracting purchases at read time, but that leaves two sources of
truth for the same number and invites them to drift. With one ledger, a Wallet's balance is a sum,
runway is arithmetic over that sum, and every movement — purchase, income, transfer, adjustment — has
the same shape and the same audit trail.
