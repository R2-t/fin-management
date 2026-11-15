"""Shared sidecar test fixtures.

Every sidecar test runs with a service key in the environment, because the HTTP
routes now refuse an unauthenticated call. Tests mint a real token rather than
disabling the check: a test that turns the check off proves nothing about it.
"""

import base64

import pytest

from token_support import SERVICE_KEY, SERVICE_KID, mint_service_token


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


@pytest.fixture
def authorization_header():
    """The header a caller sends for a tenant: `authorization_header(user_id)`."""

    def header(user_id: str, **kwargs) -> dict[str, str]:
        return {"Authorization": f"Bearer {mint_service_token(user_id, **kwargs)}"}

    return header
