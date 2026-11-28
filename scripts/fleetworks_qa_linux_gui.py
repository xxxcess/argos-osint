#!/usr/bin/env python3
"""Verify offline startup and Inbox streaming in the packaged Linux window."""

import argparse
import json
import os
from pathlib import Path
import re
import secrets
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
MEMORY_QUESTION = "What is the current memory usage ?"
SOURCES_TOGGLE = "inbox-qa-sources-toggle"
SOURCES_CONTENT = "inbox-qa-sources"
SOURCES_BACK = "inbox-qa-sources-back"
PREVIOUS_EXCHANGE = "inbox-qa-history-previous"
NEXT_EXCHANGE = "inbox-qa-history-next"
DRAWER = "inbox-layer"
COMPOSER = "inbox-qa-composer"
COMPOSER_LABEL = "Question for the Inbox assistant"
INBOX_TOOLBAR = "inbox-toggle"
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


def read_x11_clipboard():
    try:
        result = subprocess.run(["xclip", "-selection", "clipboard", "-out"], stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, check=False, timeout=1)
        return result.stdout if result.returncode == 0 else None
    except subprocess.TimeoutExpired:
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


def settled_turn(pages, first, expected_state, *, timing=""):
    require(pages and all(not row["gap"] for row in pages), "fixture IPC event stream contained a cursor gap")
    final = pages[-1]
    require(final["state"] == expected_state,
            f"same IPC turn expected {expected_state}, got {final['state']} at cursor {final['cursor']}{timing}")
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

    def find_many(keys, buttons):
        # The streaming assertion needs several nodes from the same live frame.
        found = {}
        for window in app_roots(app, registry):
            for node in descendants(window):
                key = node.get_accessible_id()
                if key not in keys or key in found:
                    continue
                state = node.getState()
                if state.contains(atspi.STATE_DEFUNCT):
                    continue
                if key in buttons and (node.getRole() != atspi.ROLE_PUSH_BUTTON
                                       or not state.contains(atspi.STATE_ENABLED)):
                    continue
                found[key] = node
                if len(found) == len(keys):
                    return found
        return found

    def button(key, label=None, *, timeout=90):
        def activate():
            node = find(key, label=label, button=True)
            if node is None:
                return None
            if not node.queryAction().doAction(0):
                raise RuntimeError(f"Fleetworks could not activate {key}")
            return True

        wait_for(activate, app, key, timeout=timeout)

    def find_citation(turn_id, index):
        return find(f"chat-citation-{turn_id}-{index}")

    def message_time(message):
        node = find(f"chat-message-inbox-qa-{message}-time")
        if screen_bounds(node, atspi) is not None and re.fullmatch(r"(?:[1-9]|1[0-2]):[0-5]\d [AP]M", node.name or ""):
            return node.name
        return None

    def sources_ready(turn_id, *, expanded, nodes=None):
        citation_key = f"chat-citation-{turn_id}-0"
        if nodes is None:
            nodes = find_many({SOURCES_CONTENT, SOURCES_BACK, SOURCES_TOGGLE, citation_key},
                              {SOURCES_BACK, SOURCES_TOGGLE})
        sources = nodes.get(SOURCES_CONTENT)
        citation = nodes.get(citation_key)
        back = nodes.get(SOURCES_BACK)
        toggle = nodes.get(SOURCES_TOGGLE)
        if expanded:
            # The full-width panel replaces the reply, including its Sources trigger.
            if toggle is not None or any(screen_bounds(node, atspi) is None for node in (sources, back, citation)):
                return None
            return citation
        # Back restores the trigger while the outgoing panel can still be present in AT-SPI.
        if screen_bounds(toggle, atspi) is None or any(node is not None for node in (sources, citation, back)):
            return None
        return toggle

    def citation_details(citation):
        return tuple(child.name for child in citation if child is not None)

    def focused_button(key, label=None):
        node = find(key, label=label, button=True)
        if screen_bounds(node, atspi) is not None and node.getState().contains(atspi.STATE_FOCUSED):
            return node
        return None

    button("startup-local")

    enter_key = "startup-enter-workstation"

    def workstation_status():
        screen = find("screen-workstation")
        return screen if screen_bounds(screen, atspi) is not None else None

    def embedded_checks_ready():
        stage_key = "startup-stage-embedded-health"
        checklist_key = "startup-wiring-health-checklist"
        gateway_key = "startup-wiring-health-check-gateway"
        keys = {stage_key, checklist_key, gateway_key, enter_key}
        require(workstation_status() is None, "Workstation opened before Enter Workstation was pressed")
        nodes = find_many(keys, {enter_key})
        if any(screen_bounds(nodes.get(key), atspi) is None for key in keys):
            return None
        if (nodes[gateway_key].name in ("Gateway: Passed", "Gateway: Unavailable")
                and nodes[enter_key].name == "Enter Workstation"):
            return nodes[stage_key]
        return None

    def bundled_actor_ready():
        rows = find("maintenance-dataset-validated", label=f"Bundled CSV · {BUNDLED_ACTOR_ROWS:,} rows validated")
        local = find("maintenance-seed", button=True)
        seed = find("maintenance-confirm", button=True)
        if any(screen_bounds(node, atspi) is None for node in (rows, local, seed)):
            return None
        return True

    def require_no_seed():
        require(not any(row.get("op") == "seed_actors" for row in read_journal(journal)),
                "the app accepted an actor seed without operator confirmation")

    wait_for(embedded_checks_ready, app, "Embedded health checks ready to enter")
    # Observe the checklist beyond the former two-second automatic handoff.
    time.sleep(2.25)
    health_stage = wait_for(embedded_checks_ready, app, "retained Embedded health checklist", timeout=3)
    buttons = [node.get_accessible_id() for node in descendants(health_stage)
               if node.getRole() == atspi.ROLE_PUSH_BUTTON and screen_bounds(node, atspi) is not None]
    require(buttons == [enter_key], f"Unexpected Embedded health controls: {buttons}")
    require_no_seed()
    subprocess.run(["scrot", "--overwrite", str(output / "startup-embedded-health.png")],
                   check=True, timeout=5)
    button(enter_key, "Enter Workstation", timeout=4)
    wait_for(workstation_status, app, "Workstation Status after Enter Workstation", timeout=8)
    require_no_seed()
    require(find("workstation-build-actor-index", button=True) is None,
            "the index build was available before a catalog was seeded")
    subprocess.run(["scrot", "--overwrite", str(output / "startup-workstation-status.png")],
                   check=True, timeout=5)
    button("readiness-action-actor_catalog-SeedActors")
    button("maintenance-bundled-csv")
    wait_for(bundled_actor_ready, app, "validated actor CSV from Workstation Status", timeout=180)
    require_no_seed()
    subprocess.run(["scrot", "--overwrite", str(output / "workstation-actor-seed-confirmation.png")],
                   check=True, timeout=5)
    button("maintenance-cancel")
    state = home / ".fleetworks/local-backend/embedded"
    fd = os.open(state / "qa-pack.json", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(fd, "w") as file:
        json.dump(pointer, file)
    button("nav-workstation")
    button("open-workstation-settings")
    button("settings-category-connection", "Connection")
    button("inbox-qa-toggle", "Turn On")
    button("settings-close", "Close Settings")

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

    button(INBOX_TOOLBAR)
    wait_for(shown_drawer, app, "Inbox drawer opened from toolbar", timeout=4)
    button("inbox-close", timeout=4)
    wait_for(lambda: True if shown_drawer() is None else None, app, "Inbox drawer closed", timeout=4)
    button(INBOX_TOOLBAR, timeout=4)
    wait_for(shown_drawer, app, "Inbox drawer reopened from toolbar", timeout=4)

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

        wait_for(focus_composer, app, "focused editable QA composer", timeout=timeout)
        subprocess.run(["xdotool", "type", "--clearmodifiers", question], check=True, timeout=5)

        def focused_with_send():
            field = shown_composer()
            if field is not None and field.getState().contains(atspi.STATE_FOCUSED):
                return find("inbox-qa-send", button=True)
            return None

        wait_for(focused_with_send, app, "open Inbox with focused editable composer and enabled Send", timeout=8)
        # Prove this copy came from the current editor even when the preceding turn
        # used the same question. The display and its clipboard belong to this fixture.
        sentinel = ("qa-copy-" + secrets.token_hex(8)).encode()
        subprocess.run(["xclip", "-selection", "clipboard", "-in"], input=sentinel,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=3)
        wait_for(lambda: True if read_x11_clipboard() == sentinel else None, app,
                 "unique X11 clipboard sentinel", timeout=3)
        subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+a", "ctrl+c"], check=True, timeout=5)
        expected = question.encode()
        wait_for(lambda: focused_with_send() if read_x11_clipboard() == expected else None, app,
                 "exact question copied from the focused QA composer", timeout=5)
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
        turn_id = admitted[-1]["turn_id"]
        pages = event_pages(records, turn_id)
        require(not any(row["state"] in TERMINAL for row in pages),
                "the turn finished before the barrier was released")
        if not pages or pages[-1]["state"] != "running" or len(pages[-1]["findings"]) != 1:
            return None
        stop_key = "inbox-qa-stop"
        first_key, second_key = (f"chat-citation-{turn_id}-{index}" for index in range(2))
        nodes = find_many({stop_key, SOURCES_TOGGLE, SOURCES_CONTENT, SOURCES_BACK, first_key, second_key},
                          {stop_key, SOURCES_TOGGLE, SOURCES_BACK})
        stop = screen_bounds(nodes.get(stop_key), atspi)
        if stop is None:
            return None
        sources = sources_ready(turn_id, expanded=expanded, nodes=nodes)
        if sources is None:
            return None
        require(second_key not in nodes, "the drawer showed another citation before release")
        companions = [row["companion_pid"] for row in records if "companion_pid" in row]
        require(len(companions) == 1, "expected one IPC companion")
        return pages[-1], companions[0], sources, stop

    _, _, toggle, _ = wait_for(lambda: first_frame(1, expanded=False), app,
                               "collapsed Sources for the first finding and enabled Stop", timeout=16)
    first_ms = round((time.monotonic() - started) * 1000, 1)
    subprocess.run(["scrot", "--overwrite", str(output / "first-finding-sources-collapsed.png")], check=True, timeout=5)
    require(toggle.queryAction().doAction(0), "Fleetworks could not activate Sources")
    early, companion, citation, _ = wait_for(lambda: first_frame(1, expanded=True), app,
                                            "an early citation and enabled Stop", timeout=4)
    early_citation = citation_details(citation)
    require(len(early_citation) == 3 and early_citation[2].startswith("Observed "),
            "the early citation omitted its title, text, or observation time")
    subprocess.run(["scrot", "--overwrite", str(output / "first-finding.png")], check=True, timeout=5)
    release_gate(companion, worker)
    released_ms = round((time.monotonic() - started) * 1000, 1)

    def completed_frame():
        records = read_journal(journal)
        pages = event_pages(records, early["turn_id"])
        if not pages or pages[-1]["state"] not in TERMINAL:
            return None
        require(len([row for row in records if row.get("op") == "qa_turn_submit"]) == 1,
                "the UI admitted another turn during completion")
        terminal_ms = (time.monotonic() - started) * 1000
        final = settled_turn(
            pages, early, "answered",
            timing=(f"; first finding {first_ms:.1f} ms, gate released {released_ms:.1f} ms, "
                    f"terminal observed {terminal_ms:.1f} ms after submit"),
        )
        if find("inbox-qa-stop") is not None:
            return None
        citation = sources_ready(final["turn_id"], expanded=True)
        if citation is None or citation_details(citation) != early_citation:
            return None
        count = len(final["findings"])
        require(all(find_citation(final["turn_id"], index) is not None for index in range(count))
                and find_citation(final["turn_id"], count) is None,
                "drawer citations do not match the canonical exchange")
        return final

    final = wait_for(completed_frame, app, "completion in the same drawer", timeout=12)
    completion_ms = round((time.monotonic() - started) * 1000, 1)
    subprocess.run(["scrot", "--overwrite", str(output / "completed-sources.png")], check=True, timeout=5)
    button(SOURCES_BACK, "Back to reply", timeout=4)

    def completed_reply():
        if sources_ready(final["turn_id"], expanded=False) is None or not all(
            message_time(message) for message in ("question", "answer")
        ):
            return None
        if any(screen_bounds(find(key), atspi) is None
               for key in ("inbox-qa-response-time", "inbox-qa-context")):
            return None
        return True

    wait_for(completed_reply, app, "the completed reply, timestamps, duration and context", timeout=8)
    subprocess.run(["scrot", "--overwrite", str(output / "completed-turn.png")], check=True, timeout=5)

    submit(MEMORY_QUESTION)
    _, metric_companion, _, _ = wait_for(
        lambda: first_frame(2, expanded=False), app,
        "first generated memory finding at the worker barrier", timeout=16,
    )
    require(metric_companion == companion, "the companion changed while producing the metric reply")
    release_gate(metric_companion, worker)

    def grounded_memory(*, expanded):
        records = read_journal(journal)
        admitted = [row for row in records if row.get("op") == "qa_turn_submit"]
        if len(admitted) < 2:
            return None
        require(len(admitted) == 2, "the GUI submitted more than one metric turn")
        pages = event_pages(records, admitted[-1]["turn_id"])
        if not pages or pages[-1]["state"] not in TERMINAL:
            return None
        turn = pages[-1]
        sample = turn["memory_observation"]
        require(turn["turn_id"] != final["turn_id"] and turn["state"] == "answered"
                and len(turn["findings"]) == 1 and sample is not None,
                "the metric did not resolve to one frozen memory observation")
        whole, tenths = divmod(sample["value"], 10)
        expected_text = f"The API host memory usage is {whole}.{tenths}%."
        sources = sources_ready(turn["turn_id"], expanded=expanded)
        if sources is None:
            return None
        if expanded:
            observation = ("API host memory usage", expected_text, f"Observed {sample['observed_at']}")
            if citation_details(sources) != observation:
                return None
            require(find("inbox-qa-sources-group-Host metric") is not None,
                    "the memory observation was not grouped as a host metric")
        else:
            answer = find("chat-message-inbox-qa-answer")
            allowed_text = (expected_text, "Here is the current status. " + expected_text,
                            "Here are the current checks. " + expected_text)
            if answer is None or not any(
                node.name in allowed_text and screen_bounds(node, atspi) is not None
                for node in descendants(answer)
            ):
                return None
        require(find_citation(turn["turn_id"], 1) is None and find("inbox-qa-stop") is None,
                "the completed memory answer showed unrelated evidence or an unfinished turn")
        return sample

    wait_for(lambda: grounded_memory(expanded=False), app,
             "grounded memory reading with collapsed Sources", timeout=16)
    subprocess.run(["scrot", "--overwrite", str(output / "grounded-memory.png")], check=True, timeout=5)
    button(SOURCES_TOGGLE)
    memory = wait_for(lambda: grounded_memory(expanded=True), app,
                      "grounded memory reading and its original timestamp", timeout=4)
    subprocess.run(["scrot", "--overwrite", str(output / "grounded-memory-sources.png")], check=True, timeout=5)
    button(SOURCES_BACK, "Back to reply", timeout=4)
    wait_for(lambda: grounded_memory(expanded=False), app, "memory reply restored after Sources", timeout=8)

    def transcript_shows(question, *, hidden_arrow):
        message = find("chat-message-inbox-qa-question")
        if message is None or find(hidden_arrow) is not None:
            return None
        return next((node for node in descendants(message)
                     if node.name == question and screen_bounds(node, atspi) is not None), None)

    button(PREVIOUS_EXCHANGE, "Previous exchange", timeout=12)
    wait_for(lambda: transcript_shows(SUMMARY_QUESTION, hidden_arrow=PREVIOUS_EXCHANGE),
             app, "oldest readiness exchange with the previous arrow hidden", timeout=8)
    wait_for(lambda: focused_button(NEXT_EXCHANGE, "Next exchange"), app,
             "keyboard focus on the newer arrow at the oldest exchange", timeout=8)
    button(SOURCES_TOGGLE, timeout=8)
    previous_source = wait_for(lambda: sources_ready(final["turn_id"], expanded=True),
                               app, "previous exchange with its original source", timeout=8)
    require(citation_details(previous_source) == early_citation,
            "history replaced the previous exchange's original citation")
    subprocess.run(["scrot", "--overwrite", str(output / "previous-exchange-sources.png")], check=True, timeout=5)
    wait_for(lambda: focused_button(SOURCES_BACK, "Back to reply"), app,
             "keyboard focus on Back to reply", timeout=8)
    subprocess.run(["xdotool", "key", "--clearmodifiers", "space"], check=True, timeout=5)
    wait_for(lambda: transcript_shows(SUMMARY_QUESTION, hidden_arrow=PREVIOUS_EXCHANGE),
             app, "oldest readiness reply restored after Sources", timeout=8)
    wait_for(lambda: focused_button(SOURCES_TOGGLE), app,
             "focus restored to the original source trigger", timeout=8)
    subprocess.run(["scrot", "--overwrite", str(output / "previous-exchange.png")], check=True, timeout=5)

    def focused_next_exchange():
        focused = focused_button(NEXT_EXCHANGE, "Next exchange")
        if focused is not None:
            return focused
        arrow = find(NEXT_EXCHANGE, label="Next exchange", button=True)
        if screen_bounds(arrow, atspi) is not None:
            require(arrow.queryComponent().grabFocus(), "the next exchange arrow rejected accessible focus")
        return None

    # Space activates the focused native control. Return is reserved for a default action.
    wait_for(focused_next_exchange, app, "keyboard focus on the newer exchange arrow", timeout=8)
    subprocess.run(["xdotool", "key", "--clearmodifiers", "space"], check=True, timeout=5)
    wait_for(lambda: transcript_shows(MEMORY_QUESTION, hidden_arrow=NEXT_EXCHANGE),
             app, "latest memory exchange with the next arrow hidden", timeout=8)
    require(len([row for row in read_journal(journal) if row.get("op") == "qa_turn_submit"]) == 2,
            "browsing the transcript submitted another turn")

    submit(SUMMARY_QUESTION)
    _, _, toggle, _ = wait_for(lambda: first_frame(3, expanded=False), app,
                               "a separate Running turn with collapsed Sources", timeout=16)
    require(toggle.queryAction().doAction(0), "Fleetworks could not activate Sources")
    pending, cancel_companion, citation, position = wait_for(
        lambda: first_frame(3, expanded=True), app,
        "a separate Running turn with its first citation", timeout=4,
    )
    cancel_citation = citation_details(citation)
    require(cancel_companion == companion and pending["turn_id"] != final["turn_id"],
            "Stop used a different companion or the completed turn")
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
        citation = sources_ready(canceled["turn_id"], expanded=True)
        if citation is None or citation_details(citation) != cancel_citation:
            return None
        return canceled

    canceled = wait_for(canceled_frame, app, "the retained canceled turn after clicking Stop", timeout=12)
    subprocess.run(["scrot", "--overwrite", str(output / "canceled-sources.png")], check=True, timeout=5)
    button(SOURCES_BACK, "Back to reply", timeout=4)

    def stopped_reply():
        answer = find("chat-message-inbox-qa-answer")
        if answer is None or sources_ready(canceled["turn_id"], expanded=False) is None:
            return None
        return next((node for node in descendants(answer)
                     if node.name == "Stopped" and screen_bounds(node, atspi) is not None), None)

    wait_for(stopped_reply, app, "the stopped reply after closing its retained Sources", timeout=8)
    subprocess.run(["scrot", "--overwrite", str(output / "canceled-turn.png")], check=True, timeout=5)
    require_no_seed()
    receipts = [row["transport_attestation"] for row in read_journal(journal) if "transport_attestation" in row]
    require(len(receipts) == 1, "Expected one actual packaged companion handshake")
    result = {
        "runtime": receipts[0]["transport"], "transport_attestation": receipts[0],
        "target": digests["target"], "source_sha": digests["source_sha"],
        "graphical": "x11", "first_citation_while_running": True,
        "assistant_open_paths": ["inbox_toolbar"], "inbox_closed_and_reopened": True,
        "focused_editable_and_exact_typed_text": True,
        "typed_question_verification": "x11_clipboard",
        "sources_default_collapsed": True, "sources_retained_after_completion": True,
        "sources_retained_after_cancel": True,
        "transcript_previous_and_next": True,
        "transcript_boundary_arrows_hidden": True, "message_times_visible": True,
        "response_duration_and_context_visible": True,
        "stop_enabled_before_release": True, "same_turn": True,
        "terminal_state": final["state"], "terminal_events": final["terminal_events"],
        "stop_while_running": True, "cancelled_terminal_state": canceled["state"],
        "cancelled_terminal_events": canceled["terminal_events"],
        "finding_count": len(final["findings"]), "duplicate_findings": 0,
        "first_finding_ms": first_ms, "gate_release_ms": released_ms, "completion_ms": completion_ms,
        "grounded_memory": memory,
        "startup_advanced_without_controls": True,
        "manual_actor_validated_rows": BUNDLED_ACTOR_ROWS, "actor_seed_accepted": False,
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
