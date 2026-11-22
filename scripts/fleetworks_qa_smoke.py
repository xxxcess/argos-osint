#!/usr/bin/env python3
"""Exercise a real Inbox QA pack through a packaged companion or paired gateway."""

import argparse
import base64
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlencode

import fleetworks_local_backend_smoke as embedded
import fleetworks_qa_pack as pack

ROOT = Path(__file__).resolve().parents[1]
QA = "/api/v2/workstation/chat/"
EVIDENCE_HEADER = "X-Fleetworks-Inbox-Evidence"
TERMINAL = {"answered", "insufficient_context", "canceled", "failed"}
SUMMARY_QUESTION = "What is the current workstation readiness?"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
DIAGNOSTIC = re.compile(r'\b(qa_(?:warm|replica|queue|turn|settings)_stage)="?([a-z0-9_]{1,40})\b')
DIAGNOSTIC_DETAIL = re.compile(
    r'\b(qa_warm_reason|qa_warm_elapsed_ms|landlock_abi|sanitized_os_error|worker_exit_code|worker_signal)'
    r'="?(Some\(-?\d{1,9}\)|-?\d{1,9}|abandoned|landlock_abi)(?=["\s,|]|$)'
)
FIXED_DIAGNOSTIC_DETAIL = re.compile(
    r'\b(worker_panic_source|worker_panic_line|worker_panic_cause|qa_settings_error)="?([a-z0-9_]{1,40})(?=["\s,|]|$)'
)
SAFE_REJECTION = {
    "Backend inference is off": "inference_off",
    "Backend inference has no ready worker": "no_ready_worker",
    "Backend inference is unavailable": "coordinator_unavailable",
    "Current backend health could not be verified": "fresh_health_unavailable",
    "Inbox inference is busy": "busy",
}
CATALOG_TIMEOUT_SECONDS = 15 * 60
CATALOG_ROWS = 94_987
TRAINING_ROWS = 1_020
ROCKSDB_OPTIONS = {
    "write_buffer_size": 16 * 1024 * 1024,
    "max_write_buffer_number": 2,
    "min_write_buffer_number_to_merge": 1,
    "max_background_jobs": 2,
    "max_subcompactions": 1,
}
ROCKSDB_BLOCK_CACHE_BYTES = 32 * 1024 * 1024
TERMINAL_JOB_STATES = {"completed", "failed", "cancelled", "canceled", "stopped", "skipped", "superseded"}
ACTIVE_JOB_STATES = {"queued", "running", "waiting", "reconciling", "retry_scheduled"}
SAFE_CATALOG_FAILURES = {
    "setup_worker_resource_pressure", "setup_dataset_resource_budget",
    "setup_worker_memory_pressure", "setup_worker_disk_pressure", "setup_worker_pressure_unavailable",
    "setup_interrupted", "native_domain_operation_failed",
}


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


def report_diagnostics(logs):
    stages = set()
    lines = ANSI.sub("", logs).splitlines()
    for line in lines:
        for stage, code in DIAGNOSTIC.findall(line):
            details = DIAGNOSTIC_DETAIL.findall(line) + FIXED_DIAGNOSTIC_DETAIL.findall(line)
            stages.add((stage, code, tuple(sorted(details))))
    print(json.dumps({
        "qa_failure_stages": [
            {"stage": stage, "code": code, **dict(details)} for stage, code, details in sorted(stages)
        ],
        "diagnostic_log_lines": len(lines),
    }), file=sys.stderr)


def cited_finding(turn, kind, source_id):
    require(turn["state"] == "answered" and len(turn["findings"]) == 1,
            f"{source_id} did not produce exactly one answer")
    finding, = turn["findings"]
    require(finding["source"] == {"kind": kind, "id": source_id},
            f"{source_id} answer cited a different source")
    require(0 <= finding["start"] < finding["end"]
            and finding["end"] - finding["start"] == len(finding["text"])
            and datetime.fromisoformat(finding["observed_at"]).tzinfo is not None,
            f"{source_id} answer lacks valid offsets or an observation time")
    return finding


