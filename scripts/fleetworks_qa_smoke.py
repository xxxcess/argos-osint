#!/usr/bin/env python3
"""Exercise a real Inbox QA pack through a packaged companion or paired gateway."""

import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlencode

import fleetworks_local_backend_smoke as embedded
import fleetworks_qa_pack as pack

ROOT = Path(__file__).resolve().parents[1]
QA = "/api/v2/workstation/chat/"
TERMINAL = {"answered", "insufficient_context", "canceled", "failed"}
QUESTION = "What is the state of the gateway?"


def require(condition, description):
    if not condition:
        raise ValueError(description)


def authorize(registration):
    return {
        "registration_id": registration["registration_id"],
        "expected_registration_revision": registration["registration_revision"],
        "expected_context_revision": registration["context_revision"],
    }


def until(probe, ready, description, *, timeout=90):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = probe()
        if ready(value):
            return value
        time.sleep(0.25)
    raise TimeoutError(description)


def exercise(request, manage, registration, manager, model, readiness):
    """Run the same real QA lifecycle over the supplied transport."""
    before = request("settings")
    require(not before["enabled"] and before["phase"] == "off", "QA did not default to Off")
    require(not model(), "Off retained a live model process")
    require(request("latest") is None, "Off exposes an exchange")
    turn = {"request_id": "inbox-qa-smoke-1", "context": authorize(registration), "question": QUESTION}
    request("turns", turn, expected=503)

    revision = before["revision"]
    started = time.monotonic()
    manage("settings", {"context": authorize(manager), "expected_revision": revision, "enabled": True})
    current = until(lambda: request("settings"), lambda value: value["phase"] == "ready",
                    "QA worker did not warm")
    ready_ms = round((time.monotonic() - started) * 1000, 1)
    active = model()
    require(len(active) == 1, "Ready did not retain one live model process")
    worker_rss_bytes = embedded.read_process_rss_bytes(active[0])
    require(readiness(), "QA warm-up blocked normal workstation readiness")

    started = time.monotonic()
    accepted = request("turns", turn, expected=(200, 201, 202))
    admission_ms = round((time.monotonic() - started) * 1000, 1)
    turn_id = accepted["turn_id"]
    replay = request("turns", turn, expected=(200, 201, 202))
    require(replay["turn_id"] == turn_id, "the replay admitted another turn")
    request("turns", {**turn, "question": "Different question"}, expected=409)
    cursor = 0
    terminal_events = 0
    first_finding_ms = None
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        page = request("events", query={"turn_id": turn_id, "after": cursor, "wait_seconds": 4})
        snapshot = page["turn"]
        require(snapshot["turn_id"] == turn_id and not page["gap"], "event cursor could not resume")
        for event in page["events"]:
            require(event["sequence"] == cursor + 1, "event sequence is not consecutive")
            cursor = event["sequence"]
            if first_finding_ms is None and event.get("finding"):
                first_finding_ms = round((time.monotonic() - started) * 1000, 1)
            terminal_events += int(event["state"] in TERMINAL)
        if snapshot["state"] in TERMINAL:
            break
    else:
        raise TimeoutError("QA turn did not finish")
    completed_ms = round((time.monotonic() - started) * 1000, 1)
    require(terminal_events == 1 and snapshot["cursor"] == cursor, "terminal delivery was not exactly once")
    require(snapshot["state"] == "answered" and first_finding_ms is not None,
            "real local checkpoint did not return a cited answer")
    require(1 <= len(snapshot["findings"]) <= 3, "QA finding count exceeds its contract")
    for finding in snapshot["findings"]:
        require(0 <= finding["start"] < finding["end"]
                and finding["end"] - finding["start"] == len(finding["text"])
                and finding["observed_at"] and finding["source"]["id"], "invalid cited span")
    require(request("latest") == snapshot, "latest exchange differs from event snapshot")
    resumed = request("events", query={"turn_id": turn_id, "after": cursor, "wait_seconds": 0})
    require(not resumed["events"] and resumed["turn"] == snapshot, "terminal replay duplicated an event")
    require(readiness(), "ordinary readiness failed after QA inference")

    follow_up = request("turns", {**turn, "request_id": "inbox-qa-smoke-2"}, expected=(200, 201, 202))
    follow_up_id = follow_up["turn_id"]
    require(follow_up_id != turn_id, "the next exchange reused the previous turn")
    next_turn = until(
        lambda: request("events", query={"turn_id": follow_up_id, "after": 0, "wait_seconds": 0})["turn"],
        lambda value: value["state"] in TERMINAL, "fresh-evidence turn did not finish")
    require(next_turn["state"] == "answered" and next_turn["findings"], "fresh evidence did not support the follow-up")

    def observed(finding):
        return datetime.fromisoformat(finding["observed_at"].replace("Z", "+00:00"))

    require(min(map(observed, next_turn["findings"])) > max(map(observed, snapshot["findings"])),
            "the follow-up reused an observation from before the previous terminal turn")

    started = time.monotonic()
    manage("settings", {"context": authorize(manager), "expected_revision": current["revision"], "enabled": False})
    until(lambda: request("settings"), lambda value: value["phase"] == "off",
          "QA did not finish unloading", timeout=40)
    require(not model(), "Off retained a live model process")
    require(request("latest") is None, "Off retained latest question and answer")
    return {"model_warm_ms": ready_ms, "admission_ms": admission_ms, "first_cited_finding_ms": first_finding_ms,
            "completion_ms": completed_ms, "unload_ms": round((time.monotonic() - started) * 1000, 1),
            "model_pid_count": len(active), "warm_model_rss_bytes": worker_rss_bytes,
            "fresh_observation": True, "terminal_events": terminal_events}


