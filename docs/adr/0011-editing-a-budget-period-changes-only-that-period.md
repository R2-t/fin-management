# Editing a Budget period changes that period only

A Budget period's limit is fixed once the period has been materialised. Editing it changes that
period and nothing else: the next period keeps the limit it was given, even when the edit would
change what the next period's carry should have been.

We considered recomputing the next period whenever an earlier period is edited, and rejected it.
The next period's limit was computed from a specific reading of the previous period at a specific
time. Changing it later would mean an edit silently changes a number the user was already shown,
somewhere else, without them being told — the same problem the app avoids everywhere else, where
a reported figure is not restated after the fact (ADR-0001). It also compounds: editing two
periods would have to be ordered, and the result would depend on which edit came last.

Materialised means materialised, for the same reason the projection's `as_of` and
`projection_delay_seconds` exist: a figure is computed at a point in time, and the caller is told
when. If the user disagrees with the next period's limit, they can edit that period too — the
edit is one bounded action, and it is one the user has to take deliberately.

The same rule governs a period's limit in every other respect. Nothing raises a limit
automatically (ADR-0010), and a limit can only be lowered by a carry the user opted into.