def legacy_finding(finding):
    if finding is None or finding["source"]["kind"] not in {"host_metric", "job_count"}:
        return finding
    legacy = {**finding, "source": {**finding["source"], "kind": "readiness"}}
    del legacy["measurement"]
    return legacy


def legacy_turn(turn):
    return {**turn, "findings": [legacy_finding(finding) for finding in turn["findings"]]}


def grounded_turn(request, context, case, question, *, legacy_request=None):
    payload = {"request_id": f"inbox-qa-smoke-{case}", "context": context, "question": question}
    accepted = request("turns", payload, expected=(200, 201, 202))
    require(accepted["state"] in TERMINAL, f"{case} did not finish within admission")
    query = {"turn_id": accepted["turn_id"], "after": 0, "wait_seconds": 0}
    page = request("events", query=query)
    require(page["turn"] == accepted and not page["gap"], f"{case} admission differed from its event snapshot")
    require([event["sequence"] for event in page["events"]] == list(range(1, accepted["cursor"] + 1)),
            f"{case} event cursor skipped a sequence")
    require([event["sequence"] for event in page["events"] if event["state"] in TERMINAL] == [accepted["cursor"]],
            f"{case} did not emit exactly one terminal event")
    require([event["finding"] for event in page["events"] if event["finding"]] == accepted["findings"],
            f"{case} event evidence differs from the frozen answer")
    require(request("latest") == accepted, f"{case} latest changed the frozen answer")
    require(request("turns", payload, expected=(200, 201, 202)) == accepted, f"{case} replay changed the answer")
    cancel = {"context": context, "turn_id": accepted["turn_id"]}
    require(request("cancel", cancel) == accepted, f"{case} Stop changed a grounded terminal answer")
    resume = {**query, "after": accepted["cursor"]}
    require(request("events", query=resume) == {"turn": accepted, "events": [], "gap": False},
            f"{case} terminal replay emitted another event")

    if legacy_request is not None:
        legacy = legacy_turn(accepted)
        legacy_events = [{**event, "finding": legacy_finding(event["finding"])} for event in page["events"]]
        require(legacy_request("latest") == legacy, f"{case} latest is unreadable by a legacy client")
        require(legacy_request("events", query=query) == {"turn": legacy, "events": legacy_events, "gap": False},
                f"{case} legacy event stream changed the cursor or evidence")
        require(legacy_request("turns", payload, expected=(200, 201, 202)) == legacy,
                f"{case} legacy submit replay changed the frozen answer")
        require(legacy_request("cancel", cancel) == legacy, f"{case} legacy Stop changed the answer")
        require(legacy_request("events", query=resume) == {"turn": legacy, "events": [], "gap": False},
                f"{case} legacy resume emitted another event")
    return accepted


def cited_metric(turn, source_id, label, *, embedded_mode):
    finding = cited_finding(turn, "host_metric", source_id)
    measurement = finding["measurement"]
    value = measurement["value"]
    subject = f"{'API host' if embedded_mode else 'Connected gateway host'} {label}"
    sentence = subject if embedded_mode else subject[0].lower() + subject[1:]
    require(type(value) is int and 0 <= value <= 1000
            and measurement == {"subject": subject, "unit": "percent_tenths", "value": value},
            f"{source_id} evidence does not contain a valid utilization measurement")
    whole, tenths = divmod(value, 10)
    require(finding["label"] == subject
            and finding["text"] == f"The {sentence} is {whole}.{tenths}%.",
            f"{source_id} answer differs from its frozen measurement")
    return {"value": value, "unit": measurement["unit"], "observed_at": finding["observed_at"]}


def cited_running_jobs(turn, active_jobs):
    finding = cited_finding(turn, "job_count", "running")
    job_noun = "job" if active_jobs == 1 else "jobs"
    require(finding["text"] == f"This workstation has {active_jobs} running {job_noun}."
            and finding["measurement"] == {
                "subject": "Running workstation jobs", "unit": "jobs", "value": active_jobs,
            },
            "Inbox running-job count differs from the registered workstation dashboard")


