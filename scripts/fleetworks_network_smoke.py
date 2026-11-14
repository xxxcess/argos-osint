"""Verify the Fleetworks API, worker, and bootstrap against a disposable SurrealDB."""

import argparse
import base64
import contextlib
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time

def binaries(description):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument("--host", required=True, type=Path, help="path to the network-enabled argos-api-host executable")
    parser.add_argument("--surreal", required=True, type=Path, help="path to the checksum-verified SurrealDB 3.3.4 executable")
    options = parser.parse_args()
    binary = options.host.resolve(strict=True)
    surreal = options.surreal.resolve(strict=True)
    if not subprocess.check_output([surreal, "version"], text=True).startswith("3.3.4"):
        parser.error("the smoke requires the repository-pinned SurrealDB 3.3.4")
    return binary, surreal


safe_env = {
    key: value for key, value in os.environ.items()
    if not key.startswith(("ARGOS_", "FLEETWORKS_", "SURREAL_"))
    and not key.endswith(("_API_KEY", "_API_KEY_FALLBACK", "_API_TOKEN", "_API_TOKEN_FALLBACK"))
}


def free_ports(count):
    with contextlib.ExitStack() as resources:
        sockets = [resources.enter_context(socket.socket()) for _ in range(count)]
        for listener in sockets:
            listener.bind(("127.0.0.1", 0))
        return [listener.getsockname()[1] for listener in sockets]


def request(port, path, headers=None, body=None):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
    try:
        conn.request("POST" if body is not None else "GET", path, body, headers or {})
        response = conn.getresponse()
        raw = response.read()
        try:
            decoded = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            decoded = None
        return response.status, decoded
    finally:
        conn.close()


@contextlib.contextmanager
def process(command, env, log):
    with log.open("wb") as output:
        child = subprocess.Popen(command, env=env, stdout=output, stderr=subprocess.STDOUT)
        try:
            yield child
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=22)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()


def wait_for(child, port, path, headers=None):
    deadline = time.monotonic() + 35
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise RuntimeError(f"service exited with {child.returncode}")
        try:
            status, value = request(port, path, headers)
            if status == 200:
                return value
        except (OSError, http.client.HTTPException):
            pass
        time.sleep(0.15)
    raise RuntimeError("service did not become ready")


def instance_identity(port, password):
    credentials = base64.b64encode(("root:" + password).encode()).decode()
    headers = {
        "Authorization": "Basic " + credentials,
        "Surreal-NS": "fleetworks",
        "Surreal-DB": "fleetworks",
        "Content-Type": "application/surrealql",
        "Accept": "application/json",
    }
    status, payload = request(port, "/sql", headers, "SELECT VALUE instance_id FROM ONLY storage_meta:identity;")
    assert status == 200 and isinstance(payload, list) and payload[0]["status"] == "OK", (status, payload)
    identity = payload[0]["result"]
    assert isinstance(identity, str) and identity
    return identity


def write_private_file(path, text):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.write(fd, text.encode())
    finally:
        os.close(fd)


def create_native_credentials(workspace, token, scope="smoke-scope", epoch="smoke-epoch"):
    directory = workspace / "credentials"
    directory.mkdir(mode=0o700, exist_ok=True)
    token_file = directory / "native-token"
    write_private_file(token_file, token + "\n")
    manifest = {
        "version": 1,
        "scope_id": scope,
        "access_epoch": epoch,
        "credentials": [{
            "id": "smoke-native",
            "purpose": "native",
            "permissions": [
                "read_backend",
                "register_workstation",
                "rebind_workstation",
                "manage_configuration",
                "maintain_backend",
                "submit_interactive",
                "read_workstation_activity",
                "retry_job",
                "cancel_job",
            ],
            "token_file": token_file.name,
        }],
    }
    manifest_file = directory / "gateway-credentials.json"
    write_private_file(manifest_file, json.dumps(manifest))
    return manifest_file


