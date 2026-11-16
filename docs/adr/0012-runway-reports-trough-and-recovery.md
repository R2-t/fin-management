# A Runway reports the trough and the recovery date, projected forward

A Runway projection answers "how long does my money last". It starts from the Wallet's current
ledger balance and walks forward day by day, subtracting the projected spending and the recurring
charges due that day and adding the scheduled income due that day. It reports two figures: the
trough, the lowest balance the projection reaches, and the recovery date, when the balance returns
above zero.

The projection runs forward even when the balance is already negative. A balance below zero does
not remove the question, it sharpens it: the user is already overdrawn, and what they need to know
is how deep the hole gets and whether the next paycheck covers it. Reporting a date the balance
"reached zero" would describe the past and leave the future unaddressed. The trough and the
recovery date are reported together because the depth is what makes the situation urgent and the
recovery date is what tells the user whether it resolves.

The simulation starts from today's ledger balance and projects from tomorrow. Today's spending is
not projected: a day that is half over cannot be completed without inventing a number, and the
response carries the moment the balance was read so the caller knows what it was based on.

The burn rate and the recurring charges are projected separately, and never overlap. The burn rate
covers discretionary spending; each recurring rule contributes dated movements on the days it fires.
A subscription that is both in the trailing spend and declared as a rule would otherwise be
subtracted twice, which would make the runway systematically pessimistic by the cost of every
subscription the user has declared. Deciding which recorded purchases are the payments of a
declared rule is a shared piece of work, needed here and again when the app detects recurring
charges the user did not intend.

The burn rate is measured over up to thirty days, falling back to however much history exists down
to a week, and below a week no burn rate is reported at all. The weekday profile — the median spend
per weekday — replaces the flat daily average once there is enough history for it, because a user
who spends nothing on weekdays genuinely has a different pace at the weekend and a flat average
would tell them they run out on a Thursday when they do not. The profile needs roughly a month to
be meaningful, so below that the flat average is the honest answer.

The horizon is ninety days, and the response says so. Without a horizon, "still solvent" and "we
stopped looking" look identical. When no income is scheduled, the answer is that the balance
survives the horizon rather than that it never recovers: the second is a prediction of doom where
the first is simply the limit of what was computed.
