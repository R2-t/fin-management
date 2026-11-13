"""Tests for the sidecar's HTTP surface.

The endpoints are driven in-process, so the tests do not need a server or a
collector.
"""

from fastapi.testclient import TestClient

from app.main import create_app


def client() -> TestClient:
    return TestClient(create_app())


def test_health_reports_ok():
    response = client().get("/health")
    assert response.status_code == 200
    assert response.json()["status"] == "ok"


USER = "11111111-1111-1111-1111-111111111111"


def test_normalize_cleans_a_descriptor():
    response = client().post(
        "/normalize-merchant", json={"raw": "SQ *Netflix", "user_id": USER}
    )
    assert response.status_code == 200
    body = response.json()
    assert body["raw"] == "SQ *Netflix"
    assert body["normalized"] == "NETFLIX"
    assert body["changed"] is True
    assert body["model_version"] == "rules-v1"


def test_normalize_says_so_when_it_changed_nothing():
    response = client().post(
        "/normalize-merchant", json={"raw": "NETFLIX", "user_id": USER}
    )
    assert response.status_code == 200
    body = response.json()
    assert body["normalized"] == "NETFLIX"
    assert body["changed"] is False


def test_normalize_of_an_empty_descriptor_is_empty():
    response = client().post(
        "/normalize-merchant", json={"raw": "", "user_id": USER}
    )
    assert response.status_code == 200
    assert response.json()["normalized"] == ""


def test_no_user_id_is_rejected():
    # A caller that forgets the user must fail loudly rather than silently
    # get no merchant match.
    response = client().post("/normalize-merchant", json={"raw": "NETFLIX"})
    assert response.status_code == 422


def test_without_a_database_there_is_no_match():
    # ANALYTICS_DB_URL is unset in the test environment, so matching is
    # skipped and the response says so by leaving the merchant null.
    response = client().post(
        "/normalize-merchant", json={"raw": "NETFLIX", "user_id": USER}
    )
    body = response.json()
    assert body["merchant_id"] is None
    assert body["match_score"] is None
