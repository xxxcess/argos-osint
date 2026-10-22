#!/usr/bin/env python3
"""Assert which SurrealDB pieces each Argos build links.

- Default build (`argos-osint-bin`, default features): no surrealdb crate and
  no embedded kv engine at all.
- `--features surreal` (remote client): surrealdb, but no embedded engine.
- `--features embedded`: the RocksDB engine is present (sanity check).
"""
import subprocess
import sys

sys.dont_write_bytecode = True
ENGINES = ("rocksdb", "surrealkv", "tikv", "surrealdb-kvs", "surrealdb-core", "indxdb", "surrealmx", "echodb")


def tree(*features):
    command = ["cargo", "tree", "--locked", "-p", "argos-osint-bin", "-e", "normal", "--prefix", "none", *features]
    out = subprocess.run(command, capture_output=True, text=True, check=True).stdout
    return {line.split(" ")[0] for line in out.splitlines() if line.strip()}


def engines(crates):
    return sorted(c for c in crates if any(e in c for e in ENGINES))


def main():
    failures = []
    default = tree()
    if any(c.startswith("surrealdb") for c in default) or engines(default):
        failures.append(f"default build links SurrealDB: {sorted(c for c in default if 'surreal' in c)}")
    remote = tree("--features", "surreal")
    if "surrealdb" not in remote:
        failures.append("--features surreal does not link surrealdb")
    if engines(remote):
        failures.append(f"--features surreal links embedded engines: {engines(remote)}")
    embedded = tree("--features", "embedded")
    if not any("rocksdb" in c for c in embedded):
        failures.append("--features embedded does not link RocksDB")
    print(f"default: {len(default)} crates, surrealdb crates 0, embedded engines {engines(default)}")
    print(f"surreal: surrealdb crates {sorted(c for c in remote if c.startswith('surrealdb'))}, embedded engines {engines(remote)}")
    print(f"embedded: engines {engines(embedded)}")
    if failures:
        sys.exit("check-storage-deps: " + "; ".join(failures))
    print("check-storage-deps: ok")


if __name__ == "__main__":
    main()
