#!/usr/bin/env python3
"""Explicit, one-shot Fleetworks configuration cutover; inventory is read-only.

No startup/build caller, recursive delete, automatic home discovery, Keychain
scan, secret backup, JSON authorization-proof import, or infrastructure-token
deletion exists. Applying requires an authenticated backend reset fence.
"""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import http.client
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
import unicodedata
from urllib.parse import urlencode, urlsplit

if sys.version_info < (3, 9):
    raise SystemExit("Fleetworks reset requires Python 3.9 or newer; no interpreter is installed automatically")

LIMIT = 2 * 1024 * 1024
ROLES = ("recon_agent", "recon_decisions", "recon_analyst", "atlas_evaluator", "atlas_analyst", "brain_summary")
FRESH_CONFIG = ("configuration_format = 1\n" + "".join(
    f'\n[defaults.{role}]\nprovider = ""\nmodel = ""\naccount = ""\nfallbacks = []\n' for role in ROLES)).encode()
FRESH = {"config.toml": FRESH_CONFIG, "auth.json": b'{"configuration_format":1,"accounts":{}}\n', "quota.json": b'{"settings":[]}\n'}
USER_ENV = frozenset(("OPENAI_API_KEY", "OPENROUTER_API_KEY", "GEMINI_API_KEY", "NVIDIA_API_KEY", "FLEETWORKS_API_TOKEN", "FLEETWORKS_API_TOKEN_FILE", "ARGOS_RUST_API_TOKEN", "ARGOS_RUST_API_TOKEN_FILE") + tuple(
    name + suffix for name in ("FIRECRAWL_API_KEY", "HUNTER_API_KEY", "SOCIAVAULT_API_KEY", "NEWSAPI_API_KEY", "COURTLISTENER_API_TOKEN", "GNEWS_API_KEY", "NEWSDATA_API_KEY", "CURRENTS_API_KEY", "WHOXY_API_KEY") for suffix in ("", "_FALLBACK")))
SAFE_ID = re.compile(r"[A-Za-z0-9_-]{1,80}\Z")
CONNECTIONS = ".native-connections.json"
NATIVE_LIMIT = 256 * 1024
NATIVE_GUARDS = (".native.lock", ".native-state.json", "default/.fleetworks-workspace.json")


class ResetError(Exception):
    """Safe operator diagnostic: never constructed from a secret body."""


class Pending(ResetError):
    """The independent backend/local authorities have not both completed."""


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def sha(body):
    return hashlib.sha256(body).hexdigest()


def identity(metadata):
    return {"dev": metadata.st_dev, "ino": metadata.st_ino, "uid": metadata.st_uid,
            "mode": stat.S_IMODE(metadata.st_mode)}


def absolute(value):
    path = Path(value)
    if not path.is_absolute() or str(path) != value or ".." in path.parts:
        raise ResetError("An exact absolute path without traversal is required")
    return path


@contextlib.contextmanager
def directory(path, *, private=False):
    """Retain no-follow directory capabilities for every absolute component."""
    path = absolute(str(path))
    descriptors = [os.open("/", os.O_RDONLY | os.O_DIRECTORY)]
    chain = []
    try:
        for part in path.parts[1:]:
            fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptors[-1])
            descriptors.append(fd)
            chain.append(identity(os.fstat(fd)))
        current = os.fstat(descriptors[-1])
        if current.st_uid != os.getuid() or current.st_mode & 0o022:
            raise ResetError("Selected directory must be owned and not writable by others")
        if private and current.st_mode & 0o077:
            raise ResetError("Reset ledger must have private directory permissions")
        yield descriptors[-1], chain
    finally:
        for fd in reversed(descriptors):
            os.close(fd)


def relative(value):
    if not isinstance(value, str) or value not in ("config/workspace.toml", "secrets/environment.toml", "secrets/api-token") and not (
        value in FRESH or value == CONNECTIONS or re.fullmatch(r"(?:config\.toml|auth\.json|quota\.json)\.[0-9]+\.(?:bak|staged)", value)
        or re.fullmatch(r"exports/[A-Za-z0-9_-]{1,100}\.json", value)):
        raise ResetError("File is outside the exact application payload allowlist")
    return value.split("/")


@contextlib.contextmanager
def parent(root, name):
    parts = relative(name)
    with directory(root) as (fd, chain):
        child_fds = []
        try:
            for part in parts[:-1]:
                fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                child_fds.append(fd)
                metadata = os.fstat(fd)
                if metadata.st_uid != os.getuid() or metadata.st_mode & 0o022:
                    raise ResetError("Payload parent is unsafe")
                chain.append(identity(metadata))
            yield fd, parts[-1], chain
        finally:
            for fd in reversed(child_fds):
                os.close(fd)


