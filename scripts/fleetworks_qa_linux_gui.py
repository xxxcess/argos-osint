#!/usr/bin/env python3
"""Verify offline startup and Inbox streaming in the packaged Linux window."""

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import time

from fleetworks_local_backend_smoke import check_zero_bound_sockets, resolve_companion_binary
from fleetworks_qa_pack import install_gated_fixture
from fleetworks_qa_smoke import FIXTURE_MODEL_MEMORY_MIB

SUMMARY_QUESTION = "What is the current workstation readiness?"
MEMORY_QUESTION = "What's our RAM consumption right now?"
SOURCES_TOGGLE = "inbox-qa-sources-toggle"
SOURCES_CONTENT = "inbox-qa-sources"
DRAWER = "inbox-layer"
COMPOSER = "inbox-qa-composer"
COMPOSER_LABEL = "Question for the Inbox assistant"
WORKSTATION_ASSISTANT = "workstation-open-assistant"
TERMINAL = {"answered", "insufficient_context", "failed", "canceled"}
BUNDLED_ACTOR_ROWS = 94_987


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
    worker, pointer = install_gated_fixture(args.archive, home / "inbox-qa-pack", args.gated_worker, digests)
    return worker, pointer, digests


def companion_proxy(home, app):
    proxy = home / "argos-api-host"
    journal = home / "qa-ipc-journal.jsonl"
    observer = Path(__file__).resolve().with_name("fleetworks_qa_ipc_observer.py")
    # The real launcher strips ambient settings before starting the fixture proxy.
    command = ["/usr/bin/env", f"FLEETWORKS_QA_MEMORY_MIB={FIXTURE_MODEL_MEMORY_MIB}",
               "/usr/bin/python3", "-B", str(observer), str(resolve_companion_binary(app=app)), str(journal)]
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


def app_roots(app, registry):
    return (node for node in registry.getDesktop(0) if node is not None and node.get_process_id() == app.pid)


def query_text(node):
    try:
        return node.queryText()
    except NotImplementedError:
        # AccessKit exports the AT-SPI Text interface only when text runs exist.
        return None


def retain_focus_diagnostics(app, registry, atspi, output, error):
    from gi.repository import GLib

    evidence = {"failure": str(error)[:240], "app_pid": app.pid, "nodes": []}
    try:
        focus = subprocess.check_output(["xdotool", "getwindowfocus"], text=True, timeout=3).strip()
        evidence["x11_focused_window"] = focus if focus.isdecimal() else None
    except (OSError, subprocess.SubprocessError):
        evidence["x11_focused_window"] = None

    states = {"focused": atspi.STATE_FOCUSED, "focusable": atspi.STATE_FOCUSABLE,
              "editable": atspi.STATE_EDITABLE, "enabled": atspi.STATE_ENABLED,
              "showing": atspi.STATE_SHOWING}
    try:
        pending = [(node, 0) for node in app_roots(app, registry)]
        for _ in range(96):
            if not pending:
                break
            node, depth = pending.pop()
            try:
                state = node.getState()
                if state.contains(atspi.STATE_DEFUNCT):
                    continue
                pending.extend((child, depth + 1) for child in reversed(list(node)) if child is not None)
                flags = {name: state.contains(value) for name, value in states.items()}
                row = {"depth": depth, "id": (node.get_accessible_id() or "")[:96], "role": node.getRoleName(),
                       "name": (node.name or "")[:120], "states": flags, "bounds": screen_bounds(node, atspi)}
                if flags["editable"] and (flags["focused"] or row["name"] == COMPOSER_LABEL):
                    edit = query_text(node)
                    row["text_interface"] = edit is not None
                    if edit is not None:
                        row["text_characters"] = edit.characterCount
                        row["text"] = edit.getText(0, min(row["text_characters"], 256))
                evidence["nodes"].append(row)
            except (GLib.Error, NotImplementedError):
                continue
        evidence["truncated"] = bool(pending)
    except GLib.Error:
        evidence["tree_unavailable"] = True
    (output / "failed-accessibility.json").write_text(json.dumps(evidence, indent=2) + "\n")


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