def register_workstation(port, headers, marker):
    status, receipt = request(
        port,
        "/api/v1/connect/registrations/ensure",
        {**headers, "Content-Type": "application/json"},
        json.dumps({
            "request_id": "smoke-register",
            "workstation_ip": "192.0.2.61",
            "installation_marker": marker,
        }),
    )
    assert status in (200, 201), (status, receipt)
    proof_headers = {
        **headers,
        "x-fleetworks-registration": receipt["registration_id"],
        "x-fleetworks-registration-revision": str(receipt["revision"]),
    }
    return receipt, proof_headers


def submission_context(port, headers):
    status, current_ctx = request(
        port,
        "/api/v1/connect/registrations/context",
        {**headers, "Content-Type": "application/json"},
        json.dumps({
            "registration_id": headers["x-fleetworks-registration"],
            "expected_revision": int(headers["x-fleetworks-registration-revision"]),
        }),
    )
    assert status == 200 and "context" in current_ctx, (status, current_ctx)
    ctx = current_ctx["context"]
    status, defaults = request(port, "/api/v1/models/defaults", headers)
    assert status == 200, (status, defaults)
    return {
        "registration_id": ctx["registration_id"],
        "expected_registration_revision": ctx["registration_revision"],
        "expected_context_revision": ctx["context_revision"],
        "expected_configuration_revision": defaults.get("configuration_revision", 0),
    }


def exercise_tools(api, port, headers, request_id="local-dork-fixture"):
    start = "/api/v1/tools/run"
    body = json.dumps({
        "request_id": request_id,
        "tool_id": "dork_generate",
        "inputs": {"objective": "Find public annual reports"},
        "context": submission_context(port, headers),
    })
    auth = {**headers, "Content-Type": "application/json"}
    assert request(port, start, {"Content-Type": "application/json"}, body)[0] == 401
    assert request(port, start, {**auth, "Origin": "http://other.test:3000"}, body)[0] == 403
    status, accepted = request(port, start, auth, body)
    assert status == 202 and accepted.get("call_id") and accepted.get("job_id") == accepted["call_id"], (status, accepted)
    assert request(port, start, auth, body) == (202, accepted)
    assert request(port, start, auth, body.replace("annual", "quarterly"))[0] == 409
    result = "/api/v1/tools/result?id=" + accepted["call_id"]
    assert request(port, result)[0] == 401
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        assert api.poll() is None, "API exited before tool publication"
        status, value = request(port, result, headers)
        assert status == 200, (status, value)
        if value["status"] not in {"queued", "running", "waiting"}:
            break
        time.sleep(0.05)
    assert value["status"] == "completed", value
    tool_result = value["result"]
    assert tool_result["observations"] and tool_result["raw"] == ""
    assert tool_result["credits_charged"] == 0
    return result, value


def exercise_control(port, headers):
    auth = {**headers, "Content-Type": "application/json"}
    status, current = request(port, "/api/v1/models/defaults", headers)
    assert status == 200 and isinstance(current.get("configuration_revision"), int), (status, current)
    revision = current["configuration_revision"]
    upsert_body = json.dumps({
        "expected_revision": revision,
        "account_id": "local",
        "provider": "local",
        "base_url": "http://127.0.0.1:11434/v1",
    })
    assert request(port, "/api/v1/profile/accounts/upsert", {"Content-Type": "application/json"}, upsert_body)[0] == 401
    status, saved_account = request(port, "/api/v1/profile/accounts/upsert", auth, upsert_body)
    assert status == 200 and saved_account["saved"] is True, (status, saved_account)
    revision = saved_account["configuration_revision"]
    path = "/api/v1/models/defaults/set"
    body = json.dumps({
        "expected_revision": revision,
        "role": "brain_summary",
        "primary": {"provider": "local", "model": "example/offline-fixture"},
    })
    assert request(port, path, {"Content-Type": "application/json"}, body)[0] == 401
    status, saved_role = request(port, path, auth, body)
    assert status == 200 and saved_role["saved"] is True, (status, saved_role)
    revision = saved_role["configuration_revision"]
    status, value = request(port, "/api/v1/models/defaults", headers)
    assert status == 200 and value["defaults"]["brain_summary"]["model"] == "example/offline-fixture"
    export = "/api/v1/profile/export"
    assert request(port, export, auth, json.dumps({"expected_revision": revision, "include_credentials": False}))[0] == 400
    status, value = request(port, export, auth, json.dumps({"expected_revision": revision, "include_credentials": True}))
    assert status == 200 and isinstance(value, dict) and "schema_version" in value, (status, value)
    assert value["roles"]["brain_summary"]["primary"]["model"] == "example/offline-fixture", value


