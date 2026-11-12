#!/usr/bin/env python3
"""Run the SurrealDB storage contract suite against a real, pinned disk server.

Downloads the pinned SurrealDB server for this host (verified by SHA-256),
caches it, checks that it starts on loopback with RocksDB under a temporary
directory (sync on every commit, versioning off, appendix tuning profile),
stops it, then runs `tests/storage_contracts.rs`, `tests/storage_jobs.rs`, `tests/storage_memory.rs` `tests/storage_insights.rs` `tests/storage_atlas.rs` `tests/storage_intel.rs` `tests/storage_lifecycle.rs` `tests/storage_atlas_delete.rs` `tests/storage_atlas_metadata.rs` `tests/storage_provenance.rs` `tests/storage_refresh.rs` `tests/storage_bootstrap.rs` `tests/storage_open.rs` `tests/storage_home.rs` `tests/storage_server.rs` `tests/storage_commands.rs` `tests/storage_install.rs` `tests/storage_runtime.rs` `tests/storage_memories.rs` `tests/storage_insight_views.rs` `tests/storage_vectors.rs` `tests/storage_browse.rs` `tests/storage_recon.rs` `tests/storage_browse_assoc.rs` `tests/storage_derivations.rs` `tests/storage_pipeline_inputs.rs` `tests/storage_refresh_exec.rs`, `tests/storage_topology.rs`, `tests/storage_topology_gaps.rs`, `tests/storage_entities.rs`, `tests/storage_live_evidence.rs`, `tests/storage_credits.rs`, `tests/storage_jobs_view.rs`, `tests/storage_recon_runs.rs`, `tests/storage_profile_view.rs`, `tests/storage_recon_exec.rs` `tests/storage_recon_delete.rs`, `tests/storage_recon_extract.rs` and `tests/storage_insight_owner.rs`, `tests/storage_recon_dispatch.rs`, `tests/storage_telemetry.rs`, `tests/storage_recon_jobs.rs`, `tests/storage_summaries.rs`, `tests/storage_brief.rs`, `tests/storage_recall_turn.rs`, `tests/storage_recon_delete_insights.rs`, `tests/storage_pipeline_exec.rs`, `tests/storage_investigation.rs`, `tests/storage_atlas_store.rs`, `tests/storage_logs.rs`, `tests/storage_atlas_memory.rs`, `tests/storage_actor_index.rs`, `tests/storage_app_jobs.rs`, `tests/storage_insight_retry.rs`, `tests/storage_index_drain.rs`, `tests/storage_actor_dedup.rs`, `tests/storage_brain_backfill.rs`, `tests/storage_summary_flush.rs`, `tests/storage_brain_scope.rs`, `tests/storage_store_bridge.rs`, `tests/storage_processes.rs` with ARGOS_SURREAL_BIN set and
ARGOS_SURREAL_REQUIRED=1, so a missing server is a failure rather than a skip.
Each contract test starts and stops its own server in its own temp directory.
Release pins and checksum provenance live in surrealdb-release.json.

`--mode embedded` instead runs the same suite against an in-process RocksDB
datastore (`--features embedded`, ARGOS_SURREAL_MODE=embedded); no server is
downloaded. `--mode both` (default) runs server mode, then embedded mode.

Never touches an Argos home, argos.db or memory_lancedb/.

Usage:
  python3 scripts/test_surreal.py            # server + embedded contracts
  python3 scripts/test_surreal.py --mode server
  python3 scripts/test_surreal.py --fetch-only
  python3 scripts/test_surreal.py --cargo cargo   # CI: plain cargo, no agent target dir
Options: --cache-dir DIR (default ~/.cache/argos-surreal), --bin PATH (skip download).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import secrets
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
RELEASE_PATH = Path(__file__).resolve().with_name("surrealdb-release.json")
HOST_ASSETS = {
    ("Linux", "x86_64"): "linux-amd64",
    ("Linux", "aarch64"): "linux-arm64",
    ("Linux", "arm64"): "linux-arm64",
    ("Darwin", "x86_64"): "darwin-amd64",
    ("Darwin", "arm64"): "darwin-arm64",
}
# These required-mode fixtures otherwise skip in the ordinary workspace run.
CUTOVER_CONTRACTS = (
    "storage_connect_registry",
    "storage_workstation_activity",
    "storage_workstation_paging",
    "storage_job_actions",
    "storage_setup_jobs",
    "storage_configuration_reset_barrier",
    "storage_actor_search_outbox",
    "storage_actor_search_rebuild",
    "storage_actor_seed_execution",
    "storage_classifier_activation",
    "storage_operational_logs",
)
# Disk profile from the storage audit appendix (section 4).
PROFILE = {
    "SURREAL_ROCKSDB_BLOCK_CACHE_SIZE": "67108864",
    "SURREAL_DISKANN_CACHE_SIZE": "33554432",
    "SURREAL_GRAPH_RESOLVE_CACHE_SIZE": "8388608",
    "SURREAL_ROCKSDB_WRITE_BUFFER_SIZE": "16777216",
    "SURREAL_ROCKSDB_MAX_WRITE_BUFFER_NUMBER": "2",
    "SURREAL_ROCKSDB_MIN_WRITE_BUFFER_NUMBER_TO_MERGE": "2",
    "SURREAL_ROCKSDB_JOBS_COUNT": "2",
    "SURREAL_ROCKSDB_THREAD_COUNT": "2",
    "SURREAL_ROCKSDB_MAX_CONCURRENT_SUBCOMPACTIONS": "1",
    "SURREAL_KVS_THREADPOOL_SIZE": "4",
    "SURREAL_SCAN_BATCH_BYTES": "524288",
    "SURREAL_MEMORY_THRESHOLD": "402653184",
    "SURREAL_DATASTORE_VERSIONED": "false",
    "SURREAL_DATASTORE_SYNC_DATA": "every",
}


def load_release():
    try:
        release = json.loads(RELEASE_PATH.read_text(encoding="utf-8"))
        version = release["version"]
        hashes = release["archive_sha256"]
        if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
            raise ValueError("invalid version")
        if not isinstance(hashes, dict) or set(hashes) != set(HOST_ASSETS.values()):
            raise ValueError("expected one checksum per supported release archive")
        if any(not isinstance(h, str) or not re.fullmatch(r"[0-9a-f]{64}", h) for h in hashes.values()):
            raise ValueError("expected lowercase SHA-256 checksums")
        if len(set(hashes.values())) != len(hashes):
            raise ValueError("expected distinct archive checksums")
        template = release["release_url_template"]
        if not isinstance(template, str) or "{v}" not in template or "{p}" not in template:
            raise ValueError("invalid release URL template")
        template.format(v=version, p="linux-amd64")
    except (OSError, ValueError, TypeError, KeyError, IndexError) as error:
        sys.exit(f"test-surreal: invalid SurrealDB release manifest: {error}")
    return release


def fetch(cache_dir, release):
    version = release["version"]
    key = (platform.system(), platform.machine())
    if key not in HOST_ASSETS:
        sys.exit(f"test-surreal: no pinned SurrealDB {version} build for {key}")
    name = HOST_ASSETS[key]
    digest = release["archive_sha256"][name]
    target = cache_dir / f"{version}-{name}" / "surreal"
    if target.is_file():
        return target
    target.parent.mkdir(parents=True, exist_ok=True)
    url = release["release_url_template"].format(v=version, p=name)
    print(f"test-surreal: downloading {url}", flush=True)
    with tempfile.TemporaryDirectory() as tmp:
        archive = Path(tmp) / "surreal.tgz"
        with urllib.request.urlopen(url, timeout=300) as response, open(archive, "wb") as out:
            shutil.copyfileobj(response, out)
        actual = hashlib.sha256(archive.read_bytes()).hexdigest()
        if actual != digest:
            sys.exit(f"test-surreal: SHA-256 mismatch for {name}: {actual} != {digest}")
        with tarfile.open(archive) as tar:
            member = tar.getmember("surreal")
            if not member.isfile():
                sys.exit("test-surreal: archive has no surreal binary")
            tar.extract(member, tmp)
        shutil.move(str(Path(tmp) / "surreal"), target)
    target.chmod(0o755)
    return target


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def smoke(binary, expected_version):
    """Start the server on loopback with RocksDB in a temp dir, then stop it."""
    version = subprocess.run([str(binary), "version"], capture_output=True, text=True, check=True).stdout
    if not version.startswith(expected_version):
        sys.exit(f"test-surreal: expected {expected_version}, got {version!r}")
    with tempfile.TemporaryDirectory() as tmp:
        port = free_port()
        env = {**os.environ, **PROFILE, "SURREAL_USER": "root", "SURREAL_PASS": secrets.token_hex(16)}
        url = f"rocksdb://{tmp}/surreal?versioned=false&sync=every"
        log_path = Path(tmp) / "server.log"
        with open(log_path, "w") as log:
            server = subprocess.Popen(
                [str(binary), "start", "--no-banner", "--log", "info", "--bind", f"127.0.0.1:{port}", url],
                env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
            )
            try:
                deadline = time.time() + 30
                while True:
                    try:
                        socket.create_connection(("127.0.0.1", port), timeout=1).close()
                        break
                    except OSError:
                        if server.poll() is not None or time.time() > deadline:
                            sys.exit("test-surreal: server failed to start:\n" + log_path.read_text())
                        time.sleep(0.1)
            finally:
                server.terminate()
                try:
                    server.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()
        text = log_path.read_text()
        for needle in ("Started rocksdb kvs store", "Sync mode: every transaction commit", f"127.0.0.1:{port}"):
            if needle not in text:
                sys.exit(f"test-surreal: server log lacks {needle!r}:\n{text}")
    print(f"test-surreal: {version.strip()} started on loopback with RocksDB (sync every commit) and stopped", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--cache-dir", type=Path, default=Path.home() / ".cache" / "argos-surreal")
    parser.add_argument("--bin", type=Path, help="use this surreal binary instead of downloading")
    parser.add_argument("--fetch-only", action="store_true")
    parser.add_argument("--mode", choices=("server", "embedded", "both"), default="both")
    parser.add_argument("--cargo", default="agent", help="'agent' (scripts/agent_cargo.py) or a cargo executable")
    args, extra = parser.parse_known_args()
    cargo = [sys.executable, str(ROOT / "scripts" / "agent_cargo.py")] if args.cargo == "agent" else [args.cargo]
    modes = ("server", "embedded") if args.mode == "both" else (args.mode,)
    binary = None
    if "server" in modes:
        release = load_release()
        binary = args.bin.resolve() if args.bin else fetch(args.cache_dir, release)
        print(f"test-surreal: server binary {binary}", flush=True)
        if args.fetch_only:
            return
        smoke(binary, release["version"])
    elif args.fetch_only:
        return
    for mode in modes:
        # Same feature sets as scripts/local_matrix.sh, so each leg reuses
        # its clippy/test build: embedded is the default build, server is
        # `--no-default-features --features server,nlp,classifier`.
        features = [] if mode == "embedded" else ["--no-default-features", "--features", "server,nlp,classifier"]
        cutover_targets = [argument for name in CUTOVER_CONTRACTS for argument in ("--test", name)]
        command = [*cargo, "test", "--workspace", "--locked", *features, "--test", "storage_contracts", "--test", "storage_jobs", "--test", "storage_memory", "--test", "storage_insights", "--test", "storage_atlas", "--test", "storage_article_discovery", "--test", "storage_classifier_admission",
            "--test", "storage_atlas_derivatives", "--test", "storage_classifier_single_domain", "--test", "storage_admission_maintenance", "--test", "storage_classifier_calibration", "--test", "storage_intel", "--test", "storage_lifecycle", "--test", "storage_atlas_delete", "--test", "storage_atlas_metadata", "--test", "storage_provenance", "--test", "storage_refresh", "--test", "storage_bootstrap", "--test", "storage_open", "--test", "storage_home", "--test", "storage_server", "--test", "storage_commands", "--test", "storage_install", "--test", "storage_runtime", "--test", "storage_memories", "--test", "storage_insight_views", "--test", "storage_vectors", "--test", "storage_browse", "--test", "storage_recon", "--test", "storage_browse_assoc", "--test", "storage_derivations", "--test", "storage_pipeline_inputs", "--test", "storage_refresh_exec", "--test", "storage_topology", "--test", "storage_topology_gaps", "--test", "storage_entities", "--test", "storage_live_evidence", "--test", "storage_credits", "--test", "storage_provider_pacer", "--test", "storage_provider_account", "--test", "storage_jobs_view", "--test", "storage_recon_runs", "--test", "storage_profile_view", "--test", "storage_recon_exec", "--test", "storage_recon_delete", "--test", "storage_recon_extract", "--test", "storage_insight_owner", "--test", "storage_recon_dispatch", "--test", "storage_telemetry", "--test", "storage_recon_jobs", "--test", "storage_summaries", "--test", "storage_brief", "--test", "storage_recall_turn", "--test", "storage_recon_delete_insights", "--test", "storage_pipeline_exec", "--test", "storage_investigation", "--test", "storage_atlas_store", "--test", "storage_logs", "--test", "storage_atlas_memory", "--test", "storage_actor_index", "--test", "storage_actor_catalog", "--test", "storage_intel_articles", "--test", "storage_app_jobs", "--test", "storage_insight_retry", "--test", "storage_index_drain", "--test", "storage_actor_dedup", "--test", "storage_brain_backfill", "--test", "storage_summary_flush", "--test", "storage_brain_scope", "--test", "storage_store_bridge", "--test", "storage_processes", "--test", "storage_brief_publish", "--test", "storage_curated_links", "--test", "storage_intelligence_contracts", "--test", "storage_indexed_links", *cutover_targets, *extra, "--", "--nocapture"]
        env = {**os.environ, "ARGOS_SURREAL_MODE": mode, "ARGOS_SURREAL_REQUIRED": "1"}
        if binary:
            env["ARGOS_SURREAL_BIN"] = str(binary)
        for name in ("SURREAL_USER", "SURREAL_PASS"):
            env.pop(name, None)
        print(f"test-surreal: [{mode}] " + " ".join(command), flush=True)
        started = time.time()
        code = subprocess.run(command, cwd=ROOT, env=env).returncode
        print(f"test-surreal: [{mode}] exit {code} in {time.time() - started:.0f}s", flush=True)
        if code:
            sys.exit(code)

if __name__ == "__main__":
    main()
