#!/usr/bin/env python3
"""Read-only host detection shared by native packaging and scope planning.

Python 3.9 or newer is supported. Detection never builds, installs a toolchain,
invokes Docker or contacts a backend. Native targets describe the Mac app;
Docker platforms describe independently built Linux backend images.
"""

from dataclasses import asdict, dataclass
import os
import platform
import re
import subprocess


NATIVE_TARGETS = {
    "x86_64-apple-darwin": "x86_64",
    "aarch64-apple-darwin": "arm64",
}
ARCHITECTURES = {"x86_64": "x86_64", "amd64": "x86_64", "arm64": "arm64", "aarch64": "arm64"}
DOCKER_PLATFORMS = {"x86_64": "linux/amd64", "arm64": "linux/arm64"}


@dataclass(frozen=True)
class PlatformInfo:
    """Observed machine and toolchain state, including unavailable Rust."""

    system: str
    physical_architecture: str
    process_architecture: str
    rosetta: bool
    rust_host: str
    rust_release: str
    native_target: str
    docker_platform: str
    rust_error: str = ""

    def as_dict(self):
        return asdict(self)


def _command(command, runner, environment):
    try:
        result = runner(command, capture_output=True, text=True, timeout=15, env=environment, check=False)
    except (OSError, subprocess.SubprocessError) as error:
        return "", str(error)
    if result.returncode:
        return "", (result.stderr or result.stdout or f"exit {result.returncode}").strip()
    return result.stdout.strip(), ""


def detect(*, system=None, process_machine=None, runner=None, environment=None):
    """Observe physical/process architecture and the selected Rust host.

    The injected command runner and platform values support deterministic
    fixtures. RUSTUP_AUTO_INSTALL is disabled: detection is not an installer.
    Missing Rust is returned as structured state so Docker-platform reporting
    does not require a host Rust installation.
    """
    runner = runner or subprocess.run
    environment = dict(os.environ if environment is None else environment)
    environment["RUSTUP_AUTO_INSTALL"] = "0"
    system = system or platform.system()
    raw_machine = process_machine or platform.machine()
    process_arch = ARCHITECTURES.get(raw_machine.lower(), raw_machine.lower())
    physical_arch = process_arch
    translated = False
    if system == "Darwin":
        value, _ = _command(["sysctl", "-in", "sysctl.proc_translated"], runner, environment)
        if value not in ("", "0", "1"):
            raise ValueError("sysctl.proc_translated returned an invalid value")
        translated = value == "1"
        arm64, _ = _command(["sysctl", "-in", "hw.optional.arm64"], runner, environment)
        if arm64 not in ("", "0", "1"):
            raise ValueError("hw.optional.arm64 returned an invalid value")
        if translated or arm64 == "1":
            physical_arch = "arm64"
        elif process_arch not in ("x86_64", "arm64"):
            raise ValueError(f"Unsupported Mac architecture: {raw_machine}")
    rust, rust_error = _command(["rustc", "-vV"], runner, environment)
    host = re.search(r"^host:\s*(\S+)\s*$", rust, re.MULTILINE)
    release = re.search(r"^release:\s*(\S+)\s*$", rust, re.MULTILINE)
    rust_host = host.group(1) if host else ""
    rust_release = release.group(1) if release else ""
    if rust and (not rust_host or not rust_release):
        rust_error = "rustc -vV did not report a host and release"
    native = next((target for target, arch in NATIVE_TARGETS.items()
                   if system == "Darwin" and arch == physical_arch), "")
    docker = DOCKER_PLATFORMS.get(physical_arch, "") if system in ("Darwin", "Linux") else ""
    return PlatformInfo(system, physical_arch, process_arch, translated,
                        rust_host, rust_release, native, docker, rust_error)


def resolve_native_target(info, requested="host"):
    """Select an explicit Mac target, refusing an ambiguous host build."""
    if info.system != "Darwin" or not info.native_target:
        raise ValueError("The native app requires a supported Intel or Apple Silicon Mac; Linux is a backend target")
    if requested not in ("host", *NATIVE_TARGETS):
        raise ValueError(f"Unsupported native target: {requested}")
    if not info.rust_host or not info.rust_release:
        raise ValueError("Rust is unavailable; install/select the repository-pinned toolchain explicitly before building")
    if requested == "host":
        if info.rosetta or info.process_architecture != info.physical_architecture:
            raise ValueError("Host build is running under Rosetta; use a native terminal and matching Rust toolchain, or choose an explicit cross target")
        if info.rust_host != info.native_target:
            raise ValueError(f"Rust host {info.rust_host} does not match physical Mac target {info.native_target}; select a matching native toolchain or an explicit cross target")
        return info.native_target
    return requested


def format_platform(info, requested_format):
    """Return one requested target without implying native Linux support."""
    if requested_format == "rust":
        return resolve_native_target(info)
    if requested_format == "docker":
        if not info.docker_platform:
            raise ValueError(f"Unsupported Linux backend platform for {info.system}/{info.physical_architecture}")
        return info.docker_platform
    if requested_format == "json":
        import json
        return json.dumps(info.as_dict(), sort_keys=True, indent=2)
    raise ValueError(f"Unsupported platform format: {requested_format}")