def exercise(request, manage, registration, manager, model, readiness, running_jobs, *,
             embedded_mode=False, legacy_request=None, verify_other_jobs=None):
    """Run the same real QA lifecycle over the supplied transport."""
    before = request("settings")
    require(not before["enabled"] and before["phase"] == "off", "QA did not default to Off")
    require(not model(), "Off retained a live model process")
    require(request("latest") is None, "Off exposes an exchange")
    turn = {"request_id": "inbox-qa-smoke-1", "context": authorize(registration), "question": SUMMARY_QUESTION}
    request("turns", turn, expected=503)

    revision = before["revision"]
    started = time.monotonic()
    manage("settings", {"context": authorize(manager), "expected_revision": revision, "enabled": True})
    try:
        current = until(lambda: request("settings"), lambda value: value["phase"] == "ready",
                        "QA worker did not warm", timeout=110)
    except TimeoutError:
        current = request("settings")
        phase = current.get("phase")
        if phase not in {"off", "starting", "ready", "stopping", "unavailable"}:
            phase = "unknown"
        raise TimeoutError(f"QA worker did not warm (last phase {phase})") from None
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
    observed_states = set()
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
            if event["state"] in TERMINAL | {"queued", "running"}:
                observed_states.add(event["state"])
        if snapshot["state"] in TERMINAL:
            break
    else:
        raise TimeoutError("QA turn did not finish")
    completed_ms = round((time.monotonic() - started) * 1000, 1)
    if snapshot["state"] != "answered" or first_finding_ms is None:
        print(json.dumps({"qa_smoke_stage": "first_turn_terminal", "state": snapshot["state"],
                          "observed_states": sorted(observed_states), "finding_count": len(snapshot["findings"]),
                          "model_warm_ms": ready_ms, "admission_ms": admission_ms,
                          "completion_ms": completed_ms}), file=sys.stderr)
    require(terminal_events == 1 and snapshot["cursor"] == cursor, "terminal delivery was not exactly once")
    require(snapshot["state"] == "answered" and first_finding_ms is not None,
            "real local checkpoint did not return a cited answer")
    require(1 <= len(snapshot["findings"]) <= 3, "QA finding count exceeds its contract")
    for finding in snapshot["findings"]:
        require(0 <= finding["start"] < finding["end"]
                and finding["end"] - finding["start"] == len(finding["text"])
                and finding["observed_at"] and finding["source"]["id"]
                and finding["source"]["kind"] == "readiness", "invalid readiness citation")
        require(re.search(r"\b\d{4}-\d{2}-\d{2}|\b\d{2}:\d{2}:\d{2}", finding["text"]) is None,
                "the real-model summary leaked an observation timestamp into its answer")
    require(request("latest") == snapshot, "latest exchange differs from event snapshot")
    resumed = request("events", query={"turn_id": turn_id, "after": cursor, "wait_seconds": 0})
    require(not resumed["events"] and resumed["turn"] == snapshot, "terminal replay duplicated an event")
    require(readiness(), "ordinary readiness failed after QA inference")
    print(json.dumps({"qa_smoke_stage": "first_turn_answered", "model_warm_ms": ready_ms,
                      "admission_ms": admission_ms, "first_cited_finding_ms": first_finding_ms,
                      "completion_ms": completed_ms, "warm_model_rss_bytes": worker_rss_bytes}),
          file=sys.stderr)

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

    context = authorize(registration)

    def ask(case, question, *, check_legacy=False):
        return grounded_turn(request, context, case, question,
                             legacy_request=legacy_request if check_legacy else None)

    gateway = ask("gateway", "What is the state of the gateway?")
    require(cited_finding(gateway, "readiness", "gateway")["text"]
            == "Gateway is ready. An authenticated gateway request succeeded.",
            "gateway answer did not report the observed ready state")
    if embedded_mode:
        catalog = ask("catalog", "Why is the actor catalog not ready?")
        require(cited_finding(catalog, "readiness", "actor_catalog")["text"]
                == "Actor catalog is not ready. No completed seed was verified, and existing records may remain.",
                "the catalog answer did not preserve its actual seed status")

    active_jobs = running_jobs()
    jobs = ask("jobs", "How many jobs are running?", check_legacy=True)
    cited_running_jobs(jobs, active_jobs)
    questions = (
        ("cpu", "What is the current CPU utilization?", "CPU usage"),
        ("memory", "What is the current memory usage?", "memory usage"),
        ("disk", "What is the current disk usage?", "storage volume usage"),
    )
    readings = {}
    for source_id, metric_question, label in questions:
        metric = ask(source_id, metric_question, check_legacy=source_id == "memory")
        readings[source_id] = cited_metric(metric, source_id, label, embedded_mode=embedded_mode)
    memory_again = ask("memory-follow-up", "What is the current memory usage?")
    readings["memory_follow_up"] = cited_metric(memory_again, "memory", "memory usage", embedded_mode=embedded_mode)
    require(datetime.fromisoformat(readings["memory_follow_up"]["observed_at"])
            > datetime.fromisoformat(readings["memory"]["observed_at"]),
            "memory follow-up reused the previous observation timestamp")
    unsupported = ask("unsupported", "What is the current GPU temperature?")
    require(unsupported["state"] == "insufficient_context" and not unsupported["findings"],
            "unsupported measurement received an answer")
    if verify_other_jobs is not None:
        verify_other_jobs()

    started = time.monotonic()
    manage("settings", {"context": authorize(manager), "expected_revision": current["revision"], "enabled": False})
    until(lambda: request("settings"), lambda value: value["phase"] == "off",
          "QA did not finish unloading", timeout=40)
    require(not model(), "Off retained a live model process")
    require(request("latest") is None, "Off retained latest question and answer")
    query = {"turn_id": unsupported["turn_id"], "after": 0, "wait_seconds": 0}
    request("events", query=query, expected=404)
    if legacy_request is not None:
        require(legacy_request("latest") is None, "Off retained an exchange in the legacy view")
        legacy_request("events", query=query, expected=404)
    return {"model_warm_ms": ready_ms, "admission_ms": admission_ms, "first_cited_finding_ms": first_finding_ms,
            "completion_ms": completed_ms, "unload_ms": round((time.monotonic() - started) * 1000, 1),
            "model_pid_count": len(active), "warm_model_rss_bytes": worker_rss_bytes,
            "fresh_observation": True, "terminal_events": terminal_events,
            "grounded_readiness": True, "live_metrics": True, "unsupported_context": True,
            "frozen_samples": readings, "legacy_http_evidence": legacy_request is not None}


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


