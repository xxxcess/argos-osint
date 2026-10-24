#!/usr/bin/env bash
# Local build/test matrix, one leg per storage mode Argos runs with.
#
#   scripts/local_matrix.sh --storage embedded|server [--surreal-bin PATH]
#
# embedded (the default build): default-features clippy, the whole
#   workspace test suite (unit + storage contract tests on the embedded
#   engine), then `test_surreal.py --mode embedded`.
# server (opt-in): `--no-default-features --features server,nlp` clippy for
#   core and bin, then the storage contract tests against a local SurrealDB
#   3.3.4 server (`test_surreal.py --mode server`, using --surreal-bin or
#   `surreal` on PATH). Non-storage tests run only in the embedded leg.
#
# Any failure exits nonzero. Builds go through scripts/agent_cargo.py.
set -euo pipefail
cd "$(dirname "$0")/.."

storage=""
surreal_bin=""
while [ $# -gt 0 ]; do
  case "$1" in
    --storage) storage="${2:-}"; shift 2 ;;
    --storage=*) storage="${1#*=}"; shift ;;
    --surreal-bin) surreal_bin="${2:-}"; shift 2 ;;
    --surreal-bin=*) surreal_bin="${1#*=}"; shift ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "local_matrix: unknown argument $1" >&2; exit 2 ;;
  esac
done

cargo=(python3 scripts/agent_cargo.py)
step() {
  echo "=== $*"
  local start=$SECONDS
  "$@"
  echo "--- $(( SECONDS - start ))s: $*"
}

case "$storage" in
  embedded)
    step cargo fmt --all -- --check
    step "${cargo[@]}" clippy --workspace --all-targets --locked -- -D warnings
    step "${cargo[@]}" test --workspace --locked
    step python3 scripts/test_surreal.py --mode embedded --no-fail-fast
    ;;
  server)
    if [ -z "$surreal_bin" ]; then
      surreal_bin="$(command -v surreal || true)"
    fi
    [ -n "$surreal_bin" ] || { echo "local_matrix: no surreal binary (pass --surreal-bin or put surreal on PATH)" >&2; exit 2; }
    step "${cargo[@]}" clippy -p argos-osint-core -p argos-osint-bin --all-targets --locked \
      --no-default-features --features server,nlp -- -D warnings
    step python3 scripts/test_surreal.py --mode server --bin "$surreal_bin" --no-fail-fast
    ;;
  *)
    echo "usage: scripts/local_matrix.sh --storage embedded|server [--surreal-bin PATH]" >&2
    exit 2
    ;;
esac
echo "=== local matrix ($storage) green"
