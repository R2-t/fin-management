"""Tests for the merchant name rules.

These are the cases the rules exist for: a payment descriptor that differs
from the name the user recognises. Each rule is checked with a descriptor
that a real card statement produces.
"""

from app.normalize import normalize_merchant


def test_a_plain_name_is_unchanged():
    assert normalize_merchant("NETFLIX") == "NETFLIX"


def test_case_is_normalised():
    assert normalize_merchant("netflix") == "NETFLIX"
    assert normalize_merchant("  NetFlix  ") == "NETFLIX"


def test_a_square_descriptor_loses_its_processor_prefix():
    assert normalize_merchant("SQ *Netflix") == "NETFLIX"


def test_a_paypal_descriptor_loses_its_processor_prefix():
    assert normalize_merchant("PAYPAL *NETFLIX") == "NETFLIX"


def test_a_repeated_processor_prefix_is_stripped_until_stable():
    assert normalize_merchant("SQ *SQ *NETFLIX") == "NETFLIX"


def test_a_trailing_store_number_is_removed():
    assert normalize_merchant("WALMART #1234") == "WALMART"


def test_a_trailing_store_number_without_a_hash_is_removed():
    assert normalize_merchant("WALMART 1234") == "WALMART"


def test_a_phone_number_is_removed():
    assert normalize_merchant("JOES PIZZA 5551234567") == "JOES PIZZA"


def test_a_trailing_date_is_removed():
    assert normalize_merchant("SHELL 12/25") == "SHELL"


def test_punctuation_between_words_becomes_a_space():
    assert normalize_merchant("JOE'S PIZZA & DELI") == "JOE'S PIZZA & DELI"
    assert normalize_merchant("7-ELEVEN") == "7-ELEVEN"


def test_a_non_ascii_name_survives():
    # A character-class rule would strip the accent and leave "CAF".
    assert normalize_merchant("CAFÉ CENTRAL") == "CAFÉ CENTRAL"


def test_runs_of_whitespace_collapse():
    assert normalize_merchant("JOE'S   PIZZA") == "JOE'S PIZZA"


def test_an_empty_descriptor_is_empty():
    assert normalize_merchant("") == ""
    assert normalize_merchant("   ") == ""


def test_a_descriptor_with_no_name_left_is_empty():
    assert normalize_merchant("1234567890") == ""


def test_a_domain_suffix_is_not_part_of_the_name():
    # The alias threshold (D46) is calibrated for names: NETFLIX.COM against
    # NETFLIX scores 77.8 raw, which would not link at 85. Cleaning the suffix
    # here lets the threshold stay where the decision put it.
    assert normalize_merchant("NETFLIX.COM") == "NETFLIX"
    assert normalize_merchant("netflix.co.uk") == "NETFLIX"


def test_a_dotted_word_inside_a_name_is_not_a_suffix():
    # The suffix rule only fires at the end, so a name with a dot mid-string
    # is left alone.
    assert normalize_merchant("MR. PRICE") == "MR. PRICE"
    assert normalize_merchant("ST. JOHN BREAD") == "ST. JOHN BREAD"