@contextlib.contextmanager
def native_runtime(workspace, binary, env, tag):
    (workspace / "staged-datasets").mkdir(mode=0o700, exist_ok=True)
    jobs_port, native_port = free_ports(2)
    jobs_token = secrets.token_hex(32)
    runtime_env = {
        **env,
        "FLEETWORKS_JOBS_TOKEN": jobs_token,
        "FLEETWORKS_JOBS_ORIGIN": f"http://127.0.0.1:{jobs_port}",
        "FLEETWORKS_JOBS_ALLOWED_HOSTS": f"127.0.0.1:{jobs_port}",
    }
    jobs_auth = {"Authorization": f"Bearer {jobs_token}"}
    with process(
        [binary, "--role", "jobs", "--port", str(jobs_port)],
        runtime_env,
        workspace / f"jobs-{tag}.log",
    ) as jobs:
        assert wait_for(jobs, jobs_port, "/api/v1/health/ready", jobs_auth)["service"] == "fleetworks-jobs"
        with process(
            [binary, "--role", "native-worker", "--port", str(native_port)],
            runtime_env,
            workspace / f"native-worker-{tag}.log",
        ):
            yield runtime_env


def wait_tool_pool(api_port, headers):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        status, value = request(api_port, "/api/v1/capabilities", headers)
        if status == 200 and "tool-execution" in value.get("features", []):
            return
        time.sleep(0.05)
    raise RuntimeError("manual_tools dispatch pool was not enabled")


