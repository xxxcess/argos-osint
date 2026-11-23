#!/usr/bin/env bash
# Stage and check the pinned QA checkpoints before publishing them locally.
set -euo pipefail

with_generator=false
if [[ ${1:-} == --with-generator ]]; then
  with_generator=true
  shift
fi
if [[ $# != 1 || -z "$1" ]]; then
  echo "usage: bash tools/assistant-bert/scripts/prepare-model.sh [--with-generator] <new-model-dir>" >&2
  exit 2
fi

umask 077
mkdir -p -- "$(dirname "$1")"
parent_dir=$(cd "$(dirname "$1")" && pwd -P)
model_dir="$parent_dir/$(basename "$1")"
if [[ -e "$model_dir" || -L "$model_dir" ]]; then
  echo "Choose a new QA model directory." >&2
  exit 1
fi
staging=$(mktemp -d "$parent_dir/.assistant-bert-model.XXXXXX")
trap 'rm -rf -- "$staging"' EXIT
model_url=https://huggingface.co/distilbert/distilbert-base-cased-distilled-squad/resolve/564e9b582944a57a3e586bbb98fd6f0a4118db7f
for file in config.json vocab.txt rust_model.ot; do
  curl -fsSL --retry 3 --max-time 300 --max-filesize 261000000 "$model_url/$file" -o "$staging/$file"
done
(
  cd "$staging"
  shasum -a 256 -c <<'CHECKSUMS'
0b5cb15ec08645604ef7085acfaf9c4131158ac22207a76634574cf2771b1515  config.json
eeaa9875b23b04b4c54ef759d03db9d1ba1554838f8fb26c5d96fa551df93d02  vocab.txt
8a9f9b2f153ac9ff230aca4548fa3286be9d2f9ea4eb7e9169665b1a8e983f44  rust_model.ot
CHECKSUMS
)
if "$with_generator"; then
  script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
  python3 -B - "$script_dir/../../../scripts" "$staging/generator" <<'PY'
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, sys.argv[1])
from fleetworks_qa_pack import GENERATOR, GENERATOR_REVISION, MAX_GENERATOR_BYTES, check, sha256

generator = Path(sys.argv[2])
check(sum(size for size, _ in GENERATOR.values()) <= MAX_GENERATOR_BYTES,
      "pinned generator exceeds its size limit")
generator.mkdir(mode=0o700)
model_url = f"https://huggingface.co/google/flan-t5-small/resolve/{GENERATOR_REVISION}"
for relative, (size, digest) in GENERATOR.items():
    path = generator / Path(relative).name
    subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location", "--retry", "3",
                    "--proto", "=https", "--proto-redir", "=https",
                    "--retry-max-time", "300", "--max-time", "300", "--max-filesize", str(size),
                    f"{model_url}/{path.name}", "--output", str(path)], check=True, timeout=320)
    check(path.stat().st_size == size and sha256(path) == digest,
          f"{path.name} does not match the pinned generator")
PY
fi
python3 - "$staging" "$model_dir" <<'PY'
import os
import sys

if os.path.lexists(sys.argv[2]):
    raise SystemExit("QA model destination already exists")
os.rename(sys.argv[1], sys.argv[2])
PY
