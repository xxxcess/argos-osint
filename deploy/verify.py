"""Build and smoke-test the local Fleetworks Compose stack through Venture."""

import argparse
import contextlib
import http.client
import json
import os
import re
import secrets
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


IMAGE_OVERRIDES = (
    "FLEETWORKS_RUNTIME_IMAGE",
    "FLEETWORKS_WEB_IMAGE",
    "FLEETWORKS_SURREAL_IMAGE",
    "FLEETWORKS_RABBITMQ_IMAGE",
)


def fixture_ports(separate_database=False):
    addresses = ["127.0.0.1", "127.0.0.1"]
    if separate_database:
        # A Linux container connects through the host gateway, beyond loopback.
        addresses.append("0.0.0.0")
    with contextlib.ExitStack() as resources:
        ports = []
        for address in addresses:
            listener = resources.enter_context(socket.socket())
            listener.bind((address, 0))
            ports.append(listener.getsockname()[1])
        return ports


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


def external_ready(database, port):
    if database.poll() is not None:
        return False
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=1)
    try:
        connection.request("GET", "/health")
        return connection.getresponse().status == 200
    except (OSError, http.client.HTTPException):
        return False
    finally:
        connection.close()


@contextlib.contextmanager
def external_database(directory, environment, surreal, port):
    server_env = {name: value for name, value in environment.items() if not name.startswith("SURREAL_")}
    version = subprocess.check_output([surreal, "version"], env=server_env, text=True, timeout=5).split()
    if version[:1] != ["3.3.4"]:
        raise RuntimeError("The external fixture requires the repository-pinned SurrealDB 3.3.4")
    password = secrets.token_hex(32)
    server_env.update(SURREAL_USER="root", SURREAL_PASS=password)
    backend = f'rocksdb://{directory / "external-surrealdb"}?versioned=false&sync=every'
    with tempfile.NamedTemporaryFile(mode="w", dir=directory, encoding="utf-8") as credential, (
        directory / "external-surreal.log"
    ).open("w+b") as log:
        credential.write(password + "\n")
        credential.flush()
        environment.update(
            FLEETWORKS_STORAGE_URL=f"ws://host.docker.internal:{port}",
            FLEETWORKS_STORAGE_ROOT_USERNAME="root",
            FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE=credential.name,
        )
        with subprocess.Popen(
            [surreal, "start", "--no-banner", "--log", "warn", "--bind", f"0.0.0.0:{port}", backend],
            env=server_env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
        ) as database:
            try:
                deadline = time.monotonic() + 35
                while not external_ready(database, port):
                    if database.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError("External SurrealDB did not become ready")
                    time.sleep(0.15)
                yield database
            except Exception:
                log.seek(max(0, log.tell() - 4096))
                details = log.read(4096).decode(errors="replace").replace(password, "[redacted]")
                if details:
                    print(details, file=sys.stderr)
                raise
            finally:
                if database.poll() is None:
                    database.terminate()
                    try:
                        database.wait(timeout=22)
                    except subprocess.TimeoutExpired:
                        database.kill()
                        database.wait()


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


def exercise_tools(port, token):
    status, receipt = request(port, "/api/v1/tools/run", token, {
        "request_id": "compose-dork", "tool_id": "dork_generate",
        "inputs": {"objective": "Find public annual reports"},
    })
    assert (status, receipt) == (202, {"call_id": "fleet-tool-compose-dork"})
    result = "/api/v1/tools/result?id=" + receipt["call_id"]
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        status, value = request(port, result, token)
        assert status == 200, (status, value)
        if value["status"] != "running":
            assert value["status"] == "completed", value
            assert value["result"]["observations"] and value["result"]["credits_charged"] == 0
            return result, value
        time.sleep(0.1)
    raise RuntimeError("Packaged Tools service did not complete the local fixture")


def check_container_images(workspace, environment, expected):
    owner = json.loads((workspace / "storage/venture-owner.json").read_text())
    docker_env = {
        name: value for name, value in environment.items()
        if name not in ("DOCKER_CONTEXT", "DOCKER_HOST", "DOCKER_TLS", "DOCKER_TLS_VERIFY")
    }

    def docker(*args):
        return subprocess.check_output(
            ["docker", "--host", owner["docker_host"], *args],
            env=docker_env, text=True, timeout=30,
        ).splitlines()

    project = owner["project"]
    containers = docker("ps", "--all", "--filter", f"label=com.docker.compose.project={project}", "--quiet")
    if not containers:
        raise RuntimeError("Venture's owned project has no containers to verify")
    rows = docker(
        "inspect", "--type", "container", "--format",
        '{{ index .Config.Labels "com.docker.compose.service" }} {{ .Image }}',
        *containers,
    )
    actual = dict(row.split() for row in rows)
    if len(actual) != len(containers) or actual != expected:
        raise RuntimeError(f"Compose container images differ from the expected IDs: {actual}")
    print(f"Verified exact image IDs for {len(actual)} Compose services")