def exercise(mode, tmp, binary, surreal):
    workspace = tmp / mode
    workspace.mkdir()
    (workspace / "exports").mkdir(mode=0o700)
    db_port, api_port, worker_port, brief_port = free_ports(4)
    root_key, runtime_key, token = (secrets.token_hex(32) for _ in range(3))
    manifest_file = create_native_credentials(workspace, token)
    marker = secrets.token_hex(32)
    db_env = {**safe_env, "SURREAL_USER": "root", "SURREAL_PASS": root_key}
    db_path = "memory" if mode == "memory" else f'rocksdb://{workspace / "db"}?versioned=false&sync=every'
    db = [surreal, "start", "--no-banner", "--log", "warn", "--bind", f"127.0.0.1:{db_port}", db_path]
    origins = "http://127.0.0.1:3000"
    env = {
        **safe_env, "FLEETWORKS_WORKSPACE": str(workspace), "ARGOS_HOME": str(workspace),
        "FLEETWORKS_STORAGE_URL": f"ws://127.0.0.1:{db_port}",
        "FLEETWORKS_STORAGE_PASSWORD": runtime_key,
        "FLEETWORKS_API_CREDENTIALS_FILE": str(manifest_file),
        "FLEETWORKS_ACCESS_SCOPE_ID": "smoke-scope",
        "FLEETWORKS_ACCESS_EPOCH": "smoke-epoch",
        "FLEETWORKS_API_ALLOWED_HOSTS": f"127.0.0.1:{api_port}",
        "FLEETWORKS_API_ALLOWED_ORIGINS": origins,
    }
    headers = {"Authorization": "Bearer " + token}
    generations = []
    saved_tool = None
    proof_headers = None
    for attempt in (1, 2):
        with process(db, db_env, workspace / f"surreal-{attempt}.log") as database:
            wait_for(database, db_port, "/health")
            # Bootstrap is privileged and repeatable. Runtime receives only
            # its database editor credential and API token.
            subprocess.run(
                [binary, "--role", "bootstrap"],
                env={**env, "FLEETWORKS_STORAGE_ROOT_PASSWORD": root_key},
                check=True, timeout=35,
            )
            generations.append(instance_identity(db_port, root_key))
            with native_runtime(workspace, binary, env, f"attempt-{attempt}") as runtime_env:
                with process([binary, "--role", "api", "--port", str(api_port)], runtime_env, workspace / f"api-{attempt}.log") as api:
                    ready = wait_for(api, api_port, "/api/v1/health/ready", headers)
                    assert ready["ready"] and ready["storage"] and ready["api_version"] == 1
                    assert request(api_port, "/api/v1/health/ready")[0] == 401
                    assert request(api_port, "/api/v1/health/ready", {**headers, "Host": "example.test:8080"})[0] == 403
                    assert request(api_port, "/api/v1/health/ready", {**headers, "Origin": "http://example.test:3000"})[0] == 403
                    assert request(api_port, "/api/v1/jobs", headers)[0] == 401
                    _, proof_headers = register_workstation(api_port, headers, marker)
                    wait_tool_pool(api_port, proof_headers)
                    for path, key in (("tools", "tools"), ("models/defaults", "defaults"), ("profile", "accounts"), ("logs", "rows")):
                        status, value = request(api_port, f"/api/v1/{path}", {**proof_headers, "Origin": origins})
                        assert status == 200 and key in value, (path, status, value)
                    if attempt == 1:
                        saved_tool = exercise_tools(api, api_port, proof_headers)
                        exercise_control(api_port, proof_headers)
                    elif mode == "disk":
                        path, value = saved_tool
                        assert request(api_port, path, proof_headers) == (200, value)
                    if attempt == 1:
                        with contextlib.ExitStack() as workers:
                            roles = (("worker", worker_port), ("brief-worker", brief_port))
                            children = [(role, port, workers.enter_context(process(
                                [binary, "--role", role, "--port", str(port)], runtime_env,
                                workspace / f"{role}.log",
                            ))) for role, port in roles]
                            time.sleep(1)
                            for role, port, child in children:
                                assert child.poll() is None, f"{role} did not stay up"
                                with socket.socket() as probe:
                                    assert probe.connect_ex(("127.0.0.1", port)) != 0, f"{role} published an HTTP listener"
            if attempt == 1 and mode == "memory":
                exercise_brain(workspace, binary, env, api_port, proof_headers)
                exercise_private_tools(workspace, binary, env, api_port, proof_headers)
                exercise_control_jobs(workspace, binary, env, api_port, proof_headers)
                exercise_execution(workspace, binary, env, proof_headers)
    assert (generations[0] == generations[1]) == (mode == "disk")
    print(f"{mode}: privileged bootstrap, scoped API and worker roles, ingress, retained reads, durable tool execution and database generation passed", flush=True)


def exercise_execution(workspace, binary, env, public_headers):
    port = free_ports(1)[0]
    token = secrets.token_hex(32)
    execution_env = {
        **env,
        "FLEETWORKS_EXECUTION_TOKEN": token,
        "FLEETWORKS_EXECUTION_ALLOWED_HOSTS": f"127.0.0.1:{port}",
    }
    ready = "/api/v1/health/ready"
    attempt = "/api/v1/execution/attempt"
    auth = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
    with process([binary, "--role", "execution", "--port", str(port)], execution_env, workspace / "execution.log") as child:
        assert wait_for(child, port, ready, auth)["service"] == "fleetworks-execution"
        assert request(port, ready, public_headers)[0] == 401
        assert request(port, attempt, public_headers, "{}")[0] == 401
        assert request(port, attempt, {**auth, "Host": "untrusted.test"}, "{}")[0] == 403
        assert request(port, attempt, auth, "{}")[0] == 400
        assert request(port, "/api/v1/jobs", auth)[0] == 404
    print("Execution: private readiness, bearer/Host isolation and typed attempt validation passed", flush=True)


