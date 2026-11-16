"""Entry point for the job worker: `python -m app.worker`.

A separate process from the HTTP app, so a slow job never blocks a request
and a request never waits on a job.
"""

from app.main import create_worker

if __name__ == "__main__":
    create_worker()
