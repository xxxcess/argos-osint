#!/usr/bin/env python3
"""Publish an immutable verified inventory, then move channel image aliases."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from fleetworks_qa_pack import MAX_COMPRESSED, MAX_MANIFEST, MODEL_REVISION
from inbox_qa_smoke_receipt import require_embedded_smoke

IDENTITY_FIELDS = ("head", "version", "tag", "channel", "prerelease", "native", "backend", "web", "bert", "image_tag", "image_targets")
INBOX_QA_TARGETS = ("x86_64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin")


def run(*command, capture=False):
    result = subprocess.run(command, check=True, stdout=subprocess.PIPE if capture else None)
    return result.stdout if capture else None


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def lookup_release(tag):
    result = subprocess.run(["gh", "release", "view", tag, "--json", "isDraft,assets,targetCommitish"],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode == 0:
        return json.loads(result.stdout)
    if b"not found" in result.stderr.lower():
        return None
    raise ValueError("Cannot establish existing release identity")


def validate_images(plan, directory):
    selected = set(plan["image_targets"])
    paths = list(directory.rglob("release-images-*.json"))
    archives = list(directory.rglob("fleetworks-images-linux-*.tar.gz"))
    checksums = list(directory.rglob("fleetworks-images-linux-*.tar.gz.sha256"))
    manifests = [json.loads(path.read_text()) for path in paths]
    if (selected and (len(manifests) != 2 or {item["arch"] for item in manifests} != {"amd64", "arm64"})) or (not selected and (manifests or archives or checksums)):
        raise ValueError("Both native Linux manifests required, without duplicates")
    if selected and (len(archives) != 2 or len(checksums) != 2):
        raise ValueError("Both native Linux image archives and checksums required")
    by_arch = {item["arch"]: item for item in manifests}
    asset_paths = list(paths)
    for path, item in zip(paths, manifests):
        arch = item["arch"]
        if (item["revision"] != plan["head"] or item["version"] != plan["version"]
                or item["image_tag"] != plan["image_tag"] or set(item["images"]) != selected):
            raise ValueError("Image provenance differs from release plan")
        archive_meta = item.get("archive")
        expected_name = f"fleetworks-images-linux-{arch}.tar.gz"
        if (not isinstance(archive_meta, dict)
                or archive_meta.get("name") != expected_name
                or not re.fullmatch(r"[0-9a-f]{64}", str(archive_meta.get("sha256", "")))
                or not isinstance(archive_meta.get("bytes"), int)
                or archive_meta["bytes"] <= 0):
            raise ValueError("Image archive metadata missing or invalid")
        archive_path = path.parent / expected_name
        checksum_path = path.parent / f"{expected_name}.sha256"
        if not archive_path.is_file() or not checksum_path.is_file():
            raise ValueError("Exported Linux image archive or checksum missing")
        if (archive_path.stat().st_size != archive_meta["bytes"]
                or sha256(archive_path) != archive_meta["sha256"]
                or checksum_path.read_text().split()[0] != archive_meta["sha256"]):
            raise ValueError("Exported Linux image archive checksum mismatch")
        asset_paths.extend((archive_path, checksum_path))
    for role in sorted(selected):
        pair = [by_arch[arch]["images"][role] for arch in ("amd64", "arm64")]
        repository = pair[0]["repository"]
        if pair[1]["repository"] != repository or not repository.endswith(f"/fleetworks-{role}") or not re.fullmatch(r"ghcr\.io/[a-z0-9_.-]+/fleetworks-(runtime|surrealdb|search-index|web)", repository):
            raise ValueError("Unexpected image repository")
        if any(not re.fullmatch(r"sha256:[0-9a-f]{64}", item["digest"]) for item in pair):
            raise ValueError("Invalid verified digest")
    return asset_paths, by_arch


def prepare_bert(plan, directory, assets):
    evidence_paths = list(directory.rglob("evidence.json"))
    bundle_paths = list(directory.rglob("atlas-bert-linux-*.tar.gz"))
    if not plan.get("bert", False):
        if evidence_paths or bundle_paths:
            raise ValueError("Unexpected Atlas BERT artifacts for unselected scope")
        return
    if len(evidence_paths) != 2 or len(bundle_paths) != 2:
        raise ValueError("Both native Linux Atlas BERT bundles and evidence required")
    evidences = [json.loads(path.read_text()) for path in evidence_paths]
    if {item.get("arch") for item in evidences} != {"amd64", "arm64"}:
        raise ValueError("Both amd64 and arm64 Atlas BERT architectures required")
    bundles_by_name = {path.name: path for path in bundle_paths}
    if len(bundles_by_name) != 2:
        raise ValueError("Duplicate Atlas BERT bundle names")
    for evidence_path, evidence in zip(evidence_paths, evidences):
        arch = evidence["arch"]
        expected_name = f"atlas-bert-linux-{arch}.tar.gz"
        archive_meta = evidence.get("archive")
        if (evidence.get("source_sha") != plan["head"]
                or evidence.get("status") != "passed"
                or evidence.get("dirty") is not False
                or evidence.get("unit_tests_passed", 0) < 1
                or evidence.get("build_inference", {}).get("status") != "passed"
                or evidence.get("extracted_inference", {}).get("status") != "passed"
                or not isinstance(archive_meta, dict)
                or archive_meta.get("file") != expected_name
                or not re.fullmatch(r"[0-9a-f]{64}", str(archive_meta.get("sha256", "")))
                or not isinstance(archive_meta.get("bytes"), int)
                or archive_meta["bytes"] <= 0):
            raise ValueError("Atlas BERT evidence provenance or runtime checks invalid")
        bundle_path = bundles_by_name.get(expected_name)
        if (bundle_path is None
                or bundle_path.stat().st_size != archive_meta["bytes"]
                or sha256(bundle_path) != archive_meta["sha256"]):
            raise ValueError("Downloaded Atlas BERT bundle checksum mismatch")
        shutil.copy2(bundle_path, assets / expected_name)
        (assets / f"{expected_name}.sha256").write_text(
            f"{archive_meta['sha256']}  {expected_name}\n", encoding="utf-8"
        )
        shutil.copy2(evidence_path, assets / f"atlas-bert-evidence-{arch}.json")


def inbox_qa_payload(plan, directory, native_digests):
    archives = list(directory.rglob("Fleetworks-Inbox-QA-Pack-*.tar.gz"))
    receipts = list(directory.rglob("Fleetworks-Inbox-QA-Pack-*.tar.gz.digests.json"))
    if not plan.get("inbox_qa", False):
        if archives or receipts:
            raise ValueError("Unexpected Inbox QA assets for an unselected release")
        return []

    expected = {f"Fleetworks-Inbox-QA-Pack-{target}.tar.gz": target for target in INBOX_QA_TARGETS}
    if (len(archives) != len(expected) or {archive.name for archive in archives} != expected.keys()
            or len(receipts) != len(expected)
            or {receipt.name for receipt in receipts} != {name + ".digests.json" for name in expected}):
        raise ValueError("Exactly three smoke-verified Inbox QA pack pairs are required")
    for archive in archives:
        receipt = archive.with_name(archive.name + ".digests.json")
        if (receipt not in receipts or archive.is_symlink() or receipt.is_symlink()
                or not archive.is_file() or not receipt.is_file()
                or not 0 < archive.stat().st_size <= MAX_COMPRESSED
                or not 0 < receipt.stat().st_size <= MAX_MANIFEST):
            raise ValueError("Inbox QA pack pair is missing or exceeds its bounds")
        target = expected[archive.name]
        evidence = json.loads(receipt.read_text())
        if (not isinstance(evidence, dict) or evidence.get("archive") != archive.name
                or evidence.get("target") != target or evidence.get("source_sha") != plan["head"]
                or evidence.get("version") != plan["version"] or evidence.get("model_revision") != MODEL_REVISION
                or evidence.get("status") != "passed"
                or any(not isinstance(evidence.get(key), str) or not re.fullmatch(r"[0-9a-f]{64}", evidence[key])
                       for key in ("archive_sha256", "manifest_sha256", "native_archive_sha256"))
                or sha256(archive) != evidence["archive_sha256"]):
            raise ValueError("Inbox QA pack digest or packaged smoke provenance differs from the release plan")
        require_embedded_smoke(evidence, target=target, head=plan["head"])
        if target.endswith("-apple-darwin") and evidence["native_archive_sha256"] != native_digests.get(target):
            raise ValueError("Inbox QA smoke used a different macOS archive from the release payload")
    return [*archives, *receipts]


def prepare_current(plan, directory, assets, image_paths):
    native = []
    native_digests = {}
    for manifest_path in directory.rglob("Fleetworks.app.manifest.json"):
        manifest = json.loads(manifest_path.read_text())
        if (manifest["revision"] != plan["head"] or manifest["version"] != plan["version"]
                or manifest["dirty"] is not False or manifest["profile"] != "release"):
            raise ValueError("Native artifact provenance differs from release plan")
        native.append(manifest["target"])
        archives = list(manifest_path.parent.glob("Fleetworks-*.zip"))
        if len(archives) != 1:
            raise ValueError("One app ZIP per architecture required")
        archive = archives[0]
        checksum = archive.with_name(archive.name + ".sha256")
        digest = sha256(archive)
        if checksum.read_text().split()[0] != digest:
            raise ValueError("Downloaded app ZIP checksum mismatch")
        native_digests[manifest["target"]] = digest
        shutil.copy2(archive, assets / archive.name)
        shutil.copy2(checksum, assets / checksum.name)
        shutil.copy2(manifest_path, assets / f"{archive.stem}.manifest.json")
    if (plan["native"] and sorted(native) != ["aarch64-apple-darwin", "x86_64-apple-darwin"]) or (not plan["native"] and native):
        raise ValueError("Native artifact selection incomplete or unexpected")
    for path in [*image_paths, *inbox_qa_payload(plan, directory, native_digests)]:
        shutil.copy2(path, assets / path.name)
    prepare_bert(plan, directory, assets)
    return {"format": 2, "plan": plan,
            "assets": {path.name: sha256(path) for path in sorted(assets.iterdir())}}


def load_existing(plan, info, assets):
    """Validate the original inventory and every present byte before promotion."""
    if info["isDraft"]:
        if info["targetCommitish"] != plan["head"]:
            raise ValueError("Existing draft targets another source")
    else:
        resolved = run("gh", "api", f"repos/{{owner}}/{{repo}}/commits/{plan['tag']}", "--jq", ".sha", capture=True).decode().strip()
        if resolved != plan["head"]:
            raise ValueError("Existing release tag points to another source")
    names = [item["name"] for item in info["assets"]]
    if len(set(names)) != len(names):
        raise ValueError("Duplicate existing release assets")
    if not names and info["isDraft"]:
        return None, set()
    if "release.json" not in names:
        raise ValueError("Existing release has no immutable completion inventory")
    run("gh", "release", "download", plan["tag"], "--pattern", "release.json", "--dir", str(assets))
    inventory = json.loads((assets / "release.json").read_text())
    if (inventory.get("format") != 2
            or any(inventory["plan"].get(key) != plan.get(key) for key in IDENTITY_FIELDS)
            or inventory["plan"].get("inbox_qa", False) is not plan.get("inbox_qa", False)):
        raise ValueError("Existing release inventory identifies another source or selection")
    hashes = inventory["assets"]
    if (not isinstance(hashes, dict) or len(hashes) > 32
            or any(not re.fullmatch(r"[A-Za-z0-9_.+-]+", name) or not re.fullmatch(r"[0-9a-f]{64}", digest) for name, digest in hashes.items())
            or set(names) - {"release.json"} - set(hashes)):
        raise ValueError("Invalid existing asset inventory")
    for name in names:
        if name == "release.json":
            continue
        run("gh", "release", "download", plan["tag"], "--pattern", name, "--dir", str(assets))
        if sha256(assets / name) != hashes[name]:
            raise ValueError("Existing immutable release asset differs")
    missing = set(hashes) - set(names)
    if missing and not info["isDraft"]:
        raise ValueError("Published release is missing immutable assets")
    return inventory, missing



def validate_payload_inventory(plan, assets, hashes):
    known = (
        {
            name
            for arch in ("amd64", "arm64")
            for name in (
                f"release-images-{arch}.json",
                f"fleetworks-images-linux-{arch}.tar.gz",
                f"fleetworks-images-linux-{arch}.tar.gz.sha256",
            )
        }
        if plan["image_targets"]
        else set()
    )
    if plan.get("bert", False):
        for arch in ("amd64", "arm64"):
            evidence_path = assets / f"atlas-bert-evidence-{arch}.json"
            archive_path = assets / f"atlas-bert-linux-{arch}.tar.gz"
            checksum_path = assets / f"atlas-bert-linux-{arch}.tar.gz.sha256"
            if not evidence_path.is_file() or not archive_path.is_file() or not checksum_path.is_file():
                raise ValueError("Original Atlas BERT inventory bytes are incomplete")
            evidence = json.loads(evidence_path.read_text())
            archive_meta = evidence.get("archive", {})
            if (evidence.get("arch") != arch
                    or evidence.get("source_sha") != plan["head"]
                    or evidence.get("status") != "passed"
                    or evidence.get("dirty") is not False
                    or archive_meta.get("file") != archive_path.name
                    or archive_path.stat().st_size != archive_meta.get("bytes")
                    or sha256(archive_path) != archive_meta.get("sha256")
                    or checksum_path.read_text().split()[0] != archive_meta.get("sha256")):
                raise ValueError("Original Atlas BERT inventory provenance differs")
            known.update((evidence_path.name, archive_path.name, checksum_path.name))
    native = []
    native_digests = {}
    for path in assets.glob("Fleetworks-*.manifest.json"):
        manifest = json.loads(path.read_text())
        if (manifest.get("revision") != plan["head"] or manifest.get("version") != plan["version"]
                or manifest.get("dirty") is not False or manifest.get("profile") != "release"):
            raise ValueError("Original native inventory provenance differs")
        native.append(manifest["target"])
        archive = path.with_name(path.name.removesuffix(".manifest.json") + ".zip")
        checksum = archive.with_name(archive.name + ".sha256")
        if not archive.is_file() or not checksum.is_file():
            raise ValueError("Original native inventory bytes are incomplete")
        digest = sha256(archive)
        if checksum.read_text().split()[0] != digest:
            raise ValueError("Original native archive checksum differs")
        native_digests[manifest["target"]] = digest
        known.update((path.name, archive.name, checksum.name))
    if (plan["native"] and sorted(native) != ["aarch64-apple-darwin", "x86_64-apple-darwin"]) or (not plan["native"] and native):
        raise ValueError("Original native architecture inventory differs")
    known.update(path.name for path in inbox_qa_payload(plan, assets, native_digests))
    if set(hashes) != known or any(sha256(assets / name) != digest for name, digest in hashes.items()):
        raise ValueError("Original payload inventory differs")


def verify_indexes(plan, by_arch):
    promoted = []
    for role in sorted(plan["image_targets"]):
        repository = by_arch["amd64"]["images"][role]["repository"]
        references = [f"{repository}@{by_arch[arch]['images'][role]['digest']}" for arch in ("amd64", "arm64")]
        version_tag = f"{repository}:{plan['image_tag']}"
        run("docker", "buildx", "imagetools", "create", "--tag", version_tag, *references)
        remote = json.loads(run("docker", "buildx", "imagetools", "inspect", "--raw", version_tag, capture=True))
        descriptors = remote.get("manifests", [])
        actual = {(item.get("platform", {}).get("os"), item.get("platform", {}).get("architecture"), item.get("digest")) for item in descriptors}
        expected = {("linux", arch, by_arch[arch]["images"][role]["digest"]) for arch in ("amd64", "arm64")}
        if actual != expected or len(descriptors) != 2:
            raise ValueError("Published multiarchitecture digest inventory mismatch")
        promoted.append((repository, version_tag))
    return promoted



def lookup_channel(channel):
    result = subprocess.run(["gh", "api", f"repos/{{owner}}/{{repo}}/git/ref/tags/{channel}"],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode == 0:
        return json.loads(result.stdout)
    if b"HTTP 404" in result.stderr:
        return None
    raise ValueError("Cannot establish current channel ref")


def advance_channel(plan, *, update=True):
    """Fast-forward only; an old rerun cannot roll back Git or image aliases."""
    channel = plan["channel"]
    current = lookup_channel(channel)
    if current is None:
        if update:
            run("gh", "api", "repos/{owner}/{repo}/git/refs", "--method", "POST",
                "--raw-field", f"ref=refs/tags/{channel}", "--raw-field", f"sha={plan['head']}")
        return True
    target = current.get("object", {})
    if target.get("type") != "commit" or not re.fullmatch(r"[0-9a-f]{40}", target.get("sha", "")):
        raise ValueError("Channel tag must be a lightweight commit ref")
    if target["sha"] == plan["head"]:
        return True
    comparison = json.loads(run("gh", "api", f"repos/{{owner}}/{{repo}}/compare/{target['sha']}...{plan['head']}", capture=True))
    if comparison.get("status") == "behind":
        print(f"Channel {channel} already points to a newer source; retained Git and image aliases.")
        return False
    if comparison.get("status") not in {"ahead", "identical"}:
        raise ValueError("Channel source diverged; refusing to replace an incomparable source")
    if update:
        run("gh", "api", f"repos/{{owner}}/{{repo}}/git/refs/tags/{channel}", "--method", "PATCH",
            "--raw-field", f"sha={plan['head']}", "--field", "force=false")
    return True


def publish(plan, directory):
    if not plan["publish"] or plan["channel"] not in {"stable", "nightly"} or not re.fullmatch(r"[0-9a-f]{40}", plan["head"]):
        raise ValueError("Trusted exact-source publish plan required")
    ship_qa = plan.get("inbox_qa", False)
    if type(ship_qa) is not bool or (ship_qa and (
        plan["channel"] != "nightly" or plan.get("native") is not True
        or plan.get("event") != "push" or plan.get("ref") != "refs/heads/nightly"
    )):
        raise ValueError("Inbox QA assets require a trusted nightly native release plan")
    image_paths, by_arch = validate_images(plan, directory)
    info = lookup_release(plan["tag"])
    assets = directory / "assets"
    assets.mkdir()
    inventory, missing = load_existing(plan, info, assets) if info else (None, set())
    if inventory is None:
        inventory = prepare_current(plan, directory, assets, image_paths)
        missing = set(inventory["assets"])
        (assets / "release.json").write_text(json.dumps(inventory, sort_keys=True) + "\n")
    elif missing:
        # A partial draft may repair only identical originally planned assets.
        # A new attempt's differently named ZIP pair cannot replace that plan.
        with tempfile.TemporaryDirectory(prefix="release-repair-") as temporary:
            current = Path(temporary)
            prepare_current(plan, directory, current, image_paths)
            for name in missing:
                source = current / name
                if not source.is_file() or sha256(source) != inventory["assets"][name]:
                    raise ValueError("Missing original immutable bytes; cannot substitute a new attempt")
                shutil.copy2(source, assets / name)
    # Completed reruns reuse the original complete inventory, ignoring the new
    # attempt's app ZIP names and bytes. No second architecture pair is appended.
    validate_payload_inventory(plan, assets, inventory["assets"])
    # Existing releases retain their original digest manifests as well as apps.
    _paths, by_arch = validate_images(plan, assets)
    promoted = verify_indexes(plan, by_arch)
    if info is None:
        notes = directory / "notes.md"
        notes.write_text(f"Channel: {plan['channel']}\n\nSource: `{plan['head']}`\n\nVersion: `{plan['version']}`\n\nMac app archives are unsigned; signing and notarization are not established by these checks.\n")
        run("gh", "release", "create", plan["tag"], "--draft", "--target", plan["head"],
            "--title", f"Fleetworks {plan['version']}", "--notes-file", str(notes),
            *(["--prerelease"] if plan["prerelease"] else []))
    if info is None or (info["isDraft"] and not info["assets"]):
        # Publish the immutable plan first so interrupted uploads cannot invent
        # another native architecture pair on a later attempt.
        run("gh", "release", "upload", plan["tag"], str(assets / "release.json"))
    if missing:
        run("gh", "release", "upload", plan["tag"], *[str(assets / name) for name in sorted(missing)])
    if info is None or info["isDraft"]:
        # A repaired older draft must not replace GitHub's newer latest release.
        latest = not plan["prerelease"] and advance_channel(plan, update=False)
        run("gh", "release", "edit", plan["tag"], "--draft=false",
            *(["--latest"] if latest else ["--latest=false"]))
    # Every selected source/artifact and complete GitHub asset publication must
    # succeed before any mutable channel alias can move.
    if advance_channel(plan):
        for repository, version_tag in promoted:
            run("docker", "buildx", "imagetools", "create", "--tag", f"{repository}:{plan['channel']}", version_tag)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    publish(json.loads(args.plan.read_text()), args.directory)
