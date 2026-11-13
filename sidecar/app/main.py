"""The sidecar's HTTP surface.

Two endpoints: liveness, and merchant name cleaning. The rest of Phase 3
(job workers, categorization) attaches here as it lands.
"""

import json
import logging
import os
import sys

from fastapi import FastAPI
from pydantic import BaseModel

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


class NormalizeResponse(BaseModel):
    raw: str
    normalized: str
    changed: bool
    # The rule set or model that produced the answer, so a later rule change
    # can be told apart from a model change.
    model_version: str


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
    def normalize(request: NormalizeRequest) -> NormalizeResponse:
        cleaned = normalize_merchant(request.raw)
        return NormalizeResponse(
            raw=request.raw,
            normalized=cleaned,
            changed=cleaned != request.raw,
            model_version=RULES_VERSION,
        )

    return app


app = create_app()
