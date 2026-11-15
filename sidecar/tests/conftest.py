"""Shared sidecar test fixtures.

Every sidecar test runs with a service key in the environment, because the HTTP
routes now refuse an unauthenticated call. Tests mint a real token rather than
disabling the check: a test that turns the check off proves nothing about it.
"""

import base64
import json
import os
import time

import pyseto
import pytest

SERVICE_KID = "service"
SERVICE_KEY = bytes(range(32))


@pytest.fixture(autouse=True)
def service_key(monkeypatch):
    """The service key the sidecar verifies with (D76: only this one)."""
    monkeypatch.setenv("PASETO_SERVICE_KID", SERVICE_KID)
    monkeypatch.setenv(
        f"PASETO_KEY_{SERVICE_KID.upper()}",
        base64.b64encode(SERVICE_KEY).decode(),
    )
    monkeypatch.delenv("PASETO_ISSUER", raising=False)
    monkeypatch.delenv("PASETO_AUDIENCE", raising=False)


def mint_service_token(user_id: str, *, ttl: int = 60, scope: str = "analytics") -> str:
    """A token shaped exactly as the Rust minter writes one."""
    payload = {
        "iss": "financial-management",
        "aud": "financial-management",
        "sub": "service:api",
        "exp": int(time.time()) + ttl,
        "iat": int(time.time()),
        "nbf": int(time.time()),
        "user_id": user_id,
        "scope": scope,
    }
    key = pyseto.Key.new(4, "local", SERVICE_KEY)
    return pyseto.encode(
        key, payload, footer=json.dumps({"kid": SERVICE_KID})
    ).decode()


@pytest.fixture
def authorization_header():
    """The header a caller sends for a tenant: `authorization_header(user_id)`."""

    def header(user_id: str, **kwargs) -> dict[str, str]:
        return {"Authorization": f"Bearer {mint_service_token(user_id, **kwargs)}"}

    return header
