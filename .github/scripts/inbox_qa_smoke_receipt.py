#!/usr/bin/env python3
"""Validate the packaged Embedded Inbox QA transport and turn evidence."""

import argparse
import json
from pathlib import Path

EMBEDDED_TRANSPORT = "embedded-unix-socket"
SOCKET_EVIDENCE = {
    "transport": EMBEDDED_TRANSPORT,
    "socket_family": "unix",
    "socket_type": "stream",
    "protocol_version": 2,
    "min_protocol_version": 2,
    "dataset_upload_chunk_bytes": 262144,
    "dataset_upload_window": 4,
}
MAX_EVIDENCE_BYTES = 64 * 1024


def require_embedded_smoke(evidence, *, target, head, graphical=False):
    """Require the exact packaged handshake and two distinct terminal results."""
    if (not isinstance(evidence, dict) or evidence.get("target") != target
            or evidence.get("source_sha") != head or evidence.get("runtime") != EMBEDDED_TRANSPORT):
        raise ValueError("Inbox QA socket evidence identifies a different package or transport")
    attestation = evidence.get("transport_attestation")
    if (not isinstance(attestation, dict) or any(
            type(attestation.get(field)) is not type(expected) or attestation[field] != expected
            for field, expected in SOCKET_EVIDENCE.items()
    )):
        raise ValueError("Inbox QA socket evidence is missing the negotiated Unix stream handshake")
    if evidence.get("terminal_state") != "answered" or evidence.get("cancelled_terminal_state") != "canceled":
        raise ValueError("Inbox QA smoke requires both an answer and a canceled turn")
    counts = ("terminal_events", "cancelled_terminal_events")
    if not graphical:
        counts += ("model_pid_count",)
    if any(type(evidence.get(field)) is not int or evidence[field] != 1 for field in counts):
        raise ValueError("Inbox QA smoke did not observe exactly one worker or terminal result")
    if graphical and (target != "x86_64-unknown-linux-gnu" or evidence.get("graphical") != "x11"
                      or evidence.get("first_citation_while_running") is not True
                      or evidence.get("stop_while_running") is not True):
        raise ValueError("Inbox QA window did not show the first citation and allow Stop during inference")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--graphical", action="store_true")
    args = parser.parse_args()
    with args.evidence.open("rb") as evidence_file:
        evidence = evidence_file.read(MAX_EVIDENCE_BYTES + 1)
    if not 0 < len(evidence) <= MAX_EVIDENCE_BYTES:
        raise ValueError("Inbox QA smoke evidence exceeds its permitted size")
    require_embedded_smoke(json.loads(evidence), target=args.target, head=args.head, graphical=args.graphical)


if __name__ == "__main__":
    main()