def sample_catalog_resources(proc, state, evidence, stop):
    while proc.poll() is None:
        phase = evidence["stage"]
        try:
            status = Path(f"/proc/{proc.pid}/status").read_text()
            disk_free = shutil.disk_usage(state).free
        except OSError:
            break
        usage = {
            name: int(kib) * 1024
            for name, kib in re.findall(r"^(VmRSS|VmHWM|RssAnon|RssFile):\s+(\d+) kB$", status, re.M)
        }
        evidence.setdefault("baseline_disk_free_bytes", disk_free)
        if "VmRSS" in usage:
            evidence.setdefault("baseline_rss_bytes", usage["VmRSS"])
            evidence["peak_sampled_rss_bytes"] = max(evidence.get("peak_sampled_rss_bytes", 0), usage["VmRSS"])
        evidence["min_disk_free_bytes"] = min(evidence.get("min_disk_free_bytes", disk_free), disk_free)
        phase_usage = evidence["resources_by_stage"].setdefault(phase, {"samples": 0})
        phase_usage["samples"] += 1
        phase_usage["min_disk_free_bytes"] = min(phase_usage.get("min_disk_free_bytes", disk_free), disk_free)
        for field, name in (("peak_sampled_rss_bytes", "VmRSS"), ("peak_sampled_anon_bytes", "RssAnon"),
                            ("peak_sampled_file_bytes", "RssFile"), ("kernel_rss_high_water_bytes", "VmHWM")):
            if name in usage:
                phase_usage[field] = max(phase_usage.get(field, 0), usage[name])
        if stop.wait(1):
            break