def settled_turn(pages, first, expected_state):
    require(pages and all(not row["gap"] for row in pages), "fixture IPC event stream contained a cursor gap")
    final = pages[-1]
    require(final["state"] == expected_state, f"same IPC turn did not finish {expected_state}")
    events = {}
    for page in pages:
        for event in page["events"]:
            prior = events.setdefault(event["sequence"], event)
            require(event == prior, "IPC replay changed an already delivered event")
    sequence = sorted(events)
    require(sequence == list(range(1, final["cursor"] + 1)), "IPC events were not consecutive")
    ordered = [events[position] for position in sequence]
    terminal = [event for event in ordered if event["state"] in TERMINAL]
    require(len(terminal) == 1 and terminal[0]["sequence"] == final["cursor"]
            and terminal[0]["state"] == expected_state,
            "expected exactly one IPC terminal event")
    findings = [event["finding"] for event in ordered if event["finding"] is not None]
    require(findings == final["findings"] and len(set(findings)) == len(findings),
            "final exchange contains duplicate or missing findings")
    require(findings[:1] == first["findings"], "final exchange replaced its early finding")
    return {**final, "terminal_events": len(terminal)}


def test_window(app, registry, atspi, home, worker, pointer, digests, journal, output):
    def find(key, *, button=False, label=None):
        # Match the process we launched, even if another application is registered.
        for window in app_roots(app, registry):
            for node in descendants(window):
                if (key is not None and node.get_accessible_id() != key
                        or label is not None and node.name != label):
                    continue
                state = node.getState()
                if state.contains(atspi.STATE_DEFUNCT):
                    continue
                if button and (node.getRole() != atspi.ROLE_PUSH_BUTTON or not state.contains(atspi.STATE_ENABLED)):
                    continue
                return node
        return None

    def button(key, label=None, *, timeout=90):
        def activate():
            node = find(key, label=label, button=True)
            if node is None:
                return None
            if not node.queryAction().doAction(0):
                raise RuntimeError(f"Fleetworks could not activate {key}")
            return True

        wait_for(activate, app, key, timeout=timeout)

    def sources_ready(expanded):
        toggle = find(SOURCES_TOGGLE, button=True)
        if screen_bounds(toggle, atspi) is None:
            return None
        sources = find(SOURCES_CONTENT)
        citation = find("chat-citation-0")
        if expanded:
            return citation if sources is not None and screen_bounds(citation, atspi) is not None else None
        require(sources is None and citation is None, "a new turn's citations were visible before expanding Sources")
        return toggle

    button("startup-local")
    button("connect-local")
    button("gateway-setup-done")
    state = home / ".fleetworks/local-backend/embedded"
    fd = os.open(state / "qa-pack.json", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(fd, "w") as file:
        json.dump(pointer, file)

    def bundled_actor_ready():
        ready = find(None, label="Ready to seed")
        rows = find(None, label=f"Bundled CSV · {BUNDLED_ACTOR_ROWS:,} rows validated")
        local = find(None, label="Local CSV")
        seed = find("startup-data-primary", label="Seed actors", button=True)
        if any(screen_bounds(node, atspi) is None for node in (ready, rows, local, seed)):
            return None
        require(local.getState().contains(atspi.STATE_ENABLED), "Local CSV is unavailable after validation")
        return True

    def require_no_seed():
        require(not any(row.get("op") == "seed_actors" for row in read_journal(journal)),
                "startup accepted an actor seed without operator confirmation")

    wait_for(bundled_actor_ready, app, "automatic validation of the bundled actor CSV", timeout=180)
    require_no_seed()
    subprocess.run(["scrot", "--overwrite", str(output / "startup-actor-default.png")], check=True, timeout=5)
    button("startup-data-skip", "Finish later")
    button("inbox-qa-toggle", "Turn On")
    button("startup-inbox-qa-continue")
    button("startup-data-continue", "Open Workstation")

    def shown_drawer():
        drawer = find(DRAWER)
        if screen_bounds(drawer, atspi) is not None and screen_bounds(find("inbox-close"), atspi) is not None:
            return drawer
        return None

    def shown_composer():
        if shown_drawer() is None:
            return None
        wrapper = find(COMPOSER)
        if wrapper is None:
            return None
        # Rune Kit gives the outer styled TextArea the application key. Focus
        # and text belong to its named editable child, not the containing box.
        fields = [
            node for node in descendants(wrapper)
            if node.name == COMPOSER_LABEL
            and node.getState().contains(atspi.STATE_EDITABLE)
            and screen_bounds(node, atspi) is not None
        ]
        require(len(fields) <= 1, "Inbox exposed more than one editable question field")
        return fields[0] if fields else None

    button("inbox-toggle")
    wait_for(shown_composer, app, "Inbox drawer and editable composer opened from toolbar", timeout=4)
    button("inbox-close", timeout=4)
    wait_for(lambda: True if shown_drawer() is None else None, app, "Inbox drawer closed", timeout=4)
    button(WORKSTATION_ASSISTANT, "Open Assistant", timeout=4)
    wait_for(shown_drawer, app, "Inbox drawer opened from Workstation Status", timeout=4)

    def submit(question, *, timeout=20):
        # A bare Xvfb display has no window manager to grant keyboard focus.
        windows = subprocess.check_output(
            ["xdotool", "search", "--onlyvisible", "--name", "^Fleetworks$"], text=True, timeout=5,
        ).split()
        require(len(windows) == 1, "expected one visible Fleetworks window on the fixture display")
        subprocess.run(["xdotool", "windowfocus", "--sync", windows[0]], check=True, timeout=5)

        def focus_composer():
            field = shown_composer()
            if field is None:
                return None
            if field.getState().contains(atspi.STATE_FOCUSED):
                return field
            require(field.queryComponent().grabFocus(), "Fleetworks composer rejected accessible focus")
            return None

        field = wait_for(focus_composer, app, "focused editable QA composer", timeout=timeout)
        empty = query_text(field)
        require(empty is None or empty.characterCount == 0, "QA composer retained text before the next turn")
        subprocess.run(["xdotool", "type", "--clearmodifiers", question], check=True, timeout=5)

        def typed_question():
            field = shown_composer()
            if field is None or not field.getState().contains(atspi.STATE_FOCUSED):
                return None
            text = query_text(field)
            if text is not None and text.characterCount == len(question) and text.getText(0, len(question)) == question:
                return find("inbox-qa-send", button=True)
            return None

        wait_for(typed_question, app, "open Inbox with exact typed question and enabled Send")
        submitted_at = time.monotonic()
        # Submit via the already-focused composer, using the same keyboard path as an operator.
        subprocess.run(["xdotool", "key", "--clearmodifiers", "Return"], check=True, timeout=5)
        return submitted_at

    started = submit(SUMMARY_QUESTION, timeout=110)

    def first_frame(expected_admissions, *, expanded):
        records = read_journal(journal)
        admitted = [row for row in records if row.get("op") == "qa_turn_submit"]
        if len(admitted) < expected_admissions:
            return None
        require(len(admitted) == expected_admissions, "the UI submitted more exchanges than expected")
        pages = event_pages(records, admitted[-1]["turn_id"])
        require(not any(row["state"] in TERMINAL for row in pages),
                "the turn finished before the barrier was released")
        if not pages or pages[-1]["state"] != "running" or len(pages[-1]["findings"]) != 1:
            return None
        stop = find("inbox-qa-stop", button=True)
        if screen_bounds(stop, atspi) is None:
            return None
        sources = sources_ready(expanded)
        if sources is None:
            return None
        require(find("chat-citation-1") is None, "the drawer showed another citation before release")
        companions = [row["companion_pid"] for row in records if "companion_pid" in row]
        require(len(companions) == 1, "expected one IPC companion")
        return pages[-1], companions[0], sources.name if expanded else None

    wait_for(lambda: first_frame(1, expanded=False), app,
             "collapsed Sources for the first finding and enabled Stop", timeout=16)
    first_ms = round((time.monotonic() - started) * 1000, 1)
    subprocess.run(["scrot", "--overwrite", str(output / "first-finding-sources-collapsed.png")], check=True, timeout=5)
    button(SOURCES_TOGGLE)
    early, companion, citation_name = wait_for(lambda: first_frame(1, expanded=True), app,
                                                "an early citation and enabled Stop", timeout=4)
    subprocess.run(["scrot", "--overwrite", str(output / "first-finding.png")], check=True, timeout=5)
    wait_for(lambda: first_frame(1, expanded=True), app,
             "the early citation and enabled Stop while completion is held", timeout=2)
    release_gate(companion, worker)

    def completed_frame():
        records = read_journal(journal)
        pages = event_pages(records, early["turn_id"])
        if not pages or pages[-1]["state"] not in TERMINAL:
            return None
        require(len([row for row in records if row.get("op") == "qa_turn_submit"]) == 1,
                "the UI admitted another turn during completion")
        final = settled_turn(pages, early, "answered")
        if find("inbox-qa-stop") is not None:
            return None
        citation = sources_ready(expanded=True)
        if citation is None or citation.name != citation_name:
            return None
        count = len(final["findings"])
        require(all(find(f"chat-citation-{i}") is not None for i in range(count))
                and find(f"chat-citation-{count}") is None, "drawer citations do not match the canonical exchange")
        return final

    final = wait_for(completed_frame, app, "completion in the same drawer", timeout=12)
    completion_ms = round((time.monotonic() - started) * 1000, 1)
    subprocess.run(["scrot", "--overwrite", str(output / "completed-turn.png")], check=True, timeout=5)

    submit(MEMORY_QUESTION)

    def grounded_memory(*, expanded):
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
        if not any(
            node.name == expected_text and screen_bounds(node, atspi) is not None
            for node in descendants(answer)
        ):
            return None
        sources = sources_ready(expanded)
        if sources is None or (expanded and sources.name != expected_citation):
            return None
        require(find("chat-citation-1") is None and find("inbox-qa-stop") is None,
                "the completed memory answer showed unrelated evidence or an unfinished turn")
        return sample

    wait_for(lambda: grounded_memory(expanded=False), app,
             "grounded memory reading with collapsed Sources", timeout=16)
    button(SOURCES_TOGGLE)
    memory = wait_for(lambda: grounded_memory(expanded=True), app,
                      "grounded memory reading and its original timestamp", timeout=4)
    subprocess.run(["scrot", "--overwrite", str(output / "grounded-memory.png")], check=True, timeout=5)

    submit(SUMMARY_QUESTION)
    wait_for(lambda: first_frame(3, expanded=False), app,
             "a separate Running turn with collapsed Sources", timeout=16)
    button(SOURCES_TOGGLE)
    pending, cancel_companion, cancel_citation_name = wait_for(
        lambda: first_frame(3, expanded=True), app,
        "a separate Running turn with its first citation", timeout=4,
    )
    require(cancel_companion == companion and pending["turn_id"] != final["turn_id"],
            "Stop used a different companion or the completed turn")
    position = wait_for(lambda: screen_bounds(find("inbox-qa-stop", button=True), atspi), app,
                        "enabled Stop on the held turn", timeout=2)
    subprocess.run(["scrot", "--overwrite", str(output / "before-stop.png")], check=True, timeout=5)
    x, y, width, height = position
    subprocess.run(["xdotool", "mousemove", str(x + width // 2), str(y + height // 2), "click", "1"],
                   check=True, timeout=5)

    def canceled_frame():
        records = read_journal(journal)
        stopped = [row for row in records if row.get("op") == "qa_cancel"]
        audits = [row for row in records if row.get("op") == "qa_cancel_audit"]
        if not stopped or not audits or find("inbox-qa-stop") is not None:
            return None
        require(len(stopped) == len(audits) == 1 and stopped[0]["turn_id"] == audits[0]["turn_id"] == pending["turn_id"],
                "Stop or the event audit targeted a different turn")
        canceled = settled_turn([audits[0]], pending, "canceled")
        require(stopped[0]["state"] == "canceled" and stopped[0]["cursor"] == canceled["cursor"]
                and canceled["cursor"] == pending["cursor"] + 1,
                "Stop did not add exactly one terminal cursor to the held turn")
        citation = sources_ready(expanded=True)
        if citation is None or citation.name != cancel_citation_name:
            return None
        return canceled

    canceled = wait_for(canceled_frame, app, "the retained canceled turn after clicking Stop", timeout=12)
    subprocess.run(["scrot", "--overwrite", str(output / "canceled-turn.png")], check=True, timeout=5)
    require_no_seed()
    receipts = [row["transport_attestation"] for row in read_journal(journal) if "transport_attestation" in row]
    require(len(receipts) == 1, "Expected one actual packaged companion handshake")
    result = {
        "runtime": receipts[0]["transport"], "transport_attestation": receipts[0],
        "target": digests["target"], "source_sha": digests["source_sha"],
        "graphical": "x11", "first_citation_while_running": True,
        "assistant_open_paths": ["inbox_toolbar", "workstation_status"],
        "focused_editable_and_exact_typed_text": True,
        "sources_default_collapsed": True, "sources_retained_after_completion": True,
        "sources_retained_after_cancel": True,
        "stop_enabled_before_release": True, "same_turn": True,
        "terminal_state": final["state"], "terminal_events": final["terminal_events"],
        "stop_while_running": True, "cancelled_terminal_state": canceled["state"],
        "cancelled_terminal_events": canceled["terminal_events"],
        "finding_count": len(final["findings"]), "duplicate_findings": 0,
        "first_finding_ms": first_ms, "completion_ms": completion_ms,
        "grounded_memory": memory,
        "startup_actor_validated_rows": BUNDLED_ACTOR_ROWS, "startup_actor_seed_accepted": False,
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
        worker, pointer, digests = install_gated_pack(home, args)
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
                test_window(app, pyatspi.Registry, pyatspi, home, worker, pointer, digests, journal, output)
            except Exception as error:
                try:
                    retain_focus_diagnostics(app, pyatspi.Registry, pyatspi, output, error)
                except Exception as diagnostics:
                    print(f"Could not retain AT-SPI diagnostics: {type(diagnostics).__name__}", file=sys.stderr)
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
