#!/usr/bin/env python3
"""Run the canonical Linux native verification job on Depot and collect its evidence."""
import argparse
from datetime import datetime
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import sys
from tempfile import TemporaryDirectory
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import Request, urlopen
from zipfile import BadZipFile, ZipFile

WORKFLOW = Path(".github/workflows/fleetworks-native-linux.yml")
API = "https://api.depot.dev/depot.ci.v1.CIService"
RUNNER = "depot-ubuntu-24.04-8"
ARTIFACTS = {
    "app": "Fleetworks-Linux-Desktop-App-",
    "ui": "Fleetworks-Linux-Desktop-UI-Captures-",
    "verification": "Fleetworks-Linux-Desktop-Verification-",
}
TIMED_PHASES = (
    ("linux-native-unit-tests", "Rust unit tests"),
    ("linux-native-ui-fixtures", "UI fixtures"),
    ("linux-native-startup-motion", "Startup motion"),
    ("linux-native-package-verification", "Package and smoke"),
    ("linux-native-http-ipc-parity", "HTTP/IPC parity"),
)
FAILED = {"failed", "cancelled"}


def render_workflow(source, sha, pair, phase, release_version=""):
    # Keep the canonical job, workspace, and environment. GitHub-only triggers,
    # inputs, and concurrency groups are resolved by the caller.
    env = re.search(r"(?ms)^env:\n((?:^[ \t]+[^\n]*\n)+)", source)
    _, separator, jobs = source.partition("\njobs:\n")
    runner_line = "    runs-on: ubuntu-24.04\n"
    if not env or not separator or jobs.count(runner_line) != 1:
        raise ValueError("Linux native workflow structure changed")
    inputs = {
        "expected-head": sha,
        "mode": "nightly",
        "profile": "release" if release_version else "debug",
        "release-version": release_version,
        "capture-ui": True,
        "pilot-cache-key": pair,
        "pilot-cache-phase": phase,
    }

    def resolve(match):
        key = match[1]
        if key not in inputs:
            raise ValueError(f"Missing Depot pilot value for Linux native input {key}")
        # Valid in both implicit if expressions and ${{ }} expressions.
        return f"fromJSON('{json.dumps(inputs[key])}')"

    body = re.sub(r"\binputs\.([a-z0-9-]+)\b", resolve, "env:\n" + env[1] + "\njobs:\n" + jobs)
    body = body.replace(runner_line, f"    runs-on: {RUNNER}\n")
    purpose = f"pilot ({phase})" if phase else f"release ({release_version})"
    return (
        f"name: Fleetworks Linux native {purpose}\n"
        "on: workflow_dispatch\n"
        "permissions:\n  contents: read\n\n" + body
    )


def depot_request(method, token, **fields):
    request = Request(
        f"{API}/{method}",
        data=json.dumps(fields).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Connect-Protocol-Version": "1",
        },
    )
    # Polling calls are safe to retry. Replaying Run can start a duplicate job.
    attempts = 1 if method == "Run" else 3
    for attempt in range(attempts):
        try:
            with urlopen(request, timeout=30) as response:
                return json.load(response)
        except HTTPError as error:
            status = error.code
            if status not in {429, 500, 502, 503, 504} or attempt == attempts - 1:
                raise RuntimeError(f"Depot {method} returned HTTP {status}") from None
        except (URLError, TimeoutError):
            if attempt == attempts - 1:
                raise RuntimeError(f"Depot {method} failed to connect") from None
        time.sleep(5)


def wait_for_run(token, run_id):
    deadline = time.monotonic() + 67 * 60
    previous = None
    while time.monotonic() < deadline:
        run = depot_request("GetRun", token, runId=run_id)
        status = run["status"]
        if status != previous:
            print(f"Depot run {run_id}: {status}", flush=True)
            previous = status
        if status in FAILED or status == "finished":
            return run
        time.sleep(15)
    raise RuntimeError(f"Timed out waiting for Depot run {run_id}")


def elapsed(started, finished):
    start = datetime.fromisoformat(started.replace("Z", "+00:00"))
    end = datetime.fromisoformat(finished.replace("Z", "+00:00"))
    seconds = (end - start).total_seconds()
    if seconds < 0:
        raise ValueError("Depot returned timestamps out of order")
    return seconds


