"""Verify native Atlas dispatch against a disposable SurrealDB and RabbitMQ."""

import base64
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time

from fleetworks_network_smoke import binaries, free_ports, process, request, safe_env, wait_for


def exercise(tmp, binary, surreal):
    database_port, api_port = free_ports(2)
    root_key, runtime_key, token = (secrets.token_hex(32) for _ in range(3))
    db_env = {**safe_env, "SURREAL_USER": "root", "SURREAL_PASS": root_key}
    env = {
        **safe_env,
        "ARGOS_HOME": str(tmp),
        "FLEETWORKS_WORKSPACE": str(tmp),
        "FLEETWORKS_STORAGE_URL": f"ws://127.0.0.1:{database_port}",
        "FLEETWORKS_STORAGE_PASSWORD": runtime_key,
        "FLEETWORKS_API_TOKEN": token,
        "FLEETWORKS_API_ALLOWED_HOSTS": f"127.0.0.1:{api_port}",
    }
    worker_env = {
        **env,
        "FLEETWORKS_BROKER_URL": os.environ["FLEETWORKS_BROKER_URL"],
        "FLEETWORKS_BROKER_QUEUE": "fleetworks.atlas.smoke." + secrets.token_hex(8),
    }
    db = [surreal, "start", "--no-banner", "--log", "warn", "--bind", f"127.0.0.1:{database_port}", "memory"]
    auth = {"Authorization": "Bearer " + token, "Content-Type": "application/json"}
    root = base64.b64encode(f"root:{root_key}".encode()).decode()
    sql_headers = {
        "Authorization": f"Basic {root}",
        "Surreal-NS": "fleetworks",
        "Surreal-DB": "fleetworks",
        "Content-Type": "application/surrealql",
        "Accept": "application/json",
    }
    atlas_path = "/api/v1/atlas/cycles"

    def post(path, payload, headers=auth):
        return request(api_port, path, headers, json.dumps(payload))

    def state(job_id):
        status, response = request(api_port, f"/api/v1/jobs/detail?id={job_id}", auth)
        assert status == 200, (status, response)
        return response["job"]

    def sql(query):
        status, response = request(database_port, "/sql", sql_headers, query)
        assert status == 200 and response[0]["status"] == "OK", (status, response)
        return response[0]["result"]

    def wait_for_state(worker, job_id, expected):
        """Observe committed terminal state through the authenticated API."""
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            assert worker.poll() is None, "Atlas worker exited"
            job = state(job_id)
            if job["state"] == expected:
                return job
            time.sleep(0.25)
        raise AssertionError(f"Atlas worker did not settle dispatch: {state(job_id)}")

    with process(db, db_env, tmp / "surreal.log") as database:
        wait_for(database, database_port, "/health")
        subprocess.run(
            [binary, "--role", "bootstrap"],
            env={**env, "FLEETWORKS_STORAGE_ROOT_PASSWORD": root_key, "FLEETWORKS_ATLAS_DISPATCH": "1"},
            check=True, timeout=35,
        )
        with process([binary, "--role", "api", "--port", str(api_port)], env, tmp / "api.log") as api:
            wait_for(api, api_port, "/api/v1/health/ready", auth)
            assert post(atlas_path, {"request_id": "fixture"}, {"Content-Type": "application/json"})[0] == 401
            assert post(atlas_path, {"request_id": "bad/id"})[0] == 400
            status, receipt = post(atlas_path, {"request_id": "fixture"})
            assert status == 202 and receipt["run_id"] == "fleet-atlas-run-fixture", (status, receipt)
            assert post(atlas_path, {"request_id": "fixture"}) == (status, receipt)
            job_id = receipt["job_id"]
            assert state(job_id)["state"] == "queued"
            status, cancelled = post(atlas_path, {"request_id": "cancel-fixture"})
            assert status == 202
            cancel_id = cancelled["job_id"]
            assert post("/api/v1/jobs/cancel", {"id": cancel_id})[0] == 200

            status, replay = post(atlas_path, {"request_id": "commit-before-ack"})
            assert status == 202
            replay_job = replay["job_id"]
            replay_run = replay["run_id"]
            # A previous activation committed the Atlas result and then exited
            # before settling the dispatch. No live providers or models are
            # necessary to exercise that exact replay over the real broker.
            sql(f"CREATE type::record('atlas_run','{replay_run}') SET state='completed', started_at='2026-01-01T00:00:00Z', finished_at='2026-01-01T00:00:00Z';")

            for attempt in (1, 2):
                if attempt == 2:
                    assert post("/api/v1/jobs/retry", {"id": job_id}) == (200, {"requeued": 1})
                    assert state(job_id)["state"] == "queued"
                with process([binary, "--role", "atlas-worker"], worker_env, tmp / f"worker-{attempt}.log") as worker:
                    failed = wait_for_state(worker, job_id, "failed")
                    assert failed["error_summary"] == "atlas_classifier_unavailable", failed
                    assert failed["attempt_rows"] == attempt, failed
                    completed = wait_for_state(worker, replay_job, "completed")
                    assert completed["result_ref"] == replay_run and completed["attempt_rows"] == 1, completed

            assert state(cancel_id)["state"] == "cancelled", state(cancel_id)
            assert state(cancel_id)["attempt_rows"] == 0
            assert post(atlas_path, {"request_id": "fixture"}) == (202, receipt)
            # Exact attempt identity and confirm count stay stable across a
            # worker restart and explicit retry. The fixture has no classifier,
            # so no news provider is contacted and no article is persisted.
            assert sql(f"SELECT VALUE number FROM attempt WHERE task=type::record('task','{job_id}-collect') ORDER BY number;") == [1, 2]
            assert sql(f"SELECT VALUE revision FROM job_dispatch_outbox WHERE job=type::record('job','{job_id}') AND published_at!=NONE ORDER BY revision;") == [0, 3]
    print("Atlas: authenticated replay, queued cancellation, broker confirms, completion replay and monotonic retry across restart passed", flush=True)


if __name__ == "__main__":
    binary, surreal = binaries(__doc__)
    with tempfile.TemporaryDirectory(prefix="fleetworks-atlas-") as directory:
        tmp = Path(directory)
        try:
            exercise(tmp, binary, surreal)
        except Exception:
            for log in sorted(tmp.glob("*.log")):
                print(log.name, log.read_text()[-2500:], flush=True)
            raise