def exercise_brain(workspace, binary, env, api_port, public_headers):
    brain_port = free_ports(1)[0]
    token = secrets.token_hex(32)
    brain_env = {
        **env,
        "FLEETWORKS_INTERNAL_TOKEN": token,
        "FLEETWORKS_BRAIN_ALLOWED_HOSTS": f"127.0.0.1:{brain_port}",
    }
    api_env = {**brain_env, "FLEETWORKS_BRAIN_ORIGIN": f"http://127.0.0.1:{brain_port}"}
    auth = {"Authorization": "Bearer " + token}
    ready = "/api/v1/health/ready"
    detail = "/api/v1/articles/detail?run=missing-run&id=missing-article"
    datasets = workspace / "datasets"
    datasets.mkdir(exist_ok=True)
    snapshot = datasets / "actors-fixture.csv"
    contents = (
        'actor_id,actor_name,actor_type,aliases,name_normalized\n'
        'fixture:harbor,Harbor Council,organization,"[""Port Council""]",harbor council\n'
    ).encode()
    snapshot.write_bytes(contents)
    digest = hashlib.sha256(contents).hexdigest()
    command = [binary, "--role", "seed-actors", "--file", snapshot, "--expected-sha256"]
    invalid = subprocess.run(
        [*command, "0" * 64], env=env, capture_output=True, timeout=30,
    )
    assert invalid.returncode and b"SHA-256" in invalid.stderr, invalid.stderr
    for _ in range(2):
        receipt = subprocess.check_output([*command, digest], env=env, timeout=30)
        data = json.loads(receipt)
        assert data["dataset_sha256"] == digest and data["complete"] and data["actors"] == 1, data
    with process([binary, "--role", "api", "--port", str(api_port)], api_env, workspace / "api-brain.log") as api:
        wait_for(api, api_port, ready, public_headers)
        with process([binary, "--role", "brain", "--port", str(brain_port)], brain_env, workspace / "brain.log") as brain:
            assert wait_for(brain, brain_port, ready, auth)["service"] == "fleetworks-brain"
            assert request(brain_port, ready, public_headers)[0] == 401
            assert request(brain_port, detail, auth)[0] == 401
            assert request(brain_port, "/api/v1/jobs", auth)[0] == 404
            assert request(api_port, detail, public_headers)[0] == 404
            path = "/api/v1/actors/search?query=Harbor%20Council"
            status, names = request(api_port, path, public_headers)
            assert status == 200 and len(names["rows"]) == 1, names
            actor = names["rows"][0]
            assert actor["catalog_id"] == "fixture:harbor", actor
            status, aliases = request(
                api_port, "/api/v1/actors/search?query=Port%20Council&mode=alias", public_headers,
            )
            assert status == 200 and [row["actor_id"] for row in aliases["rows"]] == [actor["actor_id"]], aliases
        assert request(api_port, detail, public_headers)[0] == 502
        assert request(api_port, "/api/v1/tools", public_headers)[0] == 200
    print("Brain: pinned actor seed and replay, private search, retained route relay and outage isolation passed", flush=True)


def exercise_private_tools(workspace, binary, env, api_port, public_headers):
    port = free_ports(1)[0]
    token = secrets.token_hex(32)
    auth = {"Authorization": "Bearer " + token}
    ready = "/api/v1/health/ready"
    catalog = "/api/v1/tools"
    with native_runtime(workspace, binary, env, "private-tools") as runtime_env:
        private_env = {
            **runtime_env,
            "FLEETWORKS_TOOLS_TOKEN": token,
            "FLEETWORKS_TOOLS_ALLOWED_HOSTS": f"127.0.0.1:{port}",
        }
        gateway_env = {
            **private_env,
            "FLEETWORKS_TOOLS_ORIGIN": f"http://127.0.0.1:{port}",
        }
        with process([binary, "--role", "api", "--port", str(api_port)], gateway_env, workspace / "api-tools.log") as api:
            wait_for(api, api_port, ready, public_headers)
            wait_tool_pool(api_port, public_headers)
            with process([binary, "--role", "tools", "--port", str(port)], private_env, workspace / "tools.log") as tools:
                health = wait_for(tools, port, ready, auth)
                assert health["service"] == "fleetworks-tools" and health["storage"]
                assert request(port, catalog, public_headers)[0] == 401
                assert request(port, catalog, auth)[0] == 401
                assert request(api_port, catalog, auth)[0] == 401
                assert request(port, ready, {**auth, "Host": "other.test"})[0] == 403
                assert request(port, "/api/v1/jobs", auth)[0] == 404
                status, listed = request(api_port, catalog, public_headers)
                assert status == 200 and listed["tools"], (status, listed)
                path, result = exercise_tools(api, api_port, public_headers, "private-dork-fixture")
                assert request(api_port, path, public_headers) == (200, result)
            assert request(api_port, catalog, public_headers)[0] == 502
            assert request(api_port, path, public_headers)[0] == 502
            assert request(api_port, "/api/v1/jobs", public_headers)[0] == 200
    print("Tools: private readiness, bearer isolation, execution relay and outage isolation passed", flush=True)


