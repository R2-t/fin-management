"""The TF-IDF centroid model (3.5) and the promotion gate (3.7/D48)."""

import pytest

from app import model


def labelled():
    return (
        [("BLUE BOTTLE COFFEE", "coffee")] * 12
        + [("SHELL GAS STATION", "fuel")] * 12
        + [("NETFLIX.COM", "streaming")] * 12
    )


def test_a_familiar_name_is_recognised():
    artifact = model.train(labelled())
    category, confidence = model.predict(artifact, "BLUE BOTTLE COFFEE")
    assert category == "coffee"
    assert confidence > 0.75, confidence


def test_a_name_it_has_never_seen_is_answered_with_the_nearest_category():
    artifact = model.train(labelled())
    category, _ = model.predict(artifact, "SHELL 4482 GAS STATION")
    assert category == "fuel"


def test_nothing_is_guessed_from_an_empty_description():
    artifact = model.train(labelled())
    category, confidence = model.predict(artifact, "")
    assert category is None
    assert confidence == 0.0


def test_too_little_history_refuses_to_train():
    """A category with one example is an anecdote, not a class."""
    with pytest.raises(model.NotEnoughData):
        model.train([("SOMETHING", "a"), ("OTHER", "b")])


def test_a_single_category_refuses_to_train():
    with pytest.raises(model.NotEnoughData):
        model.train([("SAME SHOP", "a")] * 40)


def test_the_split_is_temporal_and_measured_on_held_out_data():
    artifact = model.train(labelled())
    metrics = model.evaluate(artifact, [("NETFLIX.COM", "streaming"), ("SHELL GAS", "fuel")])
    assert metrics["n"] == 2
    assert metrics["accuracy"] == 1.0


def test_promotion_requires_no_metric_to_get_worse():
    """D48: better on average is how a model that got worse at the category you
    care about gets shipped."""
    incumbent = {"accuracy": 0.80, "coverage": 0.90}

    assert model_beats({"accuracy": 0.80, "coverage": 0.90}, incumbent) is False
    assert model_beats({"accuracy": 0.85, "coverage": 0.95}, incumbent) is True
    assert model_beats({"accuracy": 0.85, "coverage": 0.80}, incumbent) is False
    assert model_beats({"accuracy": 0.70, "coverage": 0.99}, incumbent) is False
    # The first model is promoted once it has been measured.
    assert model_beats({"accuracy": 0.10, "coverage": 0.10}, None) is True


def model_beats(candidate, incumbent):
    from app.train import beats

    return beats(candidate, incumbent)
