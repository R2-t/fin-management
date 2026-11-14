"""Verifying the credential a Rust-to-sidecar call carries (D68, D72, D76).

The point is not "the sidecar checks a token". The point is that **the token
decides the tenant**: the claim names one user, and a request that asks about a
different one is refused. Without that, a compromised caller holding a valid
token for tenant A could read tenant B, and a compromised sidecar could set
`app.user_id` to anyone it liked.

Service tokens use their own key id and secret (D76). The sidecar is given only
that key: the human bearer key would let it mint a principal for any user, which
is the exact thing this token exists to prevent.
"""

from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass

import pyseto
from pyseto import Key


class ServiceTokenError(Exception):
    """The token is missing, malformed, expired, or names another tenant."""


@dataclass(frozen=True)
class ServiceIdentity:
    """What a verified token says, and nothing more."""

    user_id: str
    scope: str


def _service_key_id() -> str:
    key_id = os.environ.get("PASETO_SERVICE_KID")
    if not key_id:
        raise ServiceTokenError(
            "PASETO_SERVICE_KID is not set, so no service token can be verified"
        )
    return key_id.lower()


def _service_key() -> Key:
    """The sidecar's one key: the service key, never the human bearer key."""
    import base64

    key_id = _service_key_id()
    raw = os.environ.get(f"PASETO_KEY_{key_id.upper()}")
    if not raw:
        raise ServiceTokenError(
            f"PASETO_KEY_{key_id.upper()} is not set, so no service token can be verified"
        )
    try:
        key_bytes = base64.b64decode(raw, validate=True)
    except Exception as exc:  # noqa: BLE001 - every decode failure is the same answer
        raise ServiceTokenError("the service key is not valid base64") from exc
    if len(key_bytes) != 32:
        raise ServiceTokenError("a v4.local key is 32 bytes")
    return Key.new(4, "local", key_bytes)


def verify(token: str | None) -> ServiceIdentity:
    """Verifies the token and returns the tenant it names.

    Every failure is `ServiceTokenError`: the caller turns that into a 401 and
    never learns which check failed, because telling it apart helps an attacker
    and nobody else.
    """
    if not token:
        raise ServiceTokenError("no service token")

    key = _service_key()
    try:
        decoded = pyseto.decode(key, token)
    except Exception as exc:  # noqa: BLE001 - signature, format, and expiry all land here
        raise ServiceTokenError("the service token did not verify") from exc

    try:
        payload = json.loads(decoded.payload)
    except Exception as exc:  # noqa: BLE001
        raise ServiceTokenError("the service token payload is not JSON") from exc

    # The footer names the key id. pyseto has already rejected a token signed
    # with a key this sidecar does not hold, but the check is stated so a future
    # key set cannot quietly widen what is accepted.
    footer = {}
    if decoded.footer:
        try:
            footer = json.loads(decoded.footer)
        except Exception as exc:  # noqa: BLE001
            raise ServiceTokenError("the token footer is not JSON") from exc
    if not isinstance(footer, dict) or footer.get("kid", "").lower() != _service_key_id():
        raise ServiceTokenError("the token was signed with another key")

    # pyseto validates `exp` when the rules are enabled; asserting the fields we
    # depend on keeps a token minted by a different issuer from passing quietly.
    exp = payload.get("exp")
    if exp is None:
        raise ServiceTokenError("the service token has no expiry")
    if int(exp) < int(time.time()):
        raise ServiceTokenError("the service token has expired")

    if payload.get("iss") != os.environ.get("PASETO_ISSUER", "financial-management"):
        raise ServiceTokenError("the service token came from another issuer")
    if payload.get("aud") != os.environ.get("PASETO_AUDIENCE", "financial-management"):
        raise ServiceTokenError("the service token is for another audience")

    user_id = payload.get("user_id")
    if not user_id:
        raise ServiceTokenError("the service token names no user")

    return ServiceIdentity(user_id=user_id, scope=payload.get("scope", ""))


def verify_for_tenant(token: str | None, requested_user_id: str) -> ServiceIdentity:
    """Verifies the token, and refuses a request about anyone else.

    This is the whole point of D68: the claim decides the tenant, so a caller
    holding a valid token for tenant A cannot ask about tenant B.
    """
    identity = verify(token)
    if identity.user_id != requested_user_id:
        raise ServiceTokenError(
            "the service token names a different tenant than the request does"
        )
    if identity.scope != "analytics":
        raise ServiceTokenError("the service token does not carry the analytics scope")
    return identity


def bearer_from_header(header: str | None) -> str | None:
    """Extracts the token from an `Authorization: Bearer ...` header."""
    if not header:
        return None
    parts = header.split(None, 1)
    if len(parts) != 2 or parts[0].lower() != "bearer":
        return None
    return parts[1].strip() or None