def persisted_rocksdb_options(state):
    files = list((state / "surreal").rglob("OPTIONS-*"))
    require(files, "Embedded RocksDB did not persist an OPTIONS file")
    options = {}
    source = max(files, key=lambda path: path.name).read_text()
    for key, value in re.findall(r"^[ \t]*([a-z_]+)=(\d+)[ \t]*$", source, re.M):
        if key in ROCKSDB_OPTIONS:
            require(key not in options or options[key] == int(value), "RocksDB options disagree across column families")
            options[key] = int(value)
    return options


def stage_bundled_csv(call, directory, kind, pin, rows):
    path = directory / pin["filename"]
    require(path.stat().st_size == pin["size_bytes"], "bundled dataset byte count changed")
    ticket = call("dataset_upload_start", {
        "kind": kind, "sha256": pin["sha256"], "size_bytes": pin["size_bytes"],
    })
    upload_id = ticket["upload_path"].rstrip("/").rsplit("/", 1)[-1]
    offset = 0
    with path.open("rb") as file:
        while chunk := file.read(256 * 1024):
            call("dataset_upload_chunk", {
                "upload_id": upload_id, "offset": offset, "data_b64": base64.b64encode(chunk).decode("ascii"),
            }, allow_non_json=True)
            offset += len(chunk)
    upload = call("dataset_upload_finish", {"upload_id": upload_id})
    require(upload["row_count"] == rows and upload["dataset"]["sha256"] == pin["sha256"],
            "backend validation did not match the bundled digest and row count")
    return upload["dataset"]


