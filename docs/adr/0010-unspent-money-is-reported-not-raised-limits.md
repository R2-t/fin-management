# Unspent money is reported, not spent on a higher limit

A Budget period's limit can be lowered by a carried overspend, but nothing raises it automatically.
Unspent money in a period is reported to the user as money left over; it does not become extra
spending limit in the next period.

The reason is that unspent money is usually not spare money. Someone who spends three hundred of a
five hundred limit because they are saving for a trip has not under-spent a budget — they have
saved two hundred, and a limit that rose to seven hundred next month would work directly against
the thing they were doing. The ledger never resets either: the two hundred is still in the wallet
balance, where it is already visible. Raising the limit would spend that money twice in the user's
understanding — once by making it spendable, and once by leaving the saving in place.

So the carry in one direction is a change to a limit the user chose, and is not made on their
behalf. The carry in the other direction — an overspend reducing the next period's limit — is
different: the overspend is a fact the user themselves created, and a lower limit restricts
spending rather than expanding it. That asymmetry is deliberate. A limit that the system raises
spends the user's savings; a limit that the system lowers charges the user for their own
overspending.

The unspent amount is reported beside the limit rather than folded into it, so the user sees both
the limit they set and the surplus they left.

Two things are deliberately kept independent. The limit is a fact about the budget; the wallet
balance is a fact about the ledger. A negative wallet balance and a budget overspend are two
separate facts about the same month, and reducing the limit for both would charge the user twice
for what they experience as one difficult month. Nothing about the carry changes when the balance
is negative, and both figures are shown.
