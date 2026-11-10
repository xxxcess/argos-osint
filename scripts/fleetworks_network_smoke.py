"""Verify the Fleetworks API, worker, and bootstrap against a disposable SurrealDB."""

import argparse
import base64
import contextlib
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


def exercise(mode, tmp, binary, surreal):
    workspace = tmp / mode
    workspace.mkdir()
    db_port, api_port, worker_port = free_ports(3)
    root_key, runtime_key, token = (secrets.token_hex(32) for _ in range(3))
    db_env = {**safe_env, "SURREAL_USER": "root", "SURREAL_PASS": root_key}
    db_path = "memory" if mode == "memory" else f'rocksdb://{workspace / "db"}?versioned=false&sync=every'
    db = [surreal, "start", "--no-banner", "--log", "warn", "--bind", f"127.0.0.1:{db_port}", db_path]
    origins = "http://127.0.0.1:3000"
    env = {
        **safe_env, "FLEETWORKS_WORKSPACE": str(workspace), "ARGOS_HOME": str(workspace),
        "FLEETWORKS_STORAGE_URL": f"ws://127.0.0.1:{db_port}",
        "FLEETWORKS_STORAGE_PASSWORD": runtime_key,
        "FLEETWORKS_API_TOKEN": token,
        "FLEETWORKS_API_ALLOWED_HOSTS": f"127.0.0.1:{api_port}",
        "FLEETWORKS_API_ALLOWED_ORIGINS": origins,
    }
    headers = {"Authorization": "Bearer " + token}
    generations = []
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
            with process([binary, "--role", "api", "--port", str(api_port)], env, workspace / f"api-{attempt}.log") as api:
                ready = wait_for(api, api_port, "/api/v1/health/ready", headers)
                assert ready["ready"] and ready["storage"] and ready["api_version"] == 1
                assert request(api_port, "/api/v1/health/ready")[0] == 401
                assert request(api_port, "/api/v1/health/ready", {**headers, "Host": "example.test:8080"})[0] == 403
                assert request(api_port, "/api/v1/health/ready", {**headers, "Origin": "http://example.test:3000"})[0] == 403
                for path, key in (("tools", "tools"), ("models/defaults", "defaults"), ("profile", "workspace"), ("logs", "rows")):
                    status, value = request(api_port, f"/api/v1/{path}", {**headers, "Origin": origins})
                    assert status == 200 and key in value, (path, status, value)
                if attempt == 1:
                    with process([binary, "--role", "worker", "--port", str(worker_port)], env, workspace / "worker.log") as worker:
                        time.sleep(1)
                        assert worker.poll() is None, "background process did not stay up"
                        with socket.socket() as probe:
                            assert probe.connect_ex(("127.0.0.1", worker_port)) != 0, "worker published an HTTP listener"
    assert (generations[0] == generations[1]) == (mode == "disk")
    print(f"{mode}: privileged bootstrap, scoped API and worker roles, ingress, retained reads and database generation passed", flush=True)


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


if __name__ == "__main__":
    binary, surreal = binaries(__doc__)
    with tempfile.TemporaryDirectory(prefix="fleetworks-native-network-") as directory:
        tmp = Path(directory)
        try:
            for mode in ("memory", "disk"):
                exercise(mode, tmp, binary, surreal)
            exercise_nlp(tmp, binary)
        except Exception:
            for log in sorted(tmp.rglob("*.log")):
                print(log.name, log.read_text()[-1500:])
            raise