def exercise_bundled_catalog(session, state, app, evidence):
    phase_started = time.monotonic()

    def next_stage(stage):
        nonlocal phase_started
        now = time.monotonic()
        evidence["elapsed_by_stage_ms"][evidence["stage"]] = round((now - phase_started) * 1000)
        evidence["stage"] = stage
        phase_started = now

    require(session.hello()["compatible"], "packaged companion rejected the fixture protocol")
    embedded.check_zero_bound_sockets(session.proc.pid)
    evidence["rocksdb_options"] = persisted_rocksdb_options(state)

    def block_cache_capacities():
        log = ANSI.sub("", session.stderr_excerpt())
        return {int(capacity) for capacity in re.findall(r"\bMemory manager: block cache size: (\d+)B\b", log)}

    capacities = until(block_cache_capacities, bool, "RocksDB startup omitted its block cache capacity", timeout=5)
    evidence["rocksdb_block_cache_bytes"] = sorted(capacities)
    require(evidence["rocksdb_options"] == ROCKSDB_OPTIONS, "persisted RocksDB options differ from the Embedded profile")
    require(capacities == {ROCKSDB_BLOCK_CACHE_BYTES}, "RocksDB startup did not report the bounded block cache")
    ensured, _ = session.request_json({"op": "registration_ensure", "args": {
        "request_id": "bundled-catalog-registration", "workstation_ip": None,
        "installation_marker": secrets.token_hex(32),
    }})
    response, _ = session.request_json({"op": "registration_context", "args": {
        "registration_id": ensured["registration_id"], "expected_revision": ensured["revision"],
    }})
    registration = response["context"]
    mutation_context = {**authorize(registration), "expected_configuration_revision": 0}

    def call(operation, args=None, *, allow_non_json=False):
        request = {"op": operation}
        if args is not None:
            request["args"] = args
        outcome, _ = session.request(request, registration=registration)
        data = outcome["data"]
        if outcome["outcome"] != "ok":
            evidence["rpc_error"] = {"operation": operation, "status": data.get("status")}
            if (operation == "reindex_actors" and data.get("status") == 422
                    and data.get("message", "").startswith("Actor index status is still checking or catching up.")):
                return None
            raise ValueError("bundled catalog IPC operation failed")
        require(allow_non_json or data["payload"] in ("json", "dataset_upload"), "unexpected catalog IPC payload")
        return data.get("value")

    def check(check_id):
        checks = call("workstation_readiness")["checks"]
        return next(item for item in checks if item["id"] == check_id)

    def jobs():
        return call("jobs_list", {"sort": "newest"})["rows"]

    def await_job(receipt, operation):
        job_id = receipt["job_id"]
        require(job_id and not receipt["already_completed"], "fresh catalog job was not durably accepted")
        while True:
            row = next((row for row in jobs() if row["id"] == job_id and row["operation"] == operation), None)
            if row is not None:
                state = row["state"]
                # Never retain job IDs, arbitrary error messages, titles or row payloads.
                observed = {
                    "state": state if state in TERMINAL_JOB_STATES | ACTIVE_JOB_STATES else "other",
                    "reported_rows": row.get("progress_done"), "expected_rows": row.get("progress_total"),
                }
                if row.get("error_summary") in SAFE_CATALOG_FAILURES:
                    observed["reason"] = row["error_summary"]
                    reasons = evidence.setdefault("job_reasons_seen", [])
                    if observed["reason"] not in reasons:
                        reasons.append(observed["reason"])
                evidence[operation] = observed
                if state in TERMINAL_JOB_STATES:
                    if operation == "actor_seed":
                        detail = call("job_detail", {"id": job_id})
                        committed = detail["maintenance_result"]
                        if committed and committed["operation"] == "actor_seed":
                            observed["committed_rows"] = committed["processed"]
                            observed["committed_total"] = committed["total"]
                            observed["database_seeded"] = committed["database_seeded"]
                    return observed
            time.sleep(2)

    pins = json.loads((ROOT / "datasets/native-defaults.json").read_text())["datasets"]
    directory = app / "share/fleetworks/resources/datasets"
    require(check("actor_catalog")["state"] != "ready", "fresh catalog reported ready without a seed")
    next_stage("actor_csv_validation")
    dataset = stage_bundled_csv(call, directory, "actor_catalog", pins["actor_catalog"], CATALOG_ROWS)
    evidence["actor_validated_rows"] = CATALOG_ROWS
    require(check("actor_catalog")["state"] != "ready", "staged catalog reported ready without a seed")

    next_stage("actor_seed")
    receipt = call("seed_actors", {
        "request_id": "bundled-catalog-seed", "context": mutation_context, "dataset": dataset,
    })
    seed = await_job(receipt, "actor_seed")
    evidence["actor_catalog_ready"] = check("actor_catalog")["state"] == "ready"
    require(seed["state"] == "completed" and seed.get("database_seeded") is True
            and seed.get("committed_rows") == seed.get("committed_total") == CATALOG_ROWS,
            "full bundled actor seed did not complete")
    if not evidence["actor_catalog_ready"]:
        until(lambda: check("actor_catalog"), lambda value: value["state"] == "ready",
              "completed actor catalog was not ready", timeout=30)
        evidence["actor_catalog_ready"] = True

    next_stage("actor_index_admission")
    receipt = until(lambda: call("reindex_actors", {
        "request_id": "bundled-catalog-reindex", "context": mutation_context,
    }), bool, "packaged actor index could not be accepted", timeout=120)
    evidence.pop("rpc_error", None)
    next_stage("actor_index")
    if receipt["already_completed"]:
        evidence["actor_reindex"] = {"state": "already_completed"}
    else:
        require(await_job(receipt, "actor_reindex")["state"] == "completed", "packaged actor index did not complete")

    next_stage("actor_search")
    # The final catalog row proves that the serving index covers the tail of the import.
    source_id = "ucdp:999"
    actor_id = "catalog-actor-" + hashlib.sha256(source_id.encode()).hexdigest()[:24]
    query = {"query": "Chakosi", "mode": "discovery", "limit": 10}

    def exact_actor_visible(response):
        index = response["index"]
        return index["state"] == "ready" and any(
            hit["actor_id"] == actor_id and hit["label"] == query["query"]
            and hit["index_generation"] == index["generation"] for hit in response["rows"]
        )

    search = until(lambda: call("actors_search", query), exact_actor_visible,
                   "bundled catalog tail actor was not reader-visible", timeout=180)
    index = search["index"]
    require(index["generation"] and index["visible_checkpoint"], "ready index lacks a visible serving generation")
    evidence["actor_search"] = {"source_id": source_id, "visible_checkpoint": index["visible_checkpoint"]}

    next_stage("training_csv_preview")
    training = check("classifier_training")
    require(any(action["id"] == "train_classifier" and action["allowed"] for action in training["actions"]),
            "training preview lacked an authorized backend operation")
    stage_bundled_csv(call, directory, "classifier_training", pins["classifier_training"], TRAINING_ROWS)
    evidence["training_validated_rows"] = TRAINING_ROWS
    evidence["training_job_accepted"] = any(row["operation"] == "classifier_train" for row in jobs())
    require(not evidence["training_job_accepted"], "training ran without confirmation")

    next_stage("shutdown")
    embedded.check_zero_bound_sockets(session.proc.pid)
    _, exit_code = session.shutdown("bundled_catalog_smoke_complete")
    require(exit_code == 0, "bundled catalog companion did not shut down cleanly")
    next_stage("complete")


