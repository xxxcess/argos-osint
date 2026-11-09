#!/usr/bin/env python3
"""Offline process smoke: readiness, ownership, restart, adoption and shutdown.

Build argos plus the optional argos-api-host fixture (for duplicate-owner and
adoption tests) through scripts/agent_cargo.py first. Build the
web export with pnpm build. All state lives in a temporary ARGOS_HOME.
"""
import argparse
import json
import os
from pathlib import Path
import re
import socket
import shutil
import sys
import signal
import subprocess
import tempfile
import time
import unicodedata
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


class TerminalScreen:
    """Minimal VT cell view for the fixed-size PTY, with byte-offset freshness.

    Ratatui emits cell differences: a repaint may retain characters between
    cursor movements. Reconstruct the current screen instead of concatenating
    historical writes, and only accept matches touched since the input mark.
    """

    CSI = re.compile(rb"\x1b\[([0-?]*)([ -/]*)([@-~])")

    def __init__(self, data, width=160, height=50):
        self.width, self.height = width, height
        self.cells = [[" "] * width for _ in range(height)]
        self.written = [[-1] * width for _ in range(height)]
        self.row = self.column = 0
        self.saved = (0, 0)
        self.primary = None
        self._read(bytes(data))

    def _clear(self, row, first, last, offset):
        for column in range(first, last):
            self.cells[row][column] = " "
            self.written[row][column] = offset

    def _newline(self, offset):
        self.row += 1
        if self.row == self.height:
            self.cells.pop(0)
            self.written.pop(0)
            self.cells.append([" "] * self.width)
            self.written.append([offset] * self.width)
            self.row -= 1

    def _control(self, parameters, command, offset):
        private = parameters.startswith(b"?")
        values = [int(value) if value else 0 for value in parameters.lstrip(b"?").split(b";")]
        amount = values[0] or 1
        if command in (b"H", b"f"):
            self.row = min((values[0] or 1) - 1, self.height - 1)
            self.column = min((values[1] or 1) - 1 if len(values) > 1 else 0, self.width - 1)
        elif command in (b"A", b"B", b"E", b"F"):
            self.row = max(0, min(self.height - 1, self.row + (amount if command in (b"B", b"E") else -amount)))
            if command in (b"E", b"F"):
                self.column = 0
        elif command in (b"C", b"D"):
            self.column = max(0, min(self.width - 1, self.column + (amount if command == b"C" else -amount)))
        elif command in (b"G", b"`"):
            self.column = min(amount - 1, self.width - 1)
        elif command == b"d":
            self.row = min(amount - 1, self.height - 1)
        elif command == b"K":
            mode = values[0]
            self._clear(self.row, 0 if mode in (1, 2) else min(self.column, self.width),
                        self.width if mode in (0, 2) else min(self.column + 1, self.width), offset)
        elif command == b"J":
            for row in range(self.height):
                if values[0] == 2 or (values[0] == 0 and row > self.row) or (values[0] == 1 and row < self.row):
                    self._clear(row, 0, self.width, offset)
                elif row == self.row and values[0] in (0, 1):
                    self._clear(row, 0 if values[0] == 1 else min(self.column, self.width),
                                min(self.column + 1, self.width) if values[0] == 1 else self.width, offset)
        elif command == b"s":
            self.saved = self.row, self.column
        elif command == b"u":
            self.row, self.column = self.saved
        elif private and command in (b"h", b"l") and 1049 in values:
            if command == b"h" and self.primary is None:
                self.primary = self.cells, self.written, self.row, self.column
                self.cells = [[" "] * self.width for _ in range(self.height)]
                self.written = [[offset] * self.width for _ in range(self.height)]
                self.row = self.column = 0
            elif command == b"l" and self.primary is not None:
                self.cells, self.written, self.row, self.column = self.primary
                self.primary = None
        else:
            assert command in (b"m", b"h", b"l", b"q"), f"unsupported PTY control: {parameters!r}{command!r}"

    def _read(self, data):
        offset = 0
        while offset < len(data):
            value = data[offset]
            if value == 27:
                if data[offset:offset + 2] == b"\x1b[":
                    match = self.CSI.match(data, offset)
                    if match is None:  # Snapshot ended in a partially received control.
                        break
                    self._control(match[1], match[3], offset)
                    offset = match.end()
                    continue
                if data[offset:offset + 2] == b"\x1b]":
                    ends = [position for position in (data.find(b"\x07", offset + 2), data.find(b"\x1b\\", offset + 2)) if position >= 0]
                    if not ends:
                        break
                    end = min(ends)
                    offset = end + (1 if data[end] == 7 else 2)
                    continue
                if offset + 1 == len(data):
                    break
                command = data[offset + 1:offset + 2]
                if command == b"7":
                    self.saved = self.row, self.column
                elif command == b"8":
                    self.row, self.column = self.saved
                else:
                    raise AssertionError(f"unsupported PTY escape: {command!r}")
                offset += 2
                continue
            if value < 32 or value == 127:
                if value == 13:
                    self.column = 0
                elif value == 10:
                    self._newline(offset)
                elif value == 8:
                    self.column = max(0, self.column - 1)
                elif value == 9:
                    self.column = min((self.column // 8 + 1) * 8, self.width - 1)
                offset += 1
                continue
            size = 1 if value < 128 else 2 if value < 224 else 3 if value < 240 else 4
            if offset + size > len(data):
                break
            character = data[offset:offset + size].decode("utf-8")
            width = 0 if unicodedata.combining(character) else 2 if unicodedata.east_asian_width(character) in ("W", "F") else 1
            if width == 0:
                if self.column:
                    self.cells[self.row][min(self.column - 1, self.width - 1)] += character
            else:
                if self.column + width > self.width:
                    self.column = 0
                    self._newline(offset)
                self.cells[self.row][self.column] = character
                self.written[self.row][self.column] = offset
                if width == 2:
                    self.cells[self.row][self.column + 1] = ""
                    self.written[self.row][self.column + 1] = offset
                self.column += width
            offset += size

    def _lines(self):
        for cells, written in zip(self.cells, self.written):
            yield "".join(cells), [stamp for cell, stamp in zip(cells, written) for _ in cell]

    def text(self, start=0):
        return "\n".join(line.rstrip() for line, stamps in self._lines() if any(stamp >= start for stamp in stamps)).encode("utf-8")

    def contains(self, label, start=0):
        label = label.decode("utf-8")
        for line, stamps in self._lines():
            position = line.find(label)
            while position >= 0:
                if any(stamp >= start for stamp in stamps[position:position + len(label)]):
                    return True
                position = line.find(label, position + 1)
        return False


def verify_terminal_screen(capture):
    prompt = b"Check Atlas classifier"
    initial = b"\x1b[2J\x1b[2;3H" + prompt
    repaint = b"\x1b[2;3HCheck Atlas cl\x1b[2;17Hassif\x1b[2;22Hier"
    assert TerminalScreen(initial + repaint).contains(prompt, len(initial)), "retained cells must complete the exact prompt"
    next_prompt = initial + repaint + b"\x1b[2;3HAPI port\x1b[K"
    assert not TerminalScreen(next_prompt).contains(prompt), "replaced prompts must not match historical output"
    unchanged = next_prompt + b"\x1b[4;1Hstatus"
    assert not TerminalScreen(unchanged).contains(b"API port", len(next_prompt)), "unrelated fresh cells cannot freshen an old match"
    raw = capture.read_bytes()
    leave = raw.rfind(b"\x1b[?1049l")
    enter = raw.rfind(b"\x1b[?1049h", 0, leave)
    returned = TerminalScreen(raw[:leave])
    assert enter >= 0 and returned.contains(b"Choose a service", enter)
    assert returned.contains(prompt, enter), "fresh Venture menu must expose the classifier check"
    assert returned.contains(b"Atlas classifier (Rust-BERT): ready", enter), "fresh Venture menu must show successful readiness"
    assert returned.contains(b"2  API Server", enter), "captured TUI exit must return to a fresh, ready Venture menu"
    print("PASS: retained-cell repaint, replaced/stale prompt guards and ready Venture menu reconstruction")


def service_row_position(screen, title):
    """Locate a live service title in the rendered table, excluding details/history."""
    matches = []
    for row, cells in enumerate(screen.cells):
        borders = [column for column, value in enumerate(cells) if value == "│"]
        if len(borders) < 2:
            continue
        table = cells[borders[0] + 1:borders[1]]
        text = "".join(table)
        position = text.find(title)
        if position < 0 or not re.search(r"\bservice\b", text[:position]):
            continue
        columns = [column for column in range(borders[0] + 1, borders[1]) for _ in cells[column]]
        matches.append((columns[position] + 1, row + 1))  # SGR uses 1-based actual cells.
    assert len(matches) <= 1, f"multiple live service rows match {title!r}"
    return matches[0] if matches else None


def install_classifier_fixture(config):
    """Install a test-only worker and model in the isolated Argos home."""
    config.parent.mkdir(parents=True, exist_ok=True)
    model = config.parent / "fixture-model"
    model.mkdir()
    (model / "config.json").write_text("{}")
    for name in ("vocab.txt", "rust_model.ot"):
        (model / name).write_text("test fixture")
    worker = ROOT / "scripts" / "fixtures" / "atlas_bert"
    config.write_text(json.dumps({"worker_path": str(worker), "model_dir": str(model)}))


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def get(url):
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url, timeout=1) as response:
        assert response.status == 200, (url, response.status)
        return response.read()


def eventually(check, process=None, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if process is not None and process.poll() is not None:
            raise AssertionError(f"managed process exited: {process.returncode}")
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    raise AssertionError("deadline exceeded")


def composer_smoke(binary):
    """Composer-launch regression: a recon prompt must not abort the runtime.

    Boots `stack up --tui` under the same PTY+kernel harness as the TUI branch,
    in its own temp ARGOS_HOME. Submitting a recon prompt from the Home
    composer used to kill the process (SIGABRT): lldb showed the embedded
    SurrealDB commit reached through `nlp_cache::put` during
    `derive_directives` consuming ~2.1 MB inside one poll on the default 2 MiB
    Tokio worker stack. The rebuilt binary gives workers an explicit 8 MiB
    stack and the composer fails closed when no runtime handles exist.
    """
    if os.name != "posix":
        print("SKIP: composer smoke needs a PTY; normal stack tests still run")
        return
    assert binary.is_file(), f"build argos first: {binary}"
    import fcntl
    import pty
    import select
    import struct
    import termios
    import threading

    with tempfile.TemporaryDirectory(prefix="argos-composer-crash-") as tmp:
        env = {**os.environ, "ARGOS_HOME": tmp, "TERM": "xterm-256color"}
        env.pop("ARGOS_EMBED", None)
        env.pop("ARGOS_STORAGE_MODE", None)
        init = subprocess.run(
            [str(binary), "storage", "init", "--mode", "embedded"],
            cwd=ROOT, env=env, capture_output=True, text=True, timeout=120,
        )
        assert init.returncode == 0, ("storage init", init.stdout, init.stderr)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 160, 0, 0))
        before = termios.tcgetattr(slave)
        capture = bytearray()
        done = threading.Event()

        def terminal_text(start=0):
            return TerminalScreen(capture).text(start)

        def sees(label, start=0):
            return TerminalScreen(capture).contains(label, start)

        def read_terminal():
            while not done.is_set():
                if select.select([master], [], [], 0.1)[0]:
                    try:
                        capture.extend(os.read(master, 65536))
                    except OSError:
                        return

        def terminal_session():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

        keeper = (
            "import subprocess,sys,signal;"
            "signal.signal(signal.SIGINT, lambda *_: None);"
            "p=subprocess.Popen(sys.argv[1:]); code=p.wait();"
            "print('ARGOS_EXIT='+str(code), flush=True);"
            "sys.stdin.readline(); sys.exit(code)"
        )
        process = subprocess.Popen(
            [sys.executable, "-c", keeper, str(binary), "stack", "up", "--tui"],
            cwd=ROOT, env=env, stdin=slave, stdout=slave, stderr=slave,
            preexec_fn=terminal_session,
        )
        reader = threading.Thread(target=read_terminal, daemon=True)
        reader.start()
        try:
            # Start from the Home composer, not the first-run session prompt.
            eventually(lambda: sees(b"Recon") and sees(b"Intel"), process, seconds=90)
            if sees(b"Open last app view session") or sees(b"Resume"):
                os.write(master, b"n")
                time.sleep(1.0)
            mark = len(capture)
            # Type a recon prompt into the Home composer and submit it.
            os.write(master, b"Investigate example.com infrastructure\r")
            eventually(lambda: sees(b"Investigation started", mark), process, seconds=60)
            # The investigation surface replaced the Home launcher.
            eventually(lambda: sees(b"transcript", mark), process, seconds=60)
            # Bounded wait: while the turn runs, no abort signature may appear
            # and the child must stay alive.
            deadline = time.monotonic() + 45
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise AssertionError(
                        f"composer launch ended the TUI: {process.returncode} "
                        f"(visible tail: {terminal_text(mark).decode('utf-8', 'replace')[-400:]})"
                    )
                text = terminal_text(mark)
                for signature in (b"stack overflow", b"fatal runtime error"):
                    assert signature not in text, (
                        f"composer launch aborted the runtime: {signature.decode()}"
                    )
                time.sleep(0.5)
            assert process.poll() is None, "composer-launched TUI died during the bounded wait"
            assert not sees(b"stack overflow", mark) and not sees(b"fatal runtime error", mark)
            assert not sees(b"Could not create the investigation", mark), (
                "composer send surfaced a recoverable failure instead of starting the turn"
            )
            # Same quit gesture as the existing TUI branch: Ctrl-Q twice, retry
            # briefly in case a startup redraw consumed input.
            quit_mark = len(capture)
            for _ in range(8):
                os.write(master, b"\x11\x11")
                time.sleep(0.25)
                if b"ARGOS_EXIT=" in capture:
                    break
            eventually(lambda: sees(b"ARGOS_EXIT=0", quit_mark), process, seconds=30)
            assert (
                termios.tcgetattr(slave)[3] & (termios.ICANON | termios.ECHO)
            ) == (before[3] & (termios.ICANON | termios.ECHO)), "terminal mode was not restored"
            os.write(master, b"\n")
            assert process.wait(timeout=10) == 0
        finally:
            if process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            done.set()
            reader.join(timeout=2)
            os.close(master)
            os.close(slave)
    print("PASS: composer-launched recon opens the investigation surface with no runtime abort, a live TUI and a clean exit")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--argos", type=Path, default=ROOT / "target/agents/debug/argos")
    parser.add_argument("--terminal-captures", type=Path, help="retain PTY lifecycle output for review")
    parser.add_argument("--tui-only", action="store_true", help="run just embedded API/Web terminal-display checks")
    parser.add_argument("--tui-case", choices=("all", "tui", "api", "web", "web-quit"), help="run one isolated terminal case")
    parser.add_argument("--tui-web-profile", choices=("dev", "installed"), default="dev", help="Web profile for terminal-display checks; installed avoids another dev server's workspace lock")
    parser.add_argument("--composer-smoke", action="store_true", help="regression: a TUI composer launch must not abort the Tokio worker runtime")
    parser.add_argument("--verify-terminal-screen", type=Path, help="check retained-cell and fresh-prompt matching against a failed PTY capture without launching services")
    args = parser.parse_args()
    if args.verify_terminal_screen:
        verify_terminal_screen(args.verify_terminal_screen)
        return
    if args.tui_case is not None:
        args.tui_only = True
    binary = args.argos.resolve()
    if args.composer_smoke:
        composer_smoke(binary)
        return
    host = binary.with_name("argos-api-host" + (".exe" if os.name == "nt" else ""))
    assert binary.is_file() and host.is_file(), "build argos and argos-api-host first"
    with tempfile.TemporaryDirectory(prefix="argos-web-stack-") as tmp:
        env = {**os.environ, "ARGOS_HOME": tmp}
        classifier_config = Path(tmp) / "classifier" / "zero-shot.json"
        env.pop("ARGOS_EMBED", None)
        env.pop("ARGOS_STORAGE_MODE", None)
        # Stub the OS URL handler: verify browser dispatch without opening tabs
        # during the regression suite. Production uses the default OS handler.
        browser_bin = Path(tmp) / "browser-bin"
        browser_bin.mkdir()
        opener = browser_bin / ("open" if sys.platform == "darwin" else "xdg-open")
        opener.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> "$ARGOS_BROWSER_TEST_LOG"\n')
        opener.chmod(0o700)
        browser_log = Path(tmp) / "browser.log"
        env["ARGOS_BROWSER_TEST_LOG"] = str(browser_log)
        env["PATH"] = str(browser_bin) + os.pathsep + env["PATH"]
        children = []
        log = open(Path(tmp) / "process.log", "w+")

        def run(*arguments, success=True):
            result = subprocess.run([str(binary), *arguments], cwd=ROOT, env=env, capture_output=True, text=True, timeout=30)
            assert (result.returncode == 0) == success, (arguments, result.stdout, result.stderr)
            return result

        def spawn(executable, *arguments):
            process = subprocess.Popen([str(executable), *arguments], cwd=ROOT, env=env, stdout=log, stderr=log)
            children.append(process)
            return process

        state_file = Path(tmp) / "stack.json"

        def state():
            return json.loads(state_file.read_text())

        def ready(port):
            value = json.loads(get(f"http://127.0.0.1:{port}/api/v1/health/ready"))
            return value["ready"] and value["storage"]

        def exercise_tui(target, quit_from_tui=False, wizard=False):
            if os.name != "posix":
                return  # PTY lifecycle is verified on Unix; normal stack tests still run.
            import fcntl
            import pty
            import select
            import struct
            import termios
            import threading

            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 160, 0, 0))
            before = termios.tcgetattr(slave)
            capture = bytearray()
            done = threading.Event()
            passed = False
            capture_name = target.removeprefix("--") + ("-quit" if quit_from_tui else "")

            def sees(label, start=0):
                return TerminalScreen(capture).contains(label, start)

            def read_terminal():
                while not done.is_set():
                    if select.select([master], [], [], 0.1)[0]:
                        try:
                            capture.extend(os.read(master, 65536))
                        except OSError:
                            return

            def terminal_session():
                os.setsid()
                fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

            api_port = free_port()
            arguments = ["stack", "up", target, "--terminal", "tui", "--api-port", str(api_port)]
            if target == "--web":
                arguments += ["--port", str(free_port()), "--no-open"]
                if args.tui_web_profile == "installed":
                    arguments += ["--profile", "installed"]
            if wizard:
                arguments = []
            # Keep a session owner alive after Argos exits. macOS revokes a
            # controlling PTY on session-owner exit, preventing a termios check.
            keeper = "import subprocess,sys,signal; signal.signal(signal.SIGINT, lambda *_: None); p=subprocess.Popen(sys.argv[1:]); code=p.wait(); print('ARGOS_EXIT='+str(code), flush=True); sys.stdin.readline(); sys.exit(code)"
            process = subprocess.Popen([sys.executable, "-c", keeper, str(binary), *arguments], cwd=ROOT,
                                       env={**env, "TERM": "xterm-256color"},
                                       stdin=slave, stdout=slave, stderr=slave,
                                       preexec_fn=terminal_session)
            children.append(process)
            reader = threading.Thread(target=read_terminal, daemon=True)
            reader.start()
            try:
                if wizard:
                    eventually(lambda: sees(b"Choose a service"), process)
                    if not classifier_config.is_file():
                        assert sees(b"Check Atlas classifier"), "the launcher must expose readiness checks"
                        assert sees(b"Atlas classifier (Rust-BERT): unavailable")
                        assert sees(b"2  API Server"), "classifier setup must leave other services available"
                        missing = json.loads(run("classifier", "status", "--json").stdout)
                        assert missing["can_infer"] is False and missing["diagnostic"]
                        install_classifier_fixture(classifier_config)
                        mark = len(capture)
                        os.write(master, b"5\r")
                        eventually(lambda: sees(b"Atlas classifier (Rust-BERT): ready", mark), process)
                        assert sees(b"Choose a service", mark)
                        status = json.loads(run("classifier", "status", "--json").stdout)
                        assert status["can_infer"] is True and status["model"]
                    if target == "--tui":
                        os.write(master, b"3\r")
                        eventually(lambda: sees(b"Web port"), process)
                        os.write(master, b"\x154010\r")
                        eventually(lambda: sees(b"Web profile"), process)
                        mark = len(capture)
                        os.write(master, b"\x1b")
                        eventually(lambda: sees(b"Web port", mark) and sees(b"4010", mark), process)
                        mark = len(capture)
                        os.write(master, b"\x1b")
                        eventually(lambda: sees(b"Choose a service", mark), process)
                        os.write(master, b"1\r")
                    else:
                        os.write(master, b"2\r")
                        eventually(lambda: sees(b"API port"), process)
                        os.write(master, b"\x15" + str(api_port).encode() + b"\r")
                    eventually(lambda: sees(b"Storage mode"), process)
                    os.write(master, b"\r")
                    # Wait for the actual terminal choices before confirming.
                    terminal_choice = b"Jobs / Logs available" if target == "--tui" else b"Server logs"
                    eventually(lambda: sees(terminal_choice), process)
                    os.write(master, b"\r" if target == "--tui" else b"\x1b[A\r")
                if target == "--tui":
                    eventually(lambda: sees(b"Intel") and sees(b"Recon"), process)
                    if sees(b"Open last app view session"):
                        os.write(master, b"n")
                    mark = len(capture)
                    # One pair arms/confirms TUI quit. Retrying while storage
                    # drains can queue Ctrl-Q for Venture and quit its menu.
                    os.write(master, b"\x11\x11")
                    eventually(lambda: sees(b"Choose a service", mark)
                               and sees(b"2  API Server", mark), process)
                    assert not state_file.exists(), "direct TUI created a managed service stack"
                    # Launch a different session in the same Venture parent:
                    # the previous TUI must have released embedded storage.
                    mark = len(capture)
                    os.write(master, b"2\r")
                    eventually(lambda: sees(b"API port", mark), process)
                    os.write(master, b"\x15" + str(api_port).encode() + b"\r")
                    eventually(lambda: sees(b"Storage mode", mark), process)
                    os.write(master, b"\r")
                    eventually(lambda: sees(b"Server logs", mark), process)
                    os.write(master, b"\r")  # Default service logs.
                    eventually(state, process, seconds=60)
                    eventually(lambda: ready(api_port), process)
                    mark = len(capture)
                    os.write(master, b"\x03")  # Foreground logs stop; parent survives.
                    eventually(lambda: sees(b"Choose a service", mark), process)
                    assert not state_file.exists()
                    os.write(master, b"\x1b")
                    eventually(lambda: sees(b"ARGOS_EXIT=0"), process)
                    assert (termios.tcgetattr(slave)[3] & (termios.ICANON | termios.ECHO)) == (before[3] & (termios.ICANON | termios.ECHO))
                    os.write(master, b"\n")
                    assert process.wait(timeout=10) == 0
                    passed = True
                    return
                managed = eventually(state, process, seconds=60)
                assert managed["terminal"] == "Tui"
                assert managed["api_pid"] == managed["supervisor_pid"] and not managed["adopted"]
                assert ready(api_port), "shared in-process API is not ready"
                eventually(lambda: (b"Intel" in capture and b"Recon" in capture) or b"Open last app view session" in capture, process)
                if b"Open last app view session" in capture:
                    os.write(master, b"n")  # Decline the prior test's saved Brain view.
                    # Redraws are terminal cell diffs, so a label need not be
                    # emitted again as one contiguous byte string.
                time.sleep(1.2)  # Include the first repository-change poll/refresh.
                assert b"Could not read memories" not in capture, "TUI boot ran synchronous repository reads inside Tokio"
                assert b"\x1b_G" not in capture and b"\x1b[5n" not in capture, "TUI probed or emitted article image graphics"
                assert not (termios.tcgetattr(slave)[3] & termios.ICANON), "TUI never entered raw mode"
                # Traffic uses durable service jobs; the RN app never reads them.
                get(f"http://127.0.0.1:{api_port}/api/v1/days?token=fixture-private-query")
                if target == "--web":
                    get(f"http://127.0.0.1:{managed['web_port']}/?token=fixture-private-query")
                rows = json.loads(get(f"http://127.0.0.1:{api_port}/api/v1/jobs"))["rows"]

                def service(operation):
                    matches = [row for row in rows if row["operation"] == operation and row["state"] == "running"]
                    assert len(matches) == 1, (operation, matches)
                    job = matches[0]
                    assert job["kind"] == "service" and not job["cancellable"]
                    return job

                api_job = service("api_server")

                def open_service_logs(job):
                    if sees(b"[Home] > Logs"):
                        os.write(master, b"\x1b")  # Logs returns to its selected job.
                    elif not sees(b"[Home] > Jobs"):
                        os.write(master, b"\x1b")
                        time.sleep(0.2)  # Finish Escape before the next key; ESC+5 means Alt+5.
                        os.write(master, b"5")
                    eventually(lambda: sees(b"[Home] > Jobs"), process)
                    column, row = eventually(
                        lambda: service_row_position(TerminalScreen(capture), job["title"]), process)
                    os.write(master, f"\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m".encode())
                    eventually(lambda: sees(job["id"].encode())
                               and sees(("Operation: " + job["operation"]).encode()), process)
                    os.write(master, b"l")
                    eventually(lambda: sees(b"[Home] > Logs")
                               and sees(job["id"][-8:].encode()), process)

                if target == "--web":
                    open_service_logs(service("web_server"))
                    eventually(lambda: sees(b"Argos Next server ready at"), process)
                    # Development Next emits access lines; the installed runtime
                    # emits server output, while owned API traffic has its own job.
                    if args.tui_web_profile == "dev":
                        eventually(lambda: sees(b"GET /"), process)
                open_service_logs(api_job)
                eventually(lambda: sees(b"GET /api/v1/days"), process)
                assert b"fixture-private-query" not in capture, "traffic included a query string"
                assert b"Unexpected text node" not in capture, "React Native rendered a raw text child inside View"
                if target == "--web":
                    assert "fixture-private-query" not in (Path(tmp) / "stack-services.log").read_text()
                return_mark = len(capture)
                if quit_from_tui:
                    # Two Ctrl-Q presses arm/confirm quit. Repeat briefly in
                    # case a startup redraw consumed input.
                    for _ in range(8):
                        os.write(master, b"\x11\x11")
                        time.sleep(0.25)
                        if b"ARGOS_EXIT=" in capture:
                            break
                else:
                    assert ready(api_port)
                    run("stack", "down", target)
                if wizard:
                    eventually(lambda: sees(b"Choose a service", return_mark), process, seconds=25)
                    os.write(master, b"\x1b")
                eventually(lambda: b"ARGOS_EXIT=0" in capture, process, seconds=25)
                assert not state_file.exists()
                assert (termios.tcgetattr(slave)[3] & (termios.ICANON | termios.ECHO)) == (before[3] & (termios.ICANON | termios.ECHO)), "terminal mode was not restored"
                os.write(master, b"\n")  # Release the keeper after inspecting restoration.
                assert process.wait(timeout=10) == 0
                try:
                    get(f"http://127.0.0.1:{api_port}/api/v1/health/ready")
                except (OSError, urllib.error.URLError):
                    pass
                else:
                    raise AssertionError("in-process API survived TUI stack shutdown")
                passed = True
            finally:
                if process.poll() is None:
                    try:
                        os.killpg(process.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                done.set()
                reader.join(timeout=2)
                if args.terminal_captures:
                    args.terminal_captures.mkdir(parents=True, exist_ok=True)
                    (args.terminal_captures / (capture_name + ".ansi")).write_bytes(capture)
                    service_log = Path(tmp) / "stack-services.log"
                    if service_log.exists() and target != "--tui":
                        (args.terminal_captures / (capture_name + "-services.log")).write_bytes(service_log.read_bytes()[-65536:])
                os.close(master)
                os.close(slave)
                if passed:
                    print(f"PASS: terminal case {capture_name}{' with Venture wizard' if wizard else ''}; service logs, shutdown and terminal restoration")

        try:
            if args.tui_only:
                run("storage", "init", "--mode", "embedded")
                if args.tui_case in ("api", "web", "web-quit"):
                    exercise_tui("--server" if args.tui_case == "api" else "--web",
                                 quit_from_tui=args.tui_case == "web-quit")
                elif args.tui_case == "tui":
                    exercise_tui("--tui", wizard=True)
                else:
                    exercise_tui("--tui", wizard=True)
                    exercise_tui("--server", wizard=True)
                    exercise_tui("--web", quit_from_tui=True)
                if args.tui_case in (None, "all"):
                    print("PASS: Venture wizard, shared embedded API/Web TUI, service Jobs/traffic Logs, clean startup, quit/down and raw-mode restoration")
                return
            run("stack", "up", "--server", success=False)
            assert not state_file.exists()
            run("storage", "init", "--mode", "embedded")
            api_port = free_port()
            # A cargo-run-style launcher must work without a companion host.
            launcher_dir = Path(tmp) / "launcher-only"
            launcher_dir.mkdir()
            launcher = launcher_dir / binary.name
            try:
                os.link(binary, launcher)
            except OSError:
                shutil.copy2(binary, launcher)
            assert not launcher.with_name(host.name).exists()
            server = spawn(launcher, "stack", "up", "--server", "--api-port", str(api_port))
            eventually(state, server)
            eventually(lambda: ready(api_port), server)
            run("stack", "status")
            run("stack", "up", "--server", "--api-port", str(api_port))
            # A second datastore owner must fail and must not stop the first.
            duplicate = subprocess.run([str(host), "--port", str(free_port())], cwd=ROOT, env=env, capture_output=True, text=True, timeout=15)
            assert duplicate.returncode != 0, "embedded ownership bypassed"
            assert ready(api_port)
            run("stack", "down", "--server")
            assert server.wait(timeout=15) == 0 and not state_file.exists()

            web_port, api_port = free_port(), free_port()
            web = spawn(launcher, "stack", "up", "--web", "--profile", "installed", "--port", str(web_port), "--api-port", str(api_port))
            managed = eventually(state, web)
            assert managed["frontend_pid"] and not managed["adopted"]
            assert browser_log.read_text().strip() == f"http://localhost:{web_port}/home"
            assert ready(api_port)
            eventually(lambda: b"ARGOS" in get(f"http://127.0.0.1:{web_port}/"), web)
            run("stack", "down", "--web")
            assert web.wait(timeout=15) == 0 and not state_file.exists()
            # Development runner and Ctrl-C propagation use owned process groups.
            web_port, api_port = free_port(), free_port()
            dev = spawn(binary, "stack", "up", "--web", "--no-open", "--port", str(web_port), "--api-port", str(api_port))
            managed = eventually(state, dev, seconds=120)
            assert ready(api_port)
            eventually(lambda: b"ARGOS" in get(f"http://127.0.0.1:{web_port}/"), dev, seconds=60)
            dev.send_signal(signal.SIGINT)
            assert dev.wait(timeout=15) == 0 and not state_file.exists()
            assert len(browser_log.read_text().splitlines()) == 1, "--no-open launched a browser"
            exercise_tui("--tui", wizard=True)
            exercise_tui("--server", wizard=True)
            exercise_tui("--web", quit_from_tui=True)
            # Restart using a separately owned API. down must preserve it.
            api_port = free_port()
            web_port = free_port()
            standalone = spawn(host, "--port", str(api_port), "--origins", f"http://localhost:{web_port},http://127.0.0.1:{web_port}")
            eventually(lambda: ready(api_port), standalone)
            adopted = spawn(binary, "stack", "up", "--web", "--no-open", "--profile", "installed", "--port", str(web_port), "--api-url", f"http://127.0.0.1:{api_port}")
            managed = eventually(state, adopted)
            assert managed["adopted"] and managed["api_pid"] is None
            run("stack", "down", "--web")
            assert adopted.wait(timeout=15) == 0
            assert standalone.poll() is None and ready(api_port)
            standalone.terminate()
            assert standalone.wait(timeout=15) == 0
            # A dead adopted endpoint refuses to start a frontend.
            failed = spawn(binary, "stack", "up", "--web", "--no-open", "--profile", "installed", "--port", str(free_port()), "--api-url", f"http://127.0.0.1:{api_port}")
            assert failed.wait(timeout=85) != 0
            assert not state_file.exists()
            print("PASS: single-binary server/web readiness, browser dispatch, exclusive storage, TUI/raw-mode restoration, restart, adoption retention, API-down refusal and cleanup")
        except Exception:
            log.flush()
            log.seek(0)
            print(log.read())
            raise
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=5)
            log.close()


if __name__ == "__main__":
    main()
