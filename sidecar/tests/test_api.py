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


def test_normalize_cleans_a_descriptor(authorization_header):
    response = client().post(
        "/normalize-merchant", json={"raw": "SQ *Netflix", "user_id": USER}, headers=authorization_header(USER)
    )
    assert response.status_code == 200
    body = response.json()
    assert body["raw"] == "SQ *Netflix"
    assert body["normalized"] == "NETFLIX"
    assert body["changed"] is True
    assert body["model_version"] == "rules-v1"


def test_normalize_says_so_when_it_changed_nothing(authorization_header):
    response = client().post(
        "/normalize-merchant", json={"raw": "NETFLIX", "user_id": USER}, headers=authorization_header(USER)
    )
    assert response.status_code == 200
    body = response.json()
    assert body["normalized"] == "NETFLIX"
    assert body["changed"] is False


def test_normalize_of_an_empty_descriptor_is_empty(authorization_header):
    response = client().post(
        "/normalize-merchant", json={"raw": "", "user_id": USER}, headers=authorization_header(USER)
    )
    assert response.status_code == 200
    assert response.json()["normalized"] == ""


def test_no_user_id_is_rejected(authorization_header):
    # A caller that forgets the user must fail loudly rather than silently
    # get no merchant match.
    response = client().post("/normalize-merchant", json={"raw": "NETFLIX"}, headers=authorization_header(USER))
    assert response.status_code == 422


def test_a_malformed_user_id_is_rejected(authorization_header):
    # Parsed as a UUID before it reaches the query, so a bad id is a 422 and not
    # a failed database call.
    response = client().post(
        "/normalize-merchant", json={"raw": "NETFLIX", "user_id": "not-a-uuid"}, headers=authorization_header(USER)
    )
    assert response.status_code == 422


def test_without_a_database_there_is_no_match(authorization_header):
    # ANALYTICS_DB_URL is unset in the test environment, so matching is
    # skipped and the response says so by leaving the merchant null.
    response = client().post(
        "/normalize-merchant", json={"raw": "NETFLIX", "user_id": USER}, headers=authorization_header(USER)
    )
    body = response.json()
    assert body["merchant_id"] is None
    assert body["match_score"] is None


def test_a_call_without_a_token_is_refused():
    response = client().post("/normalize-merchant", json={"raw": "NETFLIX", "user_id": USER})
    assert response.status_code == 401


def test_a_call_for_another_tenant_is_refused(authorization_header):
    """The whole point of the service token: the claim decides the tenant."""
    someone_else = "22222222-2222-2222-2222-222222222222"
    response = client().post(
        "/normalize-merchant",
        json={"raw": "NETFLIX", "user_id": USER},
        headers=authorization_header(someone_else),
    )
    assert response.status_code == 401


def test_runway_bands_names_its_tenant_and_is_refused_without_one(authorization_header):
    body = {"user_id": USER, "daily_history": [100] * 14, "horizon": 30}

    assert client().post("/runway-bands", json=body).status_code == 401

    for_another = client().post(
        "/runway-bands",
        json=body,
        headers=authorization_header("33333333-3333-3333-3333-333333333333"),
    )
    assert for_another.status_code == 401

    accepted = client().post("/runway-bands", json=body, headers=authorization_header(USER))
    assert accepted.status_code == 200, accepted.text
    assert accepted.json()["bands"], "the bands are still computed for the right tenant"


def test_classify_is_refused_without_a_token():
    response = client().post(
        "/classify", json={"user_id": USER, "purchase_id": str(__import__("uuid").uuid4())}
    )
    assert response.status_code == 401


def test_classify_is_refused_for_another_tenant(authorization_header):
    response = client().post(
        "/classify",
        json={"user_id": USER, "purchase_id": str(__import__("uuid").uuid4())},
        headers=authorization_header("55555555-5555-5555-5555-555555555555"),
    )
    assert response.status_code == 401