def read_at(fd, name, *, private=False):
    try:
        opened = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
    except FileNotFoundError:
        return {"exists": False}, b""
    with os.fdopen(opened, "rb") as file:
        metadata = os.fstat(file.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_nlink != 1 or metadata.st_size > LIMIT:
            raise ResetError("Payload must be a bounded, owned, singly linked regular file")
        if private and metadata.st_mode & 0o077:
            raise ResetError("Administrative input must have private file permissions")
        body = file.read(LIMIT + 1)
        after = os.fstat(file.fileno())
        if len(body) > LIMIT or (metadata.st_ino, metadata.st_size, metadata.st_mtime_ns, metadata.st_ctime_ns) != (after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise ResetError("Payload changed while reading")
        return {"exists": True, **identity(metadata), "size": len(body), "mtime_ns": metadata.st_mtime_ns,
                "ctime_ns": metadata.st_ctime_ns, "sha256": sha(body)}, body


def read_private(path):
    path = absolute(str(path))
    with directory(path.parent) as (fd, _):
        snapshot, body = read_at(fd, path.name, private=True)
    if not snapshot["exists"]:
        raise ResetError("Administrative input is missing")
    return body


def document(body, *, limit=LIMIT):
    """Strict bounded metadata parser; diagnostics never include document data."""
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ResetError("Metadata contains duplicate fields")
            result[key] = value
        return result
    try:
        if len(body) > limit:
            raise ResetError("Native metadata exceeds its size bound")
        return json.loads(body, object_pairs_hook=unique,
                          parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
    except (ValueError, UnicodeError, RecursionError):
        raise ResetError("Native metadata is not a recognized document") from None


def fields(value, required, optional=()):
    if not isinstance(value, dict) or not set(required) <= set(value) or set(value) - set(required) - set(optional):
        raise ResetError("Native metadata has missing or unknown fields")


def uint(value, bits=64):
    return type(value) is int and 0 <= value < 1 << bits


def marker(value):
    return isinstance(value, str) and re.fullmatch(r"[a-fA-F0-9]{64}", value) is not None


def native_object(value):
    fields(value, ("device", "inode", "owner"))
    if not uint(value["device"]) or not uint(value["inode"]) or not uint(value["owner"], 32):
        raise ResetError("Native metadata object identity is invalid")
    return value


def object_at(fd):
    metadata = os.fstat(fd)
    return {"device": metadata.st_dev, "inode": metadata.st_ino, "owner": metadata.st_uid}


def native_origin(value):
    """Validate persisted canonical SDK origins without a client or network."""
    try:
        if not isinstance(value, str) or len(value.encode("utf-8")) > 2048:
            raise ValueError()
        parsed = urlsplit(value)
        host, port = parsed.hostname, parsed.port
        if not host or parsed.scheme not in ("http", "https") or parsed.username is not None or parsed.password is not None or parsed.query or parsed.fragment or parsed.path != "/":
            raise ValueError()
        try:
            address = ipaddress.ip_address(host)
            normalized = "[" + address.compressed + "]" if address.version == 6 else address.compressed
            loopback = address.is_loopback
        except ValueError:
            if not re.fullmatch(r"[a-z0-9_.-]+", host) or host.encode("idna").decode("ascii") != host:
                raise ValueError()
            normalized, loopback = host, host == "localhost"
        if parsed.scheme == "http" and not loopback:
            raise ValueError()
        suffix = "" if port is None or port == (443 if parsed.scheme == "https" else 80) else ":" + str(port)
        if value != parsed.scheme + "://" + normalized + suffix + "/":
            raise ValueError()
    except (ValueError, UnicodeError):
        raise ResetError("Saved connection origin is not a supported canonical SDK origin") from None


@contextlib.contextmanager
def native_lock(root):
    """Use the existing manager barrier; inventory must never create a lock."""
    opened = os.open(".native.lock", os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=root)
    try:
        metadata = os.fstat(opened)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_nlink != 1 or metadata.st_mode & 0o077:
            raise ResetError("Native lock ownership or permissions need repair")
        try:
            fcntl.flock(opened, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ResetError("Native catalog is busy; no reset payload was changed") from None
        if identity(os.stat(".native.lock", dir_fd=root, follow_symlinks=False)) != identity(metadata):
            raise ResetError("Native lock object changed")
        yield
    finally:
        os.close(opened)


def native_authority(root):
    """Inspect only the protected root, catalog, and one reserved default."""
    root_identity = object_at(root)
    if os.fstat(root).st_mode & 0o077:
        raise ResetError("Native root permissions require repair")
    state_before, raw = read_at(root, ".native-state.json", private=True)
    if not state_before["exists"] or len(raw) > 1024 * 1024:
        raise ResetError("Native installation catalog is missing or exceeds its bound")
    state = document(raw)
    fields(state, ("version", "installation_marker", "root_object", "workspaces", "operations"), ("registration_attempts", "rebind_attempts"))
    if type(state["version"]) is not int or state["version"] != 1 or not marker(state["installation_marker"]) or native_object(state["root_object"]) != root_identity:
        raise ResetError("Native installation ownership or version needs repair")
    for key, bound in (("workspaces", 1024), ("operations", 4096), ("registration_attempts", 1024), ("rebind_attempts", 1024)):
        if not isinstance(state.get(key, []), list) or len(state.get(key, [])) > bound:
            raise ResetError("Native installation catalog exceeds its supported bounds")
    defaults = []
    names, markers = set(), set()
    for record in state["workspaces"]:
        fields(record, ("name", "marker", "object", "protected_default", "state", "bindings"), ("binding_history",))
        name = record["name"]
        if not isinstance(name, str) or not name or not marker(record["marker"]) or record["marker"] in markers or type(record["protected_default"]) is not bool or record["state"] not in ("creation_pending", "local", "registered", "deletion_pending", "cleanup_pending", "removed") or not isinstance(record["bindings"], list) or not isinstance(record.get("binding_history", []), list):
            raise ResetError("Native workspace identity needs repair")
        markers.add(record["marker"])
        if record["state"] != "removed":
            if name.casefold() in names:
                raise ResetError("Native workspace identities conflict")
            names.add(name.casefold())
        obj = record["object"]
        if obj is None:
            if record["state"] != "creation_pending":
                raise ResetError("Native workspace object identity is missing")
        elif native_object(obj) == root_identity or obj["owner"] != root_identity["owner"] or obj["device"] != root_identity["device"]:
            raise ResetError("Native workspace object identity conflicts")
        if name.casefold() == "default":
            defaults.append(record)
    if len(defaults) != 1 or defaults[0]["name"] != "default" or defaults[0]["protected_default"] is not True or defaults[0]["state"] not in ("local", "registered"):
        raise ResetError("Protected native default is not complete and valid")
    default = defaults[0]
    child = os.open("default", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
    try:
        if object_at(child) != default["object"] or os.fstat(child).st_mode & 0o077:
            raise ResetError("Protected native default object changed")
        before, raw = read_at(child, ".fleetworks-workspace.json", private=True)
        proof = document(raw)
        fields(proof, ("version", "installation_marker", "name", "marker", "object", "protected_default"))
        native_object(proof["object"])
        expected = {"version": 1, "installation_marker": state["installation_marker"], "name": "default", "marker": default["marker"], "object": default["object"], "protected_default": True}
        if not before["exists"] or proof != expected or type(proof.get("version")) is not int or proof.get("protected_default") is not True:
            raise ResetError("Protected native default marker conflicts")
    finally:
        os.close(child)
    lock_before, _ = read_at(root, ".native.lock", private=True)
    return {"installation_marker": state["installation_marker"], "root_object": root_identity}, [
        (".native.lock", lock_before), (".native-state.json", state_before), ("default/.fleetworks-workspace.json", before)]


def native_replacement(body, exists, authority, *, increment=True):
    if not exists:
        return None, []
    catalog = document(body, limit=NATIVE_LIMIT)
    fields(catalog, ("format_version", "installation_marker", "root_object", "snapshot"))
    if type(catalog["format_version"]) is not int or catalog["format_version"] != 1 or catalog["installation_marker"] != authority["installation_marker"] or native_object(catalog["root_object"]) != authority["root_object"]:
        raise ResetError("Saved connections belong to another installation or format")
    snapshot = catalog["snapshot"]
    fields(snapshot, ("revision", "selected_profile_id", "profiles"))
    if not uint(snapshot["revision"]) or increment and snapshot["revision"] == (1 << 64) - 1 or not isinstance(snapshot["profiles"], list) or len(snapshot["profiles"]) > 32:
        raise ResetError("Saved connection revision or capacity needs repair")
    ids, names, references = set(), set(), []
    for profile in snapshot["profiles"]:
        fields(profile, ("id", "revision", "name", "origin", "credential"))
        name = profile["name"]
        if not marker(profile["id"]) or profile["id"] in ids or not uint(profile["revision"]) or not 0 < profile["revision"] <= snapshot["revision"] or not isinstance(name, str) or not 1 <= len(name.encode("utf-8")) <= 128 or name.strip() != name or any(unicodedata.category(char) == "Cc" for char in name) or name.casefold() in names:
            raise ResetError("Saved connection identity, name or revision needs repair")
        ids.add(profile["id"])
        names.add(name.casefold())
        native_origin(profile["origin"])
        reference = profile["credential"]
        fields(reference, ("path", "origin", "object"))
        path = reference["path"]
        if not isinstance(path, str) or not 1 <= len(path.encode("utf-8")) <= 4096 or any(unicodedata.category(char) == "Cc" for char in path) or str(absolute(path)) != path or reference["origin"] != profile["origin"] or native_object(reference["object"])["owner"] != authority["root_object"]["owner"]:
            raise ResetError("Saved credential reference is invalid")
        # A reference is not ownership of the selected file. Do not open it or
        # expose its private path in a plan/diagnostic; retain a review digest.
        references.append({"profile_id": profile["id"], "path_sha256": sha(path.encode("utf-8")), "state": "external_reference_not_read_or_erased"})
    selected = snapshot["selected_profile_id"]
    if selected is not None and (not isinstance(selected, str) or selected not in ids):
        raise ResetError("Selected saved connection is missing")
    fresh = {"format_version": 1, **authority, "snapshot": {"revision": snapshot["revision"] + int(increment), "selected_profile_id": None, "profiles": []}}
    return fresh, references


def replacement(name, body, exists):
    if name in FRESH:
        if exists and body:
            try:
                parsed = body.decode() if name == "config.toml" else json.loads(body)
            except (ValueError, UnicodeError):
                raise ResetError("Configuration payload is not a recognized application document") from None
            if name == "config.toml" and parsed.strip() and not re.search(r"(?m)^\s*(?:\[defaults(?:\.|\])|(?:configuration_format|osint_user_agent|writer_provider|firecrawl_api_key|hunter_api_key|recon_limits)\s*=)", parsed):
                raise ResetError("Configuration ownership needs explicit operator review")
            if name == "auth.json" and (not isinstance(parsed, dict) or set(parsed) - {"configuration_format", "accounts", "text", "voice"}):
                raise ResetError("Authentication document contains unknown app ownership")
            if name == "quota.json" and (not isinstance(parsed, dict) or set(parsed) - {"settings"}):
                raise ResetError("Quota document contains unknown app ownership")
        return FRESH[name]
    if name in ("config/workspace.toml", "secrets/environment.toml"):
        # These files can contain database/internal/admin provisioning. Removing
        # a whole mixed file would strand data. The independent operator must
        # sanitize/reprovision it before a new frozen inventory is recorded.
        if body.strip():
            raise ResetError("Mixed infrastructure/user environment requires operator sanitation")
        return b""  # An already-sanitized empty payload can be frozen safely.
    if name.startswith("exports/") and exists and body:
        try:
            document = json.loads(body)
        except (ValueError, UnicodeError):
            raise ResetError("Export is not a verified portable profile") from None
        fields = {"schema_version", "providers", "tool_credentials", "model_roles", "rate_limits"}
        if not isinstance(document, dict) or not fields <= set(document) or document["schema_version"] not in (1, 2, 3, 4):
            raise ResetError("Export is not a verified portable profile")
    return b""


def manifest(value):
    if set(value) != {"version", "reset_id", "ledger", "backend", "targets", "secure_store"} or value["version"] != 1 or not SAFE_ID.fullmatch(value["reset_id"]):
        raise ResetError("Unsupported reset manifest")
    if value["secure_store"] != []:
        raise ResetError("No exact application Keychain item adapter has been established")
    with directory(absolute(value["ledger"]), private=True):
        pass
    backend = value["backend"]
    if set(backend) != {"origin", "deployment_id", "scope_id", "expected_configuration_epoch", "expected_access_epoch"}:
        raise ResetError("Exact backend reset authority is required")
    validate_origin(backend["origin"])
    if not all(isinstance(backend[key], str) and SAFE_ID.fullmatch(backend[key]) for key in backend if key != "origin"):
        raise ResetError("Invalid backend identity")
    if not isinstance(value["targets"], list) or not 1 <= len(value["targets"]) <= 64:
        raise ResetError("Explicit named targets are required")
    names, paths = set(), set()
    for target in value["targets"]:
        if set(target) != {"name", "root", "files"} or not SAFE_ID.fullmatch(target["name"]) or target["name"] in names:
            raise ResetError("Unique named reset target required")
        root = absolute(target["root"])
        if root == Path("/") or root == Path.home() or target["root"] in paths:
            raise ResetError("A home, filesystem root, or duplicate root cannot be a target")
        names.add(target["name"])
        paths.add(target["root"])
        if not isinstance(target["files"], list) or not 1 <= len(target["files"]) <= 128 or len(set(target["files"])) != len(target["files"]):
            raise ResetError("Explicit bounded payload names required")
        for name in target["files"]:
            relative(name)
        if CONNECTIONS in target["files"] and root.name != ".fleetworks":
            raise ResetError("Saved connections require the explicitly selected protected native root")
    return value


def inventory(selected):
    selected = manifest(selected)
    entries, blockers, guards = [], [], []
    for target in selected["targets"]:
        native = CONNECTIONS in target["files"]
        with directory(target["root"], private=native) as (root, root_chain), contextlib.ExitStack() as locks:
            authority = None
            if native:
                if Path(target["root"]).name != ".fleetworks":
                    raise ResetError("Saved connections require the explicitly selected protected native root")
                locks.enter_context(native_lock(root))
                authority, protected_native = native_authority(root)
                guards.extend({"root": target["root"], "file": name, "before": before}
                              for name, before in protected_native)
            protected, raw = read_at(root, ".argos-config-journal.json")
            guards.append({"root": target["root"], "file": ".argos-config-journal.json", "before": protected})
            if protected["exists"]:
                try:
                    state = json.loads(raw).get("state")
                except (ValueError, AttributeError):
                    state = None
                if state not in ("committed", "restored"):
                    blockers.append({"target": target["name"], "file": ".argos-config-journal.json", "reason": "Configuration recovery must finish before reset inventory"})
            names = os.listdir(root)
            if len(names) > 4096:
                raise ResetError("Application root inventory exceeds the bounded entry count")
            if CONNECTIONS in names and not native:
                blockers.append({"target": target["name"], "file": CONNECTIONS, "reason": "Unlisted saved connection payload needs explicit review"})
            for name in names:
                if (re.fullmatch(r"(?:config\.toml|auth\.json|quota\.json)\.[0-9]+\.(?:bak|staged)", name) or name.startswith(".argos-tmp-")) and name not in target["files"]:
                    blockers.append({"target": target["name"], "file": name, "reason": "Unlisted credential-bearing staging/backup needs exact review"})
            try:
                exports = os.open("exports", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
            except FileNotFoundError:
                exports = None
            if exports is not None:
                try:
                    metadata = os.fstat(exports)
                    if metadata.st_uid != os.getuid() or metadata.st_mode & 0o022:
                        raise ResetError("Application export directory needs ownership review")
                    export_names = os.listdir(exports)
                    if len(export_names) > 4096:
                        raise ResetError("Application export inventory exceeds its entry bound")
                    for name in export_names:
                        if name.endswith(".json") and "exports/" + name not in target["files"]:
                            blockers.append({"target": target["name"], "file": "exports/" + name, "reason": "Unlisted possible credential export needs exact review"})
                finally:
                    os.close(exports)
            for name in target["files"]:
                with parent(target["root"], name) as (fd, basename, chain):
                    snapshot, body = read_at(fd, basename, private=name == CONNECTIONS)
                extra = {}
                try:
                    if name == CONNECTIONS:
                        fresh, references = native_replacement(body, snapshot["exists"], authority)
                        extra = {"native_action": fresh, "external_references": references}
                        post_hash = sha(encoded(fresh) + b"\n") if fresh is not None else None
                    else:
                        post_hash = sha(replacement(name, body, snapshot["exists"]))
                except ResetError as error:
                    blockers.append({"target": target["name"], "file": name, "reason": str(error)})
                    post_hash = None
                entries.append({"target": target["name"], "root": target["root"], "file": name,
                                "root_chain": root_chain, "parent_chain": chain, "before": snapshot, "after_sha256": post_hash, **extra})
    plan = {"version": 1, "manifest": selected, "entries": entries, "guards": guards, "blockers": blockers,
            "limitations": ["backend_reset_seam_required", "legacy_reader_retirement_pending", "external_environment_and_unlisted_exports_require_review"]}
    if any(CONNECTIONS in target["files"] for target in selected["targets"]):
        plan["limitations"] += ["external_connection_token_references_not_read_or_erased", "native_live_client_cache_owner_ack_unsupported"]
    plan["plan_sha256"] = sha(encoded(plan))
    return plan


def write_json(fd, name, value, *, new=False):
    body = encoded(value) + b"\n"
    if new:
        temporary = name
    else:
        read_at(fd, name, private=True)
        temporary = ".reset-write-" + secrets.token_hex(12)
    try:
        opened = os.open(temporary, os.O_WRONLY | os.O_NOFOLLOW | os.O_CREAT | os.O_EXCL, 0o600, dir_fd=fd)
        with os.fdopen(opened, "wb") as file:
            file.write(body)
            file.flush()
            os.fsync(file.fileno())
        if not new:
            os.replace(temporary, name, src_dir_fd=fd, dst_dir_fd=fd)
        os.fsync(fd)
    finally:
        if not new:
            with contextlib.suppress(FileNotFoundError):
                os.unlink(temporary, dir_fd=fd)


def validate_origin(value):
    parsed = urlsplit(value)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
        raise ResetError("Backend must be an exact authenticated HTTP(S) origin")
    if parsed.scheme == "http" and parsed.hostname not in ("127.0.0.1", "::1", "localhost"):
        raise ResetError("Remote reset administration requires HTTPS")
    return parsed


class Backend:
    """Actual authenticated control channel; never accepts a proof JSON file."""
    def __init__(self, origin, token_file):
        self.origin = validate_origin(origin)
        self.token_path = absolute(str(token_file))
        self.token = read_private(token_file).decode("ascii").strip()
        if not 32 <= len(self.token) <= 256 or not re.fullmatch(r"[A-Za-z0-9._~-]+", self.token):
            raise ResetError("Invalid independent administrative credential")

    def request(self, phase, payload):
        connection_type = http.client.HTTPSConnection if self.origin.scheme == "https" else http.client.HTTPConnection
        connection = connection_type(self.origin.hostname, self.origin.port, timeout=15)
        try:
            path = "/api/v1/admin/configuration-reset/" + phase
            if phase == "status":
                path += "?" + urlencode(payload)
            connection.request("GET" if phase == "status" else "POST", path,
                               None if phase == "status" else encoded(payload),
                               {"Authorization": "Bearer " + self.token, "Content-Type": "application/json"})
            response = connection.getresponse()
            raw = response.read(64 * 1024 + 1)
            if response.status != 200 or len(raw) > 64 * 1024:
                raise Pending("Authenticated backend reset seam unavailable or denied")
            return json.loads(raw)
        except (OSError, ValueError, http.client.HTTPException):
            raise Pending("Authenticated backend reset seam unavailable") from None
        finally:
            connection.close()


def backend_receipt(backend, phase, plan, receipts=None):
    selected = plan["manifest"]
    expected = selected["backend"]
    payload = {"version": 1, "reset_id": selected["reset_id"], "plan_sha256": plan["plan_sha256"], **{key: expected[key] for key in ("deployment_id", "expected_configuration_epoch", "expected_access_epoch")}}
    if phase in ("complete", "finalize"):
        payload["local_receipts"] = receipts
    receipt = backend.request(phase, payload)
    if not isinstance(receipt, dict) or any(receipt.get(key) != value for key, value in {
        "version": 1, "reset_id": selected["reset_id"], "plan_sha256": plan["plan_sha256"],
        "deployment_id": expected["deployment_id"], "scope_id": expected["scope_id"],
        "admission_fenced": phase != "finalize", "work_drained": True, "user_access_revoked": True, "sessions_revoked": True,
        "first_use_blocked": phase != "finalize"}.items()):
        raise Pending("Backend has not established the exact durable reset fence")
    flags = ("admission_fenced", "first_use_blocked", "work_drained", "user_access_revoked", "sessions_revoked")
    if any(type(receipt.get(key)) is not bool for key in flags):
        raise Pending("Backend reset receipt has untyped authority flags")
    for key, old in (("configuration_epoch", "expected_configuration_epoch"), ("access_epoch", "expected_access_epoch")):
        if not isinstance(receipt.get(key), str) or not SAFE_ID.fullmatch(receipt[key]) or receipt[key] == expected[old]:
            raise Pending("Backend has not advanced the reset epochs")
    return receipt


def verify_entry(entry, *, intent=False):
    if entry["file"] == CONNECTIONS:
        return verify_native_entry(entry, intent=intent)
    with parent(entry["root"], entry["file"]) as (fd, name, chain):
        snapshot, body = read_at(fd, name)
    if chain != entry["parent_chain"]:
        raise ResetError("Frozen parent identity changed")
    fresh = replacement(entry["file"], b"" if intent else body, entry["before"]["exists"] if not intent else False)
    if sha(fresh) != entry["after_sha256"]:
        raise ResetError("Frozen action changed")
    if snapshot == entry["before"]:
        return fresh, snapshot
    same_object = all(snapshot.get(key) == entry["before"].get(key) for key in ("dev", "ino", "uid"))
    # Only a journaled in-place operation on the frozen inode may resume an
    # interrupted write. Accept solely a prefix of canonical empty bytes.
    if intent and same_object and fresh.startswith(body):
        return fresh, snapshot
    if intent and not entry["before"]["exists"] and snapshot.get("exists") and snapshot["size"] == 0:
        return fresh, snapshot
    raise ResetError("Frozen file changed; preserve new values and review pending reset")


def verify_native_entry(entry, *, intent=False):
    with directory(entry["root"], private=True) as (root, _):
        authority, _ = native_authority(root)
        snapshot, body = read_at(root, CONNECTIONS, private=True)
    with parent(entry["root"], CONNECTIONS) as (_, _, chain):
        if chain != entry["parent_chain"]:
            raise ResetError("Frozen native parent identity changed")
    action = entry["native_action"]
    if action is None:
        if entry["before"]["exists"] or entry["after_sha256"] is not None:
            raise ResetError("Frozen absent native action changed")
        fresh = None
    else:
        # Validate the exact empty action even when resuming a rename that
        # completed before the local ledger's done flag reached disk.
        native_replacement(encoded(action), True, authority, increment=False)
        if action["snapshot"]["profiles"] != [] or action["snapshot"]["selected_profile_id"] is not None or action["snapshot"]["revision"] == 0:
            raise ResetError("Frozen native action is not the current empty format")
        fresh = encoded(action) + b"\n"
        if sha(fresh) != entry["after_sha256"]:
            raise ResetError("Frozen native action changed")
    if snapshot == entry["before"]:
        derived, _ = native_replacement(body, snapshot["exists"], authority)
        if derived != action:
            raise ResetError("Frozen native revision action changed")
        return fresh, snapshot
    if intent and fresh is not None and snapshot["exists"] and body == fresh:
        return fresh, snapshot
    raise ResetError("Frozen saved connections changed; preserve new profiles and review reset")


def read_guard(root, name):
    with directory(root) as (fd, _):
        if name == "default/.fleetworks-workspace.json":
            child = os.open("default", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            try:
                return read_at(child, ".fleetworks-workspace.json", private=True)[0]
            finally:
                os.close(child)
        return read_at(fd, name, private=name in NATIVE_GUARDS)[0]


def write_native_entry(entry, fresh, expected):
    """Atomic empty catalog CAS under the manager lock, never an in-place tear."""
    if fresh is None or expected.get("sha256") == sha(fresh):
        return  # Absent stays absent; a journaled completed rename is resumable.
    with directory(entry["root"], private=True) as (fd, chain):
        if chain != entry["root_chain"]:
            raise ResetError("Frozen native root changed before mutation")
        temporary = ".native-reset-" + secrets.token_hex(12)
        try:
            opened = os.open(temporary, os.O_WRONLY | os.O_NOFOLLOW | os.O_CREAT | os.O_EXCL, 0o600, dir_fd=fd)
            with os.fdopen(opened, "wb") as file:
                file.write(fresh)
                file.flush()
                os.fsync(file.fileno())
            current, _ = read_at(fd, CONNECTIONS, private=True)
            if current != expected:
                raise ResetError("Frozen saved connection object changed before replacement")
            # All native readers/writers use .native.lock. Recheck its retained
            # object and the installation journals immediately before rename.
            for guard in entry["native_guards"]:
                if read_guard(entry["root"], guard["file"]) != guard["before"]:
                    raise ResetError("Frozen native authority changed before replacement")
            os.replace(temporary, CONNECTIONS, src_dir_fd=fd, dst_dir_fd=fd)
            os.fsync(fd)
            after, body = read_at(fd, CONNECTIONS, private=True)
            if not after["exists"] or body != fresh:
                raise ResetError("Native payload changed during reset; keep scope fenced")
        finally:
            with contextlib.suppress(FileNotFoundError):
                os.unlink(temporary, dir_fd=fd)


def apply(plan, backend, targets, *, after_step=None):
    claimed = dict(plan)
    digest = claimed.pop("plan_sha256", None)
    if sha(encoded(claimed)) != digest or plan["version"] != 1:
        raise ResetError("Reset plan digest mismatch")
    selected = manifest(plan["manifest"])
    expected_entries = [(target["name"], target["root"], name) for target in selected["targets"] for name in target["files"]]
    if [(entry["target"], entry["root"], entry["file"]) for entry in plan["entries"]] != expected_entries:
        raise ResetError("Plan entries do not match the exact named allowlist")
    expected_guards = [(target["root"], name) for target in selected["targets"]
                       for name in ((*NATIVE_GUARDS, ".argos-config-journal.json") if CONNECTIONS in target["files"] else (".argos-config-journal.json",))]
    if [(guard["root"], guard["file"]) for guard in plan["guards"]] != expected_guards:
        raise ResetError("Protected recovery guard list changed")
    if getattr(backend, "token_path", None) in [Path(root) / name for _, root, name in expected_entries]:
        raise ResetError("Independent administrative credential cannot be a reset payload")
    if set(targets) != {target["name"] for target in selected["targets"]}:
        raise ResetError("Apply must explicitly name every frozen target")
    if plan["blockers"]:
        raise Pending("Inventory has unresolved payload/store blockers")
    journal_name = f'reset-{selected["reset_id"]}.json'
    with directory(selected["ledger"], private=True) as (ledger, _), contextlib.ExitStack() as locks:
        guard = os.open(".reset.guard", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600, dir_fd=ledger)
        locks.callback(os.close, guard)
        fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
        snapshot, raw = read_at(ledger, journal_name, private=True)
        journal = json.loads(raw) if snapshot["exists"] else {"version": 1, "reset_id": selected["reset_id"], "plan_sha256": digest, "state": "planned", "done": [], "intent": None}
        if journal.get("plan_sha256") != digest:
            raise ResetError("Reset ID is already bound to another inventory")
        if journal["state"] == "complete":
            return {"state": "complete", "repeated": True, "reset_id": selected["reset_id"]}
        if not snapshot["exists"]:
            write_json(ledger, journal_name, journal, new=True)
        local = [{"target_name": target["name"], "inventory_sha256": digest, "state": "files_reset"} for target in selected["targets"]]
        if journal["state"] == "local_committed":
            finished = backend_receipt(backend, "finalize", plan, local)
            if any(finished[key] != journal["backend"][key] for key in ("configuration_epoch", "access_epoch")) or finished.get("legacy_reload_disabled") is not True or finished.get("backend_configuration_cleared") is not True or finished.get("state") != "complete":
                raise Pending("Final backend receipt does not match this reset")
            journal["state"] = "complete"
            write_json(ledger, journal_name, journal)
            return {"state": "complete", "repeated": False, "reset_id": selected["reset_id"]}
        # No local payload changes occur before actual administrative fencing.
        receipt = backend_receipt(backend, "prepare", plan)
        if journal.get("backend") and any(journal["backend"][key] != receipt[key] for key in ("configuration_epoch", "access_epoch")):
            raise Pending("Backend changed an already recorded reset epoch")
        journal["backend"] = {key: receipt[key] for key in ("deployment_id", "scope_id", "configuration_epoch", "access_epoch")}
        journal["state"] = "fenced"
        write_json(ledger, journal_name, journal)
        for target in selected["targets"]:
            root, chain = locks.enter_context(directory(target["root"]))
            expected = next(entry["root_chain"] for entry in plan["entries"] if entry["target"] == target["name"])
            if chain != expected:
                raise ResetError("Frozen root identity changed")
            if any(name != CONNECTIONS for name in target["files"]):
                guard = os.open(".argos-config.guard", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600, dir_fd=root)
                locks.callback(os.close, guard)
                fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if CONNECTIONS in target["files"]:
                locks.enter_context(native_lock(root))
                native_authority(root)
        for protected in plan["guards"]:
            if read_guard(protected["root"], protected["file"]) != protected["before"]:
                raise ResetError("Frozen protected recovery or native authority changed")
        for index, entry in enumerate(plan["entries"]):
            if index not in journal["done"]:
                verify_entry(entry, intent=journal["intent"] == index)
        for index, entry in enumerate(plan["entries"]):
            if index in journal["done"]:
                continue  # Never erase a newly entered value in a completed step.
            journal["intent"] = index
            write_json(ledger, journal_name, journal)
            fresh, expected = verify_entry(entry, intent=True)
            if entry["file"] == CONNECTIONS:
                write_native_entry({**entry, "native_guards": [guard for guard in plan["guards"]
                                   if guard["root"] == entry["root"] and guard["file"] in NATIVE_GUARDS]}, fresh, expected)
                journal["done"].append(index)
                journal["intent"] = None
                journal["state"] = "files_reset"
                write_json(ledger, journal_name, journal)
                if after_step:
                    after_step(index)
                continue
            with parent(entry["root"], entry["file"]) as (fd, name, chain):
                if chain != entry["parent_chain"]:
                    raise ResetError("Frozen parent changed before mutation")
                flags = os.O_RDWR | os.O_NOFOLLOW | (0 if expected["exists"] else os.O_CREAT | os.O_EXCL)
                opened = os.open(name, flags, 0o600, dir_fd=fd)
                with os.fdopen(opened, "r+b") as file:
                    current = os.fstat(file.fileno())
                    if not stat.S_ISREG(current.st_mode) or current.st_nlink != 1 or current.st_uid != os.getuid():
                        raise ResetError("Payload object became unsafe")
                    if expected["exists"] and (any(identity(current)[key] != expected[key] for key in ("dev", "ino", "uid"))
                        or current.st_size != expected["size"] or current.st_mtime_ns != expected["mtime_ns"] or current.st_ctime_ns != expected["ctime_ns"] or sha(file.read(LIMIT + 1)) != expected["sha256"]):
                        raise ResetError("Frozen payload was substituted")
                    # Keep the owned inode: pathname replacement cannot make us
                    # truncate a subsequently substituted file. Never unlink a
                    # target, recurse, create a secret backup or use Trash.
                    os.fchmod(file.fileno(), 0o600)
                    file.seek(0)
                    file.truncate(0)
                    file.write(fresh)
                    file.flush()
                    os.fsync(file.fileno())
                os.fsync(fd)
                after, _ = read_at(fd, name)
                if after.get("ino") != current.st_ino or after.get("dev") != current.st_dev or after.get("sha256") != sha(fresh):
                    raise ResetError("Payload changed during reset; keep the scope fenced")
            journal["done"].append(index)
            journal["intent"] = None
            journal["state"] = "files_reset"
            write_json(ledger, journal_name, journal)
            if after_step:
                after_step(index)
        completed = backend_receipt(backend, "complete", plan, local)
        if any(completed[key] != journal["backend"][key] for key in ("configuration_epoch", "access_epoch")) or completed.get("backend_configuration_cleared") is not True or completed.get("legacy_reload_disabled") is not True:
            raise Pending("Files reset; backend cleanup or legacy-reader retirement remains pending")
        journal["state"] = "local_committed"
        journal["local_receipts"] = local
        write_json(ledger, journal_name, journal)
        finalized = backend_receipt(backend, "finalize", plan, local)
        if any(finalized[key] != journal["backend"][key] for key in ("configuration_epoch", "access_epoch")) or finalized.get("backend_configuration_cleared") is not True or finalized.get("legacy_reload_disabled") is not True or finalized.get("state") != "complete":
            raise Pending("Final backend receipt does not match this reset")
        journal["state"] = "complete"
        write_json(ledger, journal_name, journal)
        return {"state": "complete", "repeated": False, "reset_id": selected["reset_id"]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    inventory_parser = commands.add_parser("inventory", help="Read only explicitly selected app payloads; never apply")
    inventory_parser.add_argument("--manifest", required=True)
    inventory_parser.add_argument("--out", required=True, help="New private recorded plan, absolute path")
    apply_parser = commands.add_parser("apply", help="Apply one frozen plan under independent backend reset authority")
    apply_parser.add_argument("--plan", required=True)
    apply_parser.add_argument("--target", action="append", required=True)
    apply_parser.add_argument("--admin-token-file", required=True)
    arguments = parser.parse_args(argv)
    try:
        if arguments.command == "inventory":
            plan = inventory(json.loads(read_private(arguments.manifest)))
            output = absolute(arguments.out)
            with directory(output.parent, private=True) as (fd, _):
                write_json(fd, output.name, plan, new=True)
            result = {"state": "inventory", "plan_sha256": plan["plan_sha256"], "targets": len(plan["manifest"]["targets"]), "files": len(plan["entries"]), "blockers": plan["blockers"], "applied": False}
        else:
            plan = json.loads(read_private(arguments.plan))
            backend = Backend(plan["manifest"]["backend"]["origin"], arguments.admin_token_file)
            result = apply(plan, backend, arguments.target)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (ResetError, OSError, ValueError, KeyError, TypeError) as error:
        message = str(error) if isinstance(error, ResetError) else "Reset input or filesystem verification failed"
        print(json.dumps({"state": "pending" if isinstance(error, Pending) else "blocked", "error": message}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
