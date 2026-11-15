"""The sidecar's HTTP surface.

Two endpoints: liveness, and merchant name cleaning. The rest of Phase 3
(job workers, categorization) attaches here as it lands.
"""

import json
import logging
import os
import sys
import uuid

from fastapi import FastAPI, Header
from pydantic import BaseModel

from app.db import aliases_for_user
from app.match import match_alias
from app.normalize import RULES_VERSION, normalize_merchant

SERVICE_NAME = os.environ.get("OTEL_SERVICE_NAME", "finmgmt-analytics")


def configure_telemetry() -> None:
    """Exports traces to the collector when one is configured.

    Without OTLP_ENDPOINT the service runs uninstrumented rather than
    refusing to start: a sidecar that cannot reach its collector should still
    serve.
    """
    endpoint = os.environ.get("OTLP_ENDPOINT")
    if not endpoint:
        return

    from opentelemetry import trace
    from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter
    from opentelemetry.instrumentation.fastapi import FastAPIInstrumentor
    from opentelemetry.sdk.resources import Resource
    from opentelemetry.sdk.trace import TracerProvider
    from opentelemetry.sdk.trace.export import BatchSpanProcessor

    provider = TracerProvider(
        resource=Resource.create({"service.name": SERVICE_NAME}),
    )
    provider.add_span_processor(
        BatchSpanProcessor(OTLPSpanExporter(endpoint=endpoint))
    )
    trace.set_tracer_provider(provider)
    # Recorded here so a misconfigured collector is visible in the logs
    # rather than as silent trace loss.
    logging.getLogger(__name__).info("otlp traces exporting to %s", endpoint)


class NormalizeRequest(BaseModel):
    raw: str
    # Who is asking, so the alias lookup is scoped to their merchants. The
    # Rust service authenticates; this one trusts it. Parsed as a UUID here so
    # a malformed id is a 422 rather than a failed query.
    user_id: uuid.UUID


class NormalizeResponse(BaseModel):
    raw: str
    normalized: str
    changed: bool
    # The rule set or model that produced the answer, so a later rule change
    # can be told apart from a model change.
    model_version: str
    # The merchant the name was linked to, when one was close enough (D46).
    # Null means "no merchant matched", not "not checked": a database that is
    # configured but unreachable is a 500, not a silent miss.
    merchant_id: str | None = None
    matched_alias: str | None = None
    match_score: float | None = None


def authorize(authorization: str | None, requested_user_id: str):
    """Refuses the request unless the token names the tenant it asks about.

    Returns the identity, so the handler uses the tenant the *claim* named
    rather than the one the caller's body claimed -- equal by construction after
    the check, but read from one source.

    A 401 with no detail about which check failed: telling a caller whether the
    signature, the expiry, or the tenant was wrong helps an attacker and nobody
    else.
    """
    from fastapi import HTTPException

    from app.service_auth import (
        ServiceTokenError,
        bearer_from_header,
        verify_for_tenant,
    )

    try:
        return verify_for_tenant(bearer_from_header(authorization), requested_user_id)
    except ServiceTokenError as exc:
        logging.warning("rejected a sidecar call: %s", exc)
        raise HTTPException(status_code=401, detail="Unauthorized") from exc