def download_artifact(token, artifact, destination):
    url = depot_request("GetArtifactDownloadURL", token, artifactId=artifact["artifactId"])["url"]
    if urlsplit(url).scheme != "https":
        raise RuntimeError("Depot artifact URL must use HTTPS")
    try:
        # Use a separate request so the signed storage URL never sees the CI token.
        with urlopen(url, timeout=120) as response, destination.open("wb") as file:
            shutil.copyfileobj(response, file)
    except (URLError, TimeoutError):
        raise RuntimeError(f"Unable to download artifact {artifact['name']}") from None
    if destination.stat().st_size != int(artifact["sizeBytes"]):
        raise ValueError(f"Incomplete Depot artifact {artifact['name']}")


def fetch_artifacts(token, run_id, job, directory, sha, profile, phase=""):
    response = depot_request("ListArtifacts", token, runId=run_id, jobId=job["jobId"])
    artifacts = {}
    for kind, prefix in ARTIFACTS.items():
        matches = [
            item for item in response["artifacts"]
            if item["name"].startswith(f"{prefix}x86_64-{profile}-{sha}-")
            and (not phase or item["name"].endswith(f"-{phase}"))
        ]
        if len(matches) != 1:
            raise ValueError(f"Expected exactly one {kind} artifact from Depot run {run_id}")
        artifact = matches[0]
        destination = directory / f"{kind}.zip"
        download_artifact(token, artifact, destination)
        artifacts[kind] = destination
    return artifacts


def pilot_timings(archive, sha, phase):
    with ZipFile(archive) as evidence:
        cache_state = evidence.read("pilot-cache-state.txt").decode().strip()
        expected = f"{phase}: Cargo cache {'miss' if phase == 'cold' else 'hit'}"
        if cache_state != expected:
            raise ValueError(f"Depot {phase} run did not verify the expected Cargo cache state")
        timings = {}
        for line in evidence.read("fleetworks-timings.jsonl").splitlines():
            record = json.loads(line)
            if record["revision"] != sha or record["exit"] != 0:
                raise ValueError(f"Invalid timing record for Depot {phase} run")
            name, seconds = record["phase"], record["seconds"]
            if name in timings or not isinstance(seconds, (int, float)) or not math.isfinite(seconds) or seconds < 0:
                raise ValueError(f"Invalid timing for {phase} {name}")
            timings[name] = seconds
        if timings.keys() != {name for name, _ in TIMED_PHASES}:
            raise ValueError(f"Incomplete timing evidence for Depot {phase} run")
    return cache_state, timings


def run_native_job(args, token, workflow):
    created = depot_request(
        "Run", token, repo=args.repo, sha=args.sha, ref=args.ref, workflowContent=[workflow],
    )
    run_id = created["runId"]
    url = f"https://depot.dev/orgs/{created['orgId']}/workflows/{run_id}"
    print(f"Depot Linux native run: {url}", flush=True)
    try:
        run = wait_for_run(token, run_id)
    except (KeyboardInterrupt, RuntimeError):
        try:
            depot_request("CancelRun", token, runId=run_id)
        except RuntimeError as error:
            print(error, file=sys.stderr)
        raise
    if run["sha"] != args.sha or run["repo"].lower() != args.repo.lower():
        raise RuntimeError(f"Depot run identity did not match {args.repo}@{args.sha}")
    status = depot_request("GetRunStatus", token, runId=run_id)
    jobs = [job for workflow in status["workflows"] for job in workflow["jobs"]]
    if run["status"] != "finished" or len(jobs) != 1:
        raise RuntimeError(f"Depot verification failed or produced unexpected jobs: {url}")
    job = depot_request("GetJob", token, jobId=jobs[0]["jobId"])
    if job["jobConclusion"] != "success" or job["currentAttempt"] != 1:
        raise RuntimeError(f"Depot verification did not pass on the first attempt: {url}")
    return run, job, url