def embedded_tree(parent):
    rows = subprocess.check_output(["ps", "-ww", "-axo", "pid=,ppid=,args="], text=True, timeout=10)
    members = {parent}
    processes = []
    for row in rows.splitlines():
        pid, ppid, command = row.split(maxsplit=2)
        program = command.split(" --model-dir ", 1)[0]
        processes.append((int(pid), int(ppid), Path(program).name))
    while True:
        seen = members | {pid for pid, ppid, _ in processes if ppid in members}
        if seen == members:
            break
        members = seen
    children = [pid for pid, _, name in processes if pid in members and name == Path(pack.WORKER).name]
    for pid in {parent, *children}:
        embedded.check_zero_bound_sockets(pid)
    return children


def run_embedded(args):
    # Run this program inside a network-denied environment for release verification.
    require(not any((Path.home() / path).exists() for path in (
        ".cache/torch", ".cache/huggingface", ".cache/.rustbert")), "use a fresh HOME for the offline smoke")
    binary = embedded.resolve_companion_binary(app=args.app)
    digests = json.loads(args.digests.read_text())
    with tempfile.TemporaryDirectory(prefix="fleetworks-embedded-qa-") as temporary:
        root = Path(temporary)
        state = root / "state"
        state.mkdir(mode=0o700)
        pack.install(argparse.Namespace(archive=args.archive, output=root / "pack", state_dir=state,
                                        target=digests["target"], archive_sha256=digests["archive_sha256"],
                                        manifest_sha256=digests["manifest_sha256"]))
        session = embedded.LocalIpcSession(binary, state, client_name="fleetworks-qa-smoke")
        try:
            session.hello()
            ensured, _ = session.request_json({"op": "registration_ensure", "args": {
                "request_id": "qa-smoke-registration", "workstation_ip": None,
                "installation_marker": secrets.token_hex(32),
            }})
            response, _ = session.request_json({"op": "registration_context", "args": {
                "registration_id": ensured["registration_id"], "expected_revision": ensured["revision"],
            }})
            registration = response["context"]

            def request(path, payload=None, *, query=None, expected=200):
                operation = {"settings": "qa_settings", "turns": "qa_turn_submit", "latest": "qa_latest",
                             "events": "qa_events", "cancel": "qa_cancel"}[path]
                if path == "settings" and payload is not None:
                    operation = "qa_settings_update"
                request = {"op": operation}
                if payload is not None or query is not None:
                    request["args"] = payload if payload is not None else query
                result, _ = session.request(request, registration=registration)
                data = result["data"]
                statuses = (expected,) if isinstance(expected, int) else expected
                if result["outcome"] == "ok":
                    require(200 in statuses, f"IPC {operation} unexpectedly succeeded")
                    return data["value"]
                require(data["status"] in statuses, f"IPC {operation} failed with HTTP-equivalent {data['status']}")
                return None

            def model():
                return embedded_tree(session.proc.pid)

            def readiness():
                value, _ = session.request_json({"op": "workstation_readiness"}, registration=registration)
                return bool(value.get("deployment_id"))

            result = exercise(request, request, registration, registration, model, readiness)
            _, exit_code = session.shutdown()
            require(exit_code == 0, "companion shutdown failed after unloading QA")
            return {"runtime": "embedded-stdio", "target": digests["target"], **result}
        finally:
            session.close_unconditionally()


