# Phase 6 — Forecast precision

The deterministic runway and overspend forecast answer a question correctly
but coarsely. They assume a pace, project it linearly, and report a single
number. A user whose spending is lumpy — rent on the 1st, groceries weekly,
nothing in between — gets an answer that is right on average and wrong on the
day that matters.

This phase strengthens the forecast in three layers, each building on the
last. The deterministic walk stays: it is the auditable core that everything
else is checked against (D5). What changes is what surrounds it.

## Where this came from

The review of Phase 2 found the runway honest but coarse. The decisions taken
here were not opened for debate: D49 already settled that uncertainty comes
from bootstrap resampling of the user's own spending (Jev 1.00), and D8
already kept Prophet out of the first release. This phase is the plan for
delivering those, and for the precision work between them.

## Tasks

- [x] 6.1 The overspend forecast uses the weekday profile, not a flat pace.
      It already computes the profile; it projects the remaining days with it
      rather than `daily × days_left`. Smallest change with a visible
      improvement, and it makes the two forecasts agree about the pace. (S)
- [ ] 6.2 The runway's discretionary pace uses the weekday profile for the
      whole horizon. Verify it already does; pin it with a test, because the
      overspend forecast did not and the two must not drift. (S)
- [ ] 6.3 Bootstrap uncertainty bands on the runway. Sample the user's own
      daily discretionary spending 1,000 times, rebuild the projection for
      each sample, and report the P10 / P50 / P90 trough and recovery date.
      The median band must agree with the deterministic answer when the user
      spends evenly — a property test, not a hope. (M)
- [ ] 6.4 Volatility as a first-class output. A user whose daily spending is
      steady and one who spends the same average but lumpy get the same
      runway today. Report the spread so the user knows how much to trust
      it, and let the confidence threshold decide how the client renders it. (S)
- [ ] 6.5 Recurring charges with real dates in the projection's samples. The
      bootstrap resamples the *discretionary* spend; the declared rules and
      income are added on their days in every sample, so the bands include
      the commitments rather than averaging over them. (M)
- [ ] 6.6 Cold start, stated. Below the minimum history the deterministic
      forecast says so already; the bands say it too, rather than resampling
      four days and calling it a distribution. (S)
- [ ] 6.7 Persist the bands to `forecast` with `model_version`, so a
      probabilistic forecast can be back-tested against the deterministic
      one it is meant to improve. (S)
- [ ] 6.8 Back-test harness: replay the bands over a past window and report
      how often the true balance fell outside them. A band that is too wide
      is useless; one that is too narrow lies. (M)

## What this phase deliberately does not do

- **No per-user ML models.** D49 keeps the uncertainty descriptive of the
  user's own history rather than fitted. A model that needs training data the
  user has not provided yet is a later conversation.
- **No Prophet.** D8 kept it out of the first release and D49 kept it out of
  the uncertainty work; revisit only if the bootstrap bands prove too wide to
  be useful.
- **No cross-user anything.** ADR-0002 and the tenancy boundary hold.

## Status

**6.1 done.** The overspend forecast prices every remaining day by its
weekday from the user's own history, rather than by an average -- and finds
the crossing day by walking the same per-day costs, rather than dividing by
the pace. The forecast and the runway now read the pace the same way, and a
weekend-shaped history no longer projects like a flat one.

A test pins it with a user who spends 1000 on weekdays and nothing at
weekends: the projection is the sum of the remaining days priced by their
weekday, and is asserted to differ from pace x days.

## Order

6.1 and 6.2 are the precision the user feels first and are Rust-only, so they
go first. 6.3–6.5 are the Python bootstrap. 6.6–6.8 are the honesty that makes
the numbers worth trusting. Nothing here reopens a settled decision.
