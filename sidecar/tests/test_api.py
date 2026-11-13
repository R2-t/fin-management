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


def test_normalize_cleans_a_descriptor():
    response = client().post("/normalize-merchant", json={"raw": "SQ *Netflix"})
    assert response.status_code == 200
    body = response.json()
    assert body["raw"] == "SQ *Netflix"
    assert body["normalized"] == "NETFLIX"
    assert body["changed"] is True
    assert body["model_version"] == "rules-v1"


def test_normalize_says_so_when_it_changed_nothing():
    response = client().post("/normalize-merchant", json={"raw": "NETFLIX"})
    assert response.status_code == 200
    body = response.json()
    assert body["normalized"] == "NETFLIX"
    assert body["changed"] is False


def test_normalize_of_an_empty_descriptor_is_empty():
    response = client().post("/normalize-merchant", json={"raw": ""})
    assert response.status_code == 200
    assert response.json()["normalized"] == ""