def run_distributed(args):
    sys.path.insert(0, str(ROOT / "deploy"))
    import native_review
    import verify

    port, _ = verify.fixture_ports()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    image_variables = ("FLEETWORKS_RUNTIME_IMAGE", "FLEETWORKS_SURREAL_IMAGE", "FLEETWORKS_SEARCH_IMAGE")
    images = {key: os.environ[key] for key in image_variables}
    prepared = native_review.provision(ROOT, args.name, port, args.platform, revision, images=images)
    workspace = Path(prepared["workspace"])
    policy_path = workspace / "secrets/gateway-credentials.json"
    policy = json.loads(native_review.read_private(policy_path))
    manager_entry = next(entry for entry in policy["credentials"] if entry["id"] == "native-review")
    # Reuse the fresh fixture's mounted token slot for a distinct configuration
    # manager. The regular pairing issuer never receives this optional grant.
    manager_entry.update(id="qa-manager", purpose="operator",
                         permissions=[*manager_entry["permissions"], "manage_inbox_qa"])
    with policy_path.open("w", encoding="utf-8") as output:
        json.dump(policy, output)
    # Docker daemon and Compose use the same path without a published model port.
    socket_directory = workspace / "qa"
    socket_directory.mkdir(mode=0o700)
    queue_token = workspace / "secrets/qa-jobs-token"
    native_review.private_file(queue_token, secrets.token_urlsafe(48) + "\n")
    env = {**verify.isolated_environment(os.environ), **prepared["environment"],
           "FLEETWORKS_QA_IMAGE": args.image, "FLEETWORKS_QA_SOCKET_DIR": str(socket_directory),
           "FLEETWORKS_QA_JOBS_TOKEN_HOST_FILE": str(queue_token)}
    verify.require_unused_project(env, args.platform)
    command = [*verify.compose_command(workspace), "-f", str(ROOT / "deploy/compose.inbox-qa.yml")]

    def compose(*arguments, check=True, capture=False, timeout=45):
        return subprocess.run([*command, *arguments], cwd=ROOT, env=env, check=check, text=True,
                              stdout=subprocess.PIPE if capture else None, timeout=timeout).stdout

    failed = True
    try:
        compose("up", "--detach", "--no-build", "--pull", "never", "--wait", "--wait-timeout", "240",
                timeout=300)
        model_id = compose("ps", "--quiet", "assistant-qa-model", capture=True).strip()
        info, = json.loads(subprocess.check_output(["docker", "inspect", model_id], env=env, text=True, timeout=30))
        host = info["HostConfig"]
        require(host["NetworkMode"] == "none" and host["ReadonlyRootfs"] and not host["PortBindings"],
                "model container does not enforce its private boundary")
        require({entry["Destination"] for entry in info["Mounts"] if entry["Type"] != "tmpfs"}
                == {"/run/fleetworks-qa"},
                "model got an unexpected mount")
        require(host["Memory"] > 0 and host["NanoCpus"] > 0 and host["PidsLimit"] > 0, "missing model resource bounds")
        operator = native_review.read_private(workspace / "secrets/operator-token").strip()
        manager_token = native_review.read_private(workspace / "secrets/native-token").strip()
        relay = native_review.read_private(workspace / "secrets/web-relay-token").strip()

        def paired(directory, workstation_ip):
            directory.mkdir(mode=0o700)
            identity = verify.native_fixture(directory)
            identity["workstation_ip"] = workstation_ip
            polling_secret = secrets.token_hex(32)
            challenge = verify.expect_status(verify.request(port, "/api/v1/pairing/start", payload={
                "request_id": secrets.token_hex(16), "installation_marker": identity["installation_marker"],
                "polling_secret": polling_secret,
            }), (200, 201), "start native pairing")
            decision = {name: challenge[name] for name in ("pairing_id", "approval_code")}
            verify.expect_status(verify.request(port, "/api/v1/admin/pairing/approve", operator, decision),
                                 (200, 204), "operator approval")
            issued = verify.expect_status(verify.request(port, "/api/v1/pairing/poll", payload={
                "pairing_id": challenge["pairing_id"], "polling_secret": polling_secret,
            }), 200, "poll approved pairing")
            require(issued["status"] == "approved", "gateway did not issue paired native authority")
            token = issued["access_token"]
            registration = verify.register_native(port, token, identity)
            return token, registration

        token, registration = paired(workspace / "paired-qa", "192.0.2.91")
        other_token, other = paired(workspace / "other-paired-qa", "192.0.2.92")
        manager_dir = workspace / "operator-qa"
        manager_dir.mkdir(mode=0o700)
        manager = verify.register_native(port, manager_token, verify.native_fixture(manager_dir))

        def client(bearer, context):
            def request(path, payload=None, *, query=None, expected=200):
                route = QA + path + ("?" + urlencode(query) if query else "")
                return verify.expect_status(verify.request(port, route, bearer, payload,
                                                           headers=verify.native_headers(context)),
                                            expected, route.split("?")[0])
            return request

        native = client(token, registration)
        manage = client(manager_token, manager)
        other_client = client(other_token, other)
        client(relay, registration)("settings", expected=(401, 403))
        require(not native("settings")["can_manage"], "ordinary paired native gained QA configuration authority")
        native("settings", {"context": authorize(registration), "expected_revision": native("settings")["revision"],
                            "enabled": True}, expected=403)

        def request(path, payload=None, *, query=None, expected=200):
            result = native(path, payload, query=query, expected=expected)
            if path == "turns" and result and 200 <= (expected if isinstance(expected, int) else expected[0]) < 300:
                other_client("events", query={"turn_id": result["turn_id"], "after": 0, "wait_seconds": 0},
                             expected=404)
                require(other_client("latest") is None, "another registration sees QA transcript")
                other_client("cancel", {"context": authorize(other), "turn_id": result["turn_id"]}, expected=404)
            return result

        def model():
            processes = subprocess.check_output(["docker", "top", model_id, "-eo", "pid,args"],
                                                env=env, text=True, timeout=15)
            return [int(pid) for row in processes.splitlines()[1:]
                    for pid, program in [row.strip().split(maxsplit=1)]
                    if program.split(maxsplit=1)[0].endswith("/" + pack.WORKER)]

        def readiness():
            response = verify.request(port, "/api/v1/workstation/readiness", token,
                                      headers=verify.native_headers(registration))
            return response[0] == 200

        result = exercise(request, manage, registration, manager, model, readiness)
        failed = False
        return {"runtime": "distributed-paired-http", "platform": args.platform, "model_image_id": info["Image"], **result}
    finally:
        compose("down", check=not failed, timeout=120)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="runtime", required=True)
    local = modes.add_parser("embedded")
    local.add_argument("--app", type=Path, required=True)
    local.add_argument("--archive", type=Path, required=True)
    local.add_argument("--digests", type=Path, required=True)
    remote = modes.add_parser("distributed")
    remote.add_argument("--name", required=True)
    remote.add_argument("--image", required=True)
    remote.add_argument("--platform", choices=("linux/amd64", "linux/arm64"), required=True)
    args = parser.parse_args()
    result = run_embedded(args) if args.runtime == "embedded" else run_distributed(args)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
