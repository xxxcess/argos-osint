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
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def get(url):
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url, timeout=1) as response:
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--argos", type=Path, default=ROOT / "target/agents/debug/argos")
    parser.add_argument("--terminal-captures", type=Path, help="retain PTY lifecycle output for review")
    parser.add_argument("--tui-only", action="store_true", help="run just embedded API/Web terminal-display checks")
    parser.add_argument("--tui-web-profile", choices=("dev", "installed"), default="dev", help="Web profile for terminal-display checks; installed avoids another dev server's workspace lock")
    args = parser.parse_args()
    binary = args.argos.resolve()
    host = binary.with_name("argos-api-host" + (".exe" if os.name == "nt" else ""))
    assert binary.is_file() and host.is_file(), "build argos and argos-api-host first"
    with tempfile.TemporaryDirectory(prefix="argos-web-stack-") as tmp:
        env = {**os.environ, "ARGOS_HOME": tmp}
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

            def terminal_text(start=0):
                """Return visible terminal bytes without redraw controls.

                Ratatui's incremental ANSI updates can place a cursor/style
                sequence inside a label. Match the rendered text rather than
                assuming each label arrives as one contiguous byte sequence.
                """
                return re.sub(
                    rb"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\\\))",
                    b"",
                    bytes(capture[start:]),
                )

            def sees(label, start=0):
                return label in terminal_text(start)

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
                    # Match the selected option label, not the question
                    # title: an incremental cursor repaint can overwrite
                    # part of the title in the raw PTY byte stream.
                    terminal_choice = b"Jobs / Logs available" if target == "--tui" else b"Server logs"
                    eventually(lambda: sees(terminal_choice), process)
                    os.write(master, b"\r" if target == "--tui" else b"\x1b[A\r")
                if target == "--tui":
                    eventually(lambda: sees(b"Intel") and sees(b"Recon"), process)
                    if sees(b"Open last app view session"):
                        os.write(master, b"n")
                    mark = len(capture)
                    for _ in range(8):
                        os.write(master, b"\x11\x11")
                        time.sleep(0.25)
                        if sees(b"Choose a service", mark):
                            break
                    eventually(lambda: sees(b"Choose a service", mark), process)
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
                operation = "web_server" if target == "--web" else "api_server"
                selected = next((index, row) for index, row in enumerate(rows) if row["operation"] == operation and row["state"] == "running")
                assert selected[1]["kind"] == "service" and not selected[1]["cancellable"]
                assert any(row["operation"] == "api_server" and row["state"] == "running" for row in rows)
                os.write(master, b"\x1b")
                time.sleep(0.2)
                os.write(master, b"5")
                eventually(lambda: selected[1]["title"].encode() in capture, process)
                # Jobs initially focuses its search field. Focus the measured
                # 160×50 table row before using the existing row shortcut.
                row = 10 + selected[0] * 2  # SGR mouse coordinates are 1-based.
                os.write(master, f"\x1b[<0;25;{row}M\x1b[<0;25;{row}m".encode())
                time.sleep(0.2)
                os.write(master, b"l")  # Existing Jobs → View logs shortcut.
                expected = b"GET /" if target == "--web" else b"GET /api/v1/days"
                eventually(lambda: expected in capture, process)
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
                    eventually(lambda: b"Choose a service" in capture[return_mark:], process, seconds=25)
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
                    (args.terminal_captures / (target.removeprefix("--") + ".ansi")).write_bytes(capture)
                    service_log = Path(tmp) / "stack-services.log"
                    if service_log.exists() and target != "--tui":
                        (args.terminal_captures / (target.removeprefix("--") + "-services.log")).write_bytes(service_log.read_bytes()[-65536:])
                os.close(master)
                os.close(slave)

        try:
            if args.tui_only:
                run("storage", "init", "--mode", "embedded")
                exercise_tui("--tui", wizard=True)
                exercise_tui("--server", wizard=True)
                exercise_tui("--web", quit_from_tui=True)
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
            assert browser_log.read_text().strip() == f"http://localhost:{web_port}/?api=http://127.0.0.1:{api_port}"
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