def verify(venture, compose, dev=False, *, no_build=False, image_ids=None, storage="compose-disk", surreal=None):
    separate_database = storage != "compose-disk"
    api_port, web_port, *db_ports = fixture_ports(separate_database)
    environment = {
        name: value for name, value in os.environ.items()
        if not name.startswith(("ARGOS_", "FLEETWORKS_", "COMPOSE_"))
        and not name.endswith(("_API_KEY", "_API_KEY_FALLBACK", "_API_TOKEN", "_API_TOKEN_FALLBACK"))
    }
    environment.update(
        FLEETWORKS_API_PORT=str(api_port),
        FLEETWORKS_WEB_PORT=str(web_port),
    )
    if no_build:
        environment.update({name: os.environ[name] for name in IMAGE_OVERRIDES if name in os.environ})
    if storage in ("host-memory", "host-disk"):
        environment.update(
            FLEETWORKS_HOST_DB_BIND=f"0.0.0.0:{db_ports[0]}",
            FLEETWORKS_SURREAL_BIN=str(surreal),
        )
    if separate_database and image_ids is not None:
        image_ids = {service: image for service, image in image_ids.items() if service != "surrealdb"}
    with tempfile.TemporaryDirectory(prefix="fleetworks-compose-") as directory, contextlib.ExitStack() as resources:
        directory = Path(directory)
        workspace = directory / "workspace"
        database = None
        if storage == "external":
            database = resources.enter_context(external_database(directory, environment, surreal, db_ports[0]))

        def check_external_survived():
            if database is not None and not external_ready(database, db_ports[0]):
                raise RuntimeError("External SurrealDB did not stay ready after Venture stop")

        def run(*args, check=True, timeout=60):
            return subprocess.run(
                [venture, "--workspace", workspace, *args],
                env=environment, check=check, timeout=timeout,
            )

        atlas = compose.with_name("compose.atlas.yml").is_file()
        options = []
        if atlas:
            options.append("--atlas")
        if dev:
            options.append("--dev")
        if no_build:
            options.append("--no-build")

        def start():
            run("up", "--storage", storage, "--compose-file", compose, *options, timeout=2400)
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
            if storage == "compose-disk":
                assert (workspace / "storage/surrealdb").is_dir(), "Compose disk store is missing"
            if storage == "host-disk":
                assert (workspace / "storage/host-surrealdb").is_dir(), "host disk store is missing"
            if image_ids is not None:
                check_container_images(workspace, environment, image_ids)
            return token

        try:
            token = start()
            result_path, completed = exercise_tools(api_port, token)
            if atlas:
                assert (workspace / "storage/rabbitmq").is_dir(), "managed RabbitMQ store is missing"
                exercise_atlas(api_port, token)
                print("Packaged Atlas dispatch reached the private classifier service")
            if separate_database:
                run("stop", timeout=150)
                check_external_survived()
                token = start()
                status, value = request(api_port, result_path, token)
                if storage == "host-memory":
                    assert status == 404, "host memory retained a tool result after restart"
                    exercise_tools(api_port, token)
                    print("Host memory reset its tool result and accepted new work after restart")
                else:
                    assert (status, value) == (200, completed), f"{storage} lost its tool result after restart"
                    print(f"{storage} retained its completed tool result after restart")
        finally:
            failed = sys.exc_info()[0] is not None
            try:
                if failed:
                    run("logs", check=False, timeout=30)
            except (OSError, subprocess.TimeoutExpired) as error:
                print(f"Could not collect Compose logs: {error}", file=sys.stderr)
            finally:
                run("stop", check=not failed, timeout=150)
            if not failed:
                check_external_survived()
        mode = "development" if dev else "production"
        print(f"Venture {mode} Compose startup, bearer isolation, web relay and owned shutdown passed ({storage})")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--venture", type=Path, required=True)
    parser.add_argument("--compose", type=Path, default=Path(__file__).with_name("compose.yml"))
    parser.add_argument("--dev", action="store_true", help="verify the development web override")
    parser.add_argument("--no-build", action="store_true", help="use preloaded images without building or pulling")
    parser.add_argument("--image-ids", type=Path, help="expected Docker image ID for each Compose service as JSON")
    parser.add_argument("--storage", choices=("compose-disk", "host-memory", "host-disk", "external"), default="compose-disk")
    parser.add_argument("--surreal", type=Path, help="pinned SurrealDB executable for host or external storage")
    args = parser.parse_args()
    if args.storage == "compose-disk" and args.surreal is not None:
        parser.error("--surreal requires host or external storage")
    if args.storage != "compose-disk" and args.surreal is None:
        parser.error("host and external storage modes require --surreal")
    image_ids = None
    if args.image_ids:
        if not args.no_build:
            parser.error("--image-ids requires --no-build")
        try:
            image_ids = json.loads(args.image_ids.read_text())
        except (OSError, ValueError) as error:
            parser.error(f"could not read --image-ids: {error}")
        if not isinstance(image_ids, dict) or not image_ids or not all(
            isinstance(image_id, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", image_id)
            for image_id in image_ids.values()
        ):
            parser.error("--image-ids must map service names to SHA-256 image IDs")
    verify(
        args.venture.resolve(strict=True), args.compose.resolve(strict=True), args.dev,
        no_build=args.no_build, image_ids=image_ids, storage=args.storage,
        surreal=args.surreal.resolve(strict=True) if args.surreal else None,
    )


if __name__ == "__main__":
    main()
