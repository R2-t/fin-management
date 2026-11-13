# A Budget template recurs, and each occurrence is a Budget period

A Budget is stored as a template plus a set of materialised periods. The template holds the
recurring definition — limit, category, wallet, cadence, reset day — and each period holds the
limit that actually applies to one stretch of time. Editing a period does not change the
template, so raising a limit for a bad month does not raise it forever.

We considered storing only the template and computing every period on read, and rejected it
because an edited period would then have nowhere to live: the glossary distinguishes "the
recurring definition" from "one materialised occurrence", and that distinction is the point.
Periods are materialised lazily, when a period is first asked about, rather than by a scheduler.
A scheduler would create rows for periods nobody looks at and needs a job to keep running; the
lazy path creates exactly the periods the user has seen, and a uniqueness constraint keeps two
concurrent requests from creating the same one twice.

Two rollover behaviours are kept independent, because they are separate intents: whether unspent
limit carries forward, and whether an overspend carries forward. A user who wants a strict reset
disables the first; a user who wants one bad month forgiven without resetting everything else
disables the second. The ledger is append-only and never resets, so the budget's carry behaviour
is a reporting choice about which limit applies, not a statement about the money.

A Budget's cadence is anchored to a reset day rather than to the calendar month or to an
interval from its start date. The calendar month gives people whose pay arrives mid-month a
first period of half a month, and an interval from the start date drifts so that "my February
budget" stops meaning February. A reset day is the calendar month generalised: one for most
users, fifteen for someone paid on the fifteenth, the last day of the month for someone paid at
month end. A reset day of minus one means the period *ends* on the last day of its month, which
is the same month a reset day of one covers but is expressed as an end, because that is how a
month-end payday reads.

Changing a Wallet's currency disables the Budgets attached to it, and also the income schedules and
recurring rules pointed at that wallet: all three are denominated in the old currency, and a limit
or a charge stated in dollars has no meaning against a wallet that now holds euros. This only ever
applies to a wallet with no ledger entries. A wallet that holds money keeps its currency and the
change is refused, because converting its entries would need a rate for a date in the past that was
never recorded — and inventing one is what this app refuses to do. The path a user takes instead is
to create a wallet in the new currency and record an opening balance in the amount she really has,
a number she enters herself. The one-currency-per-wallet definition is an invariant the system
enforces, not a description of what it tolerates.

A Budget's limit is a future
constraint rather than a recorded value, so point-in-time values do not decide what should
happen to it (ADR-0001), and re-denominating the limit would need a rate at the moment of the
change — inventing one, or reaching for a live rate inside an unrelated update, both of which
the rest of this app refuses to do. Disabling is honest: the user is told the budget stopped
applying and recreates it deliberately.
