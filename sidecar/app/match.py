"""Matching a cleaned merchant name against the aliases already recorded.

The threshold is 85 (D46). Below it, two merchants that happen to share a
brand get linked -- `TARGET` and `TARGET PHARMACY` become one merchant, and
the user's medicine budget disappears into groceries. Above it, the same
company shows up as two merchants, which is visible and fixable by hand.

The trade is asymmetric on purpose: an unlinked merchant is an annoyance the
user can fix, while a wrong link silently corrupts a budget.
"""

from dataclasses import dataclass
from typing import Sequence

from rapidfuzz import fuzz

# D46. See the module docstring for why it is not higher.
ALIAS_THRESHOLD = 85.0


@dataclass(frozen=True)
class AliasMatch:
    merchant_id: str
    alias: str
    score: float


def match_alias(
    cleaned: str,
    aliases: Sequence[tuple[str, str]],
    threshold: float = ALIAS_THRESHOLD,
) -> AliasMatch | None:
    """Finds the merchant an alias belongs to, if any is close enough.

    `aliases` is a sequence of `(alias, merchant_id)` pairs, so the caller
    owns the lookup and this function stays free of a database.

    Returns `None` rather than the best match below the threshold: the caller
    must be able to tell "no match" from "matched", not guess from a score.
    """
    if not cleaned or not aliases:
        return None

    best: AliasMatch | None = None
    for alias, merchant_id in aliases:
        score = float(fuzz.ratio(cleaned.upper(), alias.upper()))
        if best is None or score > best.score:
            best = AliasMatch(merchant_id=merchant_id, alias=alias, score=score)

    if best is not None and best.score >= threshold:
        return best
    return None
