"""Merchant name cleaning.

The rules are deliberately conservative. Each one exists because a real
payment descriptor differs from the name the user recognises, and stripping
too much would merge two different merchants into one.

This module is pure: it takes a string and returns a string. The fuzzy match
against the stored `merchant_alias` rows is a separate step, because it needs
the database and a threshold that the caller owns.
"""

import re

# Version of the rule set, reported with every response so a later rule
# change can be told apart from a model change.
RULES_VERSION = "rules-v1"

# Card processors and marketplaces that prefix a merchant's own name with
# their brand. Stripped so "SQ *NETFLIX" and "NETFLIX" resolve to one merchant.
#
# The list is short on purpose: a prefix that is wrong merges two merchants
# that happen to share a brand, which is worse than leaving noise in place.
PROCESSOR_PREFIXES = (
    "SQ *",
    "SQ*",
    "PAYPAL *",
    "PAYPAL*",
    "TST*",
    "AMZN MKTP",
    "AMAZON.COM*",
    "AMZN*",
    "APPLE.COM*BILL",
    "GOOGLE *",
    "GOOGLE*",
    "SP *",
)

# A trailing store number, phone number or date, which a payment descriptor
# appends but a merchant name does not contain.
#
# shortcut: a legitimate merchant whose name ends in four to six digits loses
# them (e.g. "7-ELEVEN 2471"). Worth it: descriptors append store numbers far
# more often than merchants are named after them.
TRAILING_NOISE = re.compile(
    r"""
    \s*\#\s*\d{2,6}                    # "#1234"
    | \s+\d{10,}                       # a phone number
    | \s+\d{1,2}/\d{1,2}(?:/\d{2,4})?  # a date
    | \s+\d{4,6}                       # a trailing store number
    $
    """,
    re.VERBOSE,
)

# Runs of punctuation and whitespace that separate words in a descriptor.
#
# Applied per character rather than with a character class: a class of
# `[A-Z0-9]` would strip every non-ASCII letter, turning "CAFÉ" into "CAF".
SEPARATORS = set(" &'/-. ")


def normalize_merchant(raw: str) -> str:
    """Cleans a payment descriptor into a comparable merchant name.

    Returns an empty string when nothing recognisable is left, which is the
    honest answer for a descriptor that carries no merchant name at all.
    """
    if not raw:
        return ""

    name = raw.strip().upper()

    # A processor prefix can repeat ("SQ *SQ *NAME"), so strip until stable.
    changed = True
    while changed:
        changed = False
        for prefix in PROCESSOR_PREFIXES:
            if name.startswith(prefix):
                name = name[len(prefix):].strip()
                changed = True

    name = TRAILING_NOISE.sub("", name)

    # "WALMART 1234" leaves "WALMART " behind; trim again.
    name = name.strip(" -#*")

    # Keep letters and digits, plus the separators that join words. `isalnum`
    # is Unicode-aware, so a merchant with a non-ASCII name survives.
    name = "".join(ch if (ch.isalnum() or ch in SEPARATORS) else " " for ch in name)
    name = " ".join(name.split()).strip()

    # A descriptor with no letters carries no merchant name: it is a phone
    # number or a reference code, and keeping it would match every other one
    # to the same merchant.
    if not any(ch.isalpha() for ch in name):
        return ""
    return name
