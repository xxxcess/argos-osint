"""Build and smoke-test the local Fleetworks Compose stack through Venture."""

import argparse
import contextlib
import hashlib
import http.client
import http.cookies
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
)


class ProofHeaders(dict):
    """HTTP proof references plus the actual decoded snapshot used in a form."""
    def __init__(self, headers, context):
        super().__init__(headers)
        self.context = context


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


def exchange(port, path, token=None, payload=None, *, headers=None, method=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
    try:
        headers = dict(headers or {})
        if token:
            headers["Authorization"] = f"Bearer {token}"
        body = payload
        if payload is not None and not isinstance(payload, bytes):
            headers["Content-Type"] = "application/json"
            body = json.dumps(payload)
        connection.request(method or ("GET" if payload is None else "POST"), path, body, headers)
        response = connection.getresponse()
        data = response.read(4 * 1024 * 1024 + 1)
        assert len(data) <= 4 * 1024 * 1024, "bounded smoke response exceeded limit"
        try:
            value = json.loads(data)
        except ValueError:
            value = None  # Public HTML pages are inspected by status only.
        return response.status, value, dict(response.getheaders())
    finally:
        connection.close()


def request(port, path, token=None, payload=None, *, headers=None):
    status, value, _ = exchange(port, path, token, payload, headers=headers)
    return status, value


def native_fixture(directory):
    """An actual isolated contained test child; not installed-native FS evidence."""
    home = directory / "synthetic-native-home"
    container = home / ".fleetworks"
    child = container / "compose-review"
    for path in (home, container, container / "default", child):
        path.mkdir(mode=0o700)
    assert child.resolve().parent == container.resolve() and not child.is_symlink()
    identity = {"installation_marker": secrets.token_hex(32), "workspace_marker": secrets.token_hex(32)}
    marker = child / "fixture-identity.json"
    descriptor = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(identity, output)
    return {"request_id": "compose-register", "workstation_ip": "192.0.2.61", "workspace_name": child.name, **identity}


def register_native(port, token, registration):
    status, receipt = request(port, "/api/v1/connect/registrations/ensure", token, registration)
    assert status in (200, 201), "native ensure did not establish registration"
    assert request(port, "/api/v1/connect/registrations/ensure", token, registration) == (200, receipt), "ensure replay changed identity"
    status, value = request(port, "/api/v1/connect/registrations/context", token, {
        "registration_id": receipt["registration_id"], "expected_revision": receipt["revision"],
    })
    assert status == 200, "current native context unavailable"
    context = value["context"]
    assert context["registration_id"] == receipt["registration_id"] and context["workspace_name"] == registration["workspace_name"]
    return context


def native_headers(context):
    return ProofHeaders({"x-fleetworks-registration": context["registration_id"], "x-fleetworks-registration-revision": str(context["registration_revision"])}, context)


def submission_context(port, token, proof):
    status, current = request(port, "/api/v1/models/defaults", token, headers=proof)
    revision = current.get("configuration_revision") if isinstance(current, dict) else None
    assert status == 200 and type(revision) is int and revision >= 0, "current configuration revision unavailable"
    context = proof.context
    return {"registration_id": context["registration_id"], "expected_registration_revision": context["registration_revision"],
        "expected_context_revision": context["context_revision"], "expected_configuration_revision": revision}


def web_connect(port, context):
    origin = f"http://127.0.0.1:{port}"
    assert exchange(port, "/", headers={})[0] == 307, "unconnected deep link exposed the application"
    assert request(port, "/api/v1/health/ready")[0] == 401, "web data accepted an unconnected caller"
    status, value, response_headers = exchange(port, "/api/connect", payload={"connect_id": context["connect_id"]}, headers={"Origin": origin})
    assert status == 200 and "session_token" not in value, "normal Connect login failed"
    cookie = http.cookies.SimpleCookie()
    cookie.load(next((value for key, value in response_headers.items() if key.lower() == "set-cookie"), ""))
    item = cookie.get("fleetworks_connect")
    assert item and item["httponly"] and item["samesite"] == "Strict", "Connect did not issue a protected cookie"
    assert value["context"]["registration_id"] == context["registration_id"], "browser connected to another registration"
    headers = {"Cookie": item.OutputString().split(";", 1)[0], "Origin": origin,
        "x-fleetworks-expected-context-revision": str(value["context"]["context_revision"])}
    assert request(port, "/api/connect", headers=headers)[0] == 200, "session did not resolve"
    return ProofHeaders(headers, value["context"])


def web_logout(port, headers):
    assert request(port, "/api/connect/logout", payload={}, headers=headers)[0] == 200, "normal logout failed"
    assert request(port, "/api/v1/health/ready", headers=headers)[0] == 401, "logged-out cookie remained valid"


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


def exercise_training(port, token, context, workspace, source):
    """Actual staged Natural maintenance, retaining prior activation explicitly."""
    original = source.read_bytes()
    assert 0 < len(original) <= 128 * 1024 * 1024, "training fixture exceeds supported bound"
    active = workspace / "classifier/active.json"
    previous = active.read_bytes() if active.exists() else None
    for index, content in enumerate((original, original + b"\r\n")):
        digest = hashlib.sha256(content).hexdigest()
        status, uploaded, _ = exchange(port, f"/api/v1/workstation/datasets/upload?kind=classifier_training&sha256={digest}", token,
            content, headers={**native_headers(context), "Content-Type": "text/csv"})
        assert status == 201 and uploaded["dataset"]["sha256"] == digest, "real dataset staging failed"
        payload = {"request_id": f"compose-train-{index}", "dataset": uploaded["dataset"], "engine": "natural", "context": submission_context(port, token, native_headers(context))}
        status, receipt = request(port, "/api/v1/workstation/maintenance/train-classifier", token, payload, headers=native_headers(context))
        assert status == 202 and receipt["origin"]["source"] == "native", "durable attributed training was not admitted"
        wait_job(port, token, native_headers(context), receipt["job_id"], 180)
        assert (active.read_bytes() if active.exists() else None) == previous, "maintenance training changed model activation"
    print("Registered Natural training persisted two staged dataset outcomes; prior activation retained")


def exercise_atlas(port, token, proof):
    path = "/api/v1/atlas/cycles"
    payload = {"request_id": "compose-smoke", "context": submission_context(port, token, proof)}
    status, receipt = request(port, path, token, payload, headers=proof)
    assert status == 202 and receipt["job_id"], "registered Atlas admission failed"
    assert request(port, path, token, payload, headers=proof) == (status, receipt)
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        status, value = request(port, f"/api/v1/jobs/detail?id={receipt['job_id']}", token, headers=proof)
        assert status == 200, (status, value)
        if value["job"]["state"] == "failed":
            assert value["job"]["error_summary"] == "atlas_classifier_model_not_configured", value
            assert value["job"]["attempts_used"] == 1, value
            return
        time.sleep(0.25)
    raise RuntimeError("Packaged Atlas worker did not settle the fixture delivery")


def exercise_tools(port, token, proof, request_id):
    payload = {
        "request_id": request_id, "tool_id": "dork_generate",
        "inputs": {"objective": "Find public annual reports"},
        "context": submission_context(port, token, proof),
    }
    status, receipt = request(port, "/api/v1/tools/run", token, payload, headers=proof)
    assert status == 202 and receipt["job_id"] == receipt["call_id"], "durable attributed tool admission failed"
    assert request(port, "/api/v1/tools/run", token, payload, headers=proof) == (202, receipt), "tool replay changed receipt"
    result = "/api/v1/tools/result?id=" + receipt["call_id"]
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        status, value = request(port, result, token, headers=proof)
        assert status == 200, (status, value)
        if value["status"] not in {"queued", "running", "waiting"}:
            assert value["status"] == "completed", value
            assert value["result"]["observations"] and value["result"]["credits_charged"] == 0
            wait_job(port, token, proof, receipt["job_id"], 15)
            return result, value
        time.sleep(0.1)
    raise RuntimeError("Packaged Tools service did not complete the local fixture")


def wait_job(port, token, proof, job_id, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        status, detail = request(port, "/api/v1/jobs/detail?id=" + job_id, token, headers=proof)
        assert status == 200 and detail["private_details_available"], "requester membership did not authorize job detail"
        job = detail["job"]
        if job["state"] in {"completed", "failed", "cancelled"}:
            assert job["state"] == "completed", "registered fixture job failed"
            return detail
        time.sleep(0.1)
    raise RuntimeError("Durable fixture job did not settle")


def exercise_activity(api_port, web_port, token, context, browser, job_id):
    proof = native_headers(context)
    for port, credential, headers in ((api_port, token, proof), (web_port, None, browser)):
        status, page = request(port, "/api/v1/jobs?limit=25", credential, headers=headers)
        assert status == 200 and any(row["id"] == job_id for row in page["rows"]), "attributed activity omitted fixture job"
        assert page["activity_scope"]["workstation_id"] == context["workstation_id"], "activity selected another workstation"
        status, logs = request(port, "/api/v1/logs?job_id=" + job_id, credential, headers=headers)
        assert status == 200 and isinstance(logs["rows"], list), "selected attributed logs unavailable"
    assert request(web_port, "/api/v1/jobs?workspace_id=not-a-child", headers=browser)[0] == 404, "browser selected another child"


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


def verify(
    venture, compose, dev=False, *, no_build=False, image_ids=None, storage="compose-disk",
    surreal=None, train_legacy_dataset=None,
):
    atlas = compose.with_name("compose.atlas.yml").is_file()
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
        registration = native_fixture(directory)
        other_directory = directory / "other-node-fixture"
        other_directory.mkdir(mode=0o700)
        other_registration = {**native_fixture(other_directory), "request_id":"compose-other-node", "workstation_ip":"192.0.2.62"}
        # Preserve and prove that the pre-cutover credential gains no rights.
        (workspace / "secrets").mkdir(mode=0o700, parents=True)
        legacy = secrets.token_hex(32)
        descriptor = os.open(workspace / "secrets/api-token", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w") as output:
            output.write(legacy + "\n")
        database = None
        if storage == "external":
            database = resources.enter_context(external_database(directory, environment, surreal, db_ports[0]))

        def check_external_survived():
            if database is not None and not external_ready(database, db_ports[0]):
                raise RuntimeError("External SurrealDB did not stay ready after Venture stop")

        def run(*args, check=True, timeout=60, stdout=None):
            return subprocess.run(
                [venture, "--workspace", workspace, *args],
                env=environment, check=check, timeout=timeout, stdout=stdout,
            )

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
            assert (workspace / "secrets/api-token").read_text().strip() == legacy, "legacy credential bytes changed"
            assert request(api_port, ready, legacy)[0] == 401, "old public token acquired gateway authority"
            token = (workspace / "secrets/native-api-token").read_text().strip()
            operator = (workspace / "secrets/operator-api-token").read_text().strip()
            web_token = (workspace / "secrets/web-relay-token").read_text().strip()
            assert len({token, operator, web_token, legacy}) == 4, "purpose credentials were reused"
            assert request(api_port, "/api/v1/connect/registrations/ensure", web_token, registration)[0] == 403, "WebRelay acquired registration authority"
            assert request(api_port, "/api/v1/tools/run", operator, {"request_id":"denied-operator","tool_id":"dork_generate","inputs":{"objective":"Synthetic fixture"}})[0] == 403, "read-only operator acquired mutation authority"
            assert request(api_port, "/api/v1/jobs", token)[0] == 401, "contextless activity was accepted"
            context = register_native(api_port, token, registration)
            browser = web_connect(web_port, context)
            direct_status, direct = request(api_port, ready, operator)
            relay_status, relay = request(web_port, ready, headers=browser)
            assert direct_status == relay_status == 200, "API or relay readiness failed"
            assert direct.get("ready") and direct.get("storage") and direct.get("storage") == relay.get("storage"), (
                "web readiness did not reach the same ready API"
            )
            if storage == "compose-disk":
                assert (workspace / "storage/surrealdb").is_dir(), "Compose disk store is missing"
            if storage == "host-disk":
                assert (workspace / "storage/host-surrealdb").is_dir(), "host disk store is missing"
            if image_ids is not None:
                check_container_images(workspace, environment, image_ids)
            return token, context, browser

        try:
            token, context, browser = start()
            result_path, completed = exercise_tools(api_port, token, native_headers(context), "compose-native-dork")
            web_result, _ = exercise_tools(web_port, None, browser, "compose-web-dork")
            exercise_activity(api_port, web_port, token, context, browser, completed["job_id"])
            other = register_native(api_port, token, other_registration)
            assert request(api_port, "/api/v1/jobs/detail?id=" + completed["job_id"], token, headers=native_headers(other))[0] == 404, "another workstation read private job data"
            assert request(api_port, web_result, token, headers=native_headers(other))[0] == 404, "another workstation read private tool output"
            if train_legacy_dataset is not None:
                exercise_training(api_port, token, context, workspace, train_legacy_dataset)
            if atlas:
                assert (workspace / "storage/jobs/queue").is_dir(), "embedded queue store is missing"
                exercise_atlas(api_port, token, native_headers(context))
                print("Packaged Atlas dispatch reached the private classifier service")
            web_logout(web_port, browser)
            if separate_database:
                run("stop", timeout=150)
                check_external_survived()
                token, context, browser = start()
                status, value = request(api_port, result_path, token, headers=native_headers(context))
                if storage == "host-memory":
                    assert status == 404, "host memory retained a tool result after restart"
                    exercise_tools(api_port, token, native_headers(context), "compose-after-restart")
                    print("Host memory reset its tool result and accepted new work after restart")
                else:
                    assert (status, value) == (200, completed), f"{storage} lost its tool result after restart"
                    print(f"{storage} retained its completed tool result after restart")
                web_logout(web_port, browser)
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
        print(f"Venture {mode} Compose startup, purpose isolation, real registration, attributed Tools/activity, browser session/logout and owned shutdown passed ({storage})")
        print("Backend integration only: installed-native filesystem/GUI and retired terminal presentation remain separate evidence gates")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--venture", type=Path, required=True)
    parser.add_argument("--compose", type=Path, default=Path(__file__).with_name("compose.yml"))
    parser.add_argument("--dev", action="store_true", help="verify the development web override")
    parser.add_argument("--no-build", action="store_true", help="use preloaded images without building or pulling")
    parser.add_argument("--image-ids", type=Path, help="expected Docker image ID for each Compose service as JSON")
    parser.add_argument("--storage", choices=("compose-disk", "host-memory", "host-disk", "external"), default="compose-disk")
    parser.add_argument("--surreal", type=Path, help="pinned SurrealDB executable for host or external storage")
    parser.add_argument("--train-legacy-dataset", type=Path, help="retained flag spelling: stage CSV and run registered Natural maintenance twice, retaining prior activation")
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
        train_legacy_dataset=args.train_legacy_dataset.resolve(strict=True) if args.train_legacy_dataset else None,
    )


if __name__ == "__main__":
    main()
