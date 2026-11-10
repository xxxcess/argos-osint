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
from pathlib import Path


def loopback_ports():
    with contextlib.ExitStack() as resources:
        api = resources.enter_context(socket.socket())
        web = resources.enter_context(socket.socket())
        for listener in (api, web):
            listener.bind(("127.0.0.1", 0))
        return api.getsockname()[1], web.getsockname()[1]


def readiness(port, token=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=8)
    try:
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        connection.request("GET", "/api/v1/health/ready", headers=headers)
        response = connection.getresponse()
        return response.status, json.loads(response.read())
    finally:
        connection.close()


def verify(venture, compose):
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
            run("up", "--storage", "compose-disk", "--compose-file", compose, timeout=2400)
            run("status", "--json")
            assert readiness(api_port)[0] == 401, "direct API accepted a request without a bearer"
            token = (workspace / "secrets/api-token").read_text().strip()
            direct_status, direct = readiness(api_port, token)
            relay_status, relay = readiness(web_port)
            assert direct_status == relay_status == 200, "API or relay readiness failed"
            assert direct == relay and direct.get("ready") and direct.get("storage"), (
                "web readiness did not reach the same ready API"
            )
            assert (workspace / "storage/surrealdb").is_dir(), "managed disk store is missing"
        finally:
            failed = sys.exc_info()[0] is not None
            try:
                if failed:
                    run("logs", check=False, timeout=30)
            except (OSError, subprocess.TimeoutExpired) as error:
                print(f"Could not collect Compose logs: {error}", file=sys.stderr)
            finally:
                run("stop", check=not failed, timeout=150)
        print("Venture Compose startup, bearer isolation, web relay and owned shutdown passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--venture", type=Path, required=True)
    parser.add_argument("--compose", type=Path, default=Path(__file__).with_name("compose.yml"))
    args = parser.parse_args()
    verify(args.venture.resolve(strict=True), args.compose.resolve(strict=True))


if __name__ == "__main__":
    main()
