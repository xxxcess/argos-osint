#!/usr/bin/env python3
"""Compiler scan for the SQLite/LanceDB removal (PR #53).

Drops `rusqlite` and `lancedb` from the manifests, runs `cargo check`, and
prints the remaining errors grouped by module as Markdown. The manifests and
Cargo.lock are restored afterwards. Informational: always exits 0 unless the
scan itself breaks. Delete once the removal commit lands (the dependency
guard in ci.yml replaces it).
"""
import collections, json, os, pathlib, re, subprocess, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
FILES = ["Cargo.toml", "Cargo.lock", "crates/argos-osint-core/Cargo.toml", "crates/argos-osint-bin/Cargo.toml"]
DROP = re.compile(r'^\s*(rusqlite|lancedb)(\.workspace)?\s*=.*$|^\s*lancedb\s*=\s*\[.*\]\s*$', re.M)

saved = {f: (ROOT / f).read_text() for f in FILES if (ROOT / f).exists()}
try:
    for f in FILES[:1] + FILES[2:]:
        p = ROOT / f
        if p.exists():
            p.write_text(DROP.sub("", saved[f]))
    target = os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target" / "sqlite-scan"))
    proc = subprocess.run(
        ["cargo", "check", "--workspace", "--all-targets", "--message-format=json"],
        cwd=ROOT, capture_output=True, text=True, env={**os.environ, "CARGO_TARGET_DIR": target},
    )
finally:
    for f, text in saved.items():
        (ROOT / f).write_text(text)

by_file = collections.Counter()
for line in proc.stdout.splitlines():
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    m = msg.get("message") or {}
    if msg.get("reason") != "compiler-message" or m.get("level") != "error":
        continue
    spans = [s for s in m.get("spans", []) if s.get("is_primary")]
    if spans:
        by_file[spans[0]["file_name"]] += 1

def module(path):
    parts = pathlib.PurePath(path).parts
    if "src" in parts:
        i = parts.index("src")
        rest = parts[i + 1:]
        crate = parts[i - 1].replace("argos-osint-", "")
        return f"{crate}/{rest[0] if len(rest) > 1 else 'src (top level)'}"
    return path

by_mod = collections.defaultdict(list)
for f, n in by_file.items():
    by_mod[module(f)].append((n, f))

total = sum(by_file.values())
out = [f"## SQLite removal scan: {total} errors in {len(by_file)} files", ""]
if total == 0 and proc.returncode == 0:
    out.append("Clean: everything compiles without `rusqlite` and `lancedb`.")
elif total == 0:
    out += ["Scan failed before rustc reported errors:", "```", proc.stderr[-3000:], "```"]
else:
    out += ["Counts are a floor: rustc stops at unresolved imports, and crates that depend on a failing crate are not checked.", "",
            "| Module | Errors | Files | Top files |", "|---|---:|---:|---|"]
    for mod, items in sorted(by_mod.items(), key=lambda kv: -sum(n for n, _ in kv[1])):
        items.sort(reverse=True)
        top = ", ".join(f"`{pathlib.PurePath(f).name}` {n}" for n, f in items[:4])
        out.append(f"| `{mod}` | {sum(n for n, _ in items)} | {len(items)} | {top} |")
report = "\n".join(out) + "\n"
print(report)
summary = os.environ.get("GITHUB_STEP_SUMMARY")
if summary:
    with open(summary, "a") as fh:
        fh.write(report)
