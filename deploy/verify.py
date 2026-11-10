"""Build and smoke-test the local Fleetworks Compose stack through Venture."""

import argparse
import contextlib
import http.client
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def loopback_ports():
    with contextlib.ExitStack() as resources:
        api = resources.enter_context(socket.socket())
        web = resources.enter_context(socket.socket())
        for listener in (api, web):
            listener.bind(("127.0.0.1", 0))
        return api.getsockname()[1], web.getsockname()[1]


def request(port, path, token=None, payload=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=8)
    try:
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        if payload is None:
            connection.request("GET", path, headers=headers)
        else:
            headers["Content-Type"] = "application/json"
            connection.request("POST", path, json.dumps(payload), headers=headers)
        response = connection.getresponse()
        return response.status, json.loads(response.read())
    finally:
        connection.close()


def exercise_atlas(port, token):
    path = "/api/v1/atlas/cycles"
    payload = {"request_id": "compose-smoke"}
    status, receipt = request(port, path, token, payload)
    assert status == 202 and receipt["job_id"] == "fleet-atlas-job-compose-smoke", (status, receipt)
    assert request(port, path, token, payload) == (status, receipt)
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        status, value = request(port, f"/api/v1/jobs/detail?id={receipt['job_id']}", token)
        assert status == 200, (status, value)
        if value["job"]["state"] == "failed":
            assert value["job"]["error_summary"] == "atlas_classifier_model_not_configured", value
            assert value["job"]["attempt_rows"] == 1, value
            return
        time.sleep(0.25)
    raise RuntimeError("Packaged Atlas worker did not settle the fixture delivery")


def verify(venture, compose, dev=False):
    api_port, web_port = loopback_ports()
    environment = {
        name: value for name, value in os.environ.items()
        if not name.startswith(("ARGOS_", "FLEETWORKS_", "COMPOSE_"))
        and not name.endswith(("_API_KEY", "_API_KEY_FALLBACK", "_API_TOKEN", "_API_TOKEN_FALLBACK"))
    }
    environment.update(
        FLEETWORKS_API_PORT=str(api_port),
        FLEETWORKS_WEB_PORT=str(web_port),
    )
    with tempfile.TemporaryDirectory(prefix="fleetworks-compose-") as directory:
        workspace = Path(directory) / "workspace"

        def run(*args, check=True, timeout=60):
            return subprocess.run(
                [venture, "--workspace", workspace, *args],
                env=environment, check=check, timeout=timeout,
            )

        try:
            atlas = compose.with_name("compose.atlas.yml").is_file()
            overlays = []
            if atlas:
                overlays.append("--atlas")
            if dev:
                overlays.append("--dev")
            run("up", "--storage", "compose-disk", "--compose-file", compose, *overlays, timeout=2400)
            run("status", "--json")
            ready = "/api/v1/health/ready"
            assert request(api_port, ready)[0] == 401, "direct API accepted a request without a bearer"
            token = (workspace / "secrets/api-token").read_text().strip()
            direct_status, direct = request(api_port, ready, token)
            relay_status, relay = request(web_port, ready)
            assert direct_status == relay_status == 200, "API or relay readiness failed"
            assert direct == relay and direct.get("ready") and direct.get("storage"), (
                "web readiness did not reach the same ready API"
            )
            assert (workspace / "storage/surrealdb").is_dir(), "managed disk store is missing"
            if atlas:
                assert (workspace / "storage/rabbitmq").is_dir(), "managed RabbitMQ store is missing"
                exercise_atlas(api_port, token)
                print("Packaged Atlas dispatch reached the private classifier service")
        finally:
            failed = sys.exc_info()[0] is not None
            try:
                if failed:
                    run("logs", check=False, timeout=30)
            except (OSError, subprocess.TimeoutExpired) as error:
                print(f"Could not collect Compose logs: {error}", file=sys.stderr)
            finally:
                run("stop", check=not failed, timeout=150)
        mode = "development" if dev else "production"
        print(f"Venture {mode} Compose startup, bearer isolation, web relay and owned shutdown passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--venture", type=Path, required=True)
    parser.add_argument("--compose", type=Path, default=Path(__file__).with_name("compose.yml"))
    parser.add_argument("--dev", action="store_true", help="verify the development web override")
    args = parser.parse_args()
    verify(args.venture.resolve(strict=True), args.compose.resolve(strict=True), args.dev)


if __name__ == "__main__":
    main()
