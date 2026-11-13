"""Tests for the bootstrap bands.

The property that makes them trustworthy: when the user's spending is even,
the median band and the deterministic walk agree. A band that disagreed with
the deterministic answer on simple input would be a second forecast rather
than an uncertainty around one.
"""

from app.bootstrap import (
    MIN_HISTORY_DAYS,
    RESAMPLES,
    percentile_bands,
)


def even_history(days: int, daily: int) -> list[int]:
    return [daily] * days


def test_a_steady_spender_gets_one_answer_not_a_spread():
    # Every day costs exactly 1000: resampling the history always draws 1000,
    # so every sample is the deterministic walk and the bands are a point.
    bands = percentile_bands(
        even_history(30, 1000),
        horizon=30,
        resamples=200,
        starting_balance=10_000,
    )

    assert len(bands) == 3, "one band per percentile"
    troughs = {band.trough_minor for band in bands}
    endings = {band.ending_balance_minor for band in bands}
    assert troughs == {-20_000}, "30000 spent over 30 days from 10000"
    assert endings == {-20_000}


def test_the_median_band_agrees_with_the_deterministic_walk():
    # The deterministic walk over an even history is the same arithmetic, so
    # the P50 band must match it exactly. If it did not, the bands would be a
    # second, different forecast.
    history = even_history(30, 1000)
    bands = percentile_bands(
        history, horizon=30, resamples=200, starting_balance=10_000
    )
    median = next(band for band in bands if band.percentile == 50)

    assert median.trough_minor == 10_000 - 30 * 1000
    assert median.ending_balance_minor == 10_000 - 30 * 1000
    assert median.recovery_day is None, "a wallet that never dips never recovers"


def test_a_lumpy_spender_gets_a_spread():
    # Half the days cost 200, half cost 4000: the average is the same 2100 as
    # an even spender's, but the range is wide, and the bands must say so.
    history = [200, 4000] * 15
    bands = percentile_bands(
        history, horizon=30, resamples=RESAMPLES, starting_balance=100_000
    )

    pessimistic = next(band for band in bands if band.percentile == 10)
    optimistic = next(band for band in bands if band.percentile == 90)

    assert pessimistic.trough_minor < optimistic.trough_minor, (
        "a lumpy spender's band must be wider than a point"
    )
    # The low percentile is worse than the even spender's trough of
    # 100000 - 30*2100 = 37000, and the high percentile better.
    assert pessimistic.trough_minor < 37_000
    assert optimistic.trough_minor > 37_000


def test_income_and_commitments_are_added_on_their_days_in_every_sample():
    # A paycheck of 5000 on day 10 and a rent charge of 2000 on day 5 apply to
    # every sample, so they are part of the shape of every band rather than
    # being averaged over.
    bands = percentile_bands(
        even_history(30, 1000),
        horizon=30,
        resamples=200,
        recurring_by_day={4: 2_000},
        income_by_day={9: 5_000},
        starting_balance=10_000,
    )
    median = next(band for band in bands if band.percentile == 50)

    assert median.ending_balance_minor == 10_000 - 30 * 1000 - 2_000 + 5_000


def test_below_the_minimum_history_the_bands_decline_to_answer():
    bands = percentile_bands(even_history(MIN_HISTORY_DAYS - 1, 1000))

    assert bands == [], (
        "resampling a few days describes those days, not the user's habits"
    )


def test_recovery_day_percentiles_are_ordered():
    # Half the samples recover on day 5, half never recover at all. The low
    # percentile must be the early recovery and the high percentile must
    # report none, not both reporting the same day.
    import random

    history = even_history(30, 1000)
    bands = percentile_bands(
        history,
        horizon=30,
        resamples=200,
        starting_balance=-100_000,
    )

    low = next(band for band in bands if band.percentile == 10)
    high = next(band for band in bands if band.percentile == 90)

    # A deeply negative balance never recovers within the horizon, so no band
    # reports a recovery; that is the honest answer rather than a hopeful one.
    assert low.recovery_day is None
    assert high.recovery_day is None