def exercise_control_jobs(workspace, binary, env, api_port, public_headers):
    control_port, jobs_port = free_ports(2)
    control_token, jobs_token = (secrets.token_hex(32) for _ in range(2))
    control_env = {
        **env,
        "FLEETWORKS_CONTROL_TOKEN": control_token,
        "FLEETWORKS_CONTROL_ALLOWED_HOSTS": f"127.0.0.1:{control_port}",
    }
    jobs_env = {
        **env,
        "FLEETWORKS_JOBS_TOKEN": jobs_token,
        "FLEETWORKS_JOBS_ALLOWED_HOSTS": f"127.0.0.1:{jobs_port}",
    }
    gateway_env = {
        **env,
        "FLEETWORKS_CONTROL_TOKEN": control_token,
        "FLEETWORKS_CONTROL_ORIGIN": f"http://127.0.0.1:{control_port}",
        "FLEETWORKS_JOBS_TOKEN": jobs_token,
        "FLEETWORKS_JOBS_ORIGIN": f"http://127.0.0.1:{jobs_port}",
    }
    control_auth = {"Authorization": f"Bearer {control_token}"}
    jobs_auth = {"Authorization": f"Bearer {jobs_token}"}
    ready = "/api/v1/health/ready"
    profile = "/api/v1/profile"
    defaults = "/api/v1/models/defaults"
    jobs_path = "/api/v1/jobs"
    with process([binary, "--role", "api", "--port", str(api_port)], gateway_env, workspace / "api-control-jobs.log") as api:
        wait_for(api, api_port, ready, public_headers)
        with process([binary, "--role", "jobs", "--port", str(jobs_port)], jobs_env, workspace / "jobs.log") as jobs:
            assert wait_for(jobs, jobs_port, ready, jobs_auth)["service"] == "fleetworks-jobs"
            with process([binary, "--role", "control", "--port", str(control_port)], control_env, workspace / "control.log") as control:
                assert wait_for(control, control_port, ready, control_auth)["service"] == "fleetworks-control"
                assert request(control_port, profile, public_headers)[0] == 401
                assert request(jobs_port, jobs_path, public_headers)[0] == 401
                assert request(control_port, profile, jobs_auth)[0] == 401
                assert request(jobs_port, jobs_path, control_auth)[0] == 401
                assert request(control_port, profile, control_auth)[0] == 401
                assert request(jobs_port, jobs_path, jobs_auth)[0] == 401
                assert request(api_port, profile, control_auth)[0] == 401
                assert request(api_port, jobs_path, jobs_auth)[0] == 401
                assert request(control_port, jobs_path, control_auth)[0] == 404
                assert request(jobs_port, profile, jobs_auth)[0] == 404
                assert request(jobs_port, "/api/v1/live", jobs_auth)[0] == 404
                status, relayed_profile = request(api_port, profile, public_headers)
                assert status == 200 and isinstance(relayed_profile.get("configuration_revision"), int), (status, relayed_profile)
                for path in (jobs_path, "/api/v1/logs?limit=1"):
                    status, rows = request(api_port, path, public_headers)
                    assert status == 200 and isinstance(rows["rows"], list), (path, status, rows)
                missing = "/api/v1/jobs/detail?id=missing-control-jobs-fixture"
                assert request(api_port, missing, public_headers)[0] == 404
                auth = {**public_headers, "Content-Type": "application/json"}
                assert request(api_port, "/api/v1/jobs/cancel", auth, '{"id":"missing-control-jobs-fixture","request_id":"cancel-missing-fixture"}')[0] == 404
                status, current = request(api_port, defaults, public_headers)
                assert status == 200, (status, current)
                change = json.dumps({
                    "expected_revision": current["configuration_revision"],
                    "role": "atlas_analyst",
                    "primary": {"provider": "local", "model": "offline-control-fixture"},
                })
                status, saved = request(api_port, defaults + "/set", auth, change)
                assert status == 200 and saved["saved"] is True, (status, saved)
                observed = request(api_port, defaults, public_headers)
                assert observed[0] == 200 and "offline-control-fixture" in json.dumps(observed[1])
                status, export = request(api_port, profile + "/export", auth, json.dumps({
                    "expected_revision": saved["configuration_revision"],
                    "include_credentials": True,
                }))
                assert status == 200 and export["roles"]["atlas_analyst"]["primary"]["model"] == "offline-control-fixture", (status, export)
            assert request(api_port, profile, public_headers)[0] == 502
            assert request(api_port, defaults, public_headers)[0] == 502
            assert request(api_port, jobs_path, public_headers)[0] == 200
        assert request(api_port, jobs_path, public_headers)[0] == 502
        assert request(api_port, "/api/v1/tools", public_headers)[0] == 200
    print("Control and Jobs: dedicated credentials, retained route relay, workspace writes and independent outages passed", flush=True)


