"""Build and smoke-test a fresh isolated standalone backend, optionally with Web."""

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
import time
from pathlib import Path

import native_review


IMAGE_OVERRIDES = (
    "FLEETWORKS_RUNTIME_IMAGE",
    "FLEETWORKS_WEB_IMAGE",
    "FLEETWORKS_SURREAL_IMAGE",
    "FLEETWORKS_SEARCH_IMAGE",
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


def expect_status(response, expected, operation):
    """Report bounded status evidence without credentials, headers or bodies."""
    status, value = response
    accepted = (expected,) if isinstance(expected, int) else expected
    assert status in accepted, f"{operation}: expected HTTP {'/'.join(map(str, accepted))}, received {status}"
    return value


def native_fixture(directory):
    """Protected synthetic installation identity; not installed-native evidence."""
    home = directory / "synthetic-native-home"
    container = home / ".fleetworks"
    for path in (home, container):
        path.mkdir(mode=0o700)
    assert container.resolve().parent == home.resolve() and not container.is_symlink()
    identity = {"installation_marker": secrets.token_hex(32)}
    marker = container / "fixture-installation.json"
    native_review.private_file(marker, json.dumps(identity))
    assert json.loads(native_review.read_private(marker, 4096)) == identity
    return {"request_id": "compose-register", "workstation_ip": "192.0.2.61", **identity}


def register_native(port, token, registration):
    receipt = expect_status(request(port, "/api/v1/connect/registrations/ensure", token, registration),
                            (200, 201), "native ensure")
    fields = {"deployment_id", "workstation_id", "registration_id", "connect_id", "status", "revision"}
    assert isinstance(receipt, dict) and fields <= receipt.keys() <= fields | {"workstation_ip"}, "native receipt fields inconsistent"
    assert (isinstance(receipt["workstation_id"], str)
            and re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", receipt["workstation_id"])
            and receipt["connect_id"] == receipt["workstation_id"]), "native Connect ID inconsistent"
    assert (all(isinstance(receipt[key], str) and receipt[key] for key in ("deployment_id", "registration_id"))
            and type(receipt["revision"]) is int and receipt["revision"] > 0
            and receipt["status"] == "active"), "native receipt identity/revision inconsistent"
    replay = expect_status(request(port, "/api/v1/connect/registrations/ensure", token, registration), 200, "native ensure replay")
    assert replay == receipt, "ensure replay changed identity"
    value = expect_status(request(port, "/api/v1/connect/registrations/context", token, {
        "registration_id": receipt["registration_id"], "expected_revision": receipt["revision"],
    }), 200, "native context")
    context = value.get("context") if isinstance(value, dict) else None
    fields = {"deployment_id", "workstation_id", "registration_id", "connect_id", "registration_revision",
              "context_revision", "scope_revision", "configuration_epoch", "access_epoch"}
    assert isinstance(context, dict) and fields <= context.keys() <= fields | {"workstation_ip"}, "native context fields inconsistent"
    assert (all(context[key] == receipt[key] for key in ("deployment_id", "workstation_id", "registration_id", "connect_id"))
            and context.get("workstation_ip") == receipt.get("workstation_ip")
            and type(context["registration_revision"]) is int and context["registration_revision"] == receipt["revision"]), "native context differs from receipt"
    assert (all(type(context[key]) is int and context[key] > 0 for key in ("context_revision", "scope_revision"))
            and all(isinstance(context[key], str) and context[key] for key in ("configuration_epoch", "access_epoch"))), "native context revisions/epochs inconsistent"
    return context


def check_purpose_isolation(port, token, relay, health, registration):
    assert len({token, relay, health}) == 3, "purpose credentials were reused"
    expect_status(request(port, "/api/v1/health/ready"), 401, "unauthenticated readiness")
    expect_status(request(port, "/api/v1/connect/registrations/ensure", relay, registration), 403, "WebRelay registration denial")
    # A well-formed body isolates permission denial from JSON extraction errors.
    tool = {"request_id": "denied-health", "tool_id": "dork_generate", "inputs": {"objective": "Synthetic fixture"}}
    expect_status(request(port, "/api/v1/tools/run", health, tool), 403, "health-only Tools denial")
    expect_status(request(port, "/api/v1/jobs", token), 401, "contextless activity denial")


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
        value = expect_status(request(port, f"/api/v1/jobs/detail?id={receipt['job_id']}", token, headers=proof),
                              200, "Atlas job detail")
        if value["job"]["state"] == "failed":
            assert value["job"]["error_summary"] == "atlas_classifier_model_not_configured", "Atlas fixture failed for an unexpected reason"
            assert value["job"]["attempts_used"] == 1, "Atlas fixture retry count changed"
            return
        time.sleep(0.25)
    raise RuntimeError("Packaged Atlas worker did not settle the fixture delivery")


def exercise_tools(port, token, proof, request_id):
    payload = {
        "request_id": request_id, "tool_id": "dork_generate",
        "inputs": {"objective": "Find public annual reports"},
        "context": submission_context(port, token, proof),
    }
    receipt = expect_status(request(port, "/api/v1/tools/run", token, payload, headers=proof), 202, "Tools admission")
    assert receipt["job_id"] == receipt["call_id"], "durable attributed tool admission failed"
    assert request(port, "/api/v1/tools/run", token, payload, headers=proof) == (202, receipt), "tool replay changed receipt"
    result = "/api/v1/tools/result?id=" + receipt["call_id"]
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        value = expect_status(request(port, result, token, headers=proof), 200, "Tools result")
        if value["status"] not in {"queued", "running", "waiting"}:
            assert value["status"] == "completed", "Tools fixture reached an unexpected terminal state"
            assert value["result"]["observations"] and value["result"]["credits_charged"] == 0
            wait_job(port, token, proof, receipt["job_id"], 15)
            return result, value
        time.sleep(0.1)
    raise RuntimeError("Packaged Tools service did not complete the local fixture")


def wait_job(port, token, proof, job_id, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        detail = expect_status(request(port, "/api/v1/jobs/detail?id=" + job_id, token, headers=proof), 200, "registered job detail")
        assert detail["private_details_available"], "requester membership did not authorize job detail"
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
    expect_status(request(web_port, "/api/v1/jobs?workspace_id=not-a-child", headers=browser), 400, "legacy workspace query denial")


def isolated_environment(environment):
    return {name: value for name, value in environment.items()
            if not name.startswith(("ARGOS_", "FLEETWORKS_", "COMPOSE_", "SURREAL_"))
            and not name.endswith(("_API_KEY", "_API_KEY_FALLBACK", "_API_TOKEN", "_API_TOKEN_FALLBACK"))}


def compose_command(workspace, *, web=False, dev=False, atlas=False):
    if dev and not web:
        raise ValueError("Development Web requires --web")
    command = ["docker", "compose", "--env-file", str(workspace / "compose.env"),
               "-f", str(native_review.ROOT / "deploy/compose.native-review.yml")]
    for enabled, name in ((web, "compose.yml"), (dev, "compose.dev.yml"), (atlas, "compose.atlas.yml")):
        if enabled:
            command.extend(("-f", str(native_review.ROOT / "deploy" / name)))
    return command


def check_container_images(command, environment, expected):
    containers = subprocess.check_output([*command, "ps", "--all", "--quiet"],
                                         env=environment, text=True, timeout=30).splitlines()
    if not containers:
        raise RuntimeError("The dedicated Compose project has no containers to verify")
    rows = subprocess.check_output([
        "docker", "inspect", "--type", "container", "--format",
        '{{ index .Config.Labels "com.docker.compose.service" }} {{ .Image }}', *containers,
    ], env=environment, text=True, timeout=30).splitlines()
    actual = dict(row.split() for row in rows)
    if len(actual) != len(containers) or actual != expected:
        raise RuntimeError("Compose container images differ from the downloaded image IDs")
    print(f"Verified exact image IDs for {len(actual)} Compose services")


def require_unused_project(environment, platform):
    """Reject a foreign project before this verifier can build, start or stop it."""
    daemon = subprocess.check_output(
        ["docker", "version", "--format", "{{.Server.Os}}/{{.Server.Arch}}"],
        cwd=native_review.ROOT, env=environment, text=True, timeout=30,
    ).strip()
    if daemon != platform:
        raise ValueError("The selected Docker daemon does not match the requested platform")
    project = environment["COMPOSE_PROJECT_NAME"]
    containers = subprocess.check_output(
        ["docker", "ps", "--all", "--filter", f"label=com.docker.compose.project={project}", "--quiet"],
        cwd=native_review.ROOT, env=environment, text=True, timeout=30,
    ).strip()
    if containers:
        raise ValueError("The selected Docker project already has containers; choose a fresh review name")


def verify(name, platform, *, web=False, dev=False, atlas=False, no_build=False,
           image_ids=None, source_journey=False):
    api_port, web_port = fixture_ports()
    revision = subprocess.check_output(["git", "rev-parse", "--verify", "HEAD"],
                                       cwd=native_review.ROOT, text=True).strip()
    overrides = {key: os.environ[key] for key in IMAGE_OVERRIDES if key in os.environ} if no_build else None
    receipt = native_review.provision(native_review.ROOT, name, api_port, platform, revision,
                                      web_port=web_port if web else None, images=overrides)
    workspace = Path(receipt["workspace"])
    environment = {**isolated_environment(os.environ), **receipt["environment"]}
    # No cleanup is entered until both daemon and preexisting-project checks pass.
    require_unused_project(environment, platform)
    command = compose_command(workspace, web=web, dev=dev, atlas=atlas)
    # Keep this journey distinct from native_cutover_smoke nodes 61/62.
    registration = {**native_fixture(workspace), "workstation_ip": "192.0.2.71"}
    other_directory = workspace / "other-node-fixture"
    other_directory.mkdir(mode=0o700)
    other_registration = {**native_fixture(other_directory), "request_id": "compose-other-node", "workstation_ip": "192.0.2.72"}

    def run(*arguments, check=True, timeout=60):
        return subprocess.run([*command, *arguments], cwd=native_review.ROOT, env=environment,
                              check=check, timeout=timeout)

    if not no_build:
        environment.update(VERSION=(native_review.ROOT / "version.txt").read_text().strip(),
                           IMAGE_TAG=receipt["image_tag"])
        subprocess.run(["docker", "buildx", "bake", "--load", "-f", "deploy/docker-bake.hcl",
                        "backend", *(["web"] if web and not dev else [])],
                       cwd=native_review.ROOT, env=environment, check=True, timeout=3600)
        if dev:
            run("build", "web", timeout=2400)

    def start():
        run("up", "--detach", "--no-build", "--pull", "never", "--wait", "--wait-timeout", "240", timeout=300)
        run("ps", "--all")
        ready = "/api/v1/health/ready"
        token = (workspace / "secrets/native-token").read_text().strip()
        relay = (workspace / "secrets/web-relay-token").read_text().strip()
        health = (workspace / "secrets/gateway-health-token").read_text().strip()
        check_purpose_isolation(api_port, token, relay, health, registration)
        context = register_native(api_port, token, registration)
        status, direct = request(api_port, ready, health)
        assert status == 200 and direct.get("ready") and direct.get("storage")
        browser = web_connect(web_port, context) if web else None
        if web:
            relay_status, relayed = request(web_port, ready, headers=browser)
            assert relay_status == 200 and relayed.get("storage") == direct["storage"]
        if image_ids is not None:
            check_container_images(command, environment, image_ids)
        return token, context, browser

    try:
        token, context, browser = start()
        proof = native_headers(context)
        result_path, completed = exercise_tools(api_port, token, proof, "compose-native-dork")
        if web:
            web_result, _ = exercise_tools(web_port, None, browser, "compose-web-dork")
            exercise_activity(api_port, web_port, token, context, browser, completed["job_id"])
        else:
            status, page = request(api_port, "/api/v1/jobs?limit=25", token, headers=proof)
            assert status == 200 and any(row["id"] == completed["job_id"] for row in page["rows"])
            assert request(api_port, "/api/v1/logs?job_id=" + completed["job_id"], token, headers=proof)[0] == 200
        other = register_native(api_port, token, other_registration)
        assert request(api_port, "/api/v1/jobs/detail?id=" + completed["job_id"], token,
                       headers=native_headers(other))[0] == 404
        if web:
            assert request(api_port, web_result, token, headers=native_headers(other))[0] == 404
            web_logout(web_port, browser)
        if atlas:
            exercise_atlas(api_port, token, proof)
        if source_journey:
            subprocess.run([sys.executable, str(native_review.ROOT / "deploy/native_cutover_smoke.py"), "--name", name],
                           cwd=native_review.ROOT, env=environment, check=True, timeout=1200)
        # Reopen the actual persistent deployment and prove completed output survives.
        run("stop", timeout=150)
        token, context, browser = start()
        assert request(api_port, result_path, token, headers=native_headers(context)) == (200, completed)
        if web:
            web_logout(web_port, browser)
    finally:
        failed = sys.exc_info()[0] is not None
        try:
            if failed:
                run("logs", "--no-color", "--tail", "100", check=False, timeout=30)
        finally:
            run("down", check=not failed, timeout=150)
    print("Standalone backend registration, purpose isolation, attributed Tools/activity and disk restart passed")
    print("Backend integration only; installed-native GUI and real Rust-BERT inference are separate gates")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--name", required=True, help="fresh dedicated native-review name; never reuse an existing project")
    parser.add_argument("--platform", choices=("linux/amd64", "linux/arm64"), required=True)
    parser.add_argument("--web", action="store_true", help="also exercise actual Web Connect/session/relay/logout")
    parser.add_argument("--dev", action="store_true", help="build the development Web overlay (requires --web)")
    parser.add_argument("--atlas", action="store_true", help="also exercise private Atlas dispatch")
    parser.add_argument("--source-journey", action="store_true", help="run real synthetic seed/search/rebuild/train/activation/privacy journey")
    parser.add_argument("--no-build", action="store_true", help="use explicit preloaded images without building or pulling")
    parser.add_argument("--image-ids", type=Path, help="expected image ID for every Compose service")
    args = parser.parse_args()
    if args.dev and (not args.web or args.no_build):
        parser.error("--dev requires --web and a development image build")
    expected = json.loads(args.image_ids.read_text()) if args.image_ids else None
    verify(args.name, args.platform, web=args.web, dev=args.dev, atlas=args.atlas,
           no_build=args.no_build, image_ids=expected, source_journey=args.source_journey)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
