"""Minting a service token the way the Rust side does, for tests.

Production mints in Rust only (D76): the sidecar holds the key to *verify*, and
handing it the ability to mint would defeat the separation. A test needs to mint
because it stands in for the caller.
"""

import json
import time

import pyseto

SERVICE_KID = "service"
SERVICE_KEY = bytes(range(32))


def mint_service_token(user_id: str, *, ttl: int = 60, scope: str = "analytics") -> str:
    """A token shaped exactly as `ServiceTokenMinter` writes one."""
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
    return pyseto.encode(key, payload, footer=json.dumps({"kid": SERVICE_KID})).decode()
