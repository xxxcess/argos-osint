#!/usr/bin/env python3
"""Check relative inline Markdown and HTML references in maintained documentation."""

import collections
import functools
import html
import os
import re
import subprocess
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
DOC_EXTENSIONS = {".md", ".html"}


def prose(path):
    source = path.read_text(encoding="utf-8")
    return re.sub(r"^\s*(```|~~~).*?^\s*\1\s*$", "", source, flags=re.M | re.S)


@functools.cache
def anchors(path):
    source = prose(path)
    result = set(re.findall(r"\b(?:id|name)=[\"']([^\"']+)[\"']", source))
    seen = collections.Counter()
    for heading in re.findall(r"^#{1,6}\s+(.+?)\s*#*$", source, re.M):
        heading = html.unescape(re.sub(r"<[^>]+>", "", heading)).strip().lower()
        slug = re.sub(r"[^\w\-\s]", "", heading).replace(" ", "-")
        result.add(f"{slug}-{seen[slug]}" if seen[slug] else slug)
        seen[slug] += 1
    return result


def links(path):
    source = prose(path)
    for angle, bare in re.findall(r"\]\((?:<([^>]+)>|([^()\s]+))\)", source):
        yield html.unescape(angle or bare)
    for link in re.findall(r'\b(?:href|src)="([^"]+)"', source):
        yield html.unescape(link)


def main():
    tracked = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT,
    )
    paths = {Path(os.fsdecode(path)) for path in tracked.split(b"\0") if path}
    count = broken = 0
    for path in sorted(paths):
        if path.suffix not in DOC_EXTENSIONS or "upstream" in path.parts or not (ROOT / path).is_file():
            continue
        for link in links(ROOT / path):
            parts = urlsplit(link)
            if parts.scheme or parts.netloc or link.startswith("/"):
                continue
            count += 1
            target = ROOT / path.parent / unquote(parts.path) if parts.path else ROOT / path
            if not target.exists():
                print(f"{path}: missing target {link}")
                broken += 1
            elif parts.fragment and target.suffix in DOC_EXTENSIONS and unquote(parts.fragment) not in anchors(target):
                print(f"{path}: missing anchor {link}")
                broken += 1
    print(f"Checked {count} relative documentation references, {broken} broken")
    return int(bool(broken))


if __name__ == "__main__":
    raise SystemExit(main())
