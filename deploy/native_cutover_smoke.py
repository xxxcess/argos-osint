#!/usr/bin/env python3
"""Exercise a separately started, task-owned native-review backend over HTTP.

Uses a synthetic actor fixture. Does not build/start/stop Docker, invoke a
provider, reset credentials, or use the user's native home. This is Linux backend
evidence; it does not establish native app/GUI or Rust-BERT inference acceptance.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import time
from urllib.parse import urlencode

import native_review
import verify


ACTORS = b'''actor_id,actor_name,actor_type,aliases,name_normalized
smoke-harbor-union,Harbor Union,organization,"[""Harbor Guild""]",harbor union
smoke-river-lab,River Laboratory,organization,"[""River Lab""]",river laboratory
'''


def private_read(path, limit):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        assert stat.S_ISREG(metadata.st_mode) and metadata.st_uid == os.getuid()
        assert metadata.st_nlink == 1 and metadata.st_mode & 0o077 == 0
        assert 0 <= metadata.st_size <= limit
        value = source.read(limit + 1)
        assert len(value) <= limit
        return value


class Journey:
    def __init__(self, workspace, receipt):
        self.workspace = workspace
        self.port = int(receipt["environment"]["FLEETWORKS_API_PORT"])
        self.token = private_read(workspace / "secrets/native-token", 4096).decode().strip()
        self.events = []
        self.step = "start"

    def record(self, step, **safe_metadata):
        self.step = step
        self.events.append({"step": step, **safe_metadata})
        print(f"PASS: {step}", flush=True)

    def request(self, path, payload=None, *, proof=None, expected=200):
        status, value = verify.request(self.port, path, self.token, payload, headers=proof)
        assert status == expected, f"HTTP contract failed with status {status}"
        return value

    def origin(self, name, ip):
        root = self.workspace / "smoke-fixtures" / name
        root.mkdir(mode=0o700, parents=True, exist_ok=True)
        saved = root / "origin.json"
        if saved.exists():
            registration = json.loads(private_read(saved, 4096))
        else:
            registration = verify.native_fixture(root)
            registration.update(request_id="cutover-register-" + name, workstation_ip=ip)
            native_review.private_file(saved, json.dumps(registration))
        assert registration["workstation_ip"] == ip
        return verify.register_native(self.port, self.token, registration)

    def upload(self, proof, kind, content):
        digest = hashlib.sha256(content).hexdigest()
        status, value, _ = verify.exchange(self.port,
            "/api/v1/workstation/datasets/upload?" + urlencode({"kind": kind, "sha256": digest}),
            self.token, content, headers={**proof, "Content-Type": "text/csv"})
        assert status == 201 and value["dataset"]["sha256"] == digest
        assert value["dataset"]["size_bytes"] == len(content)
        return value["dataset"]

    def maintain(self, proof, path, request_id, **input_fields):
        payload = {"request_id": request_id, "context": verify.submission_context(self.port, self.token, proof), **input_fields}
        receipt = self.request(path, payload, proof=proof, expected=202)
        replay = self.request(path, payload, proof=proof, expected=202)
        assert replay["job_id"] == receipt["job_id"] and replay["origin"] == receipt["origin"]
        assert receipt["origin"]["source"] == "native"
        detail = verify.wait_job(self.port, self.token, proof, receipt["job_id"], 180)
        return receipt["job_id"], detail

    def search(self, proof, query, expected_id=None):
        deadline = time.monotonic() + 120
        while True:
            value = self.request("/api/v1/actors/search?" + urlencode({"mode": "discovery", "query": query, "limit": 10}), proof=proof)
            matches = [row for row in value["rows"] if row["label"] == "Harbor Union"]
            if matches and value["index"]["state"] == "ready":
                assert len(matches) == 1, "synthetic name must resolve uniquely"
                canonical = matches[0]["actor_id"]
                assert isinstance(canonical, str) and canonical
                if expected_id is not None:
                    assert canonical == expected_id, "alias resolved to another canonical actor"
                return value, canonical
            assert time.monotonic() < deadline, "seeded source never became searchable"
            time.sleep(0.25)

    def run(self):
        self.step = "authority"
        assert verify.request(self.port, "/api/v1/health/ready")[0] == 401
        assert verify.request(self.port, "/api/v1/jobs", self.token)[0] in (401, 403, 428)
        context = self.origin("node-a", "192.0.2.61")
        proof = verify.native_headers(context)
        self.request("/api/v1/workstation/readiness", proof=proof)
        self.record("registered authority", deployment_id=context["deployment_id"], workstation_id=context["workstation_id"])

        self.step = "actor seed"
        dataset = self.upload(proof, "actor_catalog", ACTORS)
        seed, detail = self.maintain(proof, "/api/v1/workstation/maintenance/seed-actors", "cutover-seed-actors", dataset=dataset)
        assert detail["maintenance_result"]["database_seeded"] is True
        self.record("durable actor seed", job_id=seed, dataset_sha256=dataset["sha256"])

        self.step = "source to visible"
        found, canonical = self.search(proof, "Harbor")
        self.search(proof, "Harbor Guild", canonical)
        self.request("/api/v1/actors/detail?" + urlencode({"id": canonical}), proof=proof)
        self.record("source to visible name alias and dossier", actor_id=canonical, index=found["index"])
        rebuilt, _ = self.maintain(proof, "/api/v1/search/actors/reindex", "cutover-rebuild-index")
        self.search(proof, "Harbor Guild", canonical)
        self.record("durable reindex", job_id=rebuilt)

        self.step = "legacy classifier retirement"
        readiness = self.request("/api/v1/workstation/readiness", proof=proof)
        training = next(check for check in readiness["checks"] if check["id"] == "classifier_training")
        assert training["reason_code"] == "legacy_classifier_retired" and not training["actions"]
        self.record("legacy classifier actions retired")

        self.step = "scoped activity"
        logs = self.request("/api/v1/logs?limit=100", proof=proof)
        assert logs["private_logs_only"] and logs["available"]
        assert all("details" not in row and "seq" not in row for row in logs["rows"])
        foreign = verify.native_headers(self.origin("node-b", "192.0.2.62"))
        path = "/api/v1/jobs/detail?id=" + seed
        assert verify.request(self.port, path, self.token, headers=foreign)[0] in (403, 404)
        self.search(foreign, "Harbor Guild", canonical)
        self.record("private activity and shared global search", observed_log_rows=len(logs["rows"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--name", required=True, help="fresh native_review.py project name, already running")
    args = parser.parse_args()
    workspace = native_review.review_path(native_review.ROOT, args.name)
    assert workspace.is_dir() and not workspace.is_symlink()
    receipt = json.loads(private_read(workspace / "review.json", 64 * 1024))
    assert receipt["workspace"] == str(workspace) and receipt["environment"]["COMPOSE_PROJECT_NAME"] == "fleetworks-native-review-" + args.name
    journey = Journey(workspace, receipt)
    report = {"version": 1, "revision": receipt["revision"], "project": receipt["environment"]["COMPOSE_PROJECT_NAME"], "platform": receipt["environment"]["PLATFORM"], "events": journey.events}
    try:
        journey.run()
        report["status"] = "pass"
        code = 0
    except Exception as error:
        report.update(status="fail", failed_step=journey.step, error_type=type(error).__name__)
        print(f"FAIL: {journey.step} ({type(error).__name__}); original backend evidence retained")
        code = 1
    report_path = workspace / ("public-smoke-" + secrets.token_hex(6) + ".json")
    native_review.private_file(report_path, json.dumps(report, indent=2) + "\n")
    print("Bounded non-secret report: " + str(report_path))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
