#!/usr/bin/env python3
"""Verify streaming in the packaged Inbox window on an isolated X11 display."""

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import socket
import subprocess
import tempfile
import time

from fleetworks_local_backend_smoke import check_zero_bound_sockets, resolve_companion_binary
from fleetworks_qa_pack import MANIFEST, WORKER, install, sha256, verify_directory

SUMMARY_QUESTION = "What is the current workstation readiness?"
TERMINAL = {"answered", "insufficient_context", "failed", "canceled"}


def require(condition, description):
    if not condition:
        raise AssertionError(description)


def wait_for(probe, app, description, timeout=90):
    from gi.repository import GLib

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if app.poll() is not None:
            raise RuntimeError(f"Fleetworks exited at {description} with status {app.returncode}")
        try:
            value = probe()
        except GLib.Error as error:
            # Transitions can unregister a subtree between enumerating a node and
            # looking at its children. The next probe reacquires the live tree.
            if (error.domain, error.code, error.message) != (
                "atspi_error", 0, "The application no longer exists"
            ):
                raise
            if app.poll() is not None:
                raise RuntimeError(f"Fleetworks exited at {description} with status {app.returncode}") from error
            value = None
        if value is not None:
            return value
        time.sleep(0.1)
    if app.poll() is not None:
        raise RuntimeError(f"Fleetworks exited at {description} with status {app.returncode}")
    raise TimeoutError(f"Fleetworks window did not reach {description}")


def descendants(root):
    yield root
    for child in root:
        if child is not None:
            yield from descendants(child)


def install_gated_pack(home, args):
    digests = json.loads(args.digests.read_text())
    require(digests["target"] == "x86_64-unknown-linux-gnu", "expected a Linux x86_64 worker pack")
    fixture = home / "inbox-qa-pack"
    install(argparse.Namespace(
        archive=args.archive, output=fixture, state_dir=None, target=digests["target"],
        archive_sha256=digests["archive_sha256"], manifest_sha256=digests["manifest_sha256"],
    ))
    worker = fixture / WORKER
    shutil.copyfile(args.gated_worker, worker)
    worker.chmod(0o700)
    manifest = fixture / MANIFEST
    contents = json.loads(manifest.read_text())
    contents["files"][WORKER] = sha256(worker)
    manifest.write_text(json.dumps(contents, sort_keys=True) + "\n")
    digest = sha256(manifest)
    verify_directory(fixture, digests["target"], digest)
    return worker, {"root": str(fixture), "manifest_sha256": digest}


def companion_proxy(home, app):
    proxy = home / "argos-api-host"
    journal = home / "qa-ipc-journal.jsonl"
    observer = Path(__file__).resolve().with_name("fleetworks_qa_ipc_observer.py")
    command = ["/usr/bin/python3", "-B", str(observer), str(resolve_companion_binary(app=app)), str(journal)]
    proxy.write_text("#!/bin/sh\nexec " + shlex.join(command) + ' "$@"\n')
    proxy.chmod(0o700)
    return proxy, journal


def read_journal(journal):
    # Ignore an in-progress write, if any. The observer owns an exclusive fixture file.
    return [json.loads(line) for line in journal.read_text().splitlines(keepends=True) if line.endswith("\n")]


def screen_bounds(node, atspi):
    if node is None or not node.getState().contains(atspi.STATE_SHOWING):
        return None
    x, y, width, height = node.queryComponent().getExtents(atspi.DESKTOP_COORDS)
    if width <= 0 or height <= 0 or x < 0 or y < 0:
        return None
    return x, y, width, height


def release_gate(companion, worker):
    # Look up the exact pack binary and direct companion parent before signaling
    # its Linux pidfd. A recycled PID must not redirect the release signal.
    matches = []
    for process in Path("/proc").iterdir():
        if not process.name.isdigit():
            continue
        try:
            fd = os.pidfd_open(int(process.name))
        except OSError:
            continue
        try:
            parent = int((process / "stat").read_text().rpartition(") ")[2].split()[1])
            if parent == companion and (process / "exe").readlink() == worker:
                matches.append((os.dup(fd), int(process.name)))
        except (OSError, IndexError, ValueError):
            continue
        finally:
            os.close(fd)
    try:
        require(len(matches) == 1, "expected one live isolated gated worker")
        fd, pid = matches[0]
        check_zero_bound_sockets(companion)
        check_zero_bound_sockets(pid)
        signal.pidfd_send_signal(fd, signal.SIGUSR1)
    finally:
        for fd, _ in matches:
            os.close(fd)


def event_pages(records, turn_id):
    return [row for row in records if row.get("op") == "qa_events" and row.get("turn_id") == turn_id]


