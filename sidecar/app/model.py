"""A TF-IDF centroid classifier over the user's own categories (3.5).

Why not scikit-learn: a user has hundreds of labelled purchases, not millions.
A multinomial centroid classifier over character n-grams is a linear model, it is
twenty lines of numpy, and it does not add a 100 MB dependency and a pickle
format to a service that has to load the model from a database row. If the data
ever justifies a real model, D48's promotion gate is what stops the weaker one
from being replaced by guesswork -- not the choice of library.

The rules of the house that this must respect:

  - The merchant's own history is the cold start and stays first (D64). This is
    the fallback for a merchant with no usable history, or a description that
    says more than the merchant does.
  - A suggestion below D47's threshold is not a suggestion: the caller asks the
    user instead.
  - The model never decides anything by itself: it returns a category and a
    confidence, and the application writes the purchase (D66).
"""

from __future__ import annotations

import math
import unicodedata

import numpy as np

# Character n-grams, because merchant descriptors are noisy ("SQ *NETFLIX",
# "AMZN Mktp US*2H4L9") and whole-word tokens miss almost all of it.
NGRAM_SIZE = 3
MIN_TOKEN_LENGTH = 2

# Below this many labelled purchases a model cannot be trained honestly: a
# category with one example is not a class, it is an anecdote.
MIN_TRAINING_LABELS = 20
MIN_PER_CATEGORY = 2


class NotEnoughData(Exception):
    """There is not enough labelled history to train an honest model."""


def _normalise(text: str) -> str:
    decomposed = unicodedata.normalize("NFKD", text or "")
    stripped = "".join(c for c in decomposed if not unicodedata.combining(c))
    return " ".join(stripped.upper().split())


def ngrams(text: str) -> list[str]:
    """Character trigrams of the normalised text, plus the whole words."""
    normalised = _normalise(text)
    grams: list[str] = []
    for word in normalised.split():
        if len(word) >= MIN_TOKEN_LENGTH:
            grams.append(word)
        padded = f"  {word} "
        for i in range(len(padded) - NGRAM_SIZE + 1):
            grams.append(padded[i : i + NGRAM_SIZE])
    return grams


def document(store: str | None, description: str | None, merchant: str | None) -> str:
    """The text a purchase is classified by."""
    return " ".join(part for part in (store, description, merchant) if part)


def _tfidf(docs: list[list[str]], vocabulary: dict[str, int]) -> np.ndarray:
    """A document-term matrix, tf-idf weighted, L2 normalised."""
    matrix = np.zeros((len(docs), len(vocabulary)))
    for row, grams in enumerate(docs):
        for gram in grams:
            column = vocabulary.get(gram)
            if column is not None:
                matrix[row, column] += 1.0
    return _weight(matrix)


def _weight(matrix: np.ndarray) -> np.ndarray:
    """tf * idf, then L2-normalised rows.

    idf = log(N / df): a gram every purchase contains carries no information, so
    it is worth almost nothing.
    """
    if matrix.size == 0:
        return matrix
    documents_with_term = (matrix > 0).sum(axis=0)
    n_documents = matrix.shape[0]
    idf = np.log((1.0 + n_documents) / (1.0 + documents_with_term)) + 1.0
    weighted = matrix * idf
    norms = np.linalg.norm(weighted, axis=1, keepdims=True)
    norms[norms == 0] = 1.0
    return weighted / norms


def train(labelled: list[tuple[str, int]]) -> dict:
    """Trains a centroid classifier. `labelled` is (text, category_id) pairs."""
    if len(labelled) < MIN_TRAINING_LABELS:
        raise NotEnoughData(
            f"{len(labelled)} labelled purchases; {MIN_TRAINING_LABELS} are needed"
        )

    counts: dict[int, int] = {}
    for _, category in labelled:
        counts[category] = counts.get(category, 0) + 1
    usable = {c for c, n in counts.items() if n >= MIN_PER_CATEGORY}
    if len(usable) < 2:
        raise NotEnoughData(
            "fewer than two categories have enough examples to tell apart"
        )

    samples = [(text, category) for text, category in labelled if category in usable]
    docs = [ngrams(text) for text, _ in samples]

    vocabulary: dict[str, int] = {}
    for grams in docs:
        for gram in grams:
            if gram not in vocabulary:
                vocabulary[gram] = len(vocabulary)

    matrix = _tfidf(docs, vocabulary)

    # One centroid per category: the mean of its documents, re-normalised. This
    # is the linear model -- a document is assigned to the nearest centroid,
    # which is a dot product against a weight vector.
    categories = sorted(usable)
    centroids = np.zeros((len(categories), len(vocabulary)))
    for index, category in enumerate(categories):
        rows = [i for i, (_, c) in enumerate(samples) if c == category]
        centroid = matrix[rows].mean(axis=0)
        norm = np.linalg.norm(centroid)
        centroids[index] = centroid / norm if norm else centroid

    return {
        "version": "centroid-ngram-v1",
        "ngram_size": NGRAM_SIZE,
        "vocabulary": vocabulary,
        "categories": categories,
        "centroids": centroids.tolist(),
        "training_size": len(samples),
    }


def predict(artifact: dict, text: str) -> tuple[int | None, float]:
    """The nearest centroid, and how clearly it won.

    The confidence is the gap between the best and the runner-up cosine
    similarity: a purchase that sits between two categories is exactly the case
    that should be handed to the user rather than guessed.
    """
    vocabulary = artifact["vocabulary"]
    centroids = np.array(artifact["centroids"], dtype=float)
    categories = artifact["categories"]

    grams = ngrams(text)
    if not grams or not vocabulary:
        return None, 0.0

    row = np.zeros((1, len(vocabulary)))
    for gram in grams:
        column = vocabulary.get(gram)
        if column is not None:
            row[0, column] += 1.0
    vector = _weight(row)[0]

    similarities = centroids @ vector
    order = np.argsort(similarities)[::-1]
    best = similarities[order[0]]
    runner_up = similarities[order[1]] if len(order) > 1 else 0.0

    if best <= 0.0:
        return None, 0.0

    # Cosine similarities are in [0, 1]; the gap is scaled by the best score so a
    # weak winner cannot look sure.
    confidence = float(max(0.0, best) * max(0.0, best - max(0.0, runner_up)))
    return int(categories[order[0]]), confidence


def evaluate(artifact: dict, held_out: list[tuple[str, int]]) -> dict:
    """Accuracy and coverage on a held-out set."""
    if not held_out:
        return {"accuracy": 0.0, "coverage": 0.0, "n": 0}

    correct = 0
    answered = 0
    for text, expected in held_out:
        category, _confidence = predict(artifact, text)
        if category is None:
            continue
        answered += 1
        if category == expected:
            correct += 1

    return {
        "accuracy": correct / len(held_out),
        "coverage": answered / len(held_out),
        "n": len(held_out),
    }
