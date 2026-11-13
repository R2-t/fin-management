"""Bootstrap uncertainty bands for the runway.

The deterministic runway in Rust is exact but coarse: it projects ONE pace
forward and reports ONE trough and ONE recovery date. Maria whose daily
spending ranges from 200 on quiet days to 4000 when she shops gets the same
answer as Maria who spends exactly 1000 every day, because their averages
match.

The bands answer the question the average hides: not "when do I run out" but
"where is it likely to fall, and how wide is the range". The resampling uses
only the user's own daily spending (D49) -- no model, no fitting, no
distribution assumed -- so the bands describe her real variability rather
than an assumed one.

The deterministic answer is not replaced. It is the anchor: with enough
history and an even spending pattern, the median of the samples and the
deterministic walk agree, and a property test pins that. When they disagree
the disagreement is information -- it means the spending pattern is lumpy in
a way a single pace cannot express.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

# The version reported with every response, so a later change in the
# resampling can be told apart from a change in the deterministic walk.
BANDS_VERSION = "bootstrap-v1"

# How many times the user's own history is resampled. 1,000 is enough for a
# stable percentile at P90: the standard error of a percentile estimate
# shrinks with the square root of the count, and beyond this the answer
# moves by less than a minor unit.
RESAMPLES = 1_000

# The percentiles reported. P50 is the most likely path, P10 and P90 the
# edges of what is plausibly normal rather than the worst imaginable.
PERCENTILES = (10, 50, 90)

# Days of history the bands need. Below this, resampling would describe the
# last few days rather than the user's habits, and the answer would look
# precise while being a coin flip (6.6).
MIN_HISTORY_DAYS = 14

# The horizon, matching the deterministic runway's.
HORIZON_DAYS = 90


@dataclass(frozen=True)
class Bands:
    """The uncertainty around the runway, per percentile."""

    percentile: int
    trough_minor: int
    recovery_day: int | None
    ending_balance_minor: int


def percentile_bands(
    daily_history: list[int],
    horizon: int = HORIZON_DAYS,
    resamples: int = RESAMPLES,
    recurring_by_day: dict[int, int] | None = None,
    income_by_day: dict[int, int] | None = None,
    starting_balance: int = 0,
) -> list[Bands]:
    """Resamples the user's own daily spending and projects each sample.

    `daily_history` is the discretionary spend per day, oldest first, with
    zero for days that had none. Each sample draws `horizon` days from it at
    random -- with replacement, which is what bootstrap means -- and walks
    the balance forward as the deterministic runway does.

    `recurring_by_day` and `income_by_day` are the declared commitments and
    scheduled income by day offset. They are added on their days in EVERY
    sample, so the bands include the commitments rather than averaging over
    them (6.5).

    Returns one `Bands` per percentile in `PERCENTILES`.
    """
    recurring_by_day = recurring_by_day or {}
    income_by_day = income_by_day or {}

    if len(daily_history) < MIN_HISTORY_DAYS:
        # Resampling four days would describe the last few days, not the
        # user's habits. Saying so is the honest answer (6.6).
        return []

    history = np.asarray(daily_history, dtype=np.int64)
    rng = np.random.default_rng()

    troughs: list[int] = []
    recoveries: list[int | None] = []
    endings: list[int] = []

    for _ in range(resamples):
        sample = rng.choice(history, size=horizon, replace=True)
        balance = starting_balance
        trough = balance
        went_negative = balance < 0
        recovery_day: int | None = None

        for offset in range(horizon):
            balance -= int(sample[offset])
            balance -= recurring_by_day.get(offset, 0)
            balance += income_by_day.get(offset, 0)

            trough = min(trough, balance)
            if balance < 0:
                went_negative = True
            if went_negative and recovery_day is None and balance > 0:
                recovery_day = offset + 1

        troughs.append(trough)
        recoveries.append(recovery_day)
        endings.append(balance)

    bands = []
    for percentile in PERCENTILES:
        p_trough = int(np.percentile(troughs, percentile))
        p_ending = int(np.percentile(endings, percentile))

        # The recovery day is a date, not a magnitude: its percentile is the
        # day that fraction of samples recovered on or before. Samples that
        # never recovered are past the horizon and sort last.
        never = [horizon + 1 for day in recoveries if day is None]
        recovered_days = sorted(
            [day for day in recoveries if day is not None] + never
        )
        index = int(round(len(recovered_days) * percentile / 100))
        index = min(index, len(recovered_days) - 1)
        chosen = recovered_days[index]
        p_recovery = chosen if chosen <= horizon else None

        bands.append(
            Bands(
                percentile=percentile,
                trough_minor=p_trough,
                recovery_day=p_recovery,
                ending_balance_minor=p_ending,
            )
        )
    return bands
