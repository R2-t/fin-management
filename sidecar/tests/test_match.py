"""Tests for merchant alias matching.

The cases come from the decision that set the threshold (D46): a same-company
descriptor must link, a typo must link, and a genuinely different merchant
must not -- because linking `TARGET` and `TARGET PHARMACY` merges the user's
medicine budget into groceries.
"""

from app.match import ALIAS_THRESHOLD, match_alias
from app.normalize import normalize_merchant


def test_the_threshold_is_the_decided_value():
    assert ALIAS_THRESHOLD == 85.0


def test_a_same_company_descriptor_links():
    # NETFLIX.COM vs NETFLIX: same subscription, must be one merchant. The
    # suffix is cleaned by the rules first, which is what puts this pair over
    # the threshold -- raw it scores 77.8.
    match = match_alias(normalize_merchant("NETFLIX.COM"), [("NETFLIX", "m-netflix")])
    assert match is not None
    assert match.merchant_id == "m-netflix"
    assert match.score >= 85


def test_a_typo_links():
    # TARGT vs TARGET: a statement typo, must link.
    match = match_alias(normalize_merchant("TARGT"), [("TARGET", "m-target")])
    assert match is not None
    assert match.merchant_id == "m-target"


def test_a_merchant_that_adds_a_word_does_not_link():
    # TARGET PHARMACY vs TARGET: the user budgets groceries and medicine
    # separately, so these must stay apart.
    match = match_alias(normalize_merchant("TARGET PHARMACY"), [("TARGET", "m-target")])
    assert match is None, "a second word keeps the merchants distinct"


def test_an_unrelated_merchant_does_not_link():
    match = match_alias("STARBUCKS", [("TARGET", "m-target")])
    assert match is None


def test_the_closest_alias_wins():
    aliases = [
        ("NETFLIX", "m-netflix"),
        ("NETFLIX UK", "m-netflix-uk"),
    ]
    match = match_alias("NETFLIX", aliases)
    assert match is not None
    assert match.merchant_id == "m-netflix"


def test_no_aliases_means_no_match():
    assert match_alias("NETFLIX", []) is None


def test_an_empty_name_means_no_match():
    assert match_alias("", [("NETFLIX", "m-netflix")]) is None


def test_a_match_below_the_threshold_is_reported_as_none():
    # Returning the best score anyway would let the caller treat a near miss
    # as a match by forgetting to check it.
    assert match_alias("NETFLIX", [("TARGET", "m-target")]) is None