def benchmark(args, token, source, phase):
    workflow = render_workflow(source, args.sha, args.pair, phase)
    run, job, url = run_native_job(args, token, workflow)
    run_id = run["runId"]
    directory = args.output / phase
    directory.mkdir(parents=True, exist_ok=False)
    artifacts = fetch_artifacts(token, run_id, job, directory, args.sha, "debug", phase)
    cache_state, timings = pilot_timings(artifacts["verification"], args.sha, phase)
    return {
        "run_id": run_id,
        "url": url,
        "cache_state": cache_state,
        "queue_seconds": elapsed(run["createdAt"], job["jobStartedAt"]),
        "job_seconds": elapsed(job["jobStartedAt"], job["jobFinishedAt"]),
        "timings_seconds": timings,
    }


def markdown(comparison):
    cold, warm = (comparison["phases"][phase] for phase in ("cold", "warm"))
    lines = [
        f"### Depot Linux native pilot at `{comparison['sha']}`",
        "",
        "| Duration | Depot cold (Cargo cache miss) | Depot warm (Cargo cache hit) |",
        "| --- | ---: | ---: |",
        f"| Queue and startup | {cold['queue_seconds']:.1f} s | {warm['queue_seconds']:.1f} s |",
        f"| Job wall time | {cold['job_seconds']:.1f} s | {warm['job_seconds']:.1f} s |",
    ]
    for key, label in TIMED_PHASES:
        lines.append(f"| {label} | {cold['timings_seconds'][key]:.1f} s | {warm['timings_seconds'][key]:.1f} s |")
    if cold["job_seconds"] > 0:
        change = (warm["job_seconds"] / cold["job_seconds"] - 1) * 100
        lines.extend(["", f"Warm job wall time changed by {change:+.1f}% from cold."])
    lines.extend(["", f"[Depot cold run]({cold['url']}) · [Depot warm run]({warm['url']})", ""])
    return "\n".join(lines)


def nightly_release(args, token, source):
    if args.ref != "refs/heads/nightly":
        raise ValueError("Depot release dispatch requires the nightly branch")
    workflow = render_workflow(source, args.sha, "", "", args.release_version)
    run, job, url = run_native_job(args, token, workflow)
    args.output.mkdir(parents=True, exist_ok=False)
    with TemporaryDirectory(dir=args.output) as temporary:
        artifacts = fetch_artifacts(token, run["runId"], job, Path(temporary), args.sha, "release")
        for kind, archive in artifacts.items():
            with ZipFile(archive) as package:
                package.extractall(args.output / kind)
    return (
        f"### Linux native release built on Depot at `{args.sha}`\n\n"
        f"[Verified release build]({url}) · "
        f"{elapsed(job['jobStartedAt'], job['jobFinishedAt']):.1f} s job wall time\n"
    )


def run_pilot(args, token, source):
    phases = {}
    for phase in ("cold", "warm"):
        result = benchmark(args, token, source, phase)
        (args.output / phase / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        phases[phase] = result
    comparison = {"repository": args.repo, "sha": args.sha, "runner": RUNNER, "phases": phases}
    (args.output / "comparison.json").write_text(json.dumps(comparison, indent=2) + "\n", encoding="utf-8")
    return markdown(comparison)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--ref", required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--pair")
    mode.add_argument("--release-version")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.sha):
        parser.error("--sha must be a full lower-case Git SHA")
    expected_pair = r"linux-native-pilot-v1-[0-9]+-[0-9]+-" + args.sha
    if args.pair is not None and not re.fullmatch(expected_pair, args.pair):
        parser.error("--pair must identify the GitHub run, attempt and exact SHA")
    if args.release_version is not None and not re.fullmatch(r"[0-9][A-Za-z0-9.+-]*", args.release_version):
        parser.error("--release-version must be a version starting with a digit")

    token = os.environ.get("DEPOT_CI_TOKEN")
    if not token:
        parser.error("Configure the DEPOT_LINUX_NATIVE_PILOT_TOKEN Actions secret with a Depot CI token")
    source = WORKFLOW.read_text(encoding="utf-8")
    report = run_pilot(args, token, source) if args.pair else nightly_release(args, token, source)
    print(report)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(summary).open("a", encoding="utf-8") as file:
            file.write(report)


def interrupt(_signum, _frame):
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupt)
    try:
        main()
    except (BadZipFile, KeyError, OSError, RuntimeError, ValueError) as error:
        sys.exit(str(error))
    except KeyboardInterrupt:
        sys.exit("Depot Linux native pilot cancelled")