def settled_turn(pages, first):
    require(pages and all(not row["gap"] for row in pages), "fixture IPC event stream contained a cursor gap")
    final = pages[-1]
    require(final["state"] == "answered", "same IPC turn did not finish answered")
    events = {}
    for page in pages:
        for event in page["events"]:
            prior = events.setdefault(event["sequence"], event)
            require(event == prior, "IPC replay changed an already delivered event")
    sequence = sorted(events)
    require(sequence == list(range(1, final["cursor"] + 1)), "IPC events were not consecutive")
    ordered = [events[position] for position in sequence]
    terminal = [event for event in ordered if event["state"] in TERMINAL]
    require(len(terminal) == 1 and terminal[0]["sequence"] == final["cursor"],
            "expected exactly one IPC terminal event")
    findings = [event["finding"] for event in ordered if event["finding"] is not None]
    require(findings == final["findings"] and len(set(findings)) == len(findings),
            "final exchange contains duplicate or missing findings")
    require(findings[:1] == first["findings"], "final exchange replaced its early finding")
    return final


def test_window(app, registry, atspi, home, worker, pointer, journal, output):
    def find(key, *, button=False, label=None):
        # Match the process we launched, even if another application is registered.
        for window in registry.getDesktop(0):
            if window is None or window.get_process_id() != app.pid:
                continue
            for node in descendants(window):
                if node.get_accessible_id() != key or (label is not None and node.name != label):
                    continue
                state = node.getState()
                if state.contains(atspi.STATE_DEFUNCT):
                    continue
                if button and (node.getRole() != atspi.ROLE_PUSH_BUTTON or not state.contains(atspi.STATE_ENABLED)):
                    continue
                return node
        return None

    def button(key, label=None):
        def activate():
            node = find(key, label=label, button=True)
            if node is None:
                return None
            if not node.queryAction().doAction(0):
                raise RuntimeError(f"Fleetworks could not activate {key}")
            return True

        wait_for(activate, app, key)

    button("startup-local")
    button("connect-local")
    button("gateway-setup-done")
    state = home / ".fleetworks/local-backend/embedded"
    fd = os.open(state / "qa-pack.json", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(fd, "w") as file:
        json.dump(pointer, file)
    button("startup-data-skip")
    button("inbox-qa-toggle", "Turn On")
    button("startup-inbox-qa-continue")
    button("startup-data-continue", "Open Workstation")
    button("inbox-toggle")

    def submit(question, *, timeout=20):
        x, y, width, height = wait_for(
            lambda: screen_bounds(find("inbox-qa-composer"), atspi), app, "available QA composer", timeout=timeout,
        )
        # A bare Xvfb display has no window manager to grant keyboard focus.
        windows = subprocess.check_output(
            ["xdotool", "search", "--onlyvisible", "--name", "^Fleetworks$"], text=True, timeout=5,
        ).split()
        require(len(windows) == 1, "expected one visible Fleetworks window on the fixture display")
        subprocess.run(
            ["xdotool", "windowfocus", "--sync", windows[0],
             "mousemove", str(x + width // 2), str(y + height // 2), "click", "1"],
            check=True, timeout=5,
        )
        subprocess.run(["xdotool", "type", "--clearmodifiers", question], check=True, timeout=5)
        wait_for(lambda: find("inbox-qa-send", button=True), app, "enabled QA Send")
        submitted_at = time.monotonic()
        # Submit via the already-focused composer, using the same keyboard path as an operator.
        subprocess.run(["xdotool", "key", "--clearmodifiers", "Return"], check=True, timeout=5)
        return submitted_at

    started = submit(SUMMARY_QUESTION, timeout=110)

    def first_frame():
        records = read_journal(journal)
        admitted = [row for row in records if row.get("op") == "qa_turn_submit"]
        if not admitted:
            return None
        require(len(admitted) == 1, "the UI submitted more than one exchange")
        pages = event_pages(records, admitted[0]["turn_id"])
        require(not any(row["state"] in TERMINAL for row in pages),
                "the turn finished before the barrier was released")
        if not pages or pages[-1]["state"] != "running" or len(pages[-1]["findings"]) != 1:
            return None
        citation = find("chat-citation-0")
        stop = find("inbox-qa-stop", button=True)
        if screen_bounds(citation, atspi) is None or screen_bounds(stop, atspi) is None:
            return None
        require(find("chat-citation-1") is None, "the drawer showed another citation before release")
        companions = [row["companion_pid"] for row in records if "companion_pid" in row]
        require(len(companions) == 1, "expected one IPC companion")
        return pages[-1], companions[0], citation.name

    early, companion, citation_name = wait_for(first_frame, app, "an early citation and enabled Stop", timeout=16)
    first_ms = round((time.monotonic() - started) * 1000, 1)
    subprocess.run(["scrot", "--overwrite", str(output / "first-finding.png")], check=True, timeout=5)
    wait_for(first_frame, app, "the early citation and enabled Stop while completion is held", timeout=2)
    release_gate(companion, worker)

    def completed_frame():
        records = read_journal(journal)
        pages = event_pages(records, early["turn_id"])
        if not pages or pages[-1]["state"] not in TERMINAL:
            return None
        require(len([row for row in records if row.get("op") == "qa_turn_submit"]) == 1,
                "the UI admitted another turn during completion")
        final = settled_turn(pages, early)
        if find("inbox-qa-stop") is not None:
            return None
        if screen_bounds(find("chat-citation-0", label=citation_name), atspi) is None:
            return None
        count = len(final["findings"])
        require(all(find(f"chat-citation-{i}") is not None for i in range(count))
                and find(f"chat-citation-{count}") is None, "drawer citations do not match the canonical exchange")
        return final

    final = wait_for(completed_frame, app, "completion in the same drawer", timeout=12)
    completion_ms = round((time.monotonic() - started) * 1000, 1)
    subprocess.run(["scrot", "--overwrite", str(output / "completed-turn.png")], check=True, timeout=5)

    submit("What is the current memory usage?")

    def grounded_memory():
        admitted = [row for row in read_journal(journal) if row.get("op") == "qa_turn_submit"]
        if len(admitted) < 2:
            return None
        require(len(admitted) == 2, "the GUI submitted more than one metric turn")
        turn = admitted[-1]
        sample = turn["memory_observation"]
        require(turn["turn_id"] != final["turn_id"] and turn["state"] == "answered"
                and len(turn["findings"]) == 1 and sample is not None,
                "the metric did not resolve to one frozen memory observation")
        whole, tenths = divmod(sample["value"], 10)
        answer = find("chat-message-inbox-qa-answer")
        if answer is None:
            return None
        expected_text = f"The API host memory usage is {whole}.{tenths}%."
        expected_citation = f"Host metric: API host memory usage · {sample['observed_at']}"
        citation = find("chat-citation-0", label=expected_citation)
        if screen_bounds(citation, atspi) is None or not any(
            node.name == expected_text and screen_bounds(node, atspi) is not None
            for node in descendants(answer)
        ):
            return None
        require(find("chat-citation-1") is None and find("inbox-qa-stop") is None,
                "the completed memory answer showed unrelated evidence or an unfinished turn")
        return sample

    memory = wait_for(grounded_memory, app, "grounded memory reading and its original timestamp", timeout=16)
    subprocess.run(["scrot", "--overwrite", str(output / "grounded-memory.png")], check=True, timeout=5)
    result = {
        "runtime": "embedded-stdio", "graphical": "x11", "first_citation_while_running": True,
        "stop_enabled_before_release": True, "same_turn": True, "terminal_events": 1,
        "finding_count": len(final["findings"]), "duplicate_findings": 0,
        "first_finding_ms": first_ms, "completion_ms": completion_ms,
        "grounded_memory": memory,
    }
    (output / "streaming.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--digests", type=Path, required=True)
    parser.add_argument("--gated-worker", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    session = os.environ.get("DBUS_SESSION_BUS_ADDRESS", "")
    display = os.environ.get("DISPLAY", "")
    if not session.startswith("unix:") or not re.fullmatch(r":\d+(\.\d+)?", display):
        parser.error("run inside dbus-run-session and an isolated Xvfb display")
    if [name for _, name in socket.if_nameindex()] != ["lo"]:
        parser.error("run in a fresh Linux network namespace")

    import pyatspi

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    require(not any(output.iterdir()), "evidence directory must be empty")
    with tempfile.TemporaryDirectory(prefix="fleetworks-qa-linux-gui-") as directory:
        home = Path(directory).resolve()
        runtime = home / "run"
        runtime.mkdir(mode=0o700)
        worker, pointer = install_gated_pack(home, args)
        proxy, journal = companion_proxy(home, args.app)
        environment = {**os.environ, "HOME": str(home), "XDG_RUNTIME_DIR": str(runtime),
                       "XDG_CACHE_HOME": str(home / "cache"), "XDG_CONFIG_HOME": str(home / "config"),
                       "XDG_DATA_HOME": str(home / "data"), "XDG_STATE_HOME": str(home / "state")}
        environment.pop("WAYLAND_DISPLAY", None)
        subprocess.run(["gdbus", "call", "--session", "--dest", "org.a11y.Bus",
                        "--object-path", "/org/a11y/bus", "--method", "org.freedesktop.DBus.Properties.Set",
                        "org.a11y.Status", "IsEnabled", "<true>"], env=environment, check=True, timeout=10)
        with subprocess.Popen([str(args.app.resolve() / "bin/fleetworks-desktop"), "--diagnostic",
                               "--local-backend-bin", str(proxy)], env=environment) as app:
            try:
                test_window(app, pyatspi.Registry, pyatspi, home, worker, pointer, journal, output)
            except Exception:
                if app.poll() is None:
                    try:
                        subprocess.run(["scrot", "--overwrite", str(output / "failed-stage.png")], timeout=3)
                    except (OSError, subprocess.TimeoutExpired):
                        pass
                raise
            finally:
                if app.poll() is None:
                    app.terminate()
                    try:
                        app.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        app.kill()


if __name__ == "__main__":
    main()