def create_app() -> FastAPI:
    # Match the Rust service's LOG_FORMAT=json so both land in the collector
    # the same way.
    if os.environ.get("LOG_FORMAT") == "json":
        logging.basicConfig(
            stream=sys.stdout,
            format=json.dumps({"level": "%(levelname)s", "message": "%(message)s"}),
        )
    else:
        logging.basicConfig(stream=sys.stdout)

    configure_telemetry()

    app = FastAPI(title="finmgmt-analytics", version="0.1.0")

    @app.get("/health")
    def health() -> dict:
        return {"status": "ok", "service": SERVICE_NAME}

    @app.post("/normalize-merchant", response_model=NormalizeResponse)
    def normalize(
        request: NormalizeRequest,
        authorization: str | None = Header(default=None),
    ) -> NormalizeResponse:
        identity = authorize(authorization, str(request.user_id))
        cleaned = normalize_merchant(request.raw)

        # Alias matching is skipped when no database is configured, so the
        # service runs -- and its tests run -- without one.
        aliases = aliases_for_user(identity.user_id)
        match = match_alias(cleaned, aliases) if aliases else None

        return NormalizeResponse(
            raw=request.raw,
            normalized=cleaned,
            changed=cleaned != request.raw,
            model_version=RULES_VERSION,
            merchant_id=match.merchant_id if match else None,
            matched_alias=match.alias if match else None,
            match_score=match.score if match else None,
        )

    @app.post("/classify", response_model=ClassifyResponse)
    def classify(
        request: ClassifyRequest,
        authorization: str | None = Header(default=None),
    ) -> ClassifyResponse:
        identity = authorize(authorization, request.user_id)

        from app.classify import suggest_for_purchase

        # Read-only: the application owns the write (D66).
        suggestion = suggest_for_purchase(request.purchase_id, identity.user_id)
        if suggestion is None:
            return ClassifyResponse()
        return ClassifyResponse(
            category_id=suggestion["category_id"],
            confidence=suggestion["confidence"],
        )

    @app.post("/runway-bands", response_model=RunwayBandsResponse)
    def runway_bands_route(
        request: RunwayBandsRequest,
        authorization: str | None = Header(default=None),
    ) -> RunwayBandsResponse:
        return runway_bands(request, authorization)

    return app


def create_worker() -> None:
    """Entry point for the job worker process.

    Separate from the HTTP app so a slow job never blocks a request and a
    request never waits on a job.
    """
    from app.jobs import run_forever

    configure_telemetry()
    # The worker's INFO lines -- claimed job, suggested category -- are its
    # observable behaviour, so they must reach a handler even without
    # LOG_FORMAT set.
    logging.basicConfig(
        stream=sys.stdout,
        level=logging.INFO,
    )
    run_forever()


class ClassifyRequest(BaseModel):
    # The tenant, compared against the claim (the claim decides it).
    user_id: str
    purchase_id: str


class ClassifyResponse(BaseModel):
    """The suggestion, or an honest absence of one.

    `category_id` null means "nothing honest to suggest" -- the cold start has
    no history for this merchant, or the history disagrees with itself. It does
    not mean "failed to look": a database that could not be read is a 500.
    """

    category_id: int | None = None
    confidence: float = 0.0


class RunwayBandsRequest(BaseModel):
    # The tenant this answer is about. Checked against the service token's
    # claim, so holding a token for one user cannot fetch another's bands.
    user_id: str
    # The discretionary spend per day, oldest first, with zero for days that
    # had none. The Rust service derives this from the projection -- the same
    # source its deterministic walk reads.
    daily_history: list[int]
    # The horizon the bands cover, matching the deterministic runway's.
    horizon: int = 90
    # Declared commitments by day offset, and scheduled income by day offset.
    # They apply to every sample rather than being averaged over.
    recurring_by_day: dict[int, int] = {}
    income_by_day: dict[int, int] = {}
    # The balance the bands start from: the ledger's sum.
    starting_balance: int = 0


class RunwayBandsResponse(BaseModel):
    model_version: str
    bands: list[dict]
    resamples: int


def runway_bands(
    request: RunwayBandsRequest,
    authorization: str | None = None,
) -> RunwayBandsResponse:
    authorize(authorization, request.user_id)

    from app.bootstrap import PERCENTILES, RESAMPLES, BANDS_VERSION, percentile_bands

    bands = percentile_bands(
        request.daily_history,
        horizon=request.horizon,
        recurring_by_day=request.recurring_by_day,
        income_by_day=request.income_by_day,
        starting_balance=request.starting_balance,
    )
    return RunwayBandsResponse(
        model_version=BANDS_VERSION,
        resamples=RESAMPLES,
        bands=[
            {
                "percentile": band.percentile,
                "trough_minor": band.trough_minor,
                "recovery_day": band.recovery_day,
                "ending_balance_minor": band.ending_balance_minor,
            }
            for band in bands
        ],
    )


app = create_app()