def exercise_nlp(tmp, binary):
    port = free_ports(1)[0]
    token = secrets.token_hex(32)
    env = {
        **safe_env,
        "FLEETWORKS_INTERNAL_TOKEN": token,
        "FLEETWORKS_NLP_ALLOWED_HOSTS": f"127.0.0.1:{port}",
    }
    with process([binary, "--role", "nlp", "--port", str(port)], env, tmp / "nlp.log") as nlp:
        path = "/api/v1/health/ready"
        auth = {"Authorization": f"Bearer {token}"}
        assert wait_for(nlp, port, path, auth) == {
            "service": "fleetworks-nlp", "api_version": 1, "ready": True,
        }
        assert request(port, path)[0] == 401
        assert request(port, path, {**auth, "Host": "other.test"})[0] == 403
    print("NLP: warm database-free role and authenticated ingress passed", flush=True)


def exercise_classifier(tmp, binary):
    port = free_ports(1)[0]
    token = secrets.token_hex(32)
    workspace = tmp / "classifier-home"
    workspace.mkdir()
    env = {
        **safe_env, "ARGOS_HOME": str(workspace), "FLEETWORKS_WORKSPACE": str(workspace),
        "FLEETWORKS_INTERNAL_TOKEN": token,
        "FLEETWORKS_CLASSIFIER_ALLOWED_HOSTS": f"127.0.0.1:{port}",
    }
    with process([binary, "--role", "classifier", "--port", str(port)], env, tmp / "classifier.log") as worker:
        path = "/api/v1/health/ready"
        auth = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
        ready = wait_for(worker, port, path, auth)
        assert ready["service"] == "fleetworks-classifier" and ready["ready"]
        assert ready["model_state"] == "unverified" and ready["active_model"] is None
        assert request(port, path)[0] == 401
        status, error = request(port, "/api/v1/classifier/pin", auth, '{"version":1,"requested":null}')
        assert status == 424 and error["error"] == "classifier_model_not_configured", (status, error)
    print("Classifier: database-free authenticated role, distinct transport and model state passed", flush=True)


if __name__ == "__main__":
    binary, surreal = binaries(__doc__)
    with tempfile.TemporaryDirectory(prefix="fleetworks-native-network-") as directory:
        tmp = Path(directory)
        try:
            for mode in ("memory", "disk"):
                exercise(mode, tmp, binary, surreal)
            exercise_nlp(tmp, binary)
            exercise_classifier(tmp, binary)
        except Exception:
            for log in sorted(tmp.rglob("*.log")):
                print(log.name, log.read_text()[-1500:])
            raise
