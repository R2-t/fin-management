# Merchant normalization and categorisation sidecar.
#
# Built from the repository root so the root .dockerignore applies and keeps
# the 80 MB local virtualenv out of the build context:
#
#   podman build -f sidecar/Dockerfile -t finmgmt-analytics .
FROM python:3.13-slim

WORKDIR /srv

COPY sidecar/pyproject.toml ./
COPY sidecar/app ./app

# `.` installs the package plus its dependencies, so the version in
# pyproject.toml is the single source of truth for what runs.
RUN pip install --no-cache-dir .

# Runs as a non-root user: the service reads the database and serves HTTP,
# and needs neither privilege nor a writable filesystem.
RUN useradd --create-home --uid 10001 appuser
USER appuser

EXPOSE 8000

# --access-log off keeps one line per request out of the collector, which
# already records request latency as a span.
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000", "--no-access-log"]

# The job worker is a separate process from the HTTP app: run it with
#   podman run --rm finmgmt-analytics python -m app.worker
