"""The service token, from the sidecar's side (D68, D72, D76).

The property under test is not "a token is checked". It is that the claim
decides the tenant: a caller with a valid token for one user cannot ask about
another, and the sidecar never holds the key that signs human bearer tokens.
"""

import base64
import json
import os
import time

import pyseto
import pytest

from app.service_auth import (
    ServiceTokenError,
    bearer_from_header,
    verify,
    verify_for_tenant,
)

SERVICE_KID = "service"
HUMAN_KID = "human"

SERVICE_KEY = bytes(range(32))
HUMAN_KEY = bytes(range(32, 64))


@pytest.fixture(autouse=True)
def keys(monkeypatch):
    """The sidecar's environment holds the service key and nothing else."""
    monkeypatch.setenv("PASETO_SERVICE_KID", SERVICE_KID)
    monkeypatch.setenv(
        f"PASETO_KEY_{SERVICE_KID.upper()}",
        base64.b64encode(SERVICE_KEY).decode(),
    )
    monkeypatch.setenv(
        f"PASETO_KEY_{HUMAN_KID.upper()}",
        # Deliberately present in the environment, so the test proves the *kid*
        # is checked rather than the key being simply absent.
        base64.b64encode(HUMAN_KEY).decode(),
    )
    monkeypatch.delenv("PASETO_ISSUER", raising=False)
    monkeypatch.delenv("PASETO_AUDIENCE", raising=False)


def mint(
    user_id="11111111-1111-1111-1111-111111111111",
    *,
    kid=SERVICE_KID,
    key_bytes=SERVICE_KEY,
    ttl=60,
    scope="analytics",
    iss="financial-management",
    aud="financial-management",
):
    payload = {
        "iss": iss,
        "aud": aud,
        "sub": "service:api",
        "exp": int(time.time()) + ttl,
        "iat": int(time.time()),
        "nbf": int(time.time()),
        "user_id": user_id,
        "scope": scope,
    }
    key = pyseto.Key.new(4, "local", key_bytes)
    return pyseto.encode(key, payload, footer=json.dumps({"kid": kid})).decode()


def test_a_minted_service_token_names_its_tenant():
    identity = verify(mint("22222222-2222-2222-2222-222222222222"))
    assert identity.user_id == "22222222-2222-2222-2222-222222222222"
    assert identity.scope == "analytics"


def test_a_request_about_another_tenant_is_refused():
    token = mint("11111111-1111-1111-1111-111111111111")
    assert verify_for_tenant(token, "11111111-1111-1111-1111-111111111111")

    with pytest.raises(ServiceTokenError, match="different tenant"):
        verify_for_tenant(token, "99999999-9999-9999-9999-999999999999")


def test_a_token_signed_with_the_human_key_is_refused():
    """D76: sharing the bearer key would let the sidecar mint any principal."""
    human_signed = mint(kid=HUMAN_KID, key_bytes=HUMAN_KEY)
    # The signature check refuses it before the kid check is reached, which is
    # the stronger of the two answers: the sidecar simply cannot open it.
    with pytest.raises(ServiceTokenError, match="did not verify|another key"):
        verify(human_signed)


def test_an_expired_token_is_refused():
    with pytest.raises(ServiceTokenError, match="did not verify|expired"):
        verify(mint(ttl=-60))


def test_a_token_for_another_audience_or_issuer_is_refused():
    with pytest.raises(ServiceTokenError, match="another audience"):
        verify(mint(aud="something-else"))
    with pytest.raises(ServiceTokenError, match="another issuer"):
        verify(mint(iss="somebody-else"))


def test_a_token_without_the_analytics_scope_is_refused():
    token = mint(scope="something")
    with pytest.raises(ServiceTokenError, match="analytics scope"):
        verify_for_tenant(token, "11111111-1111-1111-1111-111111111111")


def test_no_token_is_refused():
    with pytest.raises(ServiceTokenError, match="no service token"):
        verify(None)


def test_a_missing_service_key_is_refused_rather_than_ignored(monkeypatch):
    monkeypatch.delenv(f"PASETO_KEY_{SERVICE_KID.upper()}")
    with pytest.raises(ServiceTokenError, match="no service token can be verified"):
        verify(mint())


def test_the_bearer_header_is_parsed():
    assert bearer_from_header("Bearer abc") == "abc"
    assert bearer_from_header("bearer abc") == "abc"
    assert bearer_from_header("Basic abc") is None
    assert bearer_from_header(None) is None
    assert bearer_from_header("Bearer ") is None


# --- The shape Rust actually writes -----------------------------------------
#
# Every test above mints epoch integers, which is what a hand-written token
# looks like. The Rust minter writes RFC3339 strings (pasetors stores the string
# it is given), and `int("2026-11-15T01:10:34+00:00")` raises -- so the whole
# feature was dead end to end while these tests were green. These tests pin the
# real shape.

import datetime as dt  # noqa: E402


def rust_shaped_token(user_id, *, ttl_seconds=60, nbf_offset=0, aud="financial-management"):
    now = dt.datetime.now(dt.timezone.utc)
    payload = {
        "iss": "financial-management",
        "aud": aud,
        "sub": "service:api",
        "exp": (now + dt.timedelta(seconds=ttl_seconds)).isoformat(),
        "iat": now.isoformat(),
        "nbf": (now + dt.timedelta(seconds=nbf_offset)).isoformat(),
        "user_id": user_id,
        "scope": "analytics",
    }
    key = pyseto.Key.new(4, "local", SERVICE_KEY)
    return pyseto.encode(key, payload, footer=json.dumps({"kid": SERVICE_KID})).decode()


def test_a_token_with_rfc3339_claims_verifies():
    """The shape the Rust minter writes must verify, or nothing works at all."""
    token = rust_shaped_token("44444444-4444-4444-4444-444444444444")
    identity = verify_for_tenant(token, "44444444-4444-4444-4444-444444444444")
    assert identity.user_id == "44444444-4444-4444-4444-444444444444"


def test_an_rfc3339_token_that_expired_is_refused_not_crashed():
    token = rust_shaped_token("44444444-4444-4444-4444-444444444444", ttl_seconds=-60)
    with pytest.raises(ServiceTokenError, match="expired"):
        verify(token)


def test_a_token_not_yet_valid_is_refused():
    token = rust_shaped_token("44444444-4444-4444-4444-444444444444", nbf_offset=3600)
    with pytest.raises(ServiceTokenError, match="not valid yet"):
        verify(token)


def test_an_unreadable_expiry_is_a_token_error_not_a_crash():
    """A 500 here would break every real call and leak which check failed."""
    from app.service_auth import _timestamp

    with pytest.raises(ServiceTokenError, match="unreadable"):
        _timestamp("not a time", "exp")
    with pytest.raises(ServiceTokenError, match="no readable"):
        _timestamp(None, "exp")