def packaged_catalog_smoke(binary, app, state):
    evidence = {"status": "failed", "stage": "startup", "resources_by_stage": {}, "elapsed_by_stage_ms": {}}
    require([name for _, name in socket.if_nameindex()] == ["lo"], "bundled catalog smoke needs an offline namespace")
    state.mkdir(mode=0o700)
    started = time.monotonic()
    stop = threading.Event()
    session = embedded.LocalIpcSession(binary, state, client_name="fleetworks-catalog-smoke",
                                       log_filter="warn,surrealdb::core::kvs::rocksdb=info")

    def expire():
        if session.proc.poll() is None:
            evidence["timed_out"] = True
            session.proc.kill()

    timeout = threading.Timer(CATALOG_TIMEOUT_SECONDS, expire)
    timeout.daemon = True
    timeout.start()
    sampler = threading.Thread(target=sample_catalog_resources, args=(session.proc, state, evidence, stop), daemon=True)
    sampler.start()
    try:
        exercise_bundled_catalog(session, state, app, evidence)
        require(evidence.get("peak_sampled_rss_bytes", 0) > 0
                and evidence["resources_by_stage"].get("actor_seed", {}).get("samples", 0) > 0,
                "bundled catalog smoke did not collect seed resource observations")
        evidence["status"] = "passed"
    except (OSError, ValueError, KeyError, StopIteration, subprocess.SubprocessError) as error:
        evidence["failure_type"] = type(error).__name__
        print(f"Offline bundled catalog smoke failed during {evidence['stage']} ({type(error).__name__})", file=sys.stderr)
    finally:
        timeout.cancel()
        stop.set()
        sampler.join(timeout=5)
        session.close_unconditionally()
    if "min_disk_free_bytes" in evidence:
        evidence["peak_disk_headroom_drop_bytes"] = evidence["baseline_disk_free_bytes"] - evidence["min_disk_free_bytes"]
    evidence["elapsed_ms"] = round((time.monotonic() - started) * 1000)
    return evidence


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
                reason = SAFE_REJECTION.get(data.get("message"), "unclassified")
                require(data["status"] in statuses,
                        f"IPC {operation} received {data['status']} ({reason})")
                return None

            def model():
                return embedded_tree(session.proc.pid)

            def readiness():
                value, _ = session.request_json({"op": "workstation_readiness"}, registration=registration)
                return bool(value.get("deployment_id"))

            def running_jobs():
                value, _ = session.request_json({"op": "jobs_list", "args": {"sort": "newest"}}, registration=registration)
                require(value["dashboard_counts_scope"] == "unfiltered_top_level_workstation_activity",
                        "IPC jobs dashboard counts used the wrong scope")
                return value["counts"]["active"]

            result = exercise(request, request, registration, registration, model, readiness, running_jobs,
                              embedded_mode=True)
            _, exit_code = session.shutdown()
            require(exit_code == 0, "companion shutdown failed after unloading QA")
        except Exception:
            report_diagnostics(session.stderr_excerpt())
            raise
        finally:
            session.close_unconditionally()

        if digests["target"] == "x86_64-unknown-linux-gnu":
            manifest = json.loads((args.app / "share/fleetworks/fleetworks-manifest.json").read_text())
            if manifest["profile"] == "debug":
                result["bundled_catalog"] = packaged_catalog_smoke(binary, args.app, root / "catalog-state")
        return {"runtime": "embedded-stdio", "target": digests["target"], **result}


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
            identity["request_id"] = "qa-register-" + directory.name
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
        manager_identity = {**verify.native_fixture(manager_dir), "request_id": "qa-register-operator"}
        manager = verify.register_native(port, manager_token, manager_identity)

        def client(bearer, context, *, typed_evidence=False):
            headers = verify.native_headers(context)
            if typed_evidence:
                headers = {**headers, EVIDENCE_HEADER: "1"}

            def request(path, payload=None, *, query=None, expected=200):
                route = QA + path + ("?" + urlencode(query) if query else "")
                status, value, response_headers = verify.exchange(port, route, bearer, payload, headers=headers)
                statuses = (expected,) if isinstance(expected, int) else expected
                error = value.get("error") if isinstance(value, dict) else None
                reason = SAFE_REJECTION.get(error if isinstance(error, str) else None, "unclassified")
                require(status in statuses, f"{path} received HTTP {status} ({reason})")
                if path != "settings" and 200 <= status < 300:
                    normalized = {name.lower(): content.lower() for name, content in response_headers.items()}
                    require({"origin", EVIDENCE_HEADER.lower()} <= {
                                name.strip() for name in normalized.get("vary", "").split(",")}
                            and {"private", "no-store"} <= {part.strip() for part in
                                                            normalized.get("cache-control", "").split(",")},
                            f"{path} response did not isolate the evidence view from HTTP caches")
                return value
            return request

        native = client(token, registration, typed_evidence=True)
        legacy = client(token, registration)
        manage = client(manager_token, manager)
        other_client = client(other_token, other, typed_evidence=True)
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

        def running_jobs(bearer, context):
            value = verify.expect_status(
                verify.request(port, "/api/v1/jobs?sort=newest", bearer, headers=verify.native_headers(context)),
                200, "registered workstation jobs",
            )
            require(value["dashboard_counts_scope"] == "unfiltered_top_level_workstation_activity",
                    "paired jobs dashboard counts used the wrong scope")
            return value["counts"]["active"]

        def verify_other_jobs():
            expected = running_jobs(other_token, other)
            observed = grounded_turn(other_client, authorize(other), "other-jobs", "How many jobs are running?")
            cited_running_jobs(observed, expected)
            native("events", query={"turn_id": observed["turn_id"], "after": 0, "wait_seconds": 0}, expected=404)

        result = exercise(request, manage, registration, manager, model, readiness,
                          lambda: running_jobs(token, registration), legacy_request=legacy,
                          verify_other_jobs=verify_other_jobs)
        require(other_client("latest") is None, "Off retained another registration's metric exchange")
        failed = False
        return {"runtime": "distributed-paired-http", "platform": args.platform, "model_image_id": info["Image"], **result}
    except Exception:
        try:
            report_diagnostics(compose("logs", "--no-color", "--tail", "100", "assistant-qa-executor",
                                       capture=True, check=False) or "")
        except (OSError, subprocess.TimeoutExpired):
            pass
        raise
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
    if args.runtime == "embedded":
        os.environ["RUST_LOG"] = "warn,argos_api_host::inbox_qa_service=info"
        result = run_embedded(args)
    else:
        result = run_distributed(args)
    print(json.dumps(result, indent=2))
    return int(result.get("bundled_catalog", {}).get("status") == "failed")


if __name__ == "__main__":
    sys.exit(main())
